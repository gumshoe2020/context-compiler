#!/usr/bin/env python3
"""PRIV-1c B: synthetic tag migration and real settle with inert adapters."""
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys

ROOT, WORK, BIN = (Path(p).resolve() for p in sys.argv[1:4])
assert WORK.is_relative_to(ROOT / 'target/priv1c')
WORK.mkdir(parents=True, exist_ok=True)
CTX = WORK / 'ctx'
for name in ['bin', 'scripts', 'palace', 'diary-root/diary', 'wings', 'transcripts', 'state', 'logs']:
    (CTX / name).mkdir(parents=True, exist_ok=True)
STORE, KG = CTX / 'palace/compiler.sqlite', CTX / 'palace/knowledge_graph.sqlite3'
PEOPLE = CTX / 'people.json'
policy = dict(people=dict(ann=dict(handles=['signal:+12025550100']), wren=dict(handles=[]), iris=dict(handles=[])),
              conversations=[dict(match='tag-settle', people=['ann'])])
PEOPLE.write_text(json.dumps(policy))
notes = {'line': '# Invented care note\nInvented lavender kite.\nTranscript: /invented/transcripts/openclaw/tagged.jsonl:1-3\n',
         'inline': '---\ntags: []\n---\n# Invented inline note\nInvented amber owl.\n',
         'block': '---\ntags:\n  - ordinary\n---\n# Invented block note\nInvented copper rook.\n',
         'union': '# Invented union note\nInvented silver finch.\n'}
for name, text in notes.items():
    (CTX / f'diary-root/diary/{name}.md').write_text(text)
with sqlite3.connect(STORE) as c:
    c.executescript((ROOT / 'crates/palace-store/src/schema.sql').read_text())
    c.execute('ALTER TABLE claims ADD COLUMN transcript TEXT')
    for name in notes:
        c.execute('INSERT INTO claims(id,number,rollup,subject,predicate,object,date,source,confidence_base,first_seen) VALUES (?,?,?,?,?,?,?,?,?,?)',
            ('c-'+name, 'C1', name+'.md', 'Invented Finch', 'uses', 'lavender kite', '2032-01-01', name+'.md', .8, '2032-01-01'))
    c.execute("INSERT INTO claims(id,number,rollup,subject,predicate,object,date,source,confidence_base,first_seen) VALUES ('c-derived','C1','derived.md','Invented Finch','uses','lavender kite','2032-01-01','line.md',.8,'2032-01-01')")
    c.execute("INSERT INTO citations VALUES ('t-cited','c-derived','derived.md','2032-01-01')")
    c.execute("INSERT INTO asks(finding,claim,kind,token,line,asked_at) VALUES ('a-derived','c-derived','check','synthetic','Invented question','2032-01-01')")
    c.execute("INSERT INTO fidelity(point,file,type,sampled,supported,missing,contradicted,unjudged,judge,items,at) VALUES ('synthetic','line.md','specific',1,1,0,0,0,'table','[]','2032-01-01')")
    c.execute("CREATE TABLE open_items(id TEXT PRIMARY KEY, person TEXT NOT NULL, owner TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('task','topic')), title TEXT NOT NULL, due TEXT, status TEXT NOT NULL DEFAULT 'open', opened_at TEXT NOT NULL, touched_at TEXT NOT NULL, closed_at TEXT, source TEXT NOT NULL, quote TEXT NOT NULL, close_quote TEXT, times_raised INTEGER NOT NULL DEFAULT 0)")
    c.execute("INSERT INTO open_items(id,person,owner,kind,title,opened_at,touched_at,source,quote) VALUES ('i-derived','ann','person','task','Invented task','2032-01-01','2032-01-01','line.md','Invented quote')")
