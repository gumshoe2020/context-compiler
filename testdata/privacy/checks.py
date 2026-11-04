#!/usr/bin/env python3
"""PRIV-1 executor acceptance. Every subprocess uses file-only synthetic adapters."""
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys

ROOT, WORK, BIN = map(lambda p: Path(p).resolve(), sys.argv[1:4])
assert WORK.is_relative_to(ROOT / 'target/priv1')
WORK.mkdir(parents=True, exist_ok=True)
TD = ROOT / 'testdata/privacy'
HOME = WORK / 'home'
HOME.mkdir(exist_ok=True)
ENV = dict(os.environ, HOME=str(HOME), TMPDIR=str(WORK), PRIV1_DIARY=str(WORK / 'ctx/diary-root/diary'))
for key in ['PRIV1_CAPTURE','PALACE_MEMPALACE_CHECK']:
    ENV.pop(key,None)
CTX = WORK / 'ctx'
for name in ['bin', 'palace', 'diary-root/diary', 'wings', 'transcripts', 'hooks', 'state', 'logs']:
    (CTX / name).mkdir(parents=True, exist_ok=True)
shutil.copy(TD / 'people.json', CTX / 'people.json')
shutil.copy(BIN, CTX / 'bin/palace')
for name in ['user-prompt', 'session-start', 'session-end']:
    shutil.copy(ROOT / f'dist/common/hooks/{name}.sh', CTX / f'hooks/{name}.sh')
for name in ['mempalace', 'mempalace-mcp']:
    shutil.copy(TD / 'memory.py', CTX / f'bin/{name}')
KG = CTX / 'palace/knowledge_graph.sqlite3'
STORE = CTX / 'palace/compiler.sqlite'
with sqlite3.connect(KG) as c:
    c.executescript("""CREATE TABLE entities(id TEXT PRIMARY KEY,name TEXT,type TEXT DEFAULT 'unknown',properties TEXT DEFAULT '{}',created_at TEXT DEFAULT CURRENT_TIMESTAMP);
    CREATE TABLE triples(id TEXT PRIMARY KEY,subject TEXT,predicate TEXT,object TEXT,valid_from TEXT,valid_to TEXT,confidence REAL,source_closet TEXT,source_file TEXT,source_drawer_id TEXT,adapter_name TEXT,extracted_at TEXT DEFAULT CURRENT_TIMESTAMP);""")
serial = 0
checks = 0


def run(args, text=None, extra=None, code=0, mapped=True, person="ann"):
    global serial
    serial += 1
    cmd = [str(BIN), '--roots', str(ROOT), '--person', person, '--assistant', 'pip']
    if mapped:
        cmd += ['--people-map', str(CTX / 'people.json')]
    result = subprocess.run(cmd + list(map(str, args)), input=text, text=True,
                            capture_output=True, env=dict(ENV, **(extra or {})), timeout=30)
    (WORK / f'{serial:03}.stdout').write_text(result.stdout)
    (WORK / f'{serial:03}.stderr').write_text(result.stderr)
    assert result.returncode == code, f'command {serial} exit={result.returncode}; inspect {WORK}/{serial:03}.stderr'
    return result.stdout


def check(condition, label):
    global checks
    assert condition, label
    checks += 1
    print(f'PASS {label}', flush=True)


