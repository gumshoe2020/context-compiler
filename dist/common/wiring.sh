# shellcheck shell=sh
# wiring.sh — what the Codex and Claude Code installers share (sourced after lib.sh, POSIX sh):
# the checks before an install, the parts it runs, the two MCP servers and the tools approved on
# them, the three hooks and the memory protocol block. The installer sets PROG, DIST, CC_HOME,
# WING, PERSON, ASSISTANT, CORE_FLAGS and SCHEDULE_FLAGS first. The scheduler is reached only
# through `schedule` (its adapter for this OS, dist/common/schedule-<os>.sh).

BEGIN_MARK='<!-- context-compiler memory protocol: begin -->'
END_MARK='<!-- context-compiler memory protocol: end -->'
# The tools the agent runs without asking: reads, the diary write (cc-graph's palace_diary_write,
# which files the entry where the nightly job reads it) and the reply to the nightly job's
# questions (cc-graph's palace_reply, the user's own answer). The graph's writes (kg_add,
# kg_invalidate, set_entity, retire_entity) keep asking.
# shellcheck disable=SC2034 # used by the installers
MEMORY_APPROVED="mempalace_status mempalace_search mempalace_check_duplicate mempalace_get_drawer
  mempalace_list_drawers mempalace_list_wings mempalace_list_rooms mempalace_get_taxonomy
  mempalace_diary_read"
# shellcheck disable=SC2034
GRAPH_APPROVED="mempalace_kg_query mempalace_kg_entities mempalace_kg_timeline mempalace_kg_stats
  palace_diary_write palace_reply"
# cc-memory never holds the palace's mine lock for a whole session (MemPalace 3.5.0's
# mempalace-mcp takes it on its first status call or write and keeps it while it runs, so every
# palace_diary_write mine and the nightly job's mines failed while the agent was open); its own
# writes still take the lock one at a time (PKG-2 item 0).
PEER_WRITER=MEMPALACE_MCP_ALLOW_PEER_WRITER=1

# --- before the install ------------------------------------------------------------------

# names <default assistant>: --person and --assistant, else an earlier install's (env.sh), else
# asked at a terminal, else the login name and <default assistant>; passed on to install-core.sh,
# which records them.
names() {
  PERSON=${PERSON:-$(recorded PERSON)} ASSISTANT=${ASSISTANT:-$(recorded ASSISTANT)}
  [ -n "$PERSON" ] && [ -n "$ASSISTANT" ] || ask_names "$1"
  PERSON=${PERSON:-$(login_name)} ASSISTANT=${ASSISTANT:-$1}
  role_names "$PERSON" "$ASSISTANT"
  CORE_FLAGS="$CORE_FLAGS --person $PERSON --assistant $ASSISTANT"
}

ask_names() {
  [ "$DRY_RUN" = 0 ] && [ -t 0 ] && [ -r /dev/tty ] || return 0
  if [ -z "$PERSON" ]; then
    printf 'Your name, as the memory records your corrections [%s]: ' "$(login_name)"
    read -r PERSON </dev/tty || PERSON=""
  fi
  if [ -z "$ASSISTANT" ]; then
    printf "The assistant's name [%s]: " "$1"
    read -r ASSISTANT </dev/tty || ASSISTANT=""
  fi
  PERSON=$(printf '%s' "$PERSON" | LC_ALL=C tr '[:upper:]' '[:lower:]')
  ASSISTANT=$(printf '%s' "$ASSISTANT" | LC_ALL=C tr '[:upper:]' '[:lower:]')
}

# core_preconditions: install-core.sh's own checks (platform, uv, the MemPalace version), by its
# dry run, which changes nothing.
core_preconditions() {
  out="${TMPDIR:-/tmp}/cc-core-check.$$"
  # shellcheck disable=SC2086 # the flags are single words
  if ! "$DIST/common/install-core.sh" --dry-run --wing "$WING" $CORE_FLAGS >"$out" 2>&1; then
    tail -1 "$out" >&2
    rm -f "$out"
    exit 1
  fi
  rm -f "$out"
}

# job_path <tool>: JOB_PATH, the tool's directory for the job's PATH, and node's when the tool is
# a node script (an npm install), since launchd's PATH has neither.
job_path() {
  JOB_PATH=$(dirname "$1")
  if [ "$(head -c 2 "$1")" = '#!' ] && head -1 "$1" | grep -q node; then
    node=$(command -v node) || die "$(basename "$1") ($1) runs on node, and node is not on PATH"
    JOB_PATH="$JOB_PATH:$(dirname "$node")"
  fi
  safe_path "$JOB_PATH"
}

# --- the parts and the job ----------------------------------------------------------------

# schedule <adapter arguments…>: the scheduler adapter for this OS (lib.sh adapter), as a part.
# The installers wake their jobs only through it; an OS without an adapter is refused (their
# preconditions call `adapter` first, so the refusal comes before any change).
schedule() {
  part "$(adapter "$DIST/common")" "$@"
}

# unload_job: after a failed install, the jobs are not left running on a half install.
unload_job() {
  case " $SCHEDULE_FLAGS " in *" --no-load "*) return 0 ;; esac
  say "$PROG: the install failed: the jobs are unloaded"
  "$(adapter "$DIST/common")" --unload || true
}

