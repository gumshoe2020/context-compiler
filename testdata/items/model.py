#!/usr/bin/env python3
"""Synthetic command: stand-in; execute only from a copied test scratch directory."""
import json
from pathlib import Path
import re
import sys

root = Path(__file__).resolve().parent
prompt = sys.stdin.read()
match = re.search(r"New transcript lines \((\d+-\d+)\):", prompt)
if not match:
    raise SystemExit("missing window")
(root / ("prompt-" + match[1] + ".txt")).write_text(prompt)
print(json.dumps(json.loads((root / "replies.json").read_text())[match[1]]))
