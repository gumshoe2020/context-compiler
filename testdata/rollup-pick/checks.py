"""Executable acceptance checks shared by cargo integration and the dist entry point."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sqlite3
import sys

root, work, binary = map(Path, sys.argv[1:])
work.mkdir(parents=True, exist_ok=True)
checks = 0


def check(ok, label):
    global checks
    assert ok, label
    checks += 1
    print('ok ' + label, flush=True)


def scenario(name, tags, extra=(), env_extra=None):
    base = work / name
    if base.exists():
        shutil.rmtree(base)
    diary = base / 'root' / 'diary'
    diary.mkdir(parents=True)
    (base / 'palace').mkdir()
    for script in ('model.py', 'mcp.py'):
        shutil.copyfile(root / 'testdata/rollup-pick' / script, base / script)
        (base / script).chmod(0o755)
    names = []
    for i, tag in enumerate(tags):
        filename = f'2031-05-10-{i:012x}'
        names.append(filename)
        (diary / (filename + '.md')).write_text(
            f'# Observatory record {i}\n\nTags: {tag}.\n\n'
            f'Elira Fen chose copper panels for the observatory dome, recorded as panel {i}.\n')
    # This legacy file-only fixture explicitly declares its source entries shared.
    with sqlite3.connect(base / 'palace/compiler.sqlite') as c:
        c.executescript("CREATE TABLE privacy_markers(kind TEXT,id TEXT,source TEXT,people TEXT,audience TEXT,private INTEGER,quarantine INTEGER,PRIMARY KEY(kind,id,source)); CREATE TABLE privacy_retired(kind TEXT,id TEXT,source TEXT,PRIMARY KEY(kind,id,source));")
        c.executemany("INSERT INTO privacy_markers VALUES ('entry',?,'synthetic:rollup-fixture','[]','[]',0,0)", [(n,) for n in names])
    body = '# ROLLUP: dome-plan — state as of 2031-05-10\n\n## Claims\n'
    for i, (filename, tag) in enumerate(zip(names, tags)):
        if tag == 'dome-plan':
            body += f'- C{i+1}. Elira Fen | chose | copper panels — 2031-05-10 — {filename}.md\n'
    body += '\n## Current consolidated truth\n' + (
        'Elira Fen chose copper panels for the observatory dome. The panel records preserve '
        'the choice and keep the work together as a single dome plan. ' * 14)
    body += '\n\n## Supersedes\n' + ''.join(
        f'- {n}.md\n' for n, t in zip(names, tags) if t == 'dome-plan')
    relation = {'from': 'Elira Fen', 'to': 'copper panels', 'type': 'chose',
                'evidence': [{'query': 'copper panels'}], 'confidence': 0.85,
                'supplied_by': 'palace-distill', 'claim': next(
                    (f'C{i+1}' for i, t in enumerate(tags) if t == 'dome-plan'), 'C1')}
    body += '\n## Open items\n- None.\n\n## Graph relations (proposed)\n```json relations\n'
    body += json.dumps([relation]) + '\n```\n'
    (base / 'body.md').write_text(body)
    env = dict(os.environ, PICK_WORK=str(base), EXCLUDE_WINGS='hidden')
    env.pop('ROLLUP_MIN_CLUSTER', None)
    env.update(env_extra or {})
    cmd = [str(binary), '--roots=' + str(root), '--person', 'orin', '--assistant', 'finch', 'rollup', '--diary', str(base / 'root'),
           '--palace', str(base / 'palace'), '--kg', str(base / 'palace/kg.sqlite'),
           '--as-of', '2031-05-10', '--llm', 'command:' + str(base / 'model.py'),
           '--mempalace', str(base / 'mcp.py'), '--mempalace-mcp', str(base / 'mcp.py'),
           '--prompts', str(root / 'prompts'), '--canaries', str(base / 'canaries.json'),
           '--keep', str(base / 'kept'), '--dry-run'] + list(extra)
    (base / 'canaries.json').write_text('[]')
    with (base / 'stdout.log').open('w') as out, (base / 'stderr.log').open('w') as err:
        result = subprocess.run(cmd, cwd=root, env=env, stdout=out, stderr=err)
    stdout = (base / 'stdout.log').read_text()
    stderr = (base / 'stderr.log').read_text()
    check(result.returncode == 0, name + ' exit zero: ' + stderr[-500:])
    check(not (base / 'kept').exists(), name + ' no kept failures')
    return base, stdout, stderr


base, out, err = scenario('mixed', ['a-decoy'] * 4 + ['dome-plan'] * 3)
check('thread=missing-thread cluster=0 cluster empty' in out, 'invented thread skipped')
check('"passed": true' in out, 'next eligible thread validates')
calls = (base / 'model.log').read_text()
check(calls.count('=== END CALL ===') == 3, 'two selections and exactly one writer call')
check('0 entries' not in calls, 'writer never sees empty cluster')
searches = [json.loads(line) for line in (base / 'search.log').read_text().splitlines()]
check(all(row.get('wing') == 'openclaw' for row in searches if row['query'] != 'copper panels'),
      'every cluster search uses selected wing')

base, out, err = scenario('eligible', ['dome-plan'] * 3)
check('"passed": true' in out, 'eligible thread validates directly')
check((base / 'model.log').read_text().count('=== END CALL ===') == 2, 'eligible selection and writer unchanged')
check((base / 'body.md').read_text().strip() in out, 'eligible body preserved exactly')

base, out, err = scenario('none', ['a-decoy'] * 3)
check('no rollup due' in out, 'none eligible is clean no rollup due')
check((base / 'model.log').read_text().count('=== END CALL ===') == 1, 'none eligible never calls writer')

base, out, err = scenario('generic-tag', ['decision'] * 3)
check('thread=decision cluster=0 cluster empty' in out, 'matching generic tag yields empty cluster and is skipped')
check((base / 'model.log').read_text().count('=== END CALL ===') == 1, 'generic tag never reaches writer')
base, out, err = scenario('excluded-hits', ['a-decoy'] * 3, ['--thread', 'hidden-observatory'])
check('cluster=0 cluster empty' in out, 'excluded-wing search hits cannot make a cluster')
check(not (base / 'model.log').exists(), 'explicit empty thread calls no model')

base, out, err = scenario('small', ['dome-plan'] * 2 + ['other'] )
check('too small (minimum 3)' in out and 'no rollup due' in out, 'two-entry cluster skipped')
base, out, err = scenario('excluded', ['dome-plan'] * 3, ['--wing', 'hidden'])
check('in an excluded wing hidden' in out, 'excluded wing skipped')
check(not (base / 'model.log').exists(), 'excluded wing has no model call')
base, out, err = scenario('minimum', ['dome-plan'] * 3, env_extra={'ROLLUP_MIN_CLUSTER': '4'})
check('too small (minimum 4)' in out, 'environment minimum honored')
base, out, err = scenario('override', ['dome-plan'] * 3, ['--min-cluster', '3'], {'ROLLUP_MIN_CLUSTER': '4'})
check('"passed": true' in out, 'explicit minimum overrides environment')

# Run the actual cron driver with real rollup selection; all other stages are inert.
# Send/page executables write only inside this synthetic scenario if ever called.
base, out, err = scenario('cron-none', ['a-decoy'] * 3)
(base / 'model.log').unlink()
(base / 'palace/kg.sqlite').touch()
wrapper = base / 'palace-wrapper.py'
wrapper.write_text('''#!/usr/bin/env python3
import os
import sys
from pathlib import Path
base = Path(os.environ['PICK_WORK'])
args = sys.argv[1:]
if 'rollup' in args:
    args += ['--dry-run', '--as-of', '2031-05-10', '--mempalace', str(base / 'mcp.py'),
             '--mempalace-mcp', str(base / 'mcp.py')]
    names = ['--person', 'orin', '--assistant', 'finch']
    os.execv(os.environ['PICK_BINARY'], [os.environ['PICK_BINARY']] + names + args)
sys.exit(0)
''')
trap = base / 'delivery-trap.py'
trap.write_text('''#!/usr/bin/env python3
import os
from pathlib import Path
import sys
with (Path(os.environ['PICK_WORK']) / 'delivery-called').open('a') as log:
    log.write(sys.stdin.read())
''')
for script in (wrapper, trap):
    script.chmod(0o755)
env = dict(os.environ, PICK_WORK=str(base), PICK_BINARY=str(binary), EXCLUDE_WINGS='hidden')
env.pop('ROLLUP_MIN_CLUSTER', None)
cmd = ['/bin/sh', str(root / 'scripts/palace-cron.sh'), '--palace', str(base / 'palace'),
       '--diary-root', str(base / 'root'), '--kg', str(base / 'palace/kg.sqlite'),
       '--palace-bin', str(wrapper), '--send', str(trap), '--page', str(trap),
       '--state', str(base / 'state'), '--rollup', '--llm', 'command:' + str(base / 'model.py'),
       '--canaries', str(base / 'canaries.json'), '--no-deliver-stages']
with (base / 'cron.stdout.log').open('w') as out, (base / 'cron.stderr.log').open('w') as err:
    result = subprocess.run(cmd, cwd=root, env=env, stdout=out, stderr=err)
check(result.returncode == 0, 'actual cron exits zero when none eligible')
check('CRON QUIET' in (base / 'cron.stdout.log').read_text(), 'actual cron ends quietly')
check('no rollup due' in (base / 'state/last/rollup.out').read_text(), 'cron receives clean rollup skip')
check(not (base / 'delivery-called').exists(), 'actual cron never pages or sends')
check(not (base / 'state/rollup-attempts').exists(), 'actual cron keeps no attempts')
check((base / 'model.log').read_text().count('=== END CALL ===') == 1, 'cron calls selector only')
print(f'ROLLUP-PICK PASS ({checks} checks)')
