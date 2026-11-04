#!/usr/bin/env python3
"""PRIV-1b: synthetic legacy stores, no model, sender, or external data."""
from collections import Counter, defaultdict
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys

ROOT, WORK, BIN = (Path(p).resolve() for p in sys.argv[1:4])
assert WORK.is_relative_to(ROOT / 'target/priv1b')
WORK.mkdir(parents=True, exist_ok=True)
STORE, KG = WORK / 'compiler.sqlite', WORK / 'kg.sqlite'
PEOPLE, WINGS = WORK / 'people.json', WORK / 'wing-map.json'
PEOPLE.write_text(json.dumps({
    'people': {'ann': {'handles': ['signal:+12025550100']},
               'wren': {'handles': ['signal:+12025550101'], 'private': True},
               'iris': {'handles': []}},
    'conversations': [
        {'match': '/invented/seed/', 'prefix': True, 'people': ['ann']},
        {'match': '/invented/seed/private-', 'prefix': True, 'people': ['wren']},
        {'match': '/invented/seed/private-exact', 'people': ['iris']},
        {'match': 'rollout-', 'prefix': True, 'people': ['ann']},
        {'match': 'cron:private', 'people': ['wren'], 'private': True}],
    'private_wings': {'orchard': ['iris']}}))
WINGS.write_text(json.dumps({'cron:private': 'grove',
                            'agent:main:signal:direct:+12025550101': 'shelter'}))
roots = [WORK / 'diary-root', WORK / 'wings/grove', WORK / 'wings/orchard', WORK / 'wings/shelter']
for root in roots:
    (root / 'diary').mkdir(parents=True)
for root, names in zip(roots, [['hand-shared', 'precedence', 'diary-first', 'hand-linked', 'missing-wing'], ['hand-private'], ['explicit-private'], ['handle-private']]):
    for name in names:
        (root / 'diary' / (name + '.md')).write_text('# Invented legacy note\nInvented kite task.\n' + ('Transcript: /invented/transcripts/openclaw/logged.jsonl:2\n' if name == 'hand-linked' else ''))
with sqlite3.connect(STORE) as c:
    c.executescript((ROOT / 'crates/palace-store/src/schema.sql').read_text())
    c.execute('ALTER TABLE claims ADD COLUMN transcript TEXT')
    event = dict(operation='distill_commit', session='legacy:old', wing='grove', files=['precedence.md'], copy='/invented/transcripts/openclaw/logged.jsonl')
    payload = json.dumps(event)
    digest = hashlib.sha256(''.join(f'{len(p)}:{p}' for p in ['1', '', payload]).encode()).hexdigest()
    c.execute('INSERT INTO write_log(payload,previous_hash,hash) VALUES (?,?,?)', (payload, '', digest))
    payload = json.dumps(dict(operation='distill_commit', session='legacy:unresolved', files=['missing-wing.md']))
    previous = digest
    digest = hashlib.sha256(''.join(f'{len(p)}:{p}' for p in ['2', previous, payload]).encode()).hexdigest()
    c.execute('INSERT INTO write_log(payload,previous_hash,hash) VALUES (?,?,?)', (payload, previous, digest))
    claims = [('c-shared', 'hand-shared.md', 'hand-shared.md'),
              ('c-private', 'hand-private.md', 'hand-private.md'),
              ('c-explicit', 'explicit-private.md', ''),
              ('c-log', 'precedence.md', '/invented/transcripts/openclaw/logged.jsonl:1'),
              ('c-diary-first', 'diary-first.md', '/invented/transcripts/grove/unlogged.jsonl:1'),
              ('c-derived', 'derived.md', 'hand-private.md'),
              ('c-unknown', 'unknown.md', '')]
    for cid, entry, source in claims:
        c.execute('INSERT INTO claims(id,number,rollup,subject,predicate,object,date,source,confidence_base,first_seen) VALUES (?,?,?,?,?,?,?,?,?,?)',
                  (cid, 'C1', entry, 'Invented Finch', 'uses', 'silver kite', '2032-01-01', source, .8, '2032-01-01'))
    # Fully synthetic minimal legacy item rows, migrated by the real Store on application.
    c.execute("CREATE TABLE open_items(id TEXT PRIMARY KEY, person TEXT NOT NULL, owner TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('task','topic')), title TEXT NOT NULL, due TEXT, status TEXT NOT NULL DEFAULT 'open', opened_at TEXT NOT NULL, touched_at TEXT NOT NULL, closed_at TEXT, source TEXT NOT NULL, quote TEXT NOT NULL, close_quote TEXT, times_raised INTEGER NOT NULL DEFAULT 0)")
    for name, source in [('prefix', '/invented/seed/file.jsonl'), ('longest', '/invented/seed/private-file'),
                         ('exact', '/invented/seed/private-exact'), ('rollout', 'rollout-synthetic'),
                         ('handle', 'agent:main:signal:direct:+12025550100'), ('unknown', ''),
                         ('wing-shared', '/invented/transcripts/openclaw/manual.jsonl:1'),
                         ('wing-private', '/invented/transcripts/grove/manual.jsonl:1'),
                         ('derived', 'hand-private.md')]:
        c.execute('INSERT INTO open_items(id,person,owner,kind,title,opened_at,touched_at,source,quote) VALUES (?,?,?,?,?,?,?,?,?)',
                  (name, 'ann', 'person', 'task', 'Invented task', '2032-01-01', '2032-01-01', source, 'Invented quote'))
    c.execute("INSERT INTO asks(finding,claim,kind,token,line,asked_at) VALUES ('a-private','c-derived','check','synthetic','Invented question','2032-01-01')")
    c.execute("INSERT INTO fidelity(point,file,type,sampled,supported,missing,contradicted,unjudged,judge,items,at) VALUES ('synthetic','hand-private.md','specific',1,1,0,0,0,'table','[]','2032-01-01')")
