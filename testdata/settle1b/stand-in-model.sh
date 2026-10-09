#!/bin/sh
# stand-in-model.sh — a stand-in model for the SETTLE-1b tests (`--llm command:<this>`): appends
# its input (the prompt, on stdin) to $STANDIN_LOG after a `=== call <n> ===` line, and prints the
# n-th reply file of $STANDIN_REPLIES (space-separated; the last repeats). No model is called.
set -eu
n=$(($(cat "$STANDIN_LOG.count" 2>/dev/null || echo 0) + 1))
echo "$n" >"$STANDIN_LOG.count"
{ echo "=== call $n ==="; cat; echo; } >>"$STANDIN_LOG"
i=0 reply=""
for file in $STANDIN_REPLIES; do
  i=$((i + 1))
  reply=$file
  [ "$i" -lt "$n" ] || break
done
cat "$reply"
