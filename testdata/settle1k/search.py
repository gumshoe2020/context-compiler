#!/usr/bin/env python3
"""Synthetic search transport: literal matching, ranked rows, no real memory access."""
import json
import os
from pathlib import Path
import sys

base = Path(os.environ['SETTLE1K_SEARCH'])
with (base / 'server-starts').open('a') as stream:
    stream.write('start\n')
rows = json.loads((base / 'rows.json').read_text())
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    if request['method'] == 'initialize':
        result = dict(protocolVersion='2024-11-05', capabilities={},
                      serverInfo=dict(name='synthetic-search', version='3.5.0'))
    elif request['method'] == 'tools/list':
        result = dict(tools=[dict(name='mempalace_search')])
    else:
        args = request['params']['arguments']
        matches = [r for r in rows if args['query'] in r['text']
                   and (not args.get('source_file') or r['source_file'] == args['source_file'])]
        result = dict(content=[dict(type='text', text=json.dumps(dict(results=matches[:args['limit']])) )])
    print(json.dumps(dict(jsonrpc='2.0', id=request['id'], result=result)), flush=True)
