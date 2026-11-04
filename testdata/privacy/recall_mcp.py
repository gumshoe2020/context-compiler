#!/usr/bin/env python3
"""Constructed drawer MCP replies; file-only, invented content and provenance."""
import json
import os
from pathlib import Path
import sys
import time

work = Path(os.environ['PRIV1C_RECALL'])
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    if request['method'] == 'initialize':
        result = dict(protocolVersion='2024-11-05', capabilities={},
                      serverInfo=dict(name='synthetic-drawers', version='3.5.0'))
    elif request['method'] == 'tools/list':
        result = dict(tools=[dict(name='mempalace_search')])
    else:
        args = request['params']['arguments']
        with (work / 'requests.jsonl').open('a') as log:
            log.write(json.dumps(args) + '\n')
        time.sleep(.01)
        rows = json.loads((work / 'response.json').read_text())['results']
        if args.get('source_file'):
            rows = [r for r in rows if r['source_file'] == args['source_file']]
        if args.get('wing'):
            rows = [r for r in rows if r['wing'] == args['wing']]
        result = dict(content=[dict(type='text', text=json.dumps(dict(results=rows[:args['limit']])) )])
    print(json.dumps(dict(jsonrpc='2.0', id=request['id'], result=result)), flush=True)
