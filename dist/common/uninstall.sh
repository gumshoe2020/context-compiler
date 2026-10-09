#!/bin/sh
# shellcheck source-path=SCRIPTDIR
# uninstall.sh — remove what the installers added; keep the palace and the diary.
#
# Usage: $CC_HOME/dist/common/uninstall.sh [--dry-run] [--purge]
#   --dry-run  print each change, make none
#   --purge    also remove $CC_HOME/palace, $CC_HOME/diary-root, $CC_HOME/wings and the inbox
#              (all of $CC_HOME)
# Environment: CC_HOME (default ~/.context-compiler). CODEX_HOME: the one the Codex installer
# recorded in $CC_HOME/codex-env.sh, else $CODEX_HOME, else ~/.codex. CLAUDE_CONFIG_DIR: the one
# the Claude Code installer recorded in $CC_HOME/claude-env.sh, else $CLAUDE_CONFIG_DIR, else
# unset (~/.claude, and ~/.claude.json for the MCP servers).
# Stops and removes the scheduled jobs through the scheduler adapter for this OS
# (dist/common/schedule-<os>.sh --remove: on macOS the settle and daily launchd agents, and the
# nightly and compression agents of earlier installs). Every file an installer edited
# ($CC_HOME/state/edits) and that is unchanged since the last install is restored from the
# backup taken before the first install, byte for byte (or removed, when the installer created
# it), provided each later install's backup equals what the install before it wrote. Any other
# file (changed since, or between installs) is backed up, then loses only what was added: the hooks under $CC_HOME/hooks in hooks.json, the
# tool approval tables and the MCP servers recorded in $CC_HOME/install-state (`codex mcp
# remove`; a PKG-1 install's mempalace and palace-graph when its install.log shows the add), the
# marked block in AGENTS.md; for Claude Code, in settings.json the hooks under $CC_HOME/hooks
# and the permissions.allow entries recorded in $CC_HOME/install-state (claude-permission; an
# emptied list, permissions or hooks object goes too), the user MCP servers recorded there
# (claude-mcp-server: `claude mcp remove -s user`), the marked block in CLAUDE.md. A server or
# permission the installers did not add is never removed.
# Then removes $CC_HOME except palace/, diary-root/, wings/ and inbox.md (all of it with --purge).
# Backups (<file>.cc-backup-<time>) are kept.
set -eu
PROG=uninstall
HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/lib.sh"

PURGE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --purge) PURGE=1 ;;
    -h | --help) sed -n '/^set -eu/q;3,$p' "$0"; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
  shift
done

CC_HOME=${CC_HOME:-$HOME/.context-compiler}
CODEX_BIN=codex
# shellcheck source=/dev/null
[ ! -f "$CC_HOME/codex-env.sh" ] || . "$CC_HOME/codex-env.sh"
CODEX_HOME=${CODEX_HOME:-$HOME/.codex}
export CODEX_HOME
CLAUDE_BIN=claude
RECORDED_CLAUDE_DIR=""
if [ -f "$CC_HOME/claude-env.sh" ]; then
  RECORDED_CLAUDE_DIR=$(sed -n "s/^CLAUDE_CONFIG_DIR='\(.*\)'\$/\1/p" "$CC_HOME/claude-env.sh")
  CLAUDE_BIN=$(sed -n "s/^CLAUDE_BIN='\(.*\)'\$/\1/p" "$CC_HOME/claude-env.sh")
  CLAUDE_CONFIG_DIR=$RECORDED_CLAUDE_DIR
  if [ -n "$CLAUDE_CONFIG_DIR" ]; then export CLAUDE_CONFIG_DIR; else unset CLAUDE_CONFIG_DIR; fi
fi
CLAUDE_DIR=${CLAUDE_CONFIG_DIR:-$HOME/.claude}
CLAUDE_JSON=${CLAUDE_CONFIG_DIR:+$CLAUDE_CONFIG_DIR/.claude.json}
CLAUDE_JSON=${CLAUDE_JSON:-$HOME/.claude.json}
BEGIN_MARK='<!-- context-compiler memory protocol: begin -->'
END_MARK='<!-- context-compiler memory protocol: end -->'
TAB=$(printf '\t')
EDITS="$CC_HOME/state/edits"
RESTORED=""

main() {
  scheduled_jobs
  restore_edits
  hooks_json
  CONFIG_BACKED_UP=0
  approvals
  mcp_servers
  agents_block "$CODEX_HOME/AGENTS.md"
  claude_settings
  claude_mcp_servers
  agents_block "$CLAUDE_DIR/CLAUDE.md"
  cc_home
  summary
}

