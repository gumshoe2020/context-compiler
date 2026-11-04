#!/bin/sh
# INIT-1c sweep; inert until explicitly invoked. Default: --dry-run.
# Install initiative.py beside this script and prompts/initiative.md under CC_HOME.
# --base-hours N sets a positive decimal wait (default 4) for due and skipped checks.
# INITIATIVE_TRANSCRIPT must be explicit (or pass --transcript); never guess a person.
CC_HOME=${CC_HOME:-$(CDPATH='' cd "$(dirname "$0")/.." && pwd)}
export CC_HOME
mkdir -p "$CC_HOME/logs" 2>/dev/null || exit 0
# The model and delivery adapters are explicit argv executables; never evaluated as shell code.
python3 "$(dirname "$0")/initiative.py" "$@" >>"$CC_HOME/logs/initiative.log" 2>&1
status=$?
[ "$status" -eq 77 ] && exit 77
[ "$status" -eq 0 ] || printf '%s\n' "initiative: runner failed (exit $status)" >>"$CC_HOME/logs/initiative.log"
exit 0
