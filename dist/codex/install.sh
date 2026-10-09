#!/bin/sh
# shellcheck source-path=SCRIPTDIR
# install.sh — the context compiler's memory loop for Codex.
#
# Usage: dist/codex/install.sh [--dry-run] [--from-source] [--no-load] [--llm <spec>]
#          [--judge <spec>] [--codex-model <model>] [--wing <name>] [--exclude-wing <name>]…
#          [--person <name>] [--assistant <name>]
#   --dry-run       print each change, make none
#   --from-source   build the binaries with cargo (default: the release tarball's bin/)
#   --no-load       write the launchd plists under $CC_HOME/launchd/, load nothing
#   --llm, --judge  the nightly job's models (default: command:$CC_HOME/bin/codex-llm.sh)
#   --wing          the MemPalace wing of this memory (default memory): diary entries
#                   (palace_diary_write), `palace init` and the nightly rollups file in it;
#                   only this wing is compiled (an entry palace_diary_write files in another
#                   wing goes to $CC_HOME/wings/<wing>/diary/, which the nightly job never reads)
#   --exclude-wing  a private wing never recalled automatically (repeatable): the per-turn
#                   hook searches every wing but these
#   --person        your name as the memory records your corrections and replies (default:
#                   the one recorded by an earlier install, else asked at a terminal, else the
#                   login name)
#   --assistant     the assistant's name, whose queue holds the reversals (default: the one
#                   recorded, else asked, else codex)
#   --codex-model   the model codex-llm.sh pins (default: the top-level `model` of
#                   $CODEX_HOME/config.toml at install; none there: Codex's own default)
# Environment: CC_HOME (default ~/.context-compiler), CODEX_HOME (default ~/.codex).
# Checks every precondition first (codex, jq, a valid hooks.json, safe paths, the model name,
# no foreign cc-memory or cc-graph server, and install-core.sh's own through its dry run: the
# platform, uv, the MemPalace version), so a refused install changes nothing. Runs install-core.sh, then wires Codex, then the
# scheduler adapter for this OS last (wiring.sh `schedule`: dist/common/schedule-<os>.sh, launchd's
# settle and daily jobs on macOS; an OS without one is refused), so the jobs are loaded only once
# the rest is in place; an install that fails after its first change unloads them. The jobs' PATH gets the directory of codex
# (and of node, when codex is an npm script). Wiring: $CODEX_HOME/hooks.json gets a
# UserPromptSubmit hook ($CC_HOME/hooks/user-prompt.sh) and a SessionStart hook
# (session-start.sh, matcher startup|resume) and a SessionEnd hook (`session-end.sh codex`,
# timeout 3 s: a request for a settle run); `codex mcp add` registers cc-memory
# (mempalace-mcp, MEMPALACE_MCP_ALLOW_PEER_WRITER=1: it never holds the palace's mine lock for a
# session) and cc-graph (palace-mcp), and config.toml gets approval_mode = "approve" for
# their read tools, the diary write and the reply to the nightly job's questions; both recorded in $CC_HOME/install-state, and a server of
# either name that this installer did not add stops the install before any change. A PKG-1
# install's mempalace and palace-graph servers (its install.log shows the add) are removed in
# favour of the new names; a server of those names that the user added is left alone. The
# memory protocol (dist/codex/AGENTS-memory.md) goes between marker lines at the end of $CODEX_HOME/AGENTS.md. Each edited file is backed up
# first (<file>.cc-backup-<time>). A second run changes nothing and says so. One manual step
# remains, and every run (a dry run too) prints it as its last line: trust the hooks in Codex's
# /hooks, after any hook change, or Codex silently runs none of the new or changed ones.
set -eu
PROG=codex-install
HERE=$(cd "$(dirname "$0")" && pwd)
DIST=$(cd "$HERE/.." && pwd)
# shellcheck source=../common/lib.sh
. "$DIST/common/lib.sh"
# shellcheck source=../common/wiring.sh
. "$DIST/common/wiring.sh"

CORE_FLAGS="--agent codex" SCHEDULE_FLAGS="" LLM="" JUDGE="" CODEX_MODEL="" MODEL_SET=0 WING=memory
PERSON="" ASSISTANT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1 CORE_FLAGS="$CORE_FLAGS --dry-run" SCHEDULE_FLAGS="$SCHEDULE_FLAGS --dry-run" ;;
    --from-source) CORE_FLAGS="$CORE_FLAGS --from-source" ;;
    --no-load) SCHEDULE_FLAGS="$SCHEDULE_FLAGS --no-load" ;;
    --llm) [ $# -ge 2 ] || die "missing value for $1"; LLM=$2; shift ;;
    --judge) [ $# -ge 2 ] || die "missing value for $1"; JUDGE=$2; shift ;;
    --wing) [ $# -ge 2 ] || die "missing value for $1"; WING=$2; shift ;;
    --exclude-wing) [ $# -ge 2 ] || die "missing value for $1"; wing_name "$2"; CORE_FLAGS="$CORE_FLAGS --exclude-wing $2"; shift ;;
    --codex-model) [ $# -ge 2 ] || die "missing value for $1"; CODEX_MODEL=$2 MODEL_SET=1; shift ;;
    --person) [ $# -ge 2 ] || die "missing value for $1"; PERSON=$2; shift ;;
    --assistant) [ $# -ge 2 ] || die "missing value for $1"; ASSISTANT=$2; shift ;;
    -h | --help) sed -n '/^set -eu/q;3,$p' "$0"; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
  shift
