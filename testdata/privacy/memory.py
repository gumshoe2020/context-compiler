#!/usr/bin/env python3
"""Synthetic file-only MemPalace stand-in; no network, model, or real memory."""
import json
import os
from pathlib import Path
import sys

import re
_check = (Path(os.environ['PALACE_MEMPALACE_CHECK']) if os.environ.get('PALACE_MEMPALACE_CHECK') else
          next(parent / 'scripts/mempalace-check.sh' for parent in Path(__file__).resolve().parents
               if (parent / 'scripts/mempalace-check.sh').is_file()))
PIN = re.search(r'^PIN=["\']?([^"\'\s]+)', _check.read_text(), re.M).group(1)

if '--version' in sys.argv:
    print(f'MemPalace {PIN}')
    sys.exit(0)
if 'mine' in sys.argv:
    sys.exit(0)
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    method = request['method']
    if method == 'initialize':
        result = dict(protocolVersion='2024-11-05', capabilities={},
                      serverInfo=dict(name='synthetic-memory', version=PIN))
    elif method == 'tools/list':
        result = dict(tools=[dict(name='mempalace_search')])
    else:
        args = request['params']['arguments']
        rows = []
        for file in sorted(Path(os.environ['PRIV1_DIARY']).glob('*.md')):
            if args.get('source_file') and args['source_file'] != file.name:
                continue
            if args.get('wing', 'openclaw') != 'openclaw':
                continue
            rows.append(dict(text=file.read_text(), source_file=file.name,
                             source_path=str(file), wing='openclaw', room='synthetic', distance=0.1))
        # A ranked synthetic search must put literal matches ahead of unrelated files.
        # Hash-derived filenames otherwise randomly push private evidence beyond top-3.
        query = args['query'].casefold()
        rows.sort(key=lambda row: (query not in row['text'].casefold(), row['source_file']))
        result = dict(content=[dict(type='text', text=json.dumps(dict(results=rows[:args['limit']])) )])
    print(json.dumps(dict(jsonrpc='2.0', id=request['id'], result=result)), flush=True)
