#!/bin/sh
# settle.sh — a settle run, or with --daily the daily run (dev/SPEC-event-driven-settle.md §2, §6).
#
# Usage: $CC_HOME/bin/settle.sh [--daily] --llm <spec> -- <palace-cron.sh> <args…>
#   --llm    the rollup model, added as `--rollup --llm <spec>` when consolidation is due
# A settle run (the scheduler starts it when $CC_HOME/state/settle.request changes):
#   1. debounce: waits until settle.request has not grown for SETTLE_DEBOUNCE_SECONDS;
#   2. take: renames settle.request and appends it to settle.request.taken, dropping repeated
#      transcript paths (a trigger during the run makes a fresh settle.request, so the scheduler
#      runs again afterwards); nothing to take: SETTLE NONE, exit 0;
#   3. distill: `palace distill` on each taken transcript with complete lines after its recorded
#      position (state/distill.positions, `<source>\t<path>\t<lines read>`; none: from the
#      start), into its wing (env.sh WING, or the map in env.sh WING_MAP when set) with --llm; its
#      new position is kept in state/distill.pending; a transcript that cannot be read, or whose
#      distill fails, is named on stderr (log: logs/settle-distill.log) and kept at its old
#      position, so the daily run retries it; no new text: no palace command;
#      After each distill, `palace fidelity --distill <transcript>` (recall, observe only: spec §8);
#   4. nothing new: the diary entries (diary-root/diary and wings/*/diary, by content) are those
#      recorded in state/last-settle: SETTLE QUIET, exit 0, no palace or MemPalace command (the
#      distilled positions are recorded: a distill that filed nothing is done);
#   5. runs the mine of each diary root (retried while another process holds the palace's lock),
#      then <palace-cron.sh> <args…> with --no-digest inside ASKS_WINDOW (asks go out) or
#      --no-deliver-stages outside it, and --rollup --llm <spec> when consolidation is due:
#      SETTLE_COMPRESS_ENTRIES or more entries since the last consolidation, or the last one
#      SETTLE_COMPRESS_MAX_DAYS or more days old and at least one entry new;
#      After a run with --rollup, `palace fidelity --all` scores the new rollup. palace fidelity
#      takes palace-cron.sh's --judge and --page; its lines go to logs/settle-fidelity.log, and
#      its failure is named, never the run's;
#   6. records, only when palace-cron.sh exits 0: state/last-settle (the entries as step 4 found
#      them, so an entry written during the run is new to the next), state/last-compression
#      (`<day> <entries>`, the day being seconds since the epoch / 86400; after a consolidation,
#      or the first record, or a PKG-2 record without the count), the distilled transcripts'
#      positions into state/distill.positions, and drops the taken requests.
#      Any other exit leaves them: the next trigger or the daily run retries.
# The daily run (05:00): a settle run without the debounce when anything is pending (a request,
# taken requests, a transcript in state/distill.positions with undistilled text, new entries;
# else "settle: nothing pending"; it distills the taken and the undistilled transcripts), always
# --no-deliver-stages; then
# <palace-cron.sh> <args…> --deliver-only (the asks and the digest). Exits with the first
# failing palace-cron.sh code, else 0.
# Settings, from $CC_HOME/env.sh (each with its default; an invalid value is named on stderr and
# the default used): SETTLE_DEBOUNCE_SECONDS=60, DISTILL_MIN_TURNS=3 (palace distill --min-turns),
# SETTLE_COMPRESS_ENTRIES=40, SETTLE_COMPRESS_MAX_DAYS=3, FIDELITY_SAMPLE=10, FIDELITY_FLOOR=0.7,
# DISTILL_LLM (palace distill --llm; unset: the rollup model, --llm),
# DISTILL_RECALL_MIN_WORDS=8 (palace fidelity --distill --min-words: SETTLE-1c §1),
# DISTILL_RECALL_FOLLOWING=6 (palace fidelity --distill --following: SETTLE-1c §2),
# ASKS_WINDOW=08:00-21:00 (local time,
# start included, end excluded; SETTLE_NOW=HH:MM replaces the clock, for tests). One run at a
# time: state/settle.lock (a live holder is waited for, up to an hour: then SETTLE BUSY, exit 3;
# a dead one's lock is taken over). Prints palace-cron.sh's CRON line, then
# SETTLE RUN exit=<n> window=<in|out|daily> rollup=<0|1>; progress on stderr. TERM and HUP are
# passed on to palace-cron.sh, which pages and releases its lock.
set -u
CC_HOME=$(cd "$(dirname "$0")/.." && pwd)
STATE="$CC_HOME/state"
REQUEST="$STATE/settle.request"
TAKEN="$REQUEST.taken"
LAST_SETTLE="$STATE/last-settle"
RECORD="$STATE/last-compression"
LOCK="$STATE/settle.lock"
POSITIONS="$STATE/distill.positions"
PENDING="$STATE/distill.pending"
MINE_LOG="$CC_HOME/logs/settle-mine.log"
DISTILL_LOG="$CC_HOME/logs/settle-distill.log"
FIDELITY_LOG="$CC_HOME/logs/settle-fidelity.log"
DAILY=0 LLM=""

