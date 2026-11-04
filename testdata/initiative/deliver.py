#!/usr/bin/env python3
"""File-only delivery stand-in: never sends, never imports a sender."""
import json
import os
from pathlib import Path
import sys
home = Path(os.environ['CC_HOME'])
assert sys.argv[1:] == ['send', '--target', '+12025550123']
assert (home / 'initiative-outbox').read_text().strip(), 'reservation must precede delivery'
text = sys.stdin.read()
if os.environ.get('INITIATIVE_DELIVER_FAIL') == '1':
    raise SystemExit(9)
with (home / 'sent.jsonl').open('a') as out:
    out.write(json.dumps(dict(argv=sys.argv[1:], message=text)) + '\n')