done

CC_HOME=${CC_HOME:-$HOME/.context-compiler}
CODEX_HOME=${CODEX_HOME:-$HOME/.codex}
export CC_HOME CODEX_HOME
LLM=${LLM:-command:$CC_HOME/bin/codex-llm.sh}
JUDGE=${JUDGE:-command:$CC_HOME/bin/codex-llm.sh}
# Tools Codex runs without asking (approval_mode = "approve"): wiring.sh's MEMORY_APPROVED and
# GRAPH_APPROVED. Approved by an earlier install, no longer: its table is removed on upgrade.
NO_LONGER_APPROVED="cc-memory:mempalace_diary_write"

main() {
  preconditions
  [ "$DRY_RUN" = 1 ] || trap 'status=$?; [ "$status" = 0 ] || unload_job; exit "$status"' EXIT
  # shellcheck disable=SC2086 # the flags are single words
  part "$DIST/common/install-core.sh" --wing "$WING" $CORE_FLAGS
  codex_files
  make_dir "$CODEX_HOME" 700
  hooks_json
  mcp_servers
  protocol_block "$CODEX_HOME/AGENTS.md" "$HERE/AGENTS-memory.md"
  # shellcheck disable=SC2086
  schedule --llm "$LLM" --judge "$JUDGE" --extra-path "$JOB_PATH" \
    $SCHEDULE_FLAGS
  trap - EXIT
  summary
  # Last, a dry run too: Codex trusts a hook by its exact contents and silently skips a new or
  # changed one until the user trusts it.
  say "Codex: open /hooks${CODEX_HOME_HINT:-} and trust the context-compiler hooks (user-prompt.sh, session-start.sh, session-end.sh); until then Codex runs none of the new or changed ones."
}

# preconditions: everything that can refuse the install, before its first change.
preconditions() {
  safe_path "$CC_HOME"
  adapter "$DIST/common" >/dev/null
  safe_path "$CODEX_HOME"
  CODEX_BIN=$(command -v codex) || die "codex not found on PATH: install Codex first"
  command -v jq >/dev/null 2>&1 || die "jq not found (macOS 15 and later ship /usr/bin/jq)"
  if [ -f "$CODEX_HOME/hooks.json" ] && ! jq -e . "$CODEX_HOME/hooks.json" >/dev/null 2>&1; then
    die "$CODEX_HOME/hooks.json is not valid JSON: fix it, then run again"
  fi
  wing_name "$WING"
  codex_model
  job_path "$CODEX_BIN"
  for name in cc-memory cc-graph; do
    if codex mcp get "$name" >/dev/null 2>&1 && ! state_has "mcp-server $name"; then
      die "Codex already has an MCP server named $name that this installer did not add: rename or remove it (codex mcp remove $name), then run again"
    fi
  done
  names codex
  core_preconditions
}

# codex_model: the nightly model; --ignore-user-config hides config.toml's, so it is pinned here.
codex_model() {
  if [ "$MODEL_SET" = 0 ]; then
    CODEX_MODEL=$(awk -F '"' '/^[[:space:]]*\[/ { exit } /^[[:space:]]*model[[:space:]]*=/ { print $2; exit }' \
      "$CODEX_HOME/config.toml" 2>/dev/null) || CODEX_MODEL=""
  fi
  case "$CODEX_MODEL" in
    *[!A-Za-z0-9._:/-]*) die "unsupported model name: $CODEX_MODEL (pass --codex-model <model>)" ;;
  esac
}

codex_files() {
  copy_file "$HERE/codex-llm.sh" "$CC_HOME/bin/codex-llm.sh" 755
  write_text "$CC_HOME/codex-env.sh" "# written by dist/codex/install.sh
CODEX_BIN='$CODEX_BIN'
CODEX_HOME='$CODEX_HOME'
CODEX_LLM_MODEL='$CODEX_MODEL'"
  if [ -z "$CODEX_MODEL" ]; then
    say "codex-llm.sh: no model pinned (none in config.toml): Codex's own default; --codex-model <model> pins one"
  fi
  [ "$CODEX_HOME" = "$HOME/.codex" ] || CODEX_HOME_HINT=" (started with CODEX_HOME=$CODEX_HOME)"
}