# scheduled_jobs: the settle and daily jobs (and the retired nightly and compression ones),
# stopped and removed by the scheduler adapter for this OS (lib.sh adapter).
scheduled_jobs() {
  out="${TMPDIR:-/tmp}/cc-schedule.$$"
  flag=""
  [ "$DRY_RUN" = 0 ] || flag=--dry-run
  status=0
  "$(adapter "$HERE")" --remove $flag >"$out" 2>&1 || status=$?
  cat "$out"
  tail -1 "$out" | grep -q 'nothing to change$' || CHANGES=$((CHANGES + 1))
  rm -f "$out"
  [ "$status" = 0 ] || die "the scheduler adapter failed (exit $status)"
}

# Each edited file, in the order of its first recorded edit.
restore_edits() {
  [ -f "$EDITS" ] || return 0
  paths="${TMPDIR:-/tmp}/cc-edits.$$"
  awk -F "$TAB" '!seen[$1]++ { print $1 }' "$EDITS" >"$paths"
  while IFS= read -r path; do
    restore "$path"
  done <"$paths"
  rm -f "$paths"
}

# restore <path>: the bytes from before the first install, when the file still holds what the
# last install wrote and every later backup holds what the install before it wrote (an unbroken
# chain). Else nothing here: the strip below removes what was added, by markers and names.
restore() {
  first="" previous="" chain=ok
  records=$(awk -F "$TAB" -v p="$1" '$1 == p' "$EDITS")
  while IFS="$TAB" read -r _ backup sum; do
    if [ -z "$first" ]; then
      first=$backup
    elif [ "$backup" = - ] || [ ! -f "$backup" ] || [ "$(sha256 "$backup")" != "$previous" ]; then
      chain=broken
    fi
    previous=$sum
  done <<EOF
$records
EOF
  if [ ! -f "$1" ] || [ "$(sha256 "$1")" != "$previous" ]; then
    say "$1: changed since the install; only the added parts are removed"
  elif [ "$chain" = broken ]; then
    say "$1: changed between two installs; only the added parts are removed"
  elif [ "$first" = - ]; then
    change "remove $1 (created by the installer, unchanged since)" rm -f "$1"
    RESTORED="$RESTORED$1$TAB"
  elif [ -f "$first" ]; then
    change "restore $1 from $first" cp -p "$first" "$1"
    RESTORED="$RESTORED$1$TAB"
  else
    say "$1: its backup $first is gone; only the added parts are removed"
  fi
}

restored() {
  case "$TAB$RESTORED" in *"$TAB$1$TAB"*) return 0 ;; esac
  return 1
}

hooks_json() {
  file="$CODEX_HOME/hooks.json"
  [ -f "$file" ] && ! restored "$file" || return 0
  jq -e --arg d "$CC_HOME/hooks/" 'any(.. | objects | .command? ; type == "string" and startswith($d))' \
    "$file" >/dev/null 2>&1 || return 0
  edit_begin "$file"
  change "remove the context-compiler hooks from $file" remove_hooks "$file"
}

