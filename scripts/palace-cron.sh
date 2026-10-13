#!/bin/sh
# palace-cron.sh — the scheduled job after the M5 flip (M5-CUTOVER-PLAN, M5b-4 item 1). Names no
# channel and sends nothing itself: every line goes to the command it is given.
#
# Usage: scripts/palace-cron.sh --palace <dir> --diary-root <dir> --kg <kg> --palace-bin <palace>
#          --send <exe> --page <exe> --state <dir> [--rollup] [--llm <spec>] [--judge <spec>]
#          [--retries 3] [--retry-delay 120] [--send-timeout 60] [--canaries <file>]
#          [--mempalace-check <path>] [--wing <name>] [--person <name>] [--assistant <name>]
#          [--max-cluster <n>]
#          [--no-deliver-stages | --no-digest | --deliver-only]
#   --palace, --kg     the palace and its graph; the claim store is the compiler.sqlite beside --kg
#   --diary-root       the diary ROOT (the parent of diary/)
#   --send             run once per ask line and once for the whole digest, the text on its
#                      standard input; exit 0 = delivered. Never receives a failure.
#   --page             run once with one line naming the stage and exit code (RETRY, BROKEN), the
#                      rollup's failed validation (ROLLUP INVALID), or counting failed deliveries
#                      (UNDELIVERED); never receives an ask, the digest or any text of the rollup
#   --rollup           run `palace rollup` first (compression nights); needs --llm
#   --llm              `palace rollup --llm` (replay | claude-cli[:model] | command:<exe>)
#   --max-cluster      `palace rollup --max-cluster` (default: the rollup's own, 25): a thread
#                      whose cluster holds more entries is skipped (ROLLUP SKIP, exit 0)
#   --wing             `palace rollup --wing`: the wing the rollup is mined into (default: the
#                      rollup's own, openclaw)
#   --person, --assistant  `palace --person/--assistant` for every palace command: the two
#                      roles' --by names (default: palace's own; a release build has none)
#   --no-deliver-stages  skip the asks and the digest (a settle run outside the asks window)
#   --no-digest        run the asks, skip the digest (a settle run inside the asks window)
#   --deliver-only     the asks and the digest only: no rollup, watch or improve --auto (the
#                      daily run's second step; refused with --rollup)
#                      With one of these three, stderr names the run's stages first
#                      (`palace-cron: stages: <name>…`); a run that delivers nothing leaves
#                      <state>/send-failures and <state>/digest-since as they are. At most one.
#   --judge            the watch's and improve --auto's judge (table:<file> | replay |
#                      claude-cli[:model] | command:<exe>); none by default (watch --no-judge)
#   --state            the wrapper's own files (required: the scheduled job and a hand run must
#                      name the same one); refused inside --diary-root or --palace (symlinks
#                      resolved), where the kept rollup attempts would be mined as memory
#   --retries          reruns of a stage that exits 3 ("retry later") before paging
#   --retry-delay      seconds between those reruns
#   --send-timeout     seconds one --send or --page run may take (then its whole process tree is
#                      ended with KILL; not delivered, sent again by the next run; so too when
#                      the wrapper is signalled while it runs)
# Stages, in order: (--rollup) rollup (its failed attempts kept in <state>/rollup-attempts,
# `palace rollup --keep`); watch --store; improve --auto; asks (every line of
# `improve --unsent` to --send, then `--mark-sent <token>` after exit 0 only); digest (`improve
# --digest --since <the previous delivered digest>`, its whole text to --send once when not empty).
# Exit 0 of a stage (and 1 of watch: findings) is done; 3 is rerun, then paged; anything else is
# paged at once, except exit 4 of the rollup (it failed its validation on every attempt): --page
# gets CRON ROLLUP INVALID exit=4 kept=<state>/rollup-attempts and the night goes on; its summary
# line then ends with " rollup=invalid" and is CRON OK even when nothing was sent. Prints one
# line, last: CRON OK asks=<n> digest=<0|1> failed=<k> | CRON QUIET |
# CRON RETRY stage=<s> tries=<n> | CRON BROKEN stage=<s> exit=<n>; exit 0, 0, 3, 2. From the
# second run in a row with a failed delivery, and every run after while it lasts, --page gets
# CRON UNDELIVERED failed=<k> exit=<last code> runs=<r> (<state>/send-failures). Writes only under
# --state, the lock and what the palace commands themselves write. One run per store: the lock is
# <kg dir>/.palace-cron.lock, whatever --state.
trap '' PIPE
set -u
# SIGPIPE ignored from the first line, the summary written to the saved stdout (fd 9), and a
# closed stdout made /dev/null, as in palace-shadow.sh (M5c.1 item 2, M5c.2 item 1).
{ command exec 9>&1; } 2>/dev/null || exec 9>/dev/null >/dev/null

