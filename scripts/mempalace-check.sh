#!/bin/sh
# mempalace-check.sh — ensure the pinned MemPalace is installed and working.
#
# Usage:  scripts/mempalace-check.sh [--install] [--palace <dir>] [--diary <dir>] [--quiet]
#   --install   (re)install the exact pin with uv if missing or at another version
#   --palace    palace directory (default: the owner's ~/.mempalace/openclaw-palace; pass it on
#               any other machine, as the installers do)
#   --diary     diary directory  (default: the owner's ~/.openclaw/mempalace-diary/diary; pass
#               it on any other machine)
# Exit 0 = healthy; 1 = a check failed; 2 = usage/tooling missing.
# Read-only except with --install. Never touches the palace or diary contents.
set -u
PIN="3.5.0"
PALACE="${HOME}/.mempalace/openclaw-palace"
DIARY="${HOME}/.openclaw/mempalace-diary/diary"
INSTALL=0; QUIET=0
while [ $# -gt 0 ]; do
  case "$1" in
    --install) INSTALL=1 ;;
    --palace) PALACE="$2"; shift ;;
    --diary) DIARY="$2"; shift ;;
    --quiet) QUIET=1 ;;
    -h|--help) sed -n 2,10p "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done
fail=0
say() { [ "$QUIET" = 1 ] || printf '%s\n' "$*"; }
ok()  { say "  ok    $*"; }
bad() { say "  FAIL  $*"; fail=1; }

say "mempalace-check (pin $PIN)"

# 1. uv
if command -v uv >/dev/null 2>&1; then ok "uv $(uv --version 2>/dev/null | head -1)"; else
  bad "uv not found (install: https://docs.astral.sh/uv/)"; exit 2; fi

# 2. exact version (install/repair if asked)
have=$(mempalace --version 2>/dev/null | awk '{print $2}')
if [ "$have" != "$PIN" ]; then
  if [ "$INSTALL" = 1 ]; then
    say "  ..    installing mempalace==$PIN with uv"
    uv tool install --force "mempalace==$PIN" >/dev/null 2>&1 || bad "uv tool install mempalace==$PIN failed"
    have=$(mempalace --version 2>/dev/null | awk '{print $2}')
  fi
fi
if [ "$have" = "$PIN" ]; then ok "mempalace $have (pinned)"; else bad "mempalace version is '${have:-absent}', pin is $PIN (run with --install)"; fi
command -v mempalace-mcp >/dev/null 2>&1 && ok "mempalace-mcp on PATH ($(command -v mempalace-mcp))" || bad "mempalace-mcp not on PATH"

# 3. palace layout
if [ -d "$PALACE" ]; then
  [ -f "$PALACE/chroma.sqlite3" ] && ok "chroma.sqlite3 present" || bad "chroma.sqlite3 missing in $PALACE"
  [ -f "$PALACE/knowledge_graph.sqlite3" ] && ok "knowledge_graph.sqlite3 present" || bad "knowledge_graph.sqlite3 missing in $PALACE"
  [ -f "$PALACE/mempalace_embedder.json" ] && ok "embedder identity recorded ($(tr -d '\n ' < "$PALACE/mempalace_embedder.json" | cut -c1-60))" || say "  warn  no mempalace_embedder.json (MemPalace will assume minilm)"
else
  bad "palace directory not found: $PALACE"
fi

# 4. diary
if [ -d "$DIARY" ]; then n=$(find "$DIARY" -maxdepth 1 -name '*.md' | wc -l | tr -d ' '); ok "diary: $n entries in $DIARY"; else bad "diary directory not found: $DIARY"; fi

# 5. KG integrity (read-only). A WAL graph that no connection holds has no -shm beside it, and
# `sqlite3 -readonly` cannot open it ("unable to open database file"): then it is opened with
# PRAGMA query_only, which reads alike and lets SQLite make its -shm (PKG-2).
kg_sql() {
  kg="$PALACE/knowledge_graph.sqlite3"
  out=$(sqlite3 -readonly "$kg" "$1" 2>&1)
  case "$out" in
    *"unable to open database file"*) [ -e "$kg-shm" ] || out=$(sqlite3 -cmd 'PRAGMA query_only=1' "$kg" "$1" 2>&1) ;;
  esac
  printf '%s\n' "$out"
}
if command -v sqlite3 >/dev/null 2>&1 && [ -f "$PALACE/knowledge_graph.sqlite3" ]; then
  r=$(kg_sql "PRAGMA integrity_check;" | head -1)
  [ "$r" = "ok" ] && ok "knowledge graph integrity_check ok ($(kg_sql 'select count(*) from triples where valid_to is null;') live triples)" || bad "knowledge graph integrity: $r"
fi

# 6. MCP handshake + smoke search (read-only; uses the real server the clients use)
if command -v mempalace-mcp >/dev/null 2>&1 && [ -d "$PALACE" ]; then
  out=$(printf '%s\n%s\n%s\n%s\n' \
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"mempalace-check","version":"0"}}}' \
    '{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}' \
    '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"mempalace_search","arguments":{"query":"health check smoke search","limit":1}}}' \
    | mempalace-mcp --palace "$PALACE" 2>/dev/null)
  # MemPalace's server emits JSON with spaces after colons ("id": 1); match both spellings.
  server_version=$(printf '%s\n' "$out" | python3 -c 'import sys,json; rows=[json.loads(line) for line in sys.stdin if line.strip()]; print(next((r.get("result",{}).get("serverInfo",{}).get("version","") for r in rows if r.get("id")==1),""))')
  [ "$server_version" = "$PIN" ] && ok "mempalace-mcp $server_version (pinned)" || bad "mempalace-mcp version '${server_version:-absent}', required $PIN"
  echo "$out" | grep -q 'mempalace_search' && ok "tools/list advertises mempalace_search" || bad "mempalace_search not advertised"
  echo "$out" | grep -q -E '"id": ?3,.*"content"' && ok "smoke search answered" || bad "smoke search failed"
fi

[ "$fail" = 0 ] && say "healthy" || say "UNHEALTHY"
exit "$fail"