with sqlite3.connect(KG) as c:
    c.execute('CREATE TABLE triples(id TEXT PRIMARY KEY,source_closet TEXT)')
    for name, closet in [('t-private', {'claim_id': 'c-derived'}), ('t-entry', {'transcript': 'hand-private.md'}), ('t-fallback', {'transcript': '', 'claim_id': 'c-derived'}), ('t-unknown', {})]:
        c.execute('INSERT INTO triples VALUES (?,?)', (name, json.dumps(closet)))
    c.execute('INSERT INTO triples VALUES (?,?)', ('t-cited', '{}'))
with sqlite3.connect(STORE) as c:
    c.execute("INSERT INTO citations VALUES ('t-cited','c-derived','derived.md','2032-01-01')")
serial = checks = 0

def check(ok, label):
    global checks
    assert ok, label
    checks += 1
    print('ok', label)

def run(extra=()):
    global serial
    serial += 1
    args = [str(BIN), '--roots=' + str(ROOT), 'privacy', 'backfill', '--store', str(STORE), '--kg', str(KG), '--people', str(PEOPLE), '--wing-map', str(WINGS)]
    for root in roots:
        args += ['--diary-root', str(root)]
    result = subprocess.run(args + list(extra), capture_output=True, text=True,
                            env=dict(os.environ, HOME=str(WORK), TMPDIR=str(WORK)), timeout=20)
    (WORK / f'{serial}.stdout').write_text(result.stdout)
    (WORK / f'{serial}.stderr').write_text(result.stderr)
    assert result.returncode == 0, (serial, result.stderr[-1000:])
    return [json.loads(line) for line in result.stdout.splitlines()], json.loads(result.stderr)

fingerprint = lambda: [hashlib.sha256(p.read_bytes()).hexdigest() for p in (STORE, KG)]
before = fingerprint()
rows, report = run(['--dry-run'])
check(before == fingerprint(), 'dry run leaves both databases byte-identical')
row = lambda kind, name: next(r for r in rows if r['kind'] == kind and r['id'] == name)
for name, audience in [('hand-private', ['wren']), ('explicit-private', ['iris']), ('handle-private', ['wren']), ('precedence', ['wren']), ('hand-linked', ['wren'])]:
    r = row('entry', name)
    check(r['marker']['private'] and r['marker']['audience'] == audience, name + ' private to configured audience')
