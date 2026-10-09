#!/usr/bin/env python3
"""Local injected judge: never calls a model or network."""
import json
import sys
import time
if '--sleep' in sys.argv:
    time.sleep(5)
if '--fail' in sys.argv:
    sys.exit(7)
prompt = sys.stdin.read()
assert 'Canonical statement: test statement' in prompt
passage = prompt.split('Passage to evaluate:\n', 1)[1].split('\n\nAnswer with', 1)[0]
assert len(passage) == 12000
assert 'test statement' not in sys.argv
print(json.dumps({'verdict': 'preserved', 'quote': 'stdin received'}))