ROOT=$(cd "$(dirname "$0")/.." && pwd) || exit 2
PALACE="" DIARY="" KG="" PALACE_BIN="" SEND="" PAGE="" LLM="" JUDGE="" ROLLUP=0 STATE="" WING="" PERSON="" ASSISTANT="" MAX_CLUSTER=""
DELIVER="" # "" (asks and digest, as before the flags) | none | asks | only
RETRIES=3 DELAY=120 SEND_TIMEOUT=60
CANARIES="$ROOT/fixtures/canaries.json"
CHECK="$ROOT/scripts/mempalace-check.sh"
CHILD="" WATCHER="" DELIVERING=0 LOCKED=0 FAILED=0 FAILED_CODE=0 ROLLUP_INVALID=""

die() {
  echo "palace-cron: $*" >&2
  echo "CRON BROKEN stage=arguments exit=2" >&9
  exit 2
}

while [ $# -gt 0 ]; do
  case "$1" in
    -h|--help) sed -n '/^trap/q;2,$p' "$0"; exit 0 ;;
    --rollup) ROLLUP=1; shift; continue ;;
    --no-deliver-stages | --no-digest | --deliver-only)
      [ -z "$DELIVER" ] || die "$1: at most one of --no-deliver-stages, --no-digest, --deliver-only"
      case "$1" in --no-deliver-stages) DELIVER=none ;; --no-digest) DELIVER=asks ;; *) DELIVER=only ;; esac
      shift
      continue ;;
  esac
  [ $# -ge 2 ] || die "missing value for $1"
  case "$1" in
    --palace) PALACE="$2" ;;
    --diary-root) DIARY="$2" ;;
    --kg) KG="$2" ;;
    --palace-bin) PALACE_BIN="$2" ;;
    --send) SEND="$2" ;;
    --page) PAGE="$2" ;;
    --llm) LLM="$2" ;;
    --judge) JUDGE="$2" ;;
    --state) STATE="$2" ;;
    --retries) RETRIES="$2" ;;
    --retry-delay) DELAY="$2" ;;
    --send-timeout) SEND_TIMEOUT="$2" ;;
    --canaries) CANARIES="$2" ;;
    --mempalace-check) CHECK="$2" ;;
    --wing) WING="$2" ;;
    --max-cluster) MAX_CLUSTER="$2" ;;
    --person) PERSON="$2" ;;
    --assistant) ASSISTANT="$2" ;;
    *) die "unknown option: $1" ;;
  esac
  shift 2
done

# signalled <exit>: SIGHUP, SIGINT or SIGTERM ends the run BROKEN (exit 2): the running child's
# process tree is stopped (a palace stage with TERM, a --send or --page with KILL, so it cannot
# finish its delivery) and the lock released first (a KILL that follows leaves no lock), then the
# line is paged, bounded to 5 s whatever --send-timeout is, and printed once; further signals are
# ignored.
# shellcheck disable=SC2329 # invoked by the traps below
signalled() {
  trap '' HUP INT TERM PIPE
  ending=TERM
  [ "$DELIVERING" -eq 0 ] || ending=KILL
  [ -z "$CHILD" ] || stop_tree "$ending" "$CHILD"
  [ -z "$WATCHER" ] || stop_tree TERM "$WATCHER"
  CHILD="" WATCHER=""
  release_lock
  [ "$SEND_TIMEOUT" -le 5 ] || SEND_TIMEOUT=5
  line="CRON BROKEN stage=signal exit=$1"
  page "$line"
  echo "$line$ROLLUP_INVALID" >&9
  exit 2
}
trap 'signalled 129' HUP
trap 'signalled 130' INT
trap 'signalled 143' TERM