with sqlite3.connect(KG) as c:
    c.executescript("""CREATE TABLE entities(id TEXT PRIMARY KEY,name TEXT,type TEXT DEFAULT 'unknown',properties TEXT DEFAULT '{}',created_at TEXT DEFAULT CURRENT_TIMESTAMP);
    CREATE TABLE triples(id TEXT PRIMARY KEY,subject TEXT,predicate TEXT,object TEXT,valid_from TEXT,valid_to TEXT,confidence REAL,source_closet TEXT,source_file TEXT,source_drawer_id TEXT,adapter_name TEXT,extracted_at TEXT DEFAULT CURRENT_TIMESTAMP);""")
    for name, closet in [('t-derived', dict(claim_id='c-derived')), ('t-entry', dict(transcript='line.md')), ('t-cited', {}), ('t-independent', dict(transcript='line.md')), ('t-transcript', dict(transcript='/invented/transcripts/openclaw/tagged.jsonl:2'))]:
        c.execute('INSERT INTO triples(id,source_closet) VALUES (?,?)', (name,json.dumps(closet)))
serial = checks = 0
ENV = dict(os.environ, HOME=str(WORK), TMPDIR=str(WORK), PRIV1_DIARY=str(CTX/'diary-root/diary'))

def check(ok, label):
    global checks
    assert ok, label
    checks += 1
    print('PASS', label, flush=True)

def run(args, env=None):
    global serial
    serial += 1
    result = subprocess.run(list(map(str,args)), capture_output=True, text=True, env=env or ENV, timeout=30)
    (WORK/f'{serial}.stdout').write_text(result.stdout)
    (WORK/f'{serial}.stderr').write_text(result.stderr)
    assert result.returncode == 0, f'command failed: {WORK}/{serial}.stderr'
    return result

def backfill(dry=False):
    args = [BIN, '--roots='+str(ROOT), 'privacy', 'backfill', '--store', STORE, '--kg', KG, '--people', PEOPLE, '--diary-root', CTX/'diary-root']
    result = run(args + (['--dry-run'] if dry else []))
    return [json.loads(l) for l in result.stdout.splitlines()], json.loads(result.stderr)

def snapshot():
    with sqlite3.connect(STORE) as c:
        return [sorted(c.execute('SELECT * FROM '+table).fetchall()) for table in ['privacy_markers','privacy_sources']]

def marker(kind, name):
    with sqlite3.connect(STORE) as c:
        rows = c.execute('SELECT private,audience FROM privacy_markers WHERE kind=? AND id=?', (kind,name)).fetchall()
    return rows

backfill()
with sqlite3.connect(STORE) as c:
    c.execute("INSERT INTO privacy_markers(kind,id,source,people,audience,private,quarantine) VALUES ('triple','t-independent','independent:synthetic','[]','[]',0,0)")
check(marker('entry','line')[0][0] == 0, 'baseline shared-wing entry is marked shared')
policy['private_tags'] = {'wren-private':['wren'], 'iris-private':['iris']}
PEOPLE.write_text(json.dumps(policy))
notes['line'] += 'Tags: ordinary, wren-private.\n'
notes['inline'] = notes['inline'].replace('tags: []', 'tags: ["wren-private", ordinary]')
notes['block'] = notes['block'].replace('  - ordinary', "  - ordinary\n  - 'wren-private'")
notes['union'] += 'Tags: wren-private, iris-private.\n'
for name, text in notes.items():
    (CTX/f'diary-root/diary/{name}.md').write_text(text)
before = [p.read_bytes() for p in (STORE,KG)]
rows, report = backfill(True)
check(before == [p.read_bytes() for p in (STORE,KG)], 'tag dry run leaves databases byte-identical')
check(report['privacy_backfill_counts']['entry']['tag:wren-private'] >= 3, 'report counts tag reason')
backfill()
for kind, name in [('entry','line'), ('entry','inline'), ('entry','block'), ('entry','derived'),
                   ('claim','c-line'), ('claim','c-derived'), ('triple','t-derived'), ('triple','t-entry'), ('triple','t-transcript'),
                   ('item','i-derived'), ('ask','a-derived'), ('fidelity','line.md')]:
    ms = marker(kind,name)
    check(ms and all(private and json.loads(aud)==['wren'] for private,aud in ms), kind+'/'+name+' narrows every existing support')
