#!/bin/sh
# shellcheck source-path=SCRIPTDIR
# install.sh — the context compiler's memory loop for Claude Code.
#
# Usage: dist/claude-code/install.sh [--dry-run] [--from-source] [--no-load] [--llm <spec>]
#          [--judge <spec>] [--wing <name>] [--exclude-wing <name>]… [--person <name>]
#          [--assistant <name>]
#   --dry-run       print each change, make none
#   --from-source   build the binaries with cargo (default: the release tarball's bin/)
#   --no-load       write the launchd plists under $CC_HOME/launchd/, load nothing
#   --llm, --judge  the nightly job's models (default: claude-cli:claude-sonnet-5, palace's own
#                   `claude -p` with no tools, no MCP servers and no user settings or hooks)
#   --wing          the MemPalace wing of this memory (default memory): diary entries
#                   (palace_diary_write), `palace init` and the nightly rollups file in it;
#                   only this wing is compiled (an entry palace_diary_write files in another
#                   wing goes to $CC_HOME/wings/<wing>/diary/, which the nightly job never compiles)
#   --exclude-wing  a private wing never recalled automatically (repeatable): the per-turn
#                   hook searches every wing but these
#   --person        your name as the memory records your corrections and replies (default:
#                   the one recorded by an earlier install, else asked at a terminal, else the
#                   login name)
#   --assistant     the assistant's name, whose queue holds the reversals (default: the one
#                   recorded, else asked, else claude)
# Environment: CC_HOME (default ~/.context-compiler); CLAUDE_CONFIG_DIR (Claude Code's own:
# settings.json and CLAUDE.md in it, default ~/.claude; its user MCP servers in
# $CLAUDE_CONFIG_DIR/.claude.json, or ~/.claude.json when it is unset).
# Checks every precondition first (claude, jq, valid settings.json and .claude.json, safe paths,
# no foreign cc-memory or cc-graph user server, and install-core.sh's own through its dry run:
# the platform, uv, the MemPalace version), so a refused install changes nothing. Runs
# install-core.sh, then wires Claude Code, then the scheduler adapter for this OS last (wiring.sh
# `schedule`: dist/common/schedule-<os>.sh, launchd's settle and daily jobs on macOS; an OS
# without one is refused), so the jobs are loaded only once the rest is in place; an install that
# fails after its first change unloads them.
# Wiring: settings.json gets a UserPromptSubmit hook ($CC_HOME/hooks/user-prompt.sh) and a
# SessionStart hook (session-start.sh, matcher startup|resume), timeout 5 s each, a SessionEnd
# hook (`session-end.sh claude-code`, timeout 3 s: a request for a settle run), and
# permissions.allow entries (mcp__<server>__<tool>) for the read tools, the diary write and the
# reply to the nightly job's questions, other keys kept; `claude mcp add -s user` registers
# cc-memory (mempalace-mcp, MEMPALACE_MCP_ALLOW_PEER_WRITER=1) and cc-graph (palace-mcp), the
# same servers as the Codex installer's; the memory protocol (dist/claude-code/CLAUDE-memory.md)
# goes between marker lines at the end of CLAUDE.md. Servers and permissions it added are
# recorded in $CC_HOME/install-state (claude-mcp-server, claude-permission); a user server of
# either name that this installer did not add stops the install before any change. The nightly
# job gets claude's directory (and node's, for an npm install) on its PATH, CLAUDE_CONFIG_DIR
# when set, and CLAUDE_CODE_SAFE_MODE=1; $CC_HOME/bin/claude links to the claude found, so the
# job runs that one. Each edited file is backed up first
# (<file>.cc-backup-<time>). A second run changes nothing and says so.
set -eu
PROG=claude-code-install
HERE=$(cd "$(dirname "$0")" && pwd)
DIST=$(cd "$HERE/.." && pwd)
# shellcheck source=../common/lib.sh
. "$DIST/common/lib.sh"
# shellcheck source=../common/wiring.sh
. "$DIST/common/wiring.sh"

CORE_FLAGS="--agent claude-code" SCHEDULE_FLAGS="" LLM="" JUDGE="" WING=memory
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
    --person) [ $# -ge 2 ] || die "missing value for $1"; PERSON=$2; shift ;;
    --assistant) [ $# -ge 2 ] || die "missing value for $1"; ASSISTANT=$2; shift ;;
    -h | --help) sed -n '/^set -eu/q;3,$p' "$0"; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
  shift
done

