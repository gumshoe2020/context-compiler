#!/usr/bin/env python3
"""KG-EV1: invented evidence through real settle, with file-only adapters."""
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys

ROOT, WORK, BIN = (Path(p).resolve() for p in sys.argv[1:4])
assert WORK.is_relative_to(ROOT / 'target/kgev1')
WORK.mkdir(parents=True, exist_ok=True)
CTX = WORK / 'ctx'
for name in ['bin', 'scripts', 'palace', 'diary-root/diary', 'wings', 'transcripts', 'state', 'logs']:
    (CTX / name).mkdir(parents=True, exist_ok=True)
STORE, KG = CTX / 'palace/compiler.sqlite', CTX / 'palace/knowledge_graph.sqlite3'
(CTX / 'people.json').write_text(json.dumps(dict(people=dict(ann=dict(handles=[])),
    conversations=[dict(match='invented-evidence', people=['ann'])])))
with sqlite3.connect(KG) as c:
    c.executescript("""CREATE TABLE entities(id TEXT PRIMARY KEY,name TEXT,type TEXT DEFAULT 'unknown',properties TEXT DEFAULT '{}',created_at TEXT DEFAULT CURRENT_TIMESTAMP);
    CREATE TABLE triples(id TEXT PRIMARY KEY,subject TEXT,predicate TEXT,object TEXT,valid_from TEXT,valid_to TEXT,confidence REAL,source_closet TEXT,source_file TEXT,source_drawer_id TEXT,adapter_name TEXT,extracted_at TEXT DEFAULT CURRENT_TIMESTAMP);""")
    c.execute('INSERT INTO entities(id,name,properties) VALUES (?,?,?)',
              ('violet_glider', 'violet glider', json.dumps(dict(aliases=['VG']))))
shutil.copy(BIN, CTX / 'bin/palace')
shutil.copy(ROOT / 'dist/common/settle.sh', CTX / 'bin/settle.sh')
shutil.copytree(ROOT / 'prompts', CTX / 'prompts', dirs_exist_ok=True)
for name in ['mempalace', 'mempalace-mcp']:
    shutil.copy(ROOT / 'testdata/privacy/memory.py', CTX / 'bin' / name)
(CTX / 'scripts/mempalace-check.sh').write_text(next(line for line in (ROOT/'scripts/mempalace-check.sh').read_text().splitlines() if line.startswith('PIN=')) + '\n')
(CTX / 'env.sh').write_text(f'MEMPALACE_BIN_DIR="{CTX}/bin"\nWING=openclaw\nPERSON=ann\nASSISTANT=pip\nSETTLE_DEBOUNCE_SECONDS=0\nDISTILL_MIN_TURNS=1\n')
transcript = CTX / 'transcripts/invented-evidence.jsonl'
phrase = 'Invented Tern uses violet glider and has considered amber raft.'
transcript.write_text(json.dumps(dict(role='user', ts='2032-01-01T12:00:00Z', text=phrase)) + '\n')
(CTX / 'state/settle.request').write_text('openclaw\t' + str(transcript) + '\n')
reply = WORK / 'reply.json'
reply.write_text(json.dumps(dict(entries=[
    dict(title='Invented Tern planning', body='Invented Tern made a plan.', tags=[]),
    dict(title='Invented vehicle choice', body='Synthetic filler. ' * 100 + 'INVENTED-TERN uses VG.', tags=[])],
    claims=[], relations=[
        dict(subject='Invented Tern', predicate='uses', object='violet glider', confidence=.9, line=1),
        dict(subject='Invented Tern', predicate='considered', object='amber raft', confidence=.9, line=1)])))
env = dict(os.environ, HOME=str(WORK), TMPDIR=str(WORK), PRIV1_DIARY=str(CTX / 'diary-root/diary'), PRIV1_REPLY=str(reply))
result = subprocess.run(['sh', str(CTX / 'bin/settle.sh'), '--llm', 'command:' + str(ROOT / 'testdata/privacy/model.py'), '--', str(ROOT / 'testdata/privacy/cron.py')],
                        capture_output=True, text=True, env=env, timeout=60)
(WORK / 'stdout.log').write_text(result.stdout)
(WORK / 'stderr.log').write_text(result.stderr)
assert result.returncode == 0 and 'SETTLE RUN exit=0' in result.stdout
assert 'SETTLE relation_skipped_no_evidence=1' in result.stdout
with sqlite3.connect(KG) as c:
    rows = c.execute('SELECT id,source_file,source_closet FROM triples').fetchall()
assert len(rows) == 1, rows
tid, file, closet = rows[0]
assert 'INVENTED-TERN uses VG.' in (CTX / 'diary-root/diary' / file).read_text()
assert json.loads(closet)['evidence_entry'] == file
assert json.loads(closet)['evidence_refs'] == [file]
with sqlite3.connect(STORE) as c:
    steps = [json.loads(r[0]) for r in c.execute('SELECT payload FROM write_log')]
    supports = c.execute("SELECT private,quarantine FROM privacy_markers WHERE kind='triple' AND id=?", (tid,)).fetchall()
assert supports == [(0, 0)], supports
skips = [s for s in steps if s.get('outcome') == 'relation_skipped_no_evidence']
assert len(skips) == 1 and skips[0]['index'] == 1
assert next(s for s in steps if s.get('step') == 'propose')['relation_skipped_no_evidence'] == 1
assert set(skips[0]) == {'operation', 'run', 'step', 'index', 'outcome', 'triple_id'}
assert 'amber raft' not in (CTX / 'logs/settle-distill.log').read_text()
# A later run has no passing new entry: the gate must find the old one by search.
reply.write_text(json.dumps(dict(entries=[dict(title='Invented unrelated plan', body='Invented Petrel made a plan.', tags=[])],
    claims=[], relations=[dict(subject='Invented Tern', predicate='uses', object='violet glider', confidence=.9, line=2)])))
with transcript.open('a') as out:
    out.write(json.dumps(dict(role='user', ts='2032-01-01T13:00:00Z', text=phrase)) + '\n')
(CTX / 'state/settle.request').write_text('openclaw\t' + str(transcript) + '\n')
result = subprocess.run(['sh', str(CTX / 'bin/settle.sh'), '--llm', 'command:' + str(ROOT / 'testdata/privacy/model.py'), '--', str(ROOT / 'testdata/privacy/cron.py')],
                        capture_output=True, text=True, env=env, timeout=60)
(WORK / 'search-stdout.log').write_text(result.stdout)
(WORK / 'search-stderr.log').write_text(result.stderr)
assert result.returncode == 0 and 'SETTLE relation_skipped_no_evidence=0' in result.stdout
with sqlite3.connect(STORE) as c:
    steps = [json.loads(r[0]) for r in c.execute('SELECT payload FROM write_log')]
assert len([s for s in steps if s.get('outcome') == 'cited']) == 1
assert len([s for s in steps if s.get('outcome') == 'relation_skipped_no_evidence']) == 1
print('KG-EV1 PASS: repoint, alias, punctuation, full body, skip, ids-only log, settle count, provenance, shared privacy, search fallback')