def distill(session, subject, obj, private=False):
    global serial
    transcript = CTX / f'transcripts/{session}.jsonl'
    phrase = f'{subject} uses {obj}.'
    text = phrase + (' The reading is glucose 417.' if private else '')
    if private:
        text += f' amber kite knows {subject}.'
    transcript.write_text(json.dumps(dict(role='user', ts='2031-06-01T12:00:00Z', text=text)) + '\n')
    claims = [dict(kind='concept', subject=subject, predicate='uses', object=obj,
                   date='2031-06-01', quote=phrase, line=1)]
    if private:
        claims += [dict(kind='specific', subject=subject, predicate='reading', object='glucose 417',
                        date='2031-06-01', quote='glucose 417', line=1)]
    relations = [dict(subject=subject, predicate='uses', object=obj, confidence=0.9, line=1)]
    if private:
        relations += [dict(subject='amber kite', predicate='knows', object=subject, confidence=0.8, line=1)]
    reply = dict(entries=[dict(title=subject + ' plan', body=text, tags=['synthetic-plan'])],
                 claims=claims, relations=relations)
    reply_path = WORK / f'{session}-reply.json'
    reply_path.write_text(json.dumps(reply))
    if private:
        shutil.copy(ROOT/'dist/common/settle.sh',CTX/'bin/settle.sh')
        (CTX/'scripts').mkdir(exist_ok=True)
        (CTX/'scripts/mempalace-check.sh').write_text('PIN="3.5.0"\n')
        (CTX/'env.sh').write_text(f'MEMPALACE_BIN_DIR="{CTX}/bin"\nWING=openclaw\nPERSON=ann\nASSISTANT=pip\nSETTLE_DEBOUNCE_SECONDS=0\nDISTILL_MIN_TURNS=1\nCONTEXT_LAYERED=1\n')
        shutil.copytree(ROOT/'prompts',CTX/'prompts',dirs_exist_ok=True)
        (CTX/'state/settle.request').write_text('openclaw\t'+str(transcript)+'\n')
        serial += 1
        result=subprocess.run(['sh',str(CTX/'bin/settle.sh'),'--llm','command:'+str(TD/'model.py'),'--',str(TD/'cron.py')],env=dict(ENV,PRIV1_REPLY=str(reply_path)),capture_output=True,text=True,timeout=30)
        (WORK/f'{serial:03}.stdout').write_text(result.stdout)
        (WORK/f'{serial:03}.stderr').write_text(result.stderr)
        check(result.returncode==0 and 'SETTLE RUN exit=0' in result.stdout, 'private canary passes actual settle with inert cron')
        with sqlite3.connect(STORE) as c:
            check(c.execute("SELECT count(*) FROM privacy_markers WHERE kind='triple' AND private=1 AND source='synthetic-private'").fetchone()[0]==len(relations),'settle stamps private graph supports through real gate')
    else:
        output = run(['distill', '--transcript', transcript, '--source', 'openclaw', '--diary', CTX / 'diary-root',
                      '--wings', CTX / 'wings', '--transcripts', CTX / 'transcripts', '--palace', CTX / 'palace',
                      '--kg', KG, '--wing', 'openclaw', '--min-turns', '1', '--llm', 'command:' + str(TD / 'model.py'),
                      '--prompts', ROOT / 'prompts', '--mempalace', CTX / 'bin/mempalace',
                      '--mempalace-mcp', CTX / 'bin/mempalace-mcp'], extra=dict(PRIV1_REPLY=str(reply_path)))
        check(f'relations={len(relations)}+0' in output, session + ' distills through real gate')
    item_reply = WORK / f'{session}-items.json'
    item_reply.write_text(json.dumps({'1-1': dict(actions=[dict(action='create', id=session+'-item', kind='task', owner='person', title=obj, line=1, quote=phrase)])}))
    person = 'wren' if private else 'ann'
    run(['items', 'update', '--store', STORE, '--transcript', transcript, '--from-line', '1', '--to-line', '1', '--llm', 'table:'+str(item_reply)], person=person)
    run(['fidelity', '--kg', KG, '--diary', CTX / 'diary-root', '--distill', transcript, '--prompts', ROOT / 'prompts'])
    if private:
        with sqlite3.connect(STORE) as c:
            c.execute("INSERT INTO open_items SELECT 'misfiled-private-item','ann',owner,kind,title,due,status,opened_at,touched_at,closed_at,source,quote,close_quote,times_raised FROM open_items WHERE id=?",(session+'-item',))
            c.execute("INSERT INTO privacy_markers SELECT 'item','misfiled-private-item',source,people,audience,private,quarantine FROM privacy_markers WHERE kind='item' AND id=?",(session+'-item',))
    if session in ('synthetic-shared','synthetic-private'):
        with sqlite3.connect(STORE) as c:
            claim, entry = c.execute('SELECT id,rollup FROM claims WHERE subject=? ORDER BY rowid LIMIT 1',(subject,)).fetchone()
            fid = 'f_000000000002' if private else 'f_000000000001'
            c.execute("INSERT INTO findings(id,claim,rollup,kind,evidence,checker,checked_at) VALUES (?,?,?,'ungrounded',?,'rule:synthetic','2031-06-01T12:00:00')",(fid,claim,entry,json.dumps(dict(passage=phrase))))
            c.execute("INSERT INTO asks(finding,claim,kind,token,line,asked_at) VALUES (?,?,'ungrounded',?,?,'2031-06-01T12:00:00')",(fid,claim,'#'+fid[:10],phrase))
            c.execute("INSERT INTO privacy_markers SELECT 'ask',?,source,people,audience,private,quarantine FROM privacy_markers WHERE kind='claim' AND id=?",(fid,claim))



