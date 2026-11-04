#!/usr/bin/env python3
"""Synthetic rollup model: deterministic selection and a fixed writer reply."""
import os
from pathlib import Path
import re
import json
import sys

prompt = sys.stdin.read()
base = Path(os.environ['PICK_WORK'])
with (base / 'model.log').open('a') as log:
    log.write(prompt + '\n=== END CALL ===\n')
if '=== TASK: SELECT ONE THREAD ===' in prompt:
    choices = re.findall(r'^\[(\d+)\] tag: (.*?) —', prompt, re.M)
    number, tag = choices[0]
    thread = 'missing-thread' if tag == 'a-decoy' else tag
    print(json.dumps(dict(thread=thread, reason='Synthetic choice', candidate_ids=[int(number)])))
else:
    print((base / 'body.md').read_text())
