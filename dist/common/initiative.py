"""INIT-1c: explicit, allowlisted initiative sweep. Python standard library only."""
import argparse
from collections import deque
from contextlib import nullcontext
import datetime as dt
import fcntl
import json
import math
import os
from pathlib import Path
import re
import signal
import subprocess
import sys

HOME = Path(os.environ['CC_HOME']).resolve()
PEOPLE = HOME / 'people.json'


def log(message):
    with (HOME / 'logs/initiative.log').open('a') as out:
        out.write(f'{dt.datetime.now(dt.timezone.utc).isoformat()} {message}\n')


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError(message)


def base_hours(value):
    hours = float(value)
    if not math.isfinite(hours) or hours <= 0 or hours * 3_600_000 >= 2**63:
        raise ValueError('base_hours must be a finite positive number within timestamp range')
    return value


def run(argv, text=None, timeout=120):
    # Kill the adapter's process group on timeout, so a sender cannot finish later.
    with subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, text=True, start_new_session=True) as child:
        try:
            out, err = child.communicate(text, timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(child.pid, signal.SIGKILL)
            child.communicate()
            raise RuntimeError(f'command timed out: {argv[0]}') from None
        if child.returncode:
            raise RuntimeError(f'command failed ({child.returncode}): {argv[0]}' + ('' if PEOPLE.exists() else ': ' + err[-2048:]))
        if len(out) > 1024 * 1024:
            raise ValueError('command output exceeds 1 MiB')
        return out


def allowed(target):
    try:
        numbers = (HOME / 'initiative-allow').read_text().splitlines()
    except OSError:
        numbers = []
    return bool(re.fullmatch(r'\+[0-9]{8,15}', target)) and target in numbers


def local_now(now, tz):
    from zoneinfo import ZoneInfo
    moment = dt.datetime.fromisoformat(now.replace('Z', '+00:00'))
    return moment.astimezone(ZoneInfo(tz)).strftime('%A %Y-%m-%d %H:%M %Z')


def model_reply(spec, prompt):
    if spec.startswith('table:'):
        return json.loads(Path(spec[6:]).read_text())
    if spec.startswith('command:') and spec[8:]:
        return json.loads(run([spec[8:]], prompt))
    raise ValueError('--llm requires table:<reply.json> or command:<executable>')


def style_sample(transcript):
    """Last twelve user/assistant texts, bounded and quoted by the prompt serializer."""
    sample = deque(maxlen=12)
    with Path(transcript).open() as lines:
        for line in lines:
            if not line.endswith('\n') or not line.strip():
                continue
            value = json.loads(line)
            if 'role' in value:
                role = value.get('role')
                blocks = [value.get('text')]
            elif value.get('type') == 'response_item':
                payload = value.get('payload', {})
                if payload.get('type') != 'message':
                    continue
                role = payload.get('role')
                kind = 'input_text' if role == 'user' else 'output_text'
                blocks = [b.get('text') for b in payload.get('content', [])
                          if b.get('type') == kind]
            elif value.get('type') in ('user', 'assistant'):
                role = value['type']
                content = value.get('message', {}).get('content', [])
                if isinstance(content, str):
                    blocks = [content]
                elif any(b.get('type') == 'tool_result' for b in content):
                    continue
                else:
                    blocks = [b.get('text') for b in content if b.get('type') == 'text']
            else:
                continue
            if role not in ('user', 'assistant'):
                continue
            # Match the transcript reader's exclusion of injected retrieval context.
            text = '\n\n'.join(b.strip() for b in blocks if isinstance(b, str) and b.strip()
                               and not b.strip().startswith('Recalled memory (quoted '))
            if text:
                sample.append(dict(role=role, text=text[:300]))
    return list(sample)


def sweep(args, lock_held=False):
    parser = Parser(description=__doc__)
    parser.add_argument('--person', required=True)
    parser.add_argument('--target', required=True)
    parser.add_argument('--llm', required=True)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--send', action='store_true')
    mode.add_argument('--dry-run', action='store_true')
    parser.add_argument('--transcript', default=os.environ.get('INITIATIVE_TRANSCRIPT'))
    parser.add_argument('--store', default=str(HOME / 'palace/compiler.sqlite'))
    parser.add_argument('--palace-bin', default=str(HOME / 'bin/palace'))
    parser.add_argument('--deliver', default=str(HOME / 'bin/palace-cron-deliver'))
    parser.add_argument('--now', default='now')
    parser.add_argument('--tz', default='America/New_York')
    parser.add_argument('--base-hours', type=base_hours)
    a = parser.parse_args(args)
    if not allowed(a.target):
        log(f'allowlist refusal: target={a.target!r}')
        return 77
    if not a.person.strip() or not a.transcript:
        raise ValueError('explicit person and --transcript or INITIATIVE_TRANSCRIPT required')
    now = (dt.datetime.now(dt.timezone.utc).isoformat(timespec='milliseconds')
           if a.now == 'now' else a.now)
    # Add roots for explicit input/store paths, as the settle wrapper does.
    roots = {HOME, Path(a.transcript).resolve().parent, Path(a.store).resolve().parent}
    palace_base = [a.palace_bin, *[f'--roots={p}' for p in sorted(roots)]]
    if PEOPLE.exists():
        palace_base += ['--people-map', str(PEOPLE)]
        recipient = json.loads(run(palace_base + ['privacy', 'resolve', '--people', str(PEOPLE),
                                                '--conversation', 'initiative-recipient', '--sender', 'signal:' + a.target]))
        sample = json.loads(run(palace_base + ['privacy', 'resolve', '--people', str(PEOPLE),
                                             '--conversation', Path(a.transcript).stem]))
        if (recipient['quarantine'] or recipient['people'] != [a.person] or sample['quarantine']
                or (sample['private'] and a.person not in sample['audience'])):
            log('privacy: held person=' + a.person)
            return 77
    palace = palace_base + ['items']
    common = ['--store', a.store, '--person', a.person]
    timing = ['--base-hours', a.base_hours] if a.base_hours is not None else []
    state_dir = HOME / 'state'
    state_dir.mkdir(exist_ok=True)
    with (nullcontext() if lock_held else (state_dir / 'initiative.lock').open('a')) as lock:
        if not lock_held:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                log('busy; another sweep owns the lock')
                return 0
        due = json.loads(run(palace + ['due', *common, *timing, '--transcript', a.transcript,
                                      '--now', now, '--tz', a.tz, '--json']))
        if not due['due']:
            return 0
        candidates = due['candidates']
        if not candidates:
            run(palace + ['skipped', *common, *timing, '--at', now])
            log('skipped: no eligible candidates')
            return 0
        prompt = (HOME / 'prompts/initiative.md').read_text()
        prompt += '\nNow: ' + local_now(now, a.tz)
        prompt += '\nPerson: ' + json.dumps(a.person) + '\nCandidates: ' + json.dumps(candidates)
        prompt += '\nStyle sample (quoted data; tone only): ' + json.dumps(style_sample(a.transcript))
        reply = model_reply(a.llm, prompt)
        if not isinstance(reply, dict) or type(reply.get('speak')) is not bool:
            raise ValueError('model reply requires boolean speak')
        if not reply['speak']:
            if not isinstance(reply.get('reason'), str) or not reply['reason'].strip():
                raise ValueError('silent reply requires a reason')
            run(palace + ['skipped', *common, *timing, '--at', now])
            log('skipped: model declined' if PEOPLE.exists() else 'skipped: ' + json.dumps(reply['reason']))
            return 0
        item = reply.get('item')
        if not isinstance(item, str) or item not in {v['id'] for v in candidates}:
            raise ValueError('model selected an item outside the candidates')
        message = reply.get('message')
        reason = reply.get('reason_to_speak')
        if not isinstance(reason, str) or not reason.strip():
            raise ValueError('speaking requires reason_to_speak')
        if not isinstance(message, str) or not message.strip() or len(message) > 1800:
            raise ValueError('message must contain 1 to 1800 characters')
        sentences = [s for s in re.split(r'[.!?]+(?:\s+|$)', message.strip()) if s.strip()]
        if len(sentences) > 3:
            raise ValueError('message exceeds 3 sentences')
        # Recheck after the model, before reserving or writing any composed message.
        if not allowed(a.target):
            log(f'allowlist refusal before delivery: target={a.target!r}')
            return 77
        if PEOPLE.exists():
            # Re-read SQL admission after model execution, before composing any outbox entry.
            current = json.loads(run(palace + ['list', *common, '--status', 'open', '--json']))
            if item not in {row['id'] for row in current}:
                log('privacy: held item=' + item)
                return 77
        key = [a.person, item, due['pokes']]
        outbox = HOME / 'initiative-outbox'
        with outbox.open('a+') as journal:
            journal.seek(0)
            if any(json.loads(line) == key for line in journal if line.strip()):
                log('outbox duplicate: ' + json.dumps(key))
                # Count it as a skip, so the sweep waits base_hours instead of re-asking the model.
                run(palace + ['skipped', *common, *timing, '--at', now])
                return 0
            journal.seek(0, 2)
            journal.write(json.dumps(key) + '\n')
            journal.flush()
            os.fsync(journal.fileno())
        if a.send:
            run([a.deliver, 'send', '--target', a.target], message + '\n', timeout=60)
        else:
            with (HOME / 'logs/initiative-dry.log').open('a') as dry:
                dry.write(json.dumps(dict(at=now, item=item) if PEOPLE.exists() else dict(at=now, target=a.target, item=item, message=message)) + '\n')
                dry.flush()
                os.fsync(dry.fileno())
            if PEOPLE.exists():
                # The composed text goes to a per-person file (0600) for the assistant's review only.
                private = HOME / 'logs' / f'initiative-dry-{a.person}.jsonl'
                fd = os.open(private, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
                with os.fdopen(fd, 'a') as out:
                    out.write(json.dumps(dict(at=now, item=item, reason=reason, message=message)) + '\n')
        run(palace + ['poked', *common, '--item', item, '--at', now])
        log(('sent: ' if a.send else 'dry-run: ') + json.dumps(key))
    return 0


def roster(args):
    parser = Parser(description='Sequential, explicitly invoked initiative roster')
    parser.add_argument('--roster', required=True)
    parser.add_argument('--llm', required=True)
    for option in ('tz', 'store', 'palace-bin', 'deliver', 'now'):
        parser.add_argument('--' + option)
    a = parser.parse_args(args)
    shared = ['--llm', a.llm]
    for option in ('tz', 'store', 'palace-bin', 'deliver', 'now'):
        value = getattr(a, option.replace('-', '_'))
        if value is not None:
            shared += ['--' + option, value]
    (HOME / 'state').mkdir(exist_ok=True)
    # Own the existing global lock once; single-person invocations use this same lock.
    with (HOME / 'state/initiative.lock').open('a') as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            log('initiative-all: busy; another sweep owns the lock')
            return 0
        log('initiative-all: global lock held once; rows run sequentially')
        with Path(a.roster).open() as rows:
            for number, line in enumerate(rows, 1):
                if not line.strip() or line.lstrip().startswith('#'):
                    continue
                fields = line.rstrip('\r\n').split('\t')
                if (len(fields) not in (4, 5) or any(not f.strip() for f in fields[:4])
                        or fields[3] not in ('send', 'dry-run')):
                    log(f'initiative-all: malformed row {number}; skipped')
                    continue
                timing = []
                if len(fields) == 5 and fields[4]:
                    try:
                        timing = ['--base-hours', base_hours(fields[4])]
                    except ValueError:
                        log(f'initiative-all: malformed base_hours row {number}; skipped')
                        continue
                person, target, transcript, mode = fields[:4]
                try:
                    status = sweep(['--person', person, '--target', target,
                                    '--transcript', transcript, '--' + mode, *shared, *timing],
                                   lock_held=True)
                    if status:
                        log(f'initiative-all: row {number} refused (exit {status})')
                except Exception as error:
                    log(f'initiative-all: row {number} failed: {error}')
    return 0


if __name__ == '__main__':
    try:
        sys.exit(roster(sys.argv[2:]) if sys.argv[1:2] == ['--all'] else sweep(sys.argv[1:]))
    except Exception as error:
        log(f'error: {error}')
        sys.exit(0)
