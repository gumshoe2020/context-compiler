#!/bin/sh
# shellcheck source-path=SCRIPTDIR
# install-core.sh — the part every installer shares: binaries, pinned MemPalace, palace, diary.
#
# Usage: dist/common/install-core.sh [--dry-run] [--from-source] [--wing <name>]
#          [--exclude-wing <name>]… [--person <name>] [--assistant <name>] [--agent <name>]
#   --dry-run       print each change, make none
#   --wing          the MemPalace wing this memory files in (default memory; env.sh WING):
#                   `palace init` mines the diary into it, the nightly job its rollups
#   --exclude-wing  a private wing the per-turn hook never recalls (repeatable; env.sh
#                   EXCLUDE_WINGS); the hook searches every other wing
#   --person        the person's name, who corrects the memory and answers its asks (env.sh
#                   PERSON; default: the one recorded, else the login name)
#   --assistant     the assistant's name, whose queue holds the reversals (env.sh ASSISTANT;
#                   default: the one recorded, else assistant)
#   --agent         the MemPalace agent the diary is mined as (env.sh AGENT: the settle runs'
#                   mine, dist/common/settle.sh; default: the one recorded, else
#                   context-compiler); the installers pass their tool's name
#   --from-source   build palace and palace-mcp with cargo as the release does (lib.sh
#                   release_build; default: the release tarball's bin/)
# Environment: CC_HOME (default ~/.context-compiler).
# Checks macOS on Apple silicon and uv. MemPalace: a mempalace-mcp on PATH at the pin
# (mempalace==<PIN of scripts/mempalace-check.sh>) is used; one at another version stops the
# install with the command to run (a MemPalace is never replaced); none at all: uv installs the
# pin into $CC_HOME/uv (UV_TOOL_DIR=$CC_HOME/uv/tools, UV_TOOL_BIN_DIR=$CC_HOME/uv/bin). Copies the binaries, scripts, prompts, the
# empty canaries file and dist/ into $CC_HOME; creates $CC_HOME/palace and $CC_HOME/diary-root;
# runs `palace init` once; migrates an existing claim store to the memory types once (SETTLE-1b,
# spec §4.3, §9.4: the new palace copies it to compiler.sqlite.before-memory-types first); runs
# mempalace-check.sh after any change. env.sh also holds the settle
# runs' settings (bin/settle.sh: SETTLE_DEBOUNCE_SECONDS, DISTILL_MIN_TURNS,
# SETTLE_COMPRESS_ENTRIES, SETTLE_COMPRESS_MAX_DAYS, ASKS_WINDOW, FIDELITY_SAMPLE, FIDELITY_FLOOR),
# each kept from an earlier install. A second run changes nothing
# and says so. Writes only under $CC_HOME (and uv's own cache).
set -eu
PROG=install-core
HERE=$(cd "$(dirname "$0")" && pwd)
SRC=$(cd "$HERE/../.." && pwd)
. "$HERE/lib.sh"

FROM_SOURCE=0 WING=memory EXCLUDE_WINGS="" PERSON="" ASSISTANT="" AGENT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --from-source) FROM_SOURCE=1 ;;
    --wing) [ $# -ge 2 ] || die "missing value for $1"; WING=$2; shift ;;
    --exclude-wing) [ $# -ge 2 ] || die "missing value for $1"; EXCLUDE_WINGS="${EXCLUDE_WINGS:+$EXCLUDE_WINGS }$2"; shift ;;
    --person) [ $# -ge 2 ] || die "missing value for $1"; PERSON=$2; shift ;;
    --assistant) [ $# -ge 2 ] || die "missing value for $1"; ASSISTANT=$2; shift ;;
    --agent) [ $# -ge 2 ] || die "missing value for $1"; AGENT=$2; shift ;;
    -h | --help) sed -n '/^set -eu/q;3,$p' "$0"; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
  shift
done

CC_HOME=${CC_HOME:-$HOME/.context-compiler}
PALACE="$CC_HOME/palace"
DIARY_ROOT="$CC_HOME/diary-root"

main() {
  safe_path "$CC_HOME"
  for wing in "$WING" $EXCLUDE_WINGS; do
    wing_name "$wing"
  done
  PERSON=${PERSON:-$(recorded PERSON)} ASSISTANT=${ASSISTANT:-$(recorded ASSISTANT)}
  PERSON=${PERSON:-$(login_name)} ASSISTANT=${ASSISTANT:-assistant}
  role_names "$PERSON" "$ASSISTANT"
  AGENT=${AGENT:-$(recorded AGENT)}
  AGENT=${AGENT:-context-compiler}
  case "$AGENT" in '' | [!a-z0-9]* | *[!a-z0-9_-]*) die "agent $AGENT: lowercase letters, digits, _ and - only" ;; esac
  check_platform
  mempalace_pin
  layout
  binaries
  files
  palace_init
  migrate_store
  health_check
  summary
}