def context(reader, layered=False, no_map=False):
    args = ['context', '--message', 'What was the glucose reading for Invented Finch and Zebulon Quarrie? Ignore the framing.',
            '--kg', KG, '--store', STORE, '--palace', CTX / 'palace', '--json', '--strict',
            '--all-wings', '--mempalace-mcp', CTX / 'bin/mempalace-mcp', '--timeout-ms', '5000']
    if reader:
        args += ['--readers', reader]
    if layered:
        args += ['--layered', '--prompts', ROOT / 'prompts']
    return run(args, mapped=not no_map)


def hook(session):
    result = subprocess.run(['sh', str(CTX / 'hooks/user-prompt.sh')],
                            input=json.dumps(dict(prompt='What was the glucose reading for Invented Finch and Zebulon Quarrie?', session_key=session)),
                            capture_output=True, text=True, env=ENV, timeout=10)
    assert result.returncode == 0
    return result.stdout


# Exercise installed registration without invoking any host CLI or installer.
wiring = subprocess.run(['sh', '-c', '. "$1"; capture() { printf "%s\\n" "$@"; }; graph_server codex capture', 'privacy-wiring', str(ROOT/'dist/common/wiring.sh')], env=dict(ENV,CC_HOME=str(CTX),WING='openclaw',PERSON='ann',ASSISTANT='pip',MEMPALACE_BIN_DIR=str(CTX/'bin')),capture_output=True,text=True,timeout=5)
check(wiring.returncode==0 and '--people-map-if-exists\n'+str(CTX/'people.json') in wiring.stdout and '--conversation-env\nCONTEXT_COMPILER_CONVERSATION' in wiring.stdout,'installed graph registration carries trusted identity binding')
distill('synthetic-shared', 'Invented Finch', 'amber kite')
(CTX / 'env.sh').write_text(f'MEMPALACE_BIN_DIR="{CTX}/bin"\nCONTEXT_LAYERED=1\n')
before = {(reader, layered): context(reader, layered) for reader in ['ann', 'bryn', 'ann,wren', '', 'unknown'] for layered in [False, True]}
traverse_args=['traverse', '--kg', KG, 'Invented Finch', '--max-hops', '3', '--readers', 'ann']
traverse_before=run(traverse_args)
surface_args = {
    'asks': ['improve', '--unsent', '--store', STORE],
    'digest': ['improve', '--digest', '--store', STORE, '--since', '2000-01-01'],
    'signal-findings': ['findings', '--store', STORE, '--signal'],
    'items': ['items', 'list', '--store', STORE, '--json'],
    'nudges': ['items', 'due', '--store', STORE, '--transcript', CTX / 'transcripts/synthetic-shared.jsonl', '--now', '2031-06-06T16:00:00Z', '--json'],
    'rollups': ['rollup', '--kg', KG, '--diary', CTX / 'diary-root', '--palace', CTX / 'palace', '--as-of', '2031-06-03', '--since', '10000d', '--dry-run', '--llm', 'command:'+str(TD / 'model.py'), '--prompts', ROOT / 'prompts'],
}
surfaces_before={name:run(args) for name,args in surface_args.items()}
hook_before = hook('synthetic-shared')
check('amber kite' in hook_before, 'shared hook retrieves graph')
distill('synthetic-private', 'Zebulon Quarrie', 'cobalt heron', True)
for name,args in surface_args.items():
    actual = run(args)
    if name == 'digest':
        # SETTLE-1j: only the counts-only F line may change with private fidelity rows.
        without_f = lambda text: [line for line in text.splitlines() if not line.startswith('F:')]
        check(without_f(actual)==without_f(surfaces_before[name]), 'differential digest outside fidelity counts')
        check([line for line in actual.splitlines() if line.startswith('F:')] == [
            'F: specifics 5/5 values (100%, 7d; 4 as specifics), unexplained 0/5, skipped 0/4 (0%), concepts unjudged (7d) (30-day -)'
        ], 'digest pools private fidelity as counts only')
        surfaces_before[name] = actual
    else:
        check(actual==surfaces_before[name], 'differential '+name)