hooks_json() {
  file="$CODEX_HOME/hooks.json"
  prompt_hook="$CC_HOME/hooks/user-prompt.sh"
  session_hook="$CC_HOME/hooks/session-start.sh"
  add_prompt=true add_session=true add_end=true
  has_hook "$file" "$prompt_hook" && add_prompt=false
  has_hook "$file" "$session_hook" && add_session=false
  has_hook "$file" "$CC_HOME/hooks/session-end.sh codex" && add_end=false
  if [ "$add_prompt$add_session$add_end" = falsefalsefalse ]; then
    say "hooks.json: the context-compiler hooks are present"
    return 0
  fi
  edit_begin "$file"
  change "add the context-compiler hooks to $file" write_hooks "$file"
  edit_end "$file"
}

write_hooks() {
  scratch="$1.cc-new"
  { cat "$1" 2>/dev/null || echo '{}'; } | hooks_added "$add_prompt" "$add_session" "$add_end" \
    "$(jq -cn --arg c "$prompt_hook" '{type: "command", command: $c, timeout: 5,
      statusMessage: "context-compiler: recalling memory", additionalContextLimit: 8000}')" \
    "$(jq -cn --arg c "$session_hook" '{type: "command", command: $c, timeout: 5}')" \
    "$(end_hook codex)" >"$scratch"
  cat "$scratch" >"$1"
  rm -f "$scratch"
}

mcp_servers() {
  MCP_EDIT=0
  find_mempalace
  legacy mempalace cc-memory
  legacy palace-graph cc-graph
  memory_server --env mcp cc-memory
  graph_server codex mcp cc-graph
  for pair in $NO_LONGER_APPROVED; do
    unapprove "${pair%%:*}" "${pair#*:}"
  done
  # shellcheck disable=SC2086 # the lists are words
  approve cc-memory $MEMORY_APPROVED
  # shellcheck disable=SC2086
  approve cc-graph $GRAPH_APPROVED
  [ "$MCP_EDIT" = 0 ] || edit_end "$CODEX_HOME/config.toml"
}

config_edit() {
  [ "$MCP_EDIT" = 1 ] && return 0
  edit_begin "$CODEX_HOME/config.toml"
  MCP_EDIT=1
}

# legacy <PKG-1 name> <new name>: remove the PKG-1 install's server of that name.
legacy() {
  legacy_server "$1" || return 0
  config_edit
  change "codex mcp remove $1 (a PKG-1 name; now $2)" codex mcp remove "$1"
}

# mcp <name> <codex mcp add arguments…>: register unless Codex already has <name> with these
# arguments (ours: the preconditions stopped on any other). Registered with other arguments (an
# earlier release; none recorded in $CC_HOME/state/mcp-<name>.args): removed, then added again.
mcp() {
  name=$1
  shift
  if codex mcp get "$name" >/dev/null 2>&1; then
    if [ "$(cat "$CC_HOME/state/mcp-$name.args" 2>/dev/null)" = "$*" ]; then
      say "codex mcp: $name registered"
      return 0
    fi
    config_edit
    change "codex mcp remove $name (registered with other arguments)" codex mcp remove "$name"
  fi
  config_edit
  change "codex mcp add $name $*" add_server "$name" "$@"
}

add_server() {
  codex mcp add "$@"
  state_add "mcp-server $1"
  name=$1
  shift
  printf '%s\n' "$*" >"$CC_HOME/state/mcp-$name.args"
}

# unapprove <server> <tool>: an approval an earlier install recorded and this one no longer gives.
unapprove() {
  state_has "mcp-approval $1 $2" || return 0
  table="[mcp_servers.$1.tools.$2]"
  if grep -qxF "$table" "$CODEX_HOME/config.toml" 2>/dev/null; then
    config_edit
    change "stop approving $1's $2 in $CODEX_HOME/config.toml" remove_tables "$CODEX_HOME/config.toml" "$table"
  fi
  state_remove "mcp-approval $1 $2"
}

# approve <server> <tool…>: a [mcp_servers.<server>.tools.<tool>] table with approval_mode =
# "approve" for each tool that has no table yet, recorded in $CC_HOME/install-state.
approve() {
  server=$1
  shift
  tables=""
  for tool in "$@"; do
    grep -qxF "[mcp_servers.$server.tools.$tool]" "$CODEX_HOME/config.toml" 2>/dev/null ||
      tables="$tables $tool"
  done
  [ -n "$tables" ] || return 0
  config_edit
  # shellcheck disable=SC2086 # the tools are words
  change "approve $server's tools in $CODEX_HOME/config.toml:$tables" add_approvals "$server" $tables
}

add_approvals() {
  server=$1
  shift
  for tool in "$@"; do
    printf '\n[mcp_servers.%s.tools.%s]\napproval_mode = "approve"\n' "$server" "$tool" \
      >>"$CODEX_HOME/config.toml"
    state_add "mcp-approval $server $tool"
  done
}

main