CC_HOME=${CC_HOME:-$HOME/.context-compiler}
CLAUDE_DIR=${CLAUDE_CONFIG_DIR:-$HOME/.claude}
CLAUDE_JSON=${CLAUDE_CONFIG_DIR:+$CLAUDE_CONFIG_DIR/.claude.json}
CLAUDE_JSON=${CLAUDE_JSON:-$HOME/.claude.json}
SETTINGS="$CLAUDE_DIR/settings.json"
export CC_HOME
MODEL=claude-sonnet-5
LLM=${LLM:-claude-cli:$MODEL}
JUDGE=${JUDGE:-claude-cli:$MODEL}

main() {
  preconditions
  [ "$DRY_RUN" = 1 ] || trap 'status=$?; [ "$status" = 0 ] || unload_job; exit "$status"' EXIT
  # shellcheck disable=SC2086 # the flags are single words
  part "$DIST/common/install-core.sh" --wing "$WING" $CORE_FLAGS
  claude_files
  make_dir "$CLAUDE_DIR" 700
  settings_json
  mcp_servers
  protocol_block "$CLAUDE_DIR/CLAUDE.md" "$HERE/CLAUDE-memory.md"
  # shellcheck disable=SC2086
  schedule --llm "$LLM" --judge "$JUDGE" --extra-path "$JOB_PATH" \
    --env CLAUDE_CODE_SAFE_MODE=1 ${CLAUDE_CONFIG_DIR:+--env "CLAUDE_CONFIG_DIR=$CLAUDE_CONFIG_DIR"} \
    $SCHEDULE_FLAGS
  trap - EXIT
  summary
  say "Next: start Claude Code${CLAUDE_CONFIG_DIR:+ with CLAUDE_CONFIG_DIR=$CLAUDE_CONFIG_DIR} (restart it if it is"
  say "running) so that it loads the hooks and the cc-memory and cc-graph servers; if it asks you to"
  say "review the hooks (/hooks), accept the three context-compiler hooks (user-prompt.sh, session-start.sh,"
  say "session-end.sh)."
}

# preconditions: everything that can refuse the install, before its first change.
preconditions() {
  safe_path "$CC_HOME"
  adapter "$DIST/common" >/dev/null
  safe_path "$CLAUDE_DIR"
  safe_path "$CLAUDE_JSON"
  CLAUDE_BIN=$(command -v claude) || die "claude not found on PATH: install Claude Code first"
  [ "$CLAUDE_BIN" != "$CC_HOME/bin/claude" ] || CLAUDE_BIN=$(readlink "$CLAUDE_BIN")
  command -v jq >/dev/null 2>&1 || die "jq not found (macOS 15 and later ship /usr/bin/jq)"
  for file in "$SETTINGS" "$CLAUDE_JSON"; do
    if [ -f "$file" ] && ! jq -e 'type == "object"' "$file" >/dev/null 2>&1; then
      die "$file is not a valid JSON object: fix it, then run again"
    fi
  done
  wing_name "$WING"
  model_spec "$LLM"
  model_spec "$JUDGE"
  job_path "$CLAUDE_BIN"
  for name in cc-memory cc-graph; do
    if has_server "$name" && ! state_has "claude-mcp-server $name"; then
      die "Claude Code already has a user MCP server named $name that this installer did not add: rename or remove it (claude mcp remove -s user $name), then run again"
    fi
  done
  names claude
  core_preconditions
}

# model_spec <spec>: a model spec the plist and palace take (no blank or quote).
model_spec() {
  case "$1" in
    *[!A-Za-z0-9._:/-]*) die "unsupported model spec: $1" ;;
  esac
}

# claude_files: what uninstall.sh needs to find this Claude Code again, and $CC_HOME/bin/claude, a
# link to the claude found now: the job's PATH starts with $CC_HOME/bin, so palace's claude-cli
# runs this one, never another claude in the pinned mempalace's directory.
claude_files() {
  link_file "$CLAUDE_BIN" "$CC_HOME/bin/claude"
  write_text "$CC_HOME/claude-env.sh" "# written by dist/claude-code/install.sh
CLAUDE_BIN='$CLAUDE_BIN'
CLAUDE_CONFIG_DIR='${CLAUDE_CONFIG_DIR:-}'"
}

# --- settings.json: the two hooks and the permissions ------------------------------------