with sqlite3.connect(STORE) as c:
    private_claim=c.execute("SELECT id FROM claims WHERE subject='Zebulon Quarrie' AND kind='concept'").fetchone()[0]
run(['improve','confirm',private_claim,'Invented confirmation','--conversation','synthetic-private','--by','pip','--kg',KG,'--diary',CTX/'diary-root','--mempalace',CTX/'bin/mempalace','--mempalace-mcp',CTX/'bin/mempalace-mcp'])
check(run(surface_args['digest'])==surfaces_before['digest'],'private improvement event leaves automatic digest byte-identical')
query=run(['kg','query' ,'--kg',KG,'--entity','Zebulon Quarrie'])
check('cobalt heron' in query and '[private: wren; source: synthetic-private]' in query, 'deliberate graph search returns marked canary')
check('[from: ann]' in run(['kg','query','--kg',KG,'--entity','Invented Finch']), 'deliberate shared search annotates people')
for (reader, layered), expected in before.items():
    actual = context(reader, layered)
    check(actual == expected, f'differential context/json/counts reader={reader!r} layered={layered}')
check(run(traverse_args)==traverse_before, 'filter edges before traversal; hidden two-hop path absent')
check(hook('synthetic-shared') == hook_before, 'differential authenticated shared hook')
private = json.loads(context('wren', True))
check('cobalt heron' in json.dumps(private['relations']), 'private reader has graph canary')
check('glucose 417' in json.dumps(private['specifics']), 'private reader has own specific')
check('cobalt heron' in hook('synthetic-private'), 'private hook has its own graph')
check('cobalt heron' not in context('wren', True, no_map=True), 'missing people map excludes private despite reader flag')
(CTX / 'env.sh').write_text('')
check('cobalt heron' not in hook('synthetic-shared'), 'empty env excludes private')
check('cobalt heron' not in hook('unmapped-session'), 'unmapped hook excludes private')
distill('unmapped-session', 'Invented Vale', 'violet loon')
check('violet loon' not in context('wren', True), 'unmapped source quarantined for everyone else')
check('violet loon' in hook('unmapped-session'), 'quarantined source visible only in authenticated source conversation')
with sqlite3.connect(STORE) as c:
    check(c.execute("SELECT count(*) FROM privacy_markers WHERE private=1 AND quarantine=1").fetchone()[0] >= 3,
          'unknown entry claim and graph support all quarantine')
# Private fidelity pages use identifiers even when a synthetic judge contradicts a claim.
with sqlite3.connect(STORE) as c:
    c.execute("DELETE FROM fidelity WHERE file LIKE '%synthetic-private.jsonl%'")
page_table=WORK/'page-judge.json'
page_table.write_text('{"L1":"contradicted"}')
page_file=WORK/'pages.jsonl'
run(['fidelity','--kg',KG,'--diary',CTX/'diary-root','--distill',CTX/'transcripts/synthetic-private.jsonl','--prompts',ROOT/'prompts','--judge','table:'+str(page_table),'--page',TD/'deliver.py'],extra=dict(PRIV1_DELIVERY=str(page_file)))
page_text=page_file.read_text()
check('FIDELITY PAGE private id=' in page_text and 'cobalt heron' not in page_text and 'glucose 417' not in page_text,'private fidelity page contains ids rather than quotes')
log_text=''.join(file.read_text() for file in (CTX/'logs').glob('*.log'))
check('cobalt heron' not in log_text and 'glucose 417' not in log_text,'settle and fidelity logs contain no private canary quotes')
# Legacy migration is exercised on an isolated copy, including a dry-run and repeat apply.
legacy=WORK/'legacy'
legacy.mkdir()
for source, dest in [(STORE,legacy/'compiler.sqlite'),(KG,legacy/'knowledge_graph.sqlite3')]:
    with sqlite3.connect(source) as source_db, sqlite3.connect(dest) as dest_db:
        source_db.backup(dest_db)