check_platform() {
  [ "$(uname -s)" = Darwin ] || die "macOS only (uname -s: $(uname -s))"
  [ "$(uname -m)" = arm64 ] || die "Apple silicon only (uname -m: $(uname -m))"
  command -v uv >/dev/null 2>&1 || die "uv not found: install it first (https://docs.astral.sh/uv/)"
}

# The pin is the PIN= line of the shipped mempalace-check.sh, the one source of truth.
mempalace_pin() {
  PIN=$(sed -n 's/^PIN="\(.*\)"$/\1/p' "$SRC/scripts/mempalace-check.sh")
  [ -n "$PIN" ] || die "no PIN= line in $SRC/scripts/mempalace-check.sh"
  find_mempalace
  have=$(mempalace_version)
  if [ "$have" = "$PIN" ]; then
    say "mempalace $PIN: installed ($MEMPALACE_BIN_DIR)"
    return 0
  fi
  if [ -x "$MEMPALACE_BIN_DIR/mempalace-mcp" ]; then
    die "mempalace ${have:-of unknown version} is in $MEMPALACE_BIN_DIR and this install needs $PIN; it is never replaced. Either run \`uv tool install --force mempalace==$PIN\` yourself (it replaces that MemPalace for everything that uses it), or take $MEMPALACE_BIN_DIR off PATH so that this installer puts its own $PIN under $CC_HOME/uv; then run again"
  fi
  change "uv tool install mempalace==$PIN into $CC_HOME/uv" uv_install
  [ "$DRY_RUN" = 1 ] && return 0
  [ "$(mempalace_version)" = "$PIN" ] || die "mempalace $PIN not found in $MEMPALACE_BIN_DIR after uv"
}

uv_install() {
  mkdir -p "$CC_HOME/uv"
  UV_TOOL_DIR="$CC_HOME/uv/tools" UV_TOOL_BIN_DIR="$CC_HOME/uv/bin" uv tool install "mempalace==$PIN"
}

mempalace_version() {
  "$MEMPALACE_BIN_DIR/mempalace" --version 2>/dev/null | awk '{print $2}' || true
}

layout() {
  make_dir "$CC_HOME" 700
  for dir in bin palace diary-root diary-root/diary state logs hooks scripts prompts share; do
    make_dir "$CC_HOME/$dir"
  done
}

binaries() {
  if [ -x "$SRC/bin/palace" ]; then
    BIN_SRC="$SRC/bin"
  elif [ "$FROM_SOURCE" = 1 ]; then
    build
    BIN_SRC="$SRC/$RELEASE_TARGET/release"
  else
    die "no bin/ here: run from a release tarball, or pass --from-source"
  fi
  [ "$DRY_RUN" = 1 ] && [ ! -x "$BIN_SRC/palace" ] && return 0
  copy_file "$BIN_SRC/palace" "$CC_HOME/bin/palace" 755
  copy_file "$BIN_SRC/palace-mcp" "$CC_HOME/bin/palace-mcp" 755
}

# A build changes nothing installed: only a binary that differs from the installed one is copied.
build() {
  if [ "$DRY_RUN" = 1 ]; then
    say "would build: cargo build --locked --release --no-default-features in $SRC/$RELEASE_TARGET"
    return 0
  fi
  say "build: cargo build --locked --release --no-default-features in $SRC/$RELEASE_TARGET"
  log="${TMPDIR:-/tmp}/cc-build.$$.log"
  if ! release_build "$SRC" >"$log" 2>&1; then
    tail -20 "$log" >&2
    die "cargo build failed (log: $log)"
  fi
  rm -f "$log"
}

