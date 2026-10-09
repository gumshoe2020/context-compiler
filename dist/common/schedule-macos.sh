#!/bin/sh
# shellcheck source-path=SCRIPTDIR
# schedule-macos.sh — the scheduler adapter for macOS (launchd): the settle job and the daily job.
#
# Usage: dist/common/schedule-macos.sh --llm <spec> --judge <spec> [--extra-path <dirs>]
#          [--env <NAME=value>]… [--no-load] [--dry-run]
#        dist/common/schedule-macos.sh --unload [--no-load] [--dry-run]
#        dist/common/schedule-macos.sh --remove [--dry-run]
#   --llm      the rollup model (settle.sh --llm: consolidation, when due)
#   --judge    the watch's and improve --auto's judge (palace-cron.sh --judge)
#   --extra-path  directories (a:b) the jobs also need on PATH: codex's, node's
#   --env      a variable of the jobs' environment besides PATH (repeatable; the Claude Code
#              installer's CLAUDE_CONFIG_DIR and CLAUDE_CODE_SAFE_MODE)
#   --no-load  write the plists under $CC_HOME/launchd/, never into ~/Library/LaunchAgents, and
#              call no launchctl
#   --unload   boot out the two jobs (an install that failed after its first change); with
#              --no-load nothing
#   --remove   boot out and remove the two jobs and the retired ones (uninstall.sh)
# Environment: CC_HOME (default ~/.context-compiler; install-core.sh has run).
# Every OS has one adapter, dist/common/schedule-<os>.sh, with these modes (the installers and
# uninstall.sh call it through wiring.sh's `schedule`; nothing else calls launchd). This one
# writes into ~/Library/LaunchAgents and loads with `launchctl bootstrap gui/<uid>` (a changed
# plist is booted out first):
#   com.context-compiler.settle  WatchPaths $CC_HOME/state/settle.request (the SessionEnd hooks
#                                append to it): $CC_HOME/bin/settle.sh --llm <llm> -- <cron>
#   com.context-compiler.daily   05:00 every day: $CC_HOME/bin/settle.sh --daily --llm <llm> -- <cron>
# where <cron> is palace-cron.sh and its arguments (docs/RUNBOOK.md section 3, step 5, with this
# install's paths; --canaries because the release ships no fixtures):
#   $CC_HOME/scripts/palace-cron.sh --palace $CC_HOME/palace --diary-root $CC_HOME/diary-root
#   --kg $CC_HOME/palace/knowledge_graph.sqlite3 --palace-bin $CC_HOME/bin/palace
#   --send $CC_HOME/bin/notify-local.sh --page $CC_HOME/bin/notify-local.sh
#   --state $CC_HOME/state/cron --judge <judge> --retries 1 --retry-delay 60
#   --canaries $CC_HOME/share/canaries.json --wing <env.sh WING>
#   --person <env.sh PERSON> --assistant <env.sh ASSISTANT>
# settle.sh adds the delivery flag and, when consolidation is due, --rollup --llm <llm>.
# Retired on install (booted out, plist removed): com.context-compiler.nightly (PKG-1.1 to PKG-2)
# and com.context-compiler.compression (PKG-1); with --no-load only their plists under
# $CC_HOME/launchd go. PATH: $CC_HOME/bin, the pinned mempalace's directory, the --extra-path
# directories, /usr/bin, /bin. Output: $CC_HOME/logs/settle.out, .err and daily.out, .err.
set -eu
PROG=schedule-macos
HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/lib.sh"

JOBS="settle daily"
RETIRED="nightly compression"
LLM="" JUDGE="" NO_LOAD=0 EXTRA_PATH="" ENVS="" MODE=install
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --no-load) NO_LOAD=1 ;;
    --unload) MODE=unload ;;
    --remove) MODE=remove ;;
    --llm) [ $# -ge 2 ] || die "missing value for $1"; LLM=$2; shift ;;
    --judge) [ $# -ge 2 ] || die "missing value for $1"; JUDGE=$2; shift ;;
    --extra-path) [ $# -ge 2 ] || die "missing value for $1"; EXTRA_PATH=$2; shift ;;
    --env) [ $# -ge 2 ] || die "missing value for $1"; ENVS="${ENVS:+$ENVS }$2"; shift ;;
    -h | --help) sed -n '/^set -eu/q;3,$p' "$0"; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
  shift
done

CC_HOME=${CC_HOME:-$HOME/.context-compiler}
AGENTS="$HOME/Library/LaunchAgents"
[ "$NO_LOAD" = 0 ] || AGENTS="$CC_HOME/launchd"
DOMAIN="gui/$(id -u)"

main() {
  case "$MODE" in
    unload) unload_jobs ;;
    remove) remove_jobs ;;
    *) install_jobs ;;
  esac
}