say() { printf 'settle: %s\n' "$*" >&2; }

main() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --daily) DAILY=1 ;;
      --llm) [ $# -ge 2 ] || usage; LLM=$2; shift ;;
      --) shift; break ;;
      *) usage ;;
    esac
    shift
  done
  [ -n "$LLM" ] && [ $# -gt 0 ] || usage
  settings
  take_lock
  trap release EXIT
  if [ "$DAILY" = 1 ]; then
    daily "$@"
  else
    settle settle "$@"
  fi
}

usage() {
  echo "settle.sh: usage: settle.sh [--daily] --llm <spec> -- <palace-cron.sh> <args…>" >&2
  exit 2
}

# settings: env.sh's values (and the pinned mempalace, the wing, the agent), each checked.
settings() {
  MEMPALACE_BIN_DIR="" WING="" AGENT="" WING_MAP="" PERSON="" ASSISTANT="" DISTILL_LLM=""
  # shellcheck source=/dev/null
  [ ! -f "$CC_HOME/env.sh" ] || . "$CC_HOME/env.sh"
  AGENT=${AGENT:-context-compiler}
  DEBOUNCE=$(number SETTLE_DEBOUNCE_SECONDS "${SETTLE_DEBOUNCE_SECONDS:-60}" 60)
  MIN_TURNS=$(number DISTILL_MIN_TURNS "${DISTILL_MIN_TURNS:-3}" 3)
  COMPRESS_ENTRIES=$(number SETTLE_COMPRESS_ENTRIES "${SETTLE_COMPRESS_ENTRIES:-40}" 40)
  COMPRESS_DAYS=$(number SETTLE_COMPRESS_MAX_DAYS "${SETTLE_COMPRESS_MAX_DAYS:-3}" 3)
  SAMPLE=$(number FIDELITY_SAMPLE "${FIDELITY_SAMPLE:-10}" 10)
  MIN_WORDS=$(number DISTILL_RECALL_MIN_WORDS "${DISTILL_RECALL_MIN_WORDS:-8}" 8)
  FOLLOWING=$(number DISTILL_RECALL_FOLLOWING "${DISTILL_RECALL_FOLLOWING:-6}" 6)
  FLOOR=${FIDELITY_FLOOR:-0.7}
  case "$FLOOR" in
    0 | 1 | 0.[0-9]* | 1.0*) ;;
    *) say "FIDELITY_FLOOR=$FLOOR is not a number from 0 to 1: 0.7 used"; FLOOR=0.7 ;;
  esac
  DISTILL_MODEL=${DISTILL_LLM:-$LLM}
  # The palace by its real path: palace refuses to write through a symlink, and an install may
  # link its palace in from elsewhere (the live settle home does).
  PALACE_DIR=$(cd "$CC_HOME/palace" 2>/dev/null && pwd -P) || PALACE_DIR="$CC_HOME/palace"
  WINDOW=${ASKS_WINDOW:-08:00-21:00}
  if ! minutes "${WINDOW%%-*}" >/dev/null || ! minutes "${WINDOW#*-}" >/dev/null; then
    say "ASKS_WINDOW=$WINDOW is not HH:MM-HH:MM: 08:00-21:00 used"
    WINDOW=08:00-21:00
  fi
}

