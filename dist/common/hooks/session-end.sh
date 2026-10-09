#!/bin/sh
# session-end.sh — SessionEnd hook (Claude Code, Codex): a request for a settle run, nothing more.
#
# Usage: <hook JSON on stdin> | $CC_HOME/hooks/session-end.sh <source>
#   <source>  claude-code or codex (the installers write it into the hook's command)
# Appends one line to $CC_HOME/state/settle.request: `<source>\t<transcript path>\n`, the path
# being the hook input's transcript_path (tabs and line breaks made spaces), empty when the input
# has none or is not JSON. That file is the request file: settle.sh (run by the scheduler when it
# changes) takes it whole. Never distills, compiles, mines or waits on a lock; prints nothing
# (both tools read a hook's stdout); always exits 0, even when the line cannot be written.
# CONTEXT_COMPILER_NO_HOOKS=1 (set by codex-llm.sh) turns it off.
exec >/dev/null 2>&1
[ -z "${CONTEXT_COMPILER_NO_HOOKS:-}" ] || exit 0
CC_HOME=$(cd "$(dirname "$0")/.." && pwd) || exit 0
case "${1:-}" in '' | *[!a-z-]*) source=unknown ;; *) source=$1 ;; esac
path=$(jq -r '.transcript_path // "" | strings | gsub("[\t\r\n]"; " ")' 2>/dev/null) || path=""
printf '%s\t%s\n' "$source" "$path" >>"$CC_HOME/state/settle.request" || true
exit 0