install_jobs() {
  [ -n "$LLM" ] || die "--llm <spec> is required"
  [ -n "$JUDGE" ] || die "--judge <spec> is required"
  for pair in $ENVS; do
    env_word "$pair"
  done
  WING=memory PERSON=$(login_name) ASSISTANT=assistant
  if [ -f "$CC_HOME/env.sh" ]; then
    # shellcheck source=/dev/null
    . "$CC_HOME/env.sh"
  elif [ "$DRY_RUN" = 1 ]; then
    find_mempalace # what install-core.sh would record
  else
    die "$CC_HOME/env.sh missing: run install-core.sh first"
  fi
  make_dir "$CC_HOME/state/cron"
  make_dir "$AGENTS"
  for name in $RETIRED; do
    retire "com.context-compiler.$name"
  done
  job settle "$(watch_paths "$CC_HOME/state/settle.request")"
  job daily "$(daily 5 0)" --daily
  summary
}

unload_jobs() {
  [ "$NO_LOAD" = 0 ] || return 0
  for name in $JOBS; do
    if loaded "com.context-compiler.$name"; then
      change "launchctl bootout $DOMAIN/com.context-compiler.$name" launchctl bootout "$DOMAIN/com.context-compiler.$name"
    fi
  done
}

remove_jobs() {
  for name in $JOBS $RETIRED; do
    retire "com.context-compiler.$name"
  done
  summary
}

loaded() { launchctl print "$DOMAIN/$1" >/dev/null 2>&1; }

# job <name> <trigger> [--daily]: write the plist, then load it unless --no-load.
job() {
  name=$1 trigger=$2
  shift 2
  label="com.context-compiler.$name"
  file="$AGENTS/$label.plist"
  scratch="${TMPDIR:-/tmp}/cc-plist.$$"
  plist "$label" "$trigger" "$name" "$CC_HOME/bin/settle.sh" "$@" --llm "$LLM" -- \
    "$CC_HOME/scripts/palace-cron.sh" \
    --palace "$CC_HOME/palace" --diary-root "$CC_HOME/diary-root" \
    --kg "$CC_HOME/palace/knowledge_graph.sqlite3" --palace-bin "$CC_HOME/bin/palace" \
    --send "$CC_HOME/bin/notify-local.sh" --page "$CC_HOME/bin/notify-local.sh" \
    --state "$CC_HOME/state/cron" --judge "$JUDGE" --retries 1 --retry-delay 60 \
    --canaries "$CC_HOME/share/canaries.json" --wing "$WING" \
    --person "$PERSON" --assistant "$ASSISTANT" >"$scratch"
  changed=1
  cmp -s "$scratch" "$file" 2>/dev/null && changed=0
  copy_file "$scratch" "$file" 644
  rm -f "$scratch"
  load "$label" "$file" "$changed"
}

# retire <label>: boot out (unless --no-load) and remove an agent.
retire() {
  if [ "$NO_LOAD" = 0 ] && loaded "$1"; then
    change "launchctl bootout $DOMAIN/$1 (no longer used)" launchctl bootout "$DOMAIN/$1"
  fi
  [ ! -f "$AGENTS/$1.plist" ] || change "remove $AGENTS/$1.plist (no longer used)" rm -f "$AGENTS/$1.plist"
}

load() {
  [ "$NO_LOAD" = 1 ] && return 0
  if loaded "$1"; then
    if [ "$3" = 0 ]; then
      say "launchd: $1 loaded"
      return 0
    fi
    change "launchctl bootout $DOMAIN/$1" launchctl bootout "$DOMAIN/$1"
  fi
  change "launchctl bootstrap $DOMAIN $2" launchctl bootstrap "$DOMAIN" "$2"
}

daily() {
  printf '\t<key>StartCalendarInterval</key>\n\t<dict>\n\t\t<key>Hour</key>\n\t\t<integer>%s</integer>\n' "$1"
  printf '\t\t<key>Minute</key>\n\t\t<integer>%s</integer>\n\t</dict>' "$2"
}

watch_paths() {
  printf '\t<key>WatchPaths</key>\n\t<array>\n\t\t<string>%s</string>\n\t</array>' "$(xml "$1")"
}

# plist <label> <trigger> <log name> <argv…>
plist() {
  label=$1 trigger=$2 log=$3
  shift 3
  cat <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>$label</string>
	<key>ProgramArguments</key>
	<array>
EOF
  for arg in "$@"; do
    printf '\t\t<string>%s</string>\n' "$(xml "$arg")"
  done
  cat <<EOF
	</array>
	<key>EnvironmentVariables</key>
	<dict>
		<key>PATH</key>
		<string>$CC_HOME/bin:$MEMPALACE_BIN_DIR${EXTRA_PATH:+:$EXTRA_PATH}:/usr/bin:/bin</string>
EOF
  for pair in $ENVS; do
    printf '\t\t<key>%s</key>\n\t\t<string>%s</string>\n' "${pair%%=*}" "$(xml "${pair#*=}")"
  done
  cat <<EOF
	</dict>
$trigger
	<key>StandardOutPath</key>
	<string>$CC_HOME/logs/$log.out</string>
	<key>StandardErrorPath</key>
	<string>$CC_HOME/logs/$log.err</string>
</dict>
</plist>
EOF
}

# env_word <NAME=value>: an environment entry the plist can carry (no blank in the value).
env_word() {
  case "$1" in
    [A-Z_]*=*) ;;
    *) die "--env $1: NAME=value" ;;
  esac
  case "${1%%=*}" in *[!A-Z0-9_]*) die "--env $1: NAME=value" ;; esac
  safe_path "${1#*=}"
}

xml() { printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'; }

main