# number <name> <value> <default>: the value when a whole number, else the default (named).
number() {
  case "$2" in
    '' | *[!0-9]* | 0?*) say "$1=$2 is not a whole number: $3 used"; echo "$3" ;;
    *) echo "$2" ;;
  esac
}

# minutes <HH:MM>: minutes since midnight; fails on anything else.
minutes() {
  case "$1" in [0-2][0-9]:[0-5][0-9]) ;; *) return 1 ;; esac
  hours=${1%%:*} mins=${1#*:}
  [ "${hours#0}" -le 23 ] 2>/dev/null || return 1
  echo $((${hours#0} * 60 + ${mins#0}))
}

# --- one run at a time ------------------------------------------------------------------------

take_lock() {
  waited=0
  while ! mkdir "$LOCK" 2>/dev/null; do
    [ -d "$STATE" ] || { echo "settle.sh: no $STATE" >&2; exit 2; }
    holder=$(cat "$LOCK/pid" 2>/dev/null)
    if [ -z "$holder" ] || ! kill -0 "$holder" 2>/dev/null; then
      say "the lock of pid ${holder:-?} (not running) is taken over"
      rm -rf "$LOCK"
      continue
    fi
    if [ "$waited" -ge 3600 ]; then
      echo "SETTLE BUSY pid=$holder"
      exit 3
    fi
    sleep 5
    waited=$((waited + 5))
  done
  echo "$$" >"$LOCK/pid"
}

# shellcheck disable=SC2329 # invoked by the EXIT trap
release() { rm -rf "$LOCK"; }

# --- the daily run ----------------------------------------------------------------------------

daily() {
  status=0
  if [ -s "$REQUEST" ] || [ -s "$TAKEN" ] || [ -s "$REQUEST.taking" ] || [ -n "$(undistilled)" ] ||
    entries_changed; then
    settle daily "$@" || status=$?
  else
    say "nothing pending"
  fi
  delivered=0
  cron "$@" --deliver-only || delivered=$?
  [ "$status" != 0 ] || status=$delivered
  return "$status"
}

# --- a settle run -----------------------------------------------------------------------------

# settle <settle|daily> <palace-cron.sh> <args…>
settle() {
  mode=$1
  shift
  JUDGE=$(cron_flag --judge "$@") PAGE_EXE=$(cron_flag --page "$@")
  [ "$mode" = daily ] || debounce
  if ! take && [ "$mode" = settle ]; then
    echo "SETTLE NONE"
    return 0
  fi
  distill "$mode"
  if ! entries_changed; then
    record_positions
    rm -f "$TAKEN"
    echo "SETTLE QUIET"
    return 0
  fi
  window=daily deliver=--no-deliver-stages
  if [ "$mode" = settle ]; then
    window=out
    if in_window; then window=in deliver=--no-digest; fi
  fi
  consolidation
  mine_diaries
  # shellcheck disable=SC2086 # ROLLUP is words
  cron "$@" "$deliver" $ROLLUP && status=0 || status=$?
  [ -z "$ROLLUP" ] || fidelity --all
  if [ "$status" = 0 ]; then
    record
  else
    say "palace-cron.sh exited $status: nothing recorded, the requests kept for the next run"
  fi
  echo "SETTLE RUN exit=$status window=$window rollup=$([ -n "$ROLLUP" ] && echo 1 || echo 0)"
  return "$status"
}

# debounce: until settle.request has not grown for DEBOUNCE seconds (it only ever grows).
debounce() {
  size=$(size_of "$REQUEST")
  while [ -e "$REQUEST" ]; do
    sleep "$DEBOUNCE"
    now=$(size_of "$REQUEST")
    [ "$now" != "$size" ] || return 0
    size=$now
  done
}

size_of() { if [ -e "$1" ]; then wc -c <"$1" | tr -d ' '; else echo none; fi; }

# take: settle.request (and a take a crash cut short) appended to settle.request.taken, then
# repeated transcript paths dropped; fails when nothing is taken.
take() {
  if [ -e "$REQUEST" ] && ! [ -e "$REQUEST.taking" ]; then
    mv -f "$REQUEST" "$REQUEST.taking" || { echo "settle.sh: cannot take $REQUEST" >&2; exit 2; }
  fi
  if [ -e "$REQUEST.taking" ]; then
    cat "$REQUEST.taking" >>"$TAKEN" || { echo "settle.sh: cannot write $TAKEN" >&2; exit 2; }
    rm -f "$REQUEST.taking"
  fi
  [ -s "$TAKEN" ] || { rm -f "$TAKEN"; return 1; }
  awk -F '\t' '!seen[$2]++' "$TAKEN" >"$TAKEN.new" && mv -f "$TAKEN.new" "$TAKEN"
}

# distill <settle|daily>: palace distill on each taken transcript (and, for the daily run, each
# recorded one with undistilled text); the new positions into PENDING, recorded by record().
distill() {
  rm -f "$PENDING"
  { [ ! -s "$TAKEN" ] || cat "$TAKEN"; [ "$1" != daily ] || undistilled; } |
    awk -F '\t' '!seen[$2]++' >"$PENDING.list"
  while IFS="$(printf '\t')" read -r source path; do
    if [ -z "$path" ] || [ ! -r "$path" ]; then
      say "distill unreadable: $source ${path:-(no transcript path)}"
      continue
    fi
    from=$(position "$path")
    [ "$(lines_of "$path")" -gt "$from" ] || continue # no new text: no palace command
    out=$(distill_one "$source" "$path")
    to=$(printf '%s\n' "$out" | sed -n 's/^DISTILL .* position=\([0-9][0-9]*\)$/\1/p' | tail -1)
    if [ -n "$to" ]; then
      say "distill $(printf '%s\n' "$out" | sed -n 's/^DISTILL \([a-z-]*\).*/\1/p' | tail -1): $source $path"
      fidelity --distill "$path"
    else
      say "distill failed: $source $path (kept at line $from; log: $DISTILL_LOG)"
      to=$from
    fi
    printf '%s\t%s\t%s\n' "$source" "$path" "$to" >>"$PENDING"
  done <"$PENDING.list"
  rm -f "$PENDING.list"
}

# distill_one <source> <transcript>: palace distill's stdout (its stderr into DISTILL_LOG).
distill_one() {
  wing_map=""
  [ -z "$WING_MAP" ] || wing_map="--roots=$(dirname "$WING_MAP")"
  # shellcheck disable=SC2086 # wing_map is one word or none
  palace --roots="$(dirname "$2")" $wing_map distill \
    --transcript "$2" --source "$1" --diary "$CC_HOME/diary-root" --wings "$CC_HOME/wings" \
    --transcripts "$CC_HOME/transcripts" --palace "$PALACE_DIR" \
    --kg "$PALACE_DIR/knowledge_graph.sqlite3" --wing "$WING" \
    ${WING_MAP:+--wing-map "$WING_MAP"} --positions "$POSITIONS" --llm "$DISTILL_MODEL" \
    --prompts "$CC_HOME/prompts" --min-turns "$MIN_TURNS" \
    --mempalace-mcp "$MEMPALACE_BIN_DIR/mempalace-mcp" --mempalace "$MEMPALACE_BIN_DIR/mempalace" \
    2>>"$DISTILL_LOG"
}

# linked_roots: a --roots word for the real location of each symlink at the top of $CC_HOME or in
# its scripts/ (an install may link its palace, diary root or scripts in from elsewhere, and
# palace checks every input against --roots after resolving links).
linked_roots() {
  for link in "$CC_HOME"/* "$CC_HOME"/scripts/*; do
    [ -L "$link" ] || continue
    if [ -d "$link" ]; then target=$(cd "$link" && pwd -P) || continue
    else target=$(cd "$(dirname "$link")" && cd "$(dirname "$(readlink "$link")")" && pwd -P) || continue
    fi
    printf ' --roots=%s' "$target"
  done
}

# palace [--roots=<dir>…] <command> <args…>: the install's palace, its names, the pinned MemPalace.
palace() {
  mkdir -p "$CC_HOME/logs" 2>/dev/null || true
  # shellcheck disable=SC2046 # linked_roots prints whole --roots= words, one per link
  MEMPALACE_PALACE_PATH="$PALACE_DIR" PATH="$MEMPALACE_BIN_DIR:$PATH" \
    perl -e 'alarm 1800; exec @ARGV; exit 127' "$CC_HOME/bin/palace" --roots="$CC_HOME" $(linked_roots) "$@" \
    --mempalace-check "$CC_HOME/scripts/mempalace-check.sh" \
    ${PERSON:+--person "$PERSON"} ${ASSISTANT:+--assistant "$ASSISTANT"}
}

# fidelity <args…>: palace fidelity (spec §8, observe only: a failure is named, never the run's).
fidelity() {
  judge_roots="" transcript_roots="" page="$PAGE_EXE" distill_flags=""
  case "$JUDGE" in table:*) judge_roots="--roots=$(dirname "${JUDGE#table:}")" ;; esac
  # Distillation is meant to drop chatter, so its recall is logged but never pages (live
  # 2026-10-06: "night night" counted as missing and paged the owner); consolidation pages.
  # Its concept sample draws only from substantive messages, each judged with the messages after
  # it (SETTLE-1c §1, §2).
  [ "$1" != --distill ] || {
    transcript_roots="--roots=$(dirname "$2")" page=""
    distill_flags="--min-words $MIN_WORDS --following $FOLLOWING"
  }
  # shellcheck disable=SC2086 # each is one word or none, distill_flags whole words
  palace $judge_roots $transcript_roots fidelity $distill_flags --diary "$CC_HOME/diary-root" \
    --kg "$PALACE_DIR/knowledge_graph.sqlite3" ${JUDGE:+--judge "$JUDGE"} \
    ${page:+--page "$page"} --sample "$SAMPLE" --floor "$FLOOR" \
    --prompts "$CC_HOME/prompts" "$@" >>"$FIDELITY_LOG" 2>&1 ||
    say "palace fidelity $1 failed (log: $FIDELITY_LOG); the run goes on"
}

# cron_flag <--name> <palace-cron.sh args…>: the value after --name, else nothing.
cron_flag() {
  name=$1
  shift
  while [ $# -ge 2 ]; do
    [ "$1" != "$name" ] || { printf '%s\n' "$2"; return 0; }
    shift
  done
}

# position <transcript>: its recorded position (lines read), 0 when none.
position() {
  awk -F '\t' -v p="$1" '$2 == p { n = $3 } END { print n + 0 }' "$POSITIONS" 2>/dev/null || echo 0
}

lines_of() { wc -l <"$1" | tr -d ' '; }

# undistilled: `<source>\t<path>` of each recorded transcript with lines after its position.
undistilled() {
  [ -s "$POSITIONS" ] || return 0
  while IFS="$(printf '\t')" read -r source path at; do
    [ -r "$path" ] && [ "$(lines_of "$path")" -gt "${at:-0}" ] && printf '%s\t%s\n' "$source" "$path"
  done <"$POSITIONS"
  return 0
}

# entries: one line per diary entry (path and checksum), of the default wing and every wing.
entries() {
  (cd "$CC_HOME" && find diary-root/diary wings -path '*/diary/*' -type f -name '*.md' \
    -exec cksum {} + 2>/dev/null | LC_ALL=C sort)
}

fingerprint() { entries | cksum | tr -d ' '; }

# entries_changed: the entries differ from state/last-settle (CURRENT: their fingerprint now).
entries_changed() {
  CURRENT=$(fingerprint)
  [ "$CURRENT" != "$(cat "$LAST_SETTLE" 2>/dev/null)" ]
}

# entry_count: the default wing's entries (the ones a rollup compiles).
entry_count() { (cd "$CC_HOME" && find diary-root/diary -type f -name '*.md' 2>/dev/null | wc -l | tr -d ' '); }

# in_window: the run's local time is inside ASKS_WINDOW.
in_window() {
  now=$(minutes "${SETTLE_NOW:-$(date +%H:%M)}") || { say "SETTLE_NOW=$SETTLE_NOW is not HH:MM"; return 1; }
  start=$(minutes "${WINDOW%%-*}") end=$(minutes "${WINDOW#*-}")
  if [ "$start" -le "$end" ]; then
    [ "$now" -ge "$start" ] && [ "$now" -lt "$end" ]
  else
    [ "$now" -ge "$start" ] || [ "$now" -lt "$end" ]
  fi
}

# consolidation: ROLLUP (the flags), and NEW_RECORD for record (empty: last-compression stays).
consolidation() {
  ROLLUP="" NEW_RECORD=""
  today=$(($(date +%s) / 86400))
  day="" count=""
  [ ! -f "$RECORD" ] || read -r day count <"$RECORD" || true
  case "$day" in '' | *[!0-9]*) day="" ;; esac
  case "$count" in '' | *[!0-9]*) count="" ;; esac
  [ -z "$day" ] || [ "$day" -le "$today" ] || day="" # a clock set back: start the count again
  if [ -z "$day" ]; then
    NEW_RECORD=today
    return 0
  fi
  new=1 # a PKG-2 record has no count: this run's new entries are new since it
  if [ -n "$count" ]; then
    new=$(($(entry_count) - count))
    [ "$new" -ge 0 ] || new=0
  else
    NEW_RECORD=$day
  fi
  if { [ -n "$count" ] && [ "$new" -ge "$COMPRESS_ENTRIES" ]; } ||
    { [ $((today - day)) -ge "$COMPRESS_DAYS" ] && [ "$new" -ge 1 ]; }; then
    ROLLUP="--rollup --llm $LLM" NEW_RECORD=today
    say "consolidation due: $new entries new, the last $((today - day)) days ago"
  fi
}

# record: after a clean run, the entries as they were when it began (one written during the run
# is new to the next), the consolidation, the requests done.
record() {
  printf '%s\n' "$CURRENT" >"$LAST_SETTLE.new" && mv -f "$LAST_SETTLE.new" "$LAST_SETTLE"
  if [ -n "$NEW_RECORD" ]; then
    [ "$NEW_RECORD" != today ] || NEW_RECORD=$today
    printf '%s %s\n' "$NEW_RECORD" "$(entry_count)" >"$RECORD.new" && mv -f "$RECORD.new" "$RECORD"
  fi
  record_positions
  rm -f "$TAKEN"
}

# record_positions: each distilled transcript's new line replaces its old one (a clean run only).
record_positions() {
  [ -s "$PENDING" ] || return 0
  touch "$POSITIONS"
  { awk -F '\t' 'NR == FNR { new[$2] = 1; next } !($2 in new)' "$PENDING" "$POSITIONS" && cat "$PENDING"; } \
    >"$POSITIONS.new" && mv -f "$POSITIONS.new" "$POSITIONS" && rm -f "$PENDING"
}

# --- the mine and palace-cron.sh --------------------------------------------------------------

# mine <dir> <wing>: mempalace mine, retried while the palace's lock is held by another process.
mine() {
  for wait in 30 30 -; do
    if MEMPALACE_PALACE_PATH="$PALACE_DIR" perl -e 'alarm 900; exec @ARGV; exit 127' \
      "$MEMPALACE_BIN_DIR/mempalace" --palace "$PALACE_DIR" mine "$1" --wing "$2" \
      --agent "$AGENT" >"$MINE_LOG.now" 2>&1; then
      cat "$MINE_LOG.now" >>"$MINE_LOG" 2>/dev/null || true
      return 0
    fi
    cat "$MINE_LOG.now" >>"$MINE_LOG" 2>/dev/null || true
    if [ "$wait" = - ] || ! grep -q " is held by " "$MINE_LOG.now"; then
      break
    fi
    sleep "$wait"
  done
  say "mempalace mine $1 --wing $2 failed (log: $MINE_LOG); the run goes on"
}

# mine_diaries: the diary root into WING, then each private wing's directory into its own wing.
mine_diaries() {
  [ -x "$MEMPALACE_BIN_DIR/mempalace" ] && [ -n "$WING" ] || return 0
  mkdir -p "$CC_HOME/logs" 2>/dev/null || true
  mine "$CC_HOME/diary-root" "$WING"
  for dir in "$CC_HOME"/wings/*/; do
    [ ! -d "${dir}diary" ] || mine "${dir%/}" "$(basename "$dir")"
  done
  rm -f "$MINE_LOG.now"
}

# cron <palace-cron.sh> <args…>: run it, TERM and HUP passed on; its exit code.
cron() {
  say "run: $*"
  "$@" &
  child=$!
  trap 'kill -TERM "$child" 2>/dev/null || true' TERM
  trap 'kill -HUP "$child" 2>/dev/null || true' HUP
  code=0
  wait "$child" || code=$?
  while kill -0 "$child" 2>/dev/null; do # wait returned early for a passed-on signal
    code=0
    wait "$child" || code=$?
  done
  trap - TERM HUP
  return "$code"
}

main "$@"