# part <script> [args…]: run an installer part, show its output, count it as a change unless
# its last line says "nothing to change".
part() {
  out="${TMPDIR:-/tmp}/cc-part.$$"
  status=0
  "$@" >"$out" 2>&1 || status=$?
  cat "$out"
  tail -1 "$out" | grep -q 'nothing to change$' || CHANGES=$((CHANGES + 1))
  rm -f "$out"
  [ "$status" = 0 ] || exit "$status"
}

# --- the two MCP servers ------------------------------------------------------------------

# memory_server <env flag> <command…>: runs <command…> followed by cc-memory's environment
# (<env flag> NAME=value, twice), `--` and its command line (mempalace-mcp on the palace).
memory_server() {
  flag=$1
  shift
  "$@" "$flag" "MEMPALACE_PALACE_PATH=$CC_HOME/palace" "$flag" "$PEER_WRITER" -- \
    "$MEMPALACE_BIN_DIR/mempalace-mcp" --palace "$CC_HOME/palace"
}

# graph_server <agent> <command…>: runs <command…> followed by `--` and cc-graph's command line
# (palace-mcp: the graph tools, palace_diary_write as <agent>, palace_reply).
graph_server() {
  agent=$1
  shift
  "$@" -- "$CC_HOME/bin/palace-mcp" \
    --kg "$CC_HOME/palace/knowledge_graph.sqlite3" --palace "$CC_HOME/palace" \
    --diary-root "$CC_HOME/diary-root" --wing "$WING" --agent "$agent" \
    --mempalace "$MEMPALACE_BIN_DIR/mempalace" --mempalace-check "$CC_HOME/scripts/mempalace-check.sh" \
    --wings-root "$CC_HOME/wings" --person "$PERSON" --assistant "$ASSISTANT" \
    --palace-bin "$CC_HOME/bin/palace"
}

# --- the hooks ----------------------------------------------------------------------------

# has_hook <file> <command>: the JSON file has a hook running <command>.
has_hook() {
  [ -f "$1" ] && jq -e --arg c "$2" 'any(.. | objects | .command? ; . == $c)' "$1" >/dev/null 2>&1
}

# hooks_added <add prompt> <add session> <add end> <prompt hook> <session hook> <end hook>: the
# JSON on stdin with a UserPromptSubmit entry holding the prompt hook (a JSON object), a
# SessionStart entry (matcher startup|resume) holding the session hook and a SessionEnd entry
# holding the end hook, each when its <add …> is true.
hooks_added() {
  jq --argjson add_prompt "$1" --argjson add_session "$2" --argjson add_end "$3" \
    --argjson prompt "$4" --argjson session "$5" --argjson end "$6" '
    if $add_prompt or $add_session or $add_end then .hooks = (.hooks // {}) else . end
    | if $add_prompt then .hooks.UserPromptSubmit = ((.hooks.UserPromptSubmit // []) + [{hooks: [$prompt]}])
      else . end
    | if $add_session then .hooks.SessionStart = ((.hooks.SessionStart // []) + [{
        matcher: "startup|resume", hooks: [$session]}])
      else . end
    | if $add_end then .hooks.SessionEnd = ((.hooks.SessionEnd // []) + [{hooks: [$end]}])
      else . end'
}

# end_hook <source>: the SessionEnd hook object (session-end.sh: a request for a settle run),
# timeout 3 s (Codex allows at most 3).
end_hook() {
  jq -cn --arg c "$CC_HOME/hooks/session-end.sh $1" '{type: "command", command: $c, timeout: 3}'
}

# --- the memory protocol block ------------------------------------------------------------

# protocol_block <file> <source>: <source> (@WING@ filled in) between the marker lines at the end
# of <file>, or replacing an earlier block; nothing when the block is there as it is.
protocol_block() {
  file=$1
  block="${TMPDIR:-/tmp}/cc-block.$$"
  { echo "$BEGIN_MARK"; sed "s/@WING@/$WING/g" "$2"; echo "$END_MARK"; } >"$block"
  if [ -f "$file" ] && grep -qxF "$BEGIN_MARK" "$file"; then
    if marked_block "$file" | cmp -s - "$block"; then
      say "$(basename "$file"): the memory protocol block is present"
    else
      edit_begin "$file"
      change "replace the memory protocol block in $file" replace_block "$file"
      edit_end "$file"
    fi
  else
    edit_begin "$file"
    change "append the memory protocol block to $file" append_block "$file"
    edit_end "$file"
  fi
  rm -f "$block"
}

marked_block() {
  awk -v b="$BEGIN_MARK" -v e="$END_MARK" '$0 == b { on = 1 } on { print } $0 == e { on = 0 }' "$1"
}

append_block() {
  if [ -s "$1" ]; then
    [ "$(tail -c 1 "$1" | wc -l | tr -d ' ')" = 1 ] || echo >>"$1"
    echo >>"$1"
  fi
  cat "$block" >>"$1"
}

replace_block() {
  awk -v b="$BEGIN_MARK" -v e="$END_MARK" -v f="$block" '
    $0 == b { while ((getline line < f) > 0) print line; skip = 1; next }
    skip && $0 == e { skip = 0; next }
    !skip' "$1" >"$1.cc-new"
  cat "$1.cc-new" >"$1"
  rm -f "$1.cc-new"
}