# DROP_HOOKS (jq, $d = $CC_HOME/hooks/): every hook running a command under $d goes, then any
# entry or event left without hooks.
# shellcheck disable=SC2016 # jq's $d, not the shell's
DROP_HOOKS='.hooks |= (with_entries(.value |= (map(.hooks |= map(select(
    ((.command // "") | startswith($d)) | not))) | map(select(.hooks | length > 0))))
  | with_entries(select(.value | length > 0)))'

remove_hooks() {
  jq --arg d "$CC_HOME/hooks/" "$DROP_HOOKS" "$1" >"$1.cc-new"
  cat "$1.cc-new" >"$1"
  rm -f "$1.cc-new"
}

config_backup() {
  [ "$CONFIG_BACKED_UP" = 1 ] && return 0
  edit_begin "$CODEX_HOME/config.toml"
  CONFIG_BACKED_UP=1
}

# The [mcp_servers.<server>.tools.<tool>] tables recorded in $CC_HOME/install-state.
approvals() {
  file="$CODEX_HOME/config.toml"
  [ -f "$file" ] && [ -f "$CC_HOME/install-state" ] && ! restored "$file" || return 0
  tables=""
  while read -r kind server tool; do
    [ "$kind" = mcp-approval ] || continue
    table="[mcp_servers.$server.tools.$tool]"
    grep -qxF "$table" "$file" && tables="$tables $table"
  done <"$CC_HOME/install-state"
  [ -n "$tables" ] || return 0
  config_backup
  change "remove the tool approval tables from $file:$tables" remove_tables "$file" "$tables"
}

mcp_servers() {
  servers=$(added_servers)
  [ -n "$servers" ] || return 0
  if ! command -v "$CODEX_BIN" >/dev/null 2>&1; then
    say "codex not found: remove the MCP servers $(printf '%s' "$servers" | tr '\n' ' ')from $CODEX_HOME/config.toml by hand if present"
    return 0
  fi
  restored "$CODEX_HOME/config.toml" && return 0
  for name in $servers; do
    "$CODEX_BIN" mcp get "$name" >/dev/null 2>&1 || continue
    config_backup
    change "codex mcp remove $name" "$CODEX_BIN" mcp remove "$name"
  done
}

# added_servers: the servers recorded in $CC_HOME/install-state, and a PKG-1 install's.
added_servers() {
  [ ! -f "$CC_HOME/install-state" ] || sed -n 's/^mcp-server //p' "$CC_HOME/install-state"
  command -v "$CODEX_BIN" >/dev/null 2>&1 || return 0
  for name in mempalace palace-graph; do
    legacy_server "$name" && echo "$name"
  done
  return 0
}

# agents_block <file>: the memory protocol block (AGENTS.md, CLAUDE.md).
agents_block() {
  file=$1
  [ -f "$file" ] && ! restored "$file" && grep -qxF "$BEGIN_MARK" "$file" || return 0
  edit_begin "$file"
  change "remove the memory protocol block from $file" remove_block "$file"
}

# The block and the one blank line the installer put before it.
remove_block() {
  awk -v b="$BEGIN_MARK" -v e="$END_MARK" '
    $0 == b { held = 0; skip = 1; next }
    skip { if ($0 == e) skip = 0; next }
    held { print ""; held = 0 }
    $0 == "" { held = 1; next }
    { print }
    END { if (held) print "" }' "$1" >"$1.cc-new"
  cat "$1.cc-new" >"$1"
  rm -f "$1.cc-new"
}

# --- Claude Code ------------------------------------------------------------------------

# claude_settings: settings.json loses the hooks under $CC_HOME/hooks and the permissions the
# installer recorded.
claude_settings() {
  file="$CLAUDE_DIR/settings.json"
  [ -f "$file" ] && ! restored "$file" || return 0
  drop=$(recorded_permissions "$file")
  if [ "$drop" = '[]' ] && ! jq -e --arg d "$CC_HOME/hooks/" \
    'any(.. | objects | .command? ; type == "string" and startswith($d))' "$file" >/dev/null 2>&1; then
    return 0
  fi
  edit_begin "$file"
  change "remove the context-compiler hooks and permissions from $file" strip_settings "$file"
}

# recorded_permissions <settings>: the claude-permission entries of install-state that the
# file's permissions.allow holds, as a JSON list.
recorded_permissions() {
  sed -n 's/^claude-permission //p' "$CC_HOME/install-state" 2>/dev/null | jq -R . | jq -sc . |
    jq -c --slurpfile s "$1" '[.[] | select(. as $e | ($s[0].permissions.allow // []) | index($e))]'
}

strip_settings() {
  jq --arg d "$CC_HOME/hooks/" --argjson drop "$drop" "
    if .hooks then $DROP_HOOKS | if .hooks == {} then del(.hooks) else . end else . end"'
    | if .permissions.allow then .permissions.allow -= $drop else . end
    | if .permissions.allow == [] then del(.permissions.allow) else . end
    | if .permissions == {} then del(.permissions) else . end' "$1" >"$1.cc-new"
  cat "$1.cc-new" >"$1"
  rm -f "$1.cc-new"
}

# claude_mcp_servers: the user servers recorded in install-state, by `claude mcp remove -s user`.
claude_mcp_servers() {
  servers=$(sed -n 's/^claude-mcp-server //p' "$CC_HOME/install-state" 2>/dev/null)
  [ -n "$servers" ] || return 0
  restored "$CLAUDE_JSON" && return 0
  if ! command -v "$CLAUDE_BIN" >/dev/null 2>&1; then
    say "claude not found: remove the user MCP servers $(printf '%s' "$servers" | tr '\n' ' ')from $CLAUDE_JSON by hand if present"
    return 0
  fi
  backed_up=0
  for name in $servers; do
    if ! jq -e --arg n "$name" '.mcpServers[$n] != null' "$CLAUDE_JSON" >/dev/null 2>&1; then
      continue
    fi
    [ "$backed_up" = 1 ] || edit_begin "$CLAUDE_JSON"
    backed_up=1
    change "claude mcp remove -s user $name" "$CLAUDE_BIN" mcp remove -s user "$name"
  done
}

cc_home() {
  [ -d "$CC_HOME" ] || return 0
  if [ "$PURGE" = 1 ]; then
    change "remove $CC_HOME (palace and diary included: --purge)" rm -rf "$CC_HOME"
    return 0
  fi
  for item in bin uv hooks scripts prompts share dist logs launchd env.sh codex-env.sh \
    claude-env.sh state install-state install.log; do
    [ ! -e "$CC_HOME/$item" ] || change "remove $CC_HOME/$item" rm -rf "${CC_HOME:?}/$item"
  done
  say "kept: $CC_HOME/palace, $CC_HOME/diary-root, $CC_HOME/wings, $CC_HOME/inbox.md (--purge removes them)"
}

main
