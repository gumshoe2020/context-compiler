#!/usr/bin/env python3
"""Test delivery writes only to a local synthetic artifact. It never sends anything."""
import json
import os
from pathlib import Path
import sys
path = Path(os.environ['PRIV1_DELIVERY'])
assert '/target/priv1/' in str(path.resolve())
with path.open('a') as output:
    output.write(json.dumps(dict(args=sys.argv[1:], text=sys.stdin.read())) + '\n')