check(all(p for p,a in marker('triple','t-cited')) and any(json.loads(a)==['wren'] for p,a in marker('triple','t-cited')), 'citation narrows shared support while preserving quarantined support')
check(sorted(marker('triple','t-independent')) == [(0,'[]'),(1,'[\"wren\"]')], 'independent shared graph support remains shared while tagged support narrows')
check(marker('entry','union') == [(1,'["iris","wren"]')], 'matching tag audiences union')
first = snapshot()
backfill()
check(snapshot() == first, 'tag backfill is idempotent')
for name in ['line','inline','block','union']:
    (CTX/f'diary-root/diary/{name}.md').write_text('# Invented cleared tag\nInvented ordinary note.\n')
policy.pop('private_tags')
PEOPLE.write_text(json.dumps(policy))
backfill()
check(snapshot() == first, 'removing tags and policy never widens existing markers or sources')
# Re-enable tag policy and exercise the actual settle wrapper using command: and inert cron.
policy['private_tags'] = {'wren-private':['wren']}
PEOPLE.write_text(json.dumps(policy))
shutil.copy(BIN, CTX/'bin/palace')
shutil.copy(ROOT/'dist/common/settle.sh', CTX/'bin/settle.sh')
shutil.copytree(ROOT/'prompts', CTX/'prompts', dirs_exist_ok=True)
for name in ['mempalace','mempalace-mcp']:
    shutil.copy(ROOT/'testdata/privacy/memory.py',CTX/'bin'/name)
(CTX/'scripts/mempalace-check.sh').write_text('PIN="3.5.0"\n')
(CTX/'env.sh').write_text(f'MEMPALACE_BIN_DIR="{CTX}/bin"\nWING=openclaw\nPERSON=ann\nASSISTANT=pip\nSETTLE_DEBOUNCE_SECONDS=0\nDISTILL_MIN_TURNS=1\n')
transcript = CTX/'transcripts/tag-settle.jsonl'
phrase = 'Invented Tern uses violet glider.'
transcript.write_text(json.dumps(dict(role='user', ts='2032-01-01T12:00:00Z',text=phrase))+'\n')
(CTX/'state/settle.request').write_text('openclaw\t'+str(transcript)+'\n')
reply = WORK/'reply.json'
reply.write_text(json.dumps(dict(entries=[dict(title='Invented tag plan',body=phrase,tags=['wren-private'])],
    claims=[dict(kind='concept',subject='Invented Tern',predicate='uses',object='violet glider',date='2032-01-01',quote=phrase,line=1)],
    relations=[dict(subject='Invented Tern',predicate='uses',object='violet glider',confidence=.9,line=1)])))
result = run(['sh', CTX/'bin/settle.sh', '--llm','command:'+str(ROOT/'testdata/privacy/model.py'), '--', ROOT/'testdata/privacy/cron.py'], dict(ENV,PRIV1_REPLY=str(reply)))
check('SETTLE RUN exit=0' in result.stdout, 'new tagged entry passes actual settle with inert cron')
with sqlite3.connect(STORE) as c:
    cid, entry = c.execute("SELECT id,rollup FROM claims WHERE subject='Invented Tern'").fetchone()
for kind, name in [('entry',entry.removesuffix('.md')), ('claim',cid)]:
    check(all(p and json.loads(a)==['wren'] for p,a in marker(kind,name)), 'new settle '+kind+' inherits tag audience')
with sqlite3.connect(KG) as c:
    tid = c.execute("SELECT id FROM triples WHERE source_closet LIKE '%tag-settle%'").fetchone()[0]
check(all(p and json.loads(a)==['wren'] for p,a in marker('triple',tid)), 'new settle graph support inherits tag before proposal')
logged = run([BIN, '--roots='+str(ROOT), 'privacy', 'backfill', '--store', STORE, '--kg', KG, '--people', PEOPLE, '--dry-run'])
log_rows = [json.loads(line) for line in logged.stdout.splitlines()]
check(any(r['kind']=='entry' and r['id']==entry.removesuffix('.md') and r['reason']=='tag:wren-private' for r in log_rows), 'logged settle tags apply without diary roots')
print(f'PRIV-1c sticky tags: {checks} checks PASS')
