#!/bin/sh
# Explicit roster sweep only; installs no schedule. See initiative.sh for adapters.
# TSV: person, target, transcript, mode (send/dry-run), optional base_hours.
# Empty/absent base_hours uses 4; invalid values log and skip that row.
CC_HOME=${CC_HOME:-$(CDPATH='' cd "$(dirname "$0")/.." && pwd)}
export CC_HOME
mkdir -p "$CC_HOME/logs" 2>/dev/null || exit 0
python3 "$(dirname "$0")/initiative.py" --all "$@" >>"$CC_HOME/logs/initiative.log" 2>&1
status=$?
[ "$status" -eq 0 ] || printf '%s\n' "initiative-all: runner failed (exit $status)" >>"$CC_HOME/logs/initiative.log"
exit 0