settings_json() {
  prompt_hook="$CC_HOME/hooks/user-prompt.sh"
  session_hook="$CC_HOME/hooks/session-start.sh"
  end_command="$CC_HOME/hooks/session-end.sh claude-code"
  add_prompt=true add_session=true add_end=true
  has_hook "$SETTINGS" "$prompt_hook" && add_prompt=false
  has_hook "$SETTINGS" "$session_hook" && add_session=false
  has_hook "$SETTINGS" "$end_command" && add_end=false
  allow=$(missing_permissions)
  if [ "$add_prompt$add_session$add_end" = falsefalsefalse ] && [ -z "$allow" ]; then
    say "settings.json: the context-compiler hooks and permissions are present"
    return 0
  fi
  edit_begin "$SETTINGS"
  # shellcheck disable=SC2086 # the entries are words
  change "add the context-compiler hooks and permissions to $SETTINGS:$(printf ' %s' $allow)" \
    write_settings "$SETTINGS"
  edit_end "$SETTINGS"
}

# missing_permissions: the mcp__<server>__<tool> entries permissions.allow lacks, one a line.
missing_permissions() {
  for entry in $(permissions); do
    if ! { [ -f "$SETTINGS" ] && jq -e --arg e "$entry" '(.permissions.allow // []) | index($e)' \
      "$SETTINGS" >/dev/null 2>&1; }; then
      echo "$entry"
    fi
  done
}

# permissions: the tools Claude Code runs without asking, as the Codex installer approves them
# (wiring.sh's MEMORY_APPROVED and GRAPH_APPROVED).
# permissions: the tools Claude Code runs without asking, as the Codex installer approves them
# (wiring.sh's MEMORY_APPROVED and GRAPH_APPROVED).
permissions() {
  for tool in $MEMORY_APPROVED; do echo "mcp__cc-memory__$tool"; done
  for tool in $GRAPH_APPROVED; do echo "mcp__cc-graph__$tool"; done
}

write_settings() {
  scratch="$1.cc-new"
  # shellcheck disable=SC2086 # the entries are words
  { cat "$1" 2>/dev/null || echo '{}'; } | hooks_added "$add_prompt" "$add_session" "$add_end" \
    "$(jq -cn --arg c "$prompt_hook" '{type: "command", command: $c, timeout: 5}')" \
    "$(jq -cn --arg c "$session_hook" '{type: "command", command: $c, timeout: 5}')" \
    "$(end_hook claude-code)" | jq \
    --argjson allow "$(printf '%s\n' $allow | jq -R . | jq -sc 'map(select(length > 0))')" '
    if ($allow | length) > 0 then
        .permissions = (.permissions // {}) | .permissions.allow = ((.permissions.allow // []) + $allow)
      else . end' >"$scratch"
  cat "$scratch" >"$1"
  rm -f "$scratch"
  for entry in $allow; do
    state_add "claude-permission $entry"
  done
}

# --- the MCP servers, at user scope ------------------------------------------------------

# has_server <name>: Claude Code has a user MCP server <name> (.claude.json's mcpServers).
has_server() {
  [ -f "$CLAUDE_JSON" ] && jq -e --arg n "$1" '.mcpServers[$n] != null' "$CLAUDE_JSON" >/dev/null 2>&1
}

mcp_servers() {
  MCP_EDIT=0
  find_mempalace
  memory_server -e mcp cc-memory
  graph_server claude-code mcp cc-graph
  [ "$MCP_EDIT" = 0 ] || edit_end "$CLAUDE_JSON"
}

claude_json_edit() {
  [ "$MCP_EDIT" = 1 ] && return 0
  edit_begin "$CLAUDE_JSON"
  MCP_EDIT=1
}

# mcp <name> <claude mcp add arguments…>: register unless Claude Code already has <name> with
# these arguments (ours: the preconditions stopped on any other). Registered with other arguments
# (an earlier release; recorded in $CC_HOME/state/claude-mcp-<name>.args): removed, then added.
mcp() {
  name=$1
  shift
  if has_server "$name"; then
    if [ "$(cat "$CC_HOME/state/claude-mcp-$name.args" 2>/dev/null)" = "$*" ]; then
      say "claude mcp: $name registered"
      return 0
    fi
    claude_json_edit
    change "claude mcp remove -s user $name (registered with other arguments)" \
      "$CLAUDE_BIN" mcp remove -s user "$name"
  fi
  claude_json_edit
  change "claude mcp add -s user $name $*" add_server "$name" "$@"
}

add_server() {
  name=$1
  shift
  "$CLAUDE_BIN" mcp add -s user "$name" "$@"
  state_add "claude-mcp-server $name"
  printf '%s\n' "$*" >"$CC_HOME/state/claude-mcp-$name.args"
}

main
