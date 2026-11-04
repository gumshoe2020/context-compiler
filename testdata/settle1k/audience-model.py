#!/usr/bin/env python3
"""Select a synthetic response and reject any mixed writer prompt."""
import os
from pathlib import Path
import sys
prompt = sys.stdin.read()
reply = Path(os.environ['PRIV1_REPLY'])
if reply.is_dir():
    private = 'violet submarine' in prompt
    shared = 'amber glider' in prompt
    assert private != shared, 'mixed or absent synthetic audience'
    if shared and os.environ.get('FAIL_SHARED'):
        sys.exit(1)
    reply = reply / ('wren.md' if private else 'shared.md')
print(reply.read_text())