check(row('entry', 'missing-wing')['marker']['source'] == 'legacy:openclaw', 'write log without wing cannot suppress known diary wing')
check(row('entry', 'hand-shared')['marker'] == dict(source='legacy:openclaw', people=[], audience=[], private=False, quarantine=False), 'hand-filed shared wing uses legacy marker')
check(not row('claim', 'c-diary-first')['marker']['private'], 'diary location precedes source transcript directory')
for kind, name in [('claim', 'c-private'), ('claim', 'c-derived'), ('item', 'derived'), ('triple', 't-private'), ('triple', 't-entry'), ('triple', 't-fallback'), ('ask', 'a-private'), ('fidelity', 'hand-private.md')]:
    r = row(kind, name)
    check(r['marker']['private'] and r['marker']['audience'] == ['wren'], kind + '/' + name + ' inherits private audience')
check(row('item', 'prefix')['marker']['people'] == ['ann'] and row('item', 'prefix')['reason'] == 'prefix', 'file prefix resolves person')
check(row('item', 'longest')['marker']['people'] == ['wren'], 'longest prefix wins')
check(row('item', 'exact')['marker']['people'] == ['iris'] and row('item', 'exact')['reason'] == 'exact', 'exact row beats prefix')
check(row('item', 'handle')['reason'] == 'handle', 'handle reason recorded')
check(row('item', 'rollout')['marker']['people'] == ['ann'], 'rollout prefix resolves person')
check(row('item', 'wing-shared')['marker']['source'] == 'legacy:openclaw', 'source transcript shared wing fallback')
check(row('item', 'wing-private')['marker']['audience'] == ['wren'], 'source transcript private wing fallback')
for kind, name in [('claim', 'c-unknown'), ('item', 'unknown'), ('triple', 't-unknown')]:
    check(row(kind, name)['marker']['quarantine'] and row(kind, name)['reason'] == 'unknown', kind + ' empty source without wing quarantines')
check(any(r['id'] == 't-cited' and r['marker']['audience'] == ['wren'] for r in rows), 'citations retain private support')
counts = defaultdict(Counter)
for r in rows:
    counts[r['kind']][r['reason']] += 1
check(report['privacy_backfill_counts'] == dict(counts), 'reported per-kind reason counts match all support rows')
check(report['privacy_backfill_counts']['item'] == {'prefix': 3, 'exact': 1, 'handle': 1, 'unknown': 1, 'wing:openclaw': 1, 'wing:grove': 2}, 'item reason counts match independently specified expected counts')
run()
with sqlite3.connect(STORE) as c:
    first = c.execute('SELECT * FROM privacy_markers ORDER BY kind,id,source').fetchall()
    sources = c.execute('SELECT * FROM privacy_sources ORDER BY locator').fetchall()
    check(json.loads(c.execute("SELECT marker FROM privacy_sources WHERE locator='hand-private.md'").fetchone()[0])['audience'] == ['wren'], 'entry filename locator registered for later inheritance')
run()
with sqlite3.connect(STORE) as c:
    check(first == c.execute('SELECT * FROM privacy_markers ORDER BY kind,id,source').fetchall(), 'repeat apply leaves markers identical')
    check(sources == c.execute('SELECT * FROM privacy_sources ORDER BY locator').fetchall(), 'repeat apply leaves sources identical')
# Changing policy later must not widen an already private decision.
policy = json.loads(PEOPLE.read_text())
policy['conversations'][4]['private'] = False
policy['people']['wren']['private'] = False
PEOPLE.write_text(json.dumps(policy))
run()
with sqlite3.connect(STORE) as c:
    check(first == c.execute('SELECT * FROM privacy_markers ORDER BY kind,id,source').fetchall(), 'changed policy does not overwrite existing markers')
    check(sources == c.execute('SELECT * FROM privacy_sources ORDER BY locator').fetchall(), 'changed policy does not overwrite existing entry sources')
print(f'PRIV-1b PASS {checks} checks')