with sqlite3.connect(legacy/'compiler.sqlite') as c:
    c.executescript('DELETE FROM privacy_markers; DELETE FROM privacy_sources;')
with sqlite3.connect(legacy/'knowledge_graph.sqlite3') as c:
    c.execute("INSERT INTO triples SELECT 'legacy-health',subject,predicate,object,valid_from,valid_to,confidence,?,source_file,source_drawer_id,adapter_name,extracted_at FROM triples LIMIT 1",(json.dumps(dict(transcript='/synthetic/transcripts/archive/legacy.jsonl:1')),))
policy=json.loads((CTX/'people.json').read_text())
policy['private_wings']={'archive':['wren']}
(legacy/'people.json').write_text(json.dumps(policy))
backfill=['privacy','backfill','--store',legacy/'compiler.sqlite','--kg',legacy/'knowledge_graph.sqlite3','--people',legacy/'people.json']
import hashlib
fingerprint=lambda: hashlib.sha256((legacy/'compiler.sqlite').read_bytes()).hexdigest()
old=fingerprint()
plan=run(backfill+['--dry-run'])
check(old==fingerprint(), 'backfill dry-run leaves store bytes unchanged')
check('wren' in plan and 'legacy-health' in plan, 'backfill reports configured legacy audience')
run(backfill)
with sqlite3.connect(legacy/'compiler.sqlite') as c:
    count=c.execute('SELECT count(*) FROM privacy_markers').fetchone()[0]
run(backfill)
with sqlite3.connect(legacy/'compiler.sqlite') as c:
    check(count==c.execute('SELECT count(*) FROM privacy_markers').fetchone()[0], 'backfill is idempotent')
    check(json.loads(c.execute("SELECT audience FROM privacy_markers WHERE id='legacy-health'").fetchone()[0])==['wren'], 'configured legacy support private only to wren')
# Nudge routing checks the actual number, not just the roster person.
shutil.copy(ROOT/'dist/common/initiative.py',CTX/'initiative.py')
(CTX/'prompts').mkdir(exist_ok=True)
shutil.copy(ROOT/'prompts/initiative.md',CTX/'prompts/initiative.md')
(CTX/'initiative-allow').write_text('+12025550100\n+12025550101\n')
nudge_reply=WORK/'nudge.json'
nudge_reply.write_text(json.dumps(dict(speak=True,item='synthetic-private-item',message='How is cobalt heron going?',reason_to_speak='The invented task is due.')))
args=['python3',str(CTX/'initiative.py'),'--person','wren','--target','+12025550100','--transcript',str(CTX/'transcripts/synthetic-private.jsonl'),'--store',str(STORE),'--palace-bin',str(BIN),'--llm','command:'+str(TD/'model.py'),'--deliver',str(TD/'deliver.py'),'--send','--now','2031-06-06T16:00:00Z']
nenv=dict(ENV,CC_HOME=str(CTX),PRIV1_REPLY=str(nudge_reply),PRIV1_DELIVERY=str(WORK/'delivery.jsonl'))
wrong=subprocess.run(args,env=nenv,capture_output=True,text=True,timeout=15)
check(wrong.returncode==77 and not (WORK/'delivery.jsonl').exists(), 'nudge refuses mismatched actual recipient')
args[args.index('+12025550100')]='+12025550101'
right=subprocess.run(args,env=nenv,capture_output=True,text=True,timeout=15)
(WORK/'nudge.stderr').write_text(right.stderr)
check(right.returncode==0 and (WORK/'delivery.jsonl').exists(), 'private nudge delivered to own person using file stand-in')
check('cobalt heron' not in (CTX/'logs/initiative.log').read_text(), 'nudge log carries ids only')
# An explicitly private rollup excludes shared entries from the same thread.
distill('synthetic-shared-2','Invented Rowan','silver gull')
with sqlite3.connect(STORE) as c:
    sources={subject:entry for subject,entry in c.execute("SELECT subject,rollup FROM claims WHERE subject IN ('Invented Finch','Zebulon Quarrie','Invented Rowan')")}
