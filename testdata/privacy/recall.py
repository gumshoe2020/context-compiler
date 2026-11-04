#!/usr/bin/env python3
"""PRIV-1c A: live-sized marker set through the actual MCP adapter and context path."""
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys

ROOT, WORK, BIN = (Path(p).resolve() for p in sys.argv[1:4])
assert WORK.is_relative_to(ROOT / 'target/priv1c')
WORK.mkdir(parents=True, exist_ok=True)
DIARY = WORK / 'diary'
DIARY.mkdir(exist_ok=True)
STORE, KG = WORK / 'compiler.sqlite', WORK / 'kg.sqlite'
PEOPLE = WORK / 'people.json'
PEOPLE.write_text(json.dumps(dict(people=dict(ann=dict(handles=['signal:+12025550100']),
    wren=dict(handles=['signal:+12025550101'], private=True)), conversations=[])))
with sqlite3.connect(STORE) as c:
    c.executescript((ROOT / 'crates/palace-store/src/schema.sql').read_text())
    c.executescript('''CREATE TABLE privacy_markers(kind TEXT,id TEXT,source TEXT,people TEXT,audience TEXT,private INTEGER,quarantine INTEGER,PRIMARY KEY(kind,id,source));
    CREATE TABLE privacy_retired(kind TEXT,id TEXT,source TEXT);''')
    for i in range(910):
        c.execute("INSERT INTO privacy_markers VALUES ('entry',?,'synthetic','[]','[]',0,0)", (f'2032-01-01-{i:012x}',))
    c.execute("INSERT INTO privacy_markers VALUES ('entry','2032-01-02-aaaaaaaaaaaa','synthetic-private','[\"wren\"]','[\"wren\"]',1,0)")
with sqlite3.connect(KG) as c:
    c.executescript('''CREATE TABLE entities(id TEXT,name TEXT,properties TEXT);
    CREATE TABLE triples(id TEXT,subject TEXT,predicate TEXT,object TEXT,source_closet TEXT,valid_to TEXT);''')

def hit(name, text, distance):
    file = DIARY / (name + '.md')
    file.write_text('# Invented drawer\n' + text)
    return dict(text=file.read_text(), source_file=file.name, source_path=str(file),
                wing='openclaw', room='general', distance=distance, effective_distance=distance,
                closet_boost=0., matched_via='drawer', similarity=1-distance,
                created_at='2032-01-03T12:00:00')
rows = [hit('2032-01-02-aaaaaaaaaaaa', 'wren secret cobalt sparrow', .05),
        hit('2032-01-03-bbbbbbbbbbbb', 'unmarked hidden copper gull', .06),
        hit('2032-01-01-000000000001', 'shared silver kite', .1),
        hit('2032-01-01-000000000002', 'shared amber finch', .2)]
(WORK / 'response.json').write_text(json.dumps(dict(results=rows)))
serial = checks = 0

def check(ok, label):
    global checks
    assert ok, label
    checks += 1
    print('PASS', label, flush=True)

def context(extra=(), layered=True):
    global serial
    serial += 1
    (WORK / 'requests.jsonl').write_text('')
    args = [str(BIN), '--roots=' + str(ROOT), '--people-map', str(PEOPLE),
        'context', '--message', 'Remember the invented silver kite plans?', '--store', str(STORE),
        '--kg', str(KG), '--palace', str(WORK), '--mempalace-mcp', str(ROOT / 'testdata/privacy/recall_mcp.py'),
        '--timeout-ms', '2000', '--strict', '--json']
    if layered:
        args += ['--layered', '--prompts', str(ROOT / 'prompts')]
    result = subprocess.run(args + list(extra), capture_output=True, text=True,
        env=dict(os.environ, PRIV1C_RECALL=str(WORK), HOME=str(WORK), TMPDIR=str(WORK)), timeout=6)
    (WORK / f'{serial}.stdout').write_text(result.stdout)
    (WORK / f'{serial}.stderr').write_text(result.stderr)
    assert result.returncode == 0, f'context failed: {WORK}/{serial}.stderr'
    return json.loads(result.stdout)

for extra in [[], ['--conversation', 'agent:main:signal:direct:+12025550100'],
              ['--readers', 'ann'], ['--conversation', 'unknown:synthetic']]:
    out = context(extra)
    check('shared silver kite' in out['text'] and '[NEWER]' in out['text'], '910 shared markers retain layered recall ' + str(extra))
    check('cobalt sparrow' not in json.dumps(out) and 'copper gull' not in json.dumps(out), 'private and unmarked hits excluded')
    check(len((WORK / 'requests.jsonl').read_text().splitlines()) == 1, 'one MCP search rather than 910')
private = context(['--readers', 'wren'])
check('cobalt sparrow' in private['text'] and 'copper gull' not in json.dumps(private), 'audience sees private drawer, never unmarked')
group = context(['--readers', 'ann,wren'])
check('cobalt sparrow' not in json.dumps(group), 'group requires every reader in audience')
plain = context(['--readers', 'ann'], layered=False)
check('shared silver kite' in plain['text'] and 'cobalt sparrow' not in json.dumps(plain), 'plain recall uses same admission')
# A full window of hidden hits must not crowd all admitted hits out of recall.
(WORK / 'response.json').write_text(json.dumps(dict(results=[rows[1]] * 100 + rows[2:])))
overflow = context(['--readers', 'ann'])
check('shared silver kite' in overflow['text'] and 'copper gull' not in json.dumps(overflow), 'overflow recovers admitted hits and retains them at deadline')
print(f'PRIV-1c recall: {checks} checks PASS')
