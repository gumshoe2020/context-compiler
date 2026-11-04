#!/usr/bin/env python3
"""Synthetic stdin/stdout model. No model, service, or network access."""
import os
from pathlib import Path
import sys
home = Path(os.environ['CC_HOME'])
(home / 'model-prompt.txt').write_text(sys.stdin.read())
with (home / 'model.calls').open('a') as out:
    out.write('called\n')
if os.environ.get('INITIATIVE_DROP_ALLOW') == '1':
    (home / 'initiative-allow').unlink()
print(Path(os.environ['INITIATIVE_MODEL_REPLY']).read_text())