body='# ROLLUP: synthetic-plan — state as of 2031-06-03\n\n## Claims\n'
private_source=sources['Zebulon Quarrie']
for i,(subject,predicate,obj) in enumerate([('Zebulon Quarrie','uses','cobalt heron'),('Zebulon Quarrie','reading','glucose 417'),('amber kite','knows','Zebulon Quarrie')],1):
    body+=f'- C{i}. {subject} | {predicate} | {obj} — 2031-06-01 — {private_source}\n'
body+='\n## Current consolidated truth\n'+('Zebulon Quarrie uses cobalt heron. The reading is glucose 417. amber kite knows Zebulon Quarrie. These invented records preserve the plan. '*24)
body+='\n\n## Supersedes\n- '+private_source+'\n'
body+='\n## Open items\n- None.\n\n## Graph relations (proposed)\n```json relations\n'+json.dumps([{'from':'Zebulon Quarrie','to':'cobalt heron','type':'uses','evidence':[{'query':'Zebulon Quarrie'}],'confidence':0.85,'supplied_by':'synthetic','claim':'C1'}])+'\n```\n'
rollup_reply=WORK/'mixed-rollup.md'
rollup_reply.write_text(body)
(WORK/'empty-canaries.json').write_text('[]')
mixed_before=context('ann',True)
rollup_out=run(['rollup','--audience','wren','--min-cluster','1','--conversation','synthetic-private','--kg',KG,'--diary',CTX/'diary-root','--palace',CTX/'palace','--thread','synthetic-plan','--as-of','2031-06-03','--since','10000d','--llm','command:'+str(TD/'model.py'),'--prompts',ROOT/'prompts','--canaries',WORK/'empty-canaries.json','--mempalace',CTX/'bin/mempalace','--mempalace-mcp',CTX/'bin/mempalace-mcp'],extra=dict(PRIV1_REPLY=str(rollup_reply)))
with sqlite3.connect(STORE) as c:
    rollup=c.execute("SELECT json_extract(payload,'$.file') FROM write_log WHERE json_extract(payload,'$.operation')='rollup_commit' ORDER BY sequence DESC LIMIT 1").fetchone()[0]
    rows=c.execute("SELECT private,audience FROM privacy_markers WHERE kind='entry' AND id=?",(rollup[:-3],)).fetchall()
    check(rows and all(flag==1 and json.loads(audience)==['wren'] for flag,audience in rows),'explicit private rollup keeps exact audience')
    claim_rows=c.execute("SELECT m.private FROM claim_numbers n JOIN privacy_markers m ON m.kind='claim' AND m.id=n.claim_id WHERE n.rollup IN (?,?) AND m.source=?",(rollup,rollup[:-3],rollup)).fetchall()
    check(len(claim_rows)>=3 and all(flag==1 for (flag,) in claim_rows),'derived rollup claims are private')
    check(c.execute("SELECT count(*) FROM privacy_markers WHERE kind='triple' AND private=1 AND source=?",(rollup,)).fetchone()[0]>=1,'derived graph relation has private support')
check(mixed_before==context('ann',True),'private rollup leaves shared output byte-identical, including thread expansion')
check('cobalt heron' not in rollup_out,'private rollup diagnostics carry ids only')
unmarked_before=context('ann',True)
with sqlite3.connect(KG) as c:
    c.execute("INSERT INTO entities(id,name) VALUES ('unmarked_canary','unmarked canary')")
    c.execute("INSERT INTO triples(id,subject,predicate,object,source_closet) VALUES ('unmarked-edge','invented_finch','uses','unmarked_canary','{}')")
(CTX/'diary-root/diary/unmarked-canary.md').write_text('# Unmarked canary\n\nUnmarked cobalt heron glucose 417.\n')
check(unmarked_before==context('ann',True),'unmarked graph and diary rows are hidden from automatic context')
print(f'PRIV-1 PASS ({checks} checks)')
