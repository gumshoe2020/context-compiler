#!/bin/sh
# notify-local.sh — the --send and --page command of palace-cron.sh on a machine with no chat
# channel: the text goes to a local inbox the agent reads at the start of a session.
#
# Usage: <text on stdin> | $CC_HOME/bin/notify-local.sh [--dry-run]
# Appends each non-blank line of the text as "- [<date time zone>] <line>" to $CC_HOME/inbox.md
# ($CC_HOME: the parent of the directory this script is installed in), then posts a macOS
# notification (osascript, best effort, at most 5 s). Exit 0 only after the append.
# Contract (docs/RUNBOOK.md section 2): TERM or HUP before the append ends it with exit 143 or
# 129 and nothing written. A signal during the append itself (one write) lets that write finish
# and still exits 143 or 129: the caller counts the text undelivered and sends it again (at
# least once). A signal after the append exits 0: the text was delivered.
set -eu
umask 077
CC_HOME=$(cd "$(dirname "$0")/.." && pwd)
INBOX="$CC_HOME/inbox.md"
DRY_RUN=0
case "${1:-}" in
  --dry-run) DRY_RUN=1 ;;
  -h | --help) sed -n '/^set -eu/q;2,$p' "$0"; exit 0 ;;
esac

TEXT="" RECORD="" READER=""
stop() {
  [ -z "$READER" ] || kill "$READER" 2>/dev/null || true
  rm -f "$TEXT" "$RECORD"
  exit "$1"
}
trap 'stop 143' TERM
trap 'stop 129' HUP

TEXT=$(mktemp "${TMPDIR:-/tmp}/notify-local.XXXXXX")
RECORD=$(mktemp "${TMPDIR:-/tmp}/notify-local.XXXXXX")
# Read in the background: `wait` returns at once on a trapped signal, a foreground cat would not.
exec 3<&0
cat <&3 >"$TEXT" &
READER=$!
wait "$READER"
READER=""

stamp=$(date '+%Y-%m-%d %H:%M %Z')
awk -v stamp="$stamp" 'NF { print "- [" stamp "] " $0 }' "$TEXT" >"$RECORD"
if [ ! -s "$RECORD" ]; then
  stop 0
fi
if [ "$DRY_RUN" = 1 ]; then
  sed "s|^|would append to $INBOX: |" "$RECORD"
  stop 0
fi

cat "$RECORD" >>"$INBOX"
trap 'stop 0' TERM HUP

first=$(head -1 "$TEXT" | cut -c1-200)
perl -e 'alarm 5; exec @ARGV' osascript -e 'on run argv' \
  -e 'display notification (item 1 of argv) with title "context-compiler"' -e 'end run' \
  "$first" >/dev/null 2>&1 || true
stop 0