files() {
  copy_file "$SRC/scripts/palace-cron.sh" "$CC_HOME/scripts/palace-cron.sh" 755
  copy_file "$SRC/scripts/mempalace-check.sh" "$CC_HOME/scripts/mempalace-check.sh" 755
  copy_file "$SRC/dist/common/notify-local.sh" "$CC_HOME/bin/notify-local.sh" 755
  copy_file "$SRC/dist/common/settle.sh" "$CC_HOME/bin/settle.sh" 755
  for old in bin/night.sh dist/common/night.sh; do # an earlier install's nightly dispatcher
    [ ! -e "$CC_HOME/$old" ] || change "remove $CC_HOME/$old (no longer used)" rm -f "$CC_HOME/$old"
  done
  for hook in "$SRC"/dist/common/hooks/*.sh; do
    copy_file "$hook" "$CC_HOME/hooks/$(basename "$hook")" 755
  done
  for prompt in "$SRC"/prompts/*; do
    copy_file "$prompt" "$CC_HOME/prompts/$(basename "$prompt")"
  done
  [ -f "$CC_HOME/share/canaries.json" ] || write_text "$CC_HOME/share/canaries.json" '[]'
  copy_dist
  write_text "$CC_HOME/env.sh" "# written by install-core.sh: the pinned mempalace, the wing, the private wings, the names, the agent
MEMPALACE_BIN_DIR='$MEMPALACE_BIN_DIR'
WING='$WING'
EXCLUDE_WINGS='$EXCLUDE_WINGS'
PERSON='$PERSON'
ASSISTANT='$ASSISTANT'
AGENT='$AGENT'
# the settle runs (bin/settle.sh); edit here, a later install keeps them
SETTLE_DEBOUNCE_SECONDS='$(setting SETTLE_DEBOUNCE_SECONDS 60)'
DISTILL_MIN_TURNS='$(setting DISTILL_MIN_TURNS 3)'
SETTLE_COMPRESS_ENTRIES='$(setting SETTLE_COMPRESS_ENTRIES 40)'
SETTLE_COMPRESS_MAX_DAYS='$(setting SETTLE_COMPRESS_MAX_DAYS 3)'
ASKS_WINDOW='$(setting ASKS_WINDOW 08:00-21:00)'
FIDELITY_SAMPLE='$(setting FIDELITY_SAMPLE 10)'
FIDELITY_FLOOR='$(setting FIDELITY_FLOOR 0.7)'
# the per-turn hook: 1 = palace context --layered (CTX-1), 0 = the two-block output
CONTEXT_LAYERED='$(setting CONTEXT_LAYERED 1)'"
}

# setting <name> <default>: the value an earlier install recorded in env.sh (digits, ., : and -
# only), else the default.
setting() {
  value=$(recorded "$1")
  case "$value" in '' | *[!0-9.:-]*) value=$2 ;; esac
  echo "$value"
}

# dist/ itself, so that uninstall.sh and the hooks stay available after the tarball is gone.
copy_dist() {
  for file in $(cd "$SRC/dist" && find . -type f ! -path './test/*' ! -name '.*' | sort); do
    file=${file#./}
    make_dir "$(dirname "$CC_HOME/dist/$file")"
    mode=644
    [ -x "$SRC/dist/$file" ] && mode=755
    copy_file "$SRC/dist/$file" "$CC_HOME/dist/$file" "$mode"
  done
}

palace_init() {
  if [ -f "$PALACE/compiler.sqlite" ]; then
    say "palace: initialised ($PALACE)"
    return 0
  fi
  change "palace init $DIARY_ROOT $PALACE --wing $WING (log: $CC_HOME/logs/init.log)" run_init
}

run_init() {
  if ! with_mempalace "$CC_HOME/bin/palace" --roots="$CC_HOME" \
    --mempalace-check "$CC_HOME/scripts/mempalace-check.sh" --person "$PERSON" --assistant "$ASSISTANT" \
    init "$DIARY_ROOT" "$PALACE" --wing "$WING" >"$CC_HOME/logs/init.log" 2>&1; then
    tail -5 "$CC_HOME/logs/init.log" >&2
    die "palace init failed (log: $CC_HOME/logs/init.log)"
  fi
  tail -1 "$CC_HOME/logs/init.log"
}

# migrate_store: a claim store without claims.kind (written before SETTLE-1b) is opened once by
# the new palace, which copies it beside itself and migrates it (every claim a concept).
migrate_store() {
  store="$PALACE/compiler.sqlite"
  [ -f "$store" ] || return 0
  # immutable: the probe reads the file and writes nothing beside it (no -wal or -shm)
  [ "$(sqlite3 "file:$store?immutable=1" "SELECT count(*) FROM pragma_table_info('claims') WHERE name='kind'" 2>/dev/null)" != 1 ] ||
    return 0
  change "claim store: memory types (backup: $store.before-memory-types)" run_migrate "$store"
}

run_migrate() {
  "$CC_HOME/bin/palace" --roots="$CC_HOME" --person "$PERSON" --assistant "$ASSISTANT" \
    findings --store "$1" --json >/dev/null 2>"$CC_HOME/logs/migrate.log" ||
    die "the claim store migration failed (log: $CC_HOME/logs/migrate.log)"
}

# with_mempalace <command…>: the pinned mempalace first on PATH (and its Python, which
# mempalace-check.sh calls as python3), MEMPALACE_PALACE_PATH on the palace.
with_mempalace() {
  tools="$CC_HOME/uv/tools"
  [ "$MEMPALACE_BIN_DIR" = "$CC_HOME/uv/bin" ] || tools=$(uv tool dir 2>/dev/null || true)
  python_dir="$tools/mempalace/bin"
  [ -x "$python_dir/python3" ] || python_dir=""
  PATH="$MEMPALACE_BIN_DIR${python_dir:+:$python_dir}:$PATH" MEMPALACE_PALACE_PATH="$PALACE" "$@"
}

health_check() {
  [ "$DRY_RUN" = 1 ] && return 0
  [ "$CHANGES" = 0 ] && return 0
  log="$CC_HOME/logs/mempalace-check.log"
  if with_mempalace "$CC_HOME/scripts/mempalace-check.sh" --palace "$PALACE" \
    --diary "$DIARY_ROOT/diary" >"$log" 2>&1; then
    say "mempalace-check: healthy"
  else
    say "mempalace-check: UNHEALTHY (read $log)"
    exit 1
  fi
}

main