# stop_tree <TERM|KILL> <pid>: the process and all its descendants: level by level each is frozen
# (STOP) and its children listed (at most 32 levels); only when every level is frozen is each sent
# the signal and CONT, so no child can act, or be orphaned, in between. KILL ends a frozen process
# without its running again (no handler, no delivery). Lists are kept in variables.
stop_tree() {
  sig=$1
  shift
  level=$1 all=$1 depth=0
  while [ -n "$level" ] && [ "$depth" -lt 32 ]; do
    next=""
    for pid in $level; do
      kill -STOP "$pid" 2>/dev/null
      for child in $(pgrep -P "$pid"); do next="$next $child"; done
    done
    level=$next all="$all$next" depth=$((depth + 1))
  done
  for pid in $all; do
    kill -"$sig" "$pid" 2>/dev/null
    kill -CONT "$pid" 2>/dev/null
  done
}

# absolute_dir <dir> <flag>: the existing directory, symlinks resolved.
absolute_dir() {
  [ -d "$1" ] || die "$2 $1 is not a directory"
  (cd "$1" && pwd -P) || die "cannot enter $2 $1"
}

# absolute <path>: the path made absolute (not resolved).
absolute() {
  case "$1" in /*) printf '%s\n' "$1" ;; *) printf '%s/%s\n' "$(pwd)" "$1" ;; esac
}

# link_dir <file>: when the file is a symlink, the real directory of its target (palace resolves
# links before its roots check, and an install may link its scripts in: the live settle home
# does, SETTLE-1c §4); else nothing.
link_dir() {
  [ -L "$1" ] || return 0
  (cd "$(dirname "$1")" && cd "$(dirname "$(readlink "$1")")" && pwd -P) || true
}

# resolved <path> <flag>: the path absolute, its nearest existing directory with symlinks resolved
# (`cd` and `pwd -P`) and the rest appended; nothing is created.
resolved() {
  dir=$(absolute "$1") rest=""
  while [ ! -d "$dir" ]; do
    rest="/$(basename "$dir")$rest"
    dir=$(dirname "$dir")
  done
  top=$(cd "$dir" && pwd -P) || die "cannot enter $2 $1"
  printf '%s%s\n' "${top%/}" "$rest"
}

# inside <path> <dir>: the path is the dir or lies under it (both resolved).
inside() {
  case "$1/" in "$2"/*) return 0 ;; esac
  return 1
}

# The state directory outside the diary root and the palace (M5b-11 item 4): each commit mines
# the diary root, which would make the kept attempts under <state>/rollup-attempts memory.
state_outside() {
  state_at=$(resolved "$STATE" --state) || exit 2
  for store in "diary-root:$DIARY" "palace:$PALACE"; do
    if inside "$state_at" "${store#*:}"; then
      die "--state $STATE is inside --${store%%:*} ${store#*:}; it must stay outside the diary root and the palace"
    fi
  done
}

check_arguments() {
  for pair in "palace:$PALACE" "diary-root:$DIARY" "kg:$KG" "palace-bin:$PALACE_BIN" \
    "send:$SEND" "page:$PAGE" "state:$STATE"; do
    [ -n "${pair#*:}" ] || die "--${pair%%:*} is required"
  done
  for count in "retries:$RETRIES" "retry-delay:$DELAY" "send-timeout:$SEND_TIMEOUT"; do
    case "${count#*:}" in ''|*[!0-9]*) die "--${count%%:*} must be a whole number" ;; esac
  done
  [ "$SEND_TIMEOUT" -ge 1 ] || die "--send-timeout must be at least 1"
  [ "$ROLLUP" -eq 0 ] || [ -n "$LLM" ] || die "--rollup needs --llm (no model is assumed)"
  [ "$ROLLUP" -eq 0 ] || [ "$DELIVER" != only ] || die "--deliver-only runs no rollup: drop --rollup"
  [ -f "$KG" ] || die "--kg $KG is not a file"
  [ -d "$DIARY/diary" ] || die "--diary-root $DIARY has no diary/"
  [ -f "$CANARIES" ] || die "missing $CANARIES"
  [ -f "$CHECK" ] || die "missing $CHECK"
  for exe in "palace-bin:$PALACE_BIN" "send:$SEND" "page:$PAGE"; do
    [ -f "${exe#*:}" ] && [ -x "${exe#*:}" ] || die "--${exe%%:*} ${exe#*:} is not an executable file"
  done
}

# Every path made absolute before the palace commands run from $ROOT (whose relative defaults,
# prompts/ and fixtures/, are the release's own).
resolve_paths() {
  PALACE=$(absolute_dir "$PALACE" --palace) || exit 2
  DIARY=$(absolute_dir "$DIARY" --diary-root) || exit 2
  KG_DIR=$(absolute_dir "$(dirname "$KG")" --kg) || exit 2
  KG="$KG_DIR/$(basename "$KG")"
  STORE="$KG_DIR/compiler.sqlite"
  PALACE_BIN=$(absolute "$PALACE_BIN") SEND=$(absolute "$SEND") PAGE=$(absolute "$PAGE")
  CANARIES=$(absolute "$CANARIES") CHECK=$(absolute "$CHECK")
  CANARIES_DIR=$(dirname "$CANARIES") CHECK_DIR=$(dirname "$CHECK") CHECK_LINKED=$(link_dir "$CHECK")
  JUDGE_DIR=""
  case "$JUDGE" in table:*) JUDGE_DIR=$(absolute_dir "$(dirname "${JUDGE#table:}")" --judge) || exit 2
    JUDGE="table:$JUDGE_DIR/$(basename "${JUDGE#table:}")" ;;
  esac
  state_outside
  mkdir -p "$STATE" || state_broken "cannot create --state $STATE"
  STATE=$(absolute_dir "$STATE" --state) || exit 2
  LOCK="$KG_DIR/.palace-cron.lock" JOURNAL="$STATE/sent-unmarked" SINCE_FILE="$STATE/digest-since"
  FAILURES="$STATE/send-failures"
  MEMPALACE_PALACE_PATH="$PALACE"
  export MEMPALACE_PALACE_PATH
}

# page <line>: the line to --page (bounded); a failed page is reported on stderr only.
page() {
  deliver "$PAGE" "$1"
  code=$?
  [ "$code" -eq 0 ] || echo "palace-cron: --page $PAGE failed (exit $code) for: $1" >&2
}

# deliver <exe> <text>: exe with the text on its standard input (a pipe), its stdout discarded,
# ended with its whole process tree after --send-timeout seconds (then 124). Waited in the
# background, so a signal is handled at once (then ended with KILL: DELIVERING). Writes nothing
# under --state. HUP, INT and TERM are default again in it (signalled ignores them before its
# page), so the page can be stopped.
deliver() {
  DELIVERING=1
  ( trap - HUP INT TERM; printf '%s\n' "$2" | "$1" ) </dev/null >/dev/null 9>&- &
  CHILD=$!
  timeout_watch "$CHILD" </dev/null >/dev/null 2>&1 9>&- &
  WATCHER=$!
  wait "$CHILD"
  code=$?
  CHILD="" DELIVERING=0
  kill "$WATCHER" 2>/dev/null
  wait "$WATCHER"
  watched=$?
  WATCHER=""
  [ "$watched" -ne 124 ] || return 124
  return "$code"
}

# timeout_watch <pid> (in the background): after --send-timeout seconds the pid's tree is ended
# with KILL (a send that ignores or handles TERM cannot deliver later) and it exits 124. TERM while it sleeps ends it (exit 0); once the stopping has begun TERM is
# ignored, so deliver waits for it to finish instead of cutting it short.
timeout_watch() {
  trap 'kill "$sleeper" 2>/dev/null; exit 0' TERM
  sleep "$SEND_TIMEOUT" &
  sleeper=$!
  wait "$sleeper" || exit 0
  trap '' TERM
  stop_tree KILL "$1"
  exit 124
}

# run <out> <err> <command…>: the command with no stdin and fd 9 closed, waited in the background
# (its pid in CHILD for the signal handler).
run() {
  out=$1 err=$2
  shift 2
  "$@" </dev/null >"$out" 2>"$err" 9>&- &
  CHILD=$!
  wait "$CHILD"
  code=$?
  CHILD=""
  return "$code"
}

pause() {
  [ "$1" -eq 0 ] || run /dev/null /dev/null sleep "$1"
}

# shellcheck disable=SC2329 # invoked through stage's "$@"
palace() {
  "$PALACE_BIN" --roots="$PALACE" --roots="$DIARY" --roots="$KG_DIR" --roots="$STATE" \
    --roots="$ROOT" --roots="$CANARIES_DIR" --roots="$CHECK_DIR" ${CHECK_LINKED:+"--roots=$CHECK_LINKED"} \
    ${JUDGE_DIR:+"--roots=$JUDGE_DIR"} \
    --mempalace-check "$CHECK" ${PERSON:+--person "$PERSON"} ${ASSISTANT:+--assistant "$ASSISTANT"} "$@"
}

# state_broken <what>: a failure to write under --state, or to take the lock for any reason but
# its being held: named on stderr, paged (the page does not need --state), BROKEN stage=state.
state_broken() {
  echo "palace-cron: $1" >&2
  finish "CRON BROKEN stage=state exit=2" 2 "CRON BROKEN stage=state exit=2"
}

# finish <line> <exit> [<page line>]: pages when given a page line, prints the summary (with
# " rollup=invalid" after an invalid rollup), exits.
finish() {
  [ $# -lt 3 ] || page "$3"
  echo "$1$ROLLUP_INVALID" >&9
  exit "$2"
}

# rollup_invalid: exit 4 of the rollup stage: paged (naming only the kept attempts' directory),
# and the night goes on.
rollup_invalid() {
  ROLLUP_INVALID=" rollup=invalid"
  page "CRON ROLLUP INVALID exit=4 kept=$STATE/rollup-attempts"
}

# stage <name> <command…>: rerun while it exits 3, at most --retries more times, --retry-delay
# apart; 0 (and 1 from watch: findings recorded) is done; 4 from rollup → rollup_invalid, done;
# 3 every time → RETRY; anything else → BROKEN. Output in <state>/last/<name>.out and .err.
stage() {
  name=$1 tries=0
  shift
  while :; do
    tries=$((tries + 1))
    run "$STATE/last/$name.out" "$STATE/last/$name.err" "$@"
    code=$?
    case "$name:$code" in *:0|watch:1) return 0 ;; rollup:4) rollup_invalid; return 0 ;; esac
    [ "$code" -eq 3 ] || finish "CRON BROKEN stage=$name exit=$code" 2 "CRON BROKEN stage=$name exit=$code"
    [ "$tries" -le "$RETRIES" ] ||
      finish "CRON RETRY stage=$name tries=$tries" 3 "CRON RETRY stage=$name exit=3 tries=$tries"
    echo "palace-cron: $name: retry later (exit 3); run $((tries + 1)) in $DELAY s" >&2
    pause "$DELAY"
  done
}

# take_lock: one run per store, the lock beside it (mkdir is atomic; the lock holds the pid), so
# two runs with different --state directories exclude each other too. Only a lock that exists is
# held: "retry later" like a stage's exit 3; a stale one is named, never broken automatically.
# Any other failure to take it is BROKEN stage=state.
take_lock() {
  tries=0
  while ! mkdir "$LOCK" 2>/dev/null; do
    if [ ! -d "$LOCK" ]; then
      mkdir "$LOCK" 2>/dev/null && break
      state_broken "cannot create the lock $LOCK"
    fi
    tries=$((tries + 1))
    holder=$(cat "$LOCK/pid" 2>/dev/null)
    if [ -n "$holder" ] && kill -0 "$holder" 2>/dev/null; then
      echo "palace-cron: $LOCK is held by pid $holder, which is running; retry later" >&2
    else
      echo "palace-cron: $LOCK is held by pid ${holder:-?}, not running: if no run is active, remove it: rm -r '$LOCK'" >&2
    fi
    [ "$tries" -le "$RETRIES" ] ||
      finish "CRON RETRY stage=lock tries=$tries" 3 "CRON RETRY stage=lock exit=3 tries=$tries"
    pause "$DELAY"
  done
  LOCKED=1
  echo "$$" >"$LOCK/pid" || state_broken "cannot write $LOCK/pid"
  { rm -rf "$STATE/last" && mkdir "$STATE/last"; } || state_broken "cannot create $STATE/last"
}
# shellcheck disable=SC2329 # invoked by signalled and release
release_lock() {
  [ "$LOCKED" -eq 0 ] || rm -rf "$LOCK"
  LOCKED=0
}
# shellcheck disable=SC2329 # invoked by the EXIT trap
release() {
  trap '' HUP INT TERM PIPE
  release_lock
}

# shellcheck disable=SC2329 # invoked through stage's "$@"
watch_with_judge() {
  set -- watch --diary "$DIARY/diary" --all --store "$STORE" --reports "$STATE/watch-reports" \
    --canaries "$CANARIES"
  case "$JUDGE" in
    "") palace "$@" --no-judge ;;
    claude-cli:*) palace "$@" --judge claude-cli --model "${JUDGE#claude-cli:}" ;;
    *) palace "$@" --judge "$JUDGE" ;;
  esac
}

run_stages() {
  [ "$ROLLUP" -eq 0 ] || stage rollup palace rollup --diary "$DIARY" --palace "$PALACE" \
    --kg "$KG" --llm "$LLM" --prompts "$ROOT/prompts" --canaries "$CANARIES" \
    --keep "$STATE/rollup-attempts" ${WING:+--wing "$WING"} ${MAX_CLUSTER:+--max-cluster "$MAX_CLUSTER"}
  stage watch watch_with_judge
  stage auto palace improve --auto --kg "$KG" --palace "$PALACE" --diary "$DIARY" \
    --store "$STORE" ${JUDGE:+--judge "$JUDGE"}
}

# mark <token>: --mark-sent, then the token leaves the journal of sent-but-unmarked asks.
mark() {
  stage mark palace improve --mark-sent "$1" --store "$STORE"
  : >"$JOURNAL.new" || state_broken "cannot write $JOURNAL.new"
  grep -vxF "$1" "$JOURNAL" >>"$JOURNAL.new"
  mv "$JOURNAL.new" "$JOURNAL" || state_broken "cannot replace $JOURNAL"
}

# send_asks: first the asks a run sent but could not mark (the journal), then every unsent line to
# --send; exit 0 → journal, then --mark-sent. An ask is never marked without a delivered send and
# never sent again once delivered (the journal outlives a failed mark).
send_asks() {
  [ -f "$JOURNAL" ] || : >"$JOURNAL" || state_broken "cannot create $JOURNAL"
  cp "$JOURNAL" "$STATE/last/journal" || state_broken "cannot copy $JOURNAL"
  while IFS= read -r token; do mark "$token"; done <"$STATE/last/journal"
  stage unsent palace improve --unsent --store "$STORE"
  ASKS=0 PENDING=0
  while IFS= read -r line; do
    token=$(printf '%s\n' "$line" | sed -n 's/.*\(#f_[0-9a-f]\{8\}\).*/\1/p')
    [ -n "$token" ] || finish "CRON BROKEN stage=asks exit=2" 2 "CRON BROKEN stage=asks exit=2"
    ! grep -qxF "$token" "$JOURNAL" || continue
    PENDING=$((PENDING + 1))
    send_text "$line" || continue
    # A journal that cannot be written: the delivered ask is still marked, then the run breaks.
    echo "$token" >>"$JOURNAL" ||
      { stage mark palace improve --mark-sent "$token" --store "$STORE"; state_broken "cannot write $JOURNAL"; }
    mark "$token"
    ASKS=$((ASKS + 1))
  done <"$STATE/last/unsent.out"
}

