#!/usr/bin/env python3
"""Search stand-in. Never opens a palace or invokes a model."""
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
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    method = request['method']
    if method == 'initialize':
        result = dict(protocolVersion='2024-11-05', capabilities={},
                      serverInfo=dict(name='synthetic', version=PIN))
    elif method == 'tools/list':
        result = dict(tools=[dict(name='mempalace_search')])
    else:
        args = request['params']['arguments']
        base = Path(os.environ['PICK_WORK'])
        with (base / 'search.log').open('a') as log:
            log.write(json.dumps(args) + '\n')
        # An excluded-wing match must not introduce an entry outside the supplied diary.
        rows = [dict(
            text='Synthetic hidden observatory record', source_path=str(base / 'excluded.md'),
            source_file='2031-05-10-ffffffffffff.md', wing='hidden', room='general')]
        for file in sorted((base / 'root/diary').glob('*.md')):
            rows.append(dict(text=file.read_text(), source_path=str(file),
                             source_file=file.name, wing='openclaw', room='general'))
        result = dict(content=[dict(type='text', text=json.dumps(dict(results=rows)))])
    print(json.dumps(dict(jsonrpc='2.0', id=request['id'], result=result)), flush=True)
