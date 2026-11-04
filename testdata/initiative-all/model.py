#!/usr/bin/env python3
"""Synthetic model: capture prompt, choose first candidate, no network."""
import json
import os
from pathlib import Path
import sys

prompt = sys.stdin.read()
home = Path(os.environ['CC_HOME'])
with (home / 'model-prompts.jsonl').open('a') as out:
    out.write(json.dumps(prompt) + '\n')
candidates = json.loads(prompt.split('\nCandidates: ', 1)[1].split('\n', 1)[0])
print(json.dumps(dict(speak=True, item=candidates[0]['id'],
                     reason_to_speak='The decision is waiting.',
                     message='Want to pick a chart?')))