# send_text <text>: the text to --send; a failure is counted and stays on stderr (the operator's
# log); the text itself is never paged.
send_text() {
  deliver "$SEND" "$1"
  code=$?
  [ "$code" -eq 0 ] && return 0
  FAILED=$((FAILED + 1)) FAILED_CODE=$code
  echo "palace-cron: --send failed (exit $code); left for the next run" >&2
  return 1
}

# count_failures: the runs in a row with a failed delivery, kept in <state>/send-failures (reset
# by a run without one); from the second, one UNDELIVERED page naming no ask and no digest. A file
# holding anything but a plain decimal number without a leading zero, of at most nine digits (so
# the shell's arithmetic can hold it), reads as 0.
count_failures() {
  runs=0
  if [ "$FAILED" -gt 0 ]; then
    runs=$(cat "$FAILURES" 2>/dev/null)
    case "$runs" in ''|*[!0-9]*|0?*|??????????*) runs=0 ;; esac
    runs=$((runs + 1))
  fi
  echo "$runs" >"$FAILURES" || state_broken "cannot write $FAILURES"
  [ "$runs" -lt 2 ] || page "CRON UNDELIVERED failed=$FAILED exit=$FAILED_CODE runs=$runs"
}

# send_digest: the corrections since the previous delivered digest (the last 24 hours on the first
# run). Its window starts one second after this run's own events, so no event is in two digests.
send_digest() {
  pause 1
  at=$(date -u +%Y-%m-%dT%H:%M:%S)
  since=$(cat "$SINCE_FILE" 2>/dev/null)
  stage digest palace improve --digest --store "$STORE" ${since:+--since "$since"}
  DIGEST=0
  if [ -s "$STATE/last/digest.out" ]; then
    PENDING=$((PENDING + 1))
    send_text "$(cat "$STATE/last/digest.out")" || return 0
    DIGEST=1
  fi
  echo "$at" >"$SINCE_FILE" || state_broken "cannot write $SINCE_FILE"
}

# stages: the stages this run takes, named on stderr when a delivery flag is given.
stages() {
  case "$DELIVER" in
    "") return 0 ;;
    only) list="asks digest" ;;
    *) list="watch auto"
      [ "$ROLLUP" -eq 0 ] || list="rollup $list"
      [ "$DELIVER" = none ] || list="$list asks" ;;
  esac
  echo "palace-cron: stages: $list" >&2
}

check_arguments
resolve_paths
cd "$ROOT" || die "cannot enter $ROOT"
trap release EXIT
take_lock
stages
ASKS=0 DIGEST=0 PENDING=0
[ "$DELIVER" = only ] || run_stages
[ "$DELIVER" = none ] || send_asks
[ "$DELIVER" = none ] || [ "$DELIVER" = asks ] || send_digest
[ "$DELIVER" = none ] || count_failures
[ "$PENDING" -gt 0 ] || [ -n "$ROLLUP_INVALID" ] || finish "CRON QUIET" 0
finish "CRON OK asks=$ASKS digest=$DIGEST failed=$FAILED" 0
