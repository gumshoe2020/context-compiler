#!/usr/bin/env python3
"""A command: model that returns an explicitly selected synthetic reply file."""
import os
from pathlib import Path
import sys
prompt = sys.stdin.read()
if os.environ.get('PRIV1_CAPTURE'):
    Path(os.environ['PRIV1_CAPTURE']).write_text(prompt)
print(Path(os.environ['PRIV1_REPLY']).read_text())
