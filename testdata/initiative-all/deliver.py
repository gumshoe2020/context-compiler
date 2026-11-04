#!/usr/bin/env python3
"""File-only delivery stand-in. Never sends anything."""
import json
import os
from pathlib import Path
import sys

with (Path(os.environ['CC_HOME']) / 'sent.jsonl').open('a') as out:
    out.write(json.dumps(dict(argv=sys.argv[1:], message=sys.stdin.read())) + '\n')
