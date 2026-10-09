# shellcheck shell=sh
# lib.sh — helpers shared by the dist/ installers (sourced, POSIX sh).
#
# Every change goes through `change`, which prints it first and makes it only without --dry-run.
# A file that already exists is copied to <file>.cc-backup-<time> before it is edited, and the
# edit is recorded in $CC_HOME/state/edits (path, backup or "-", sha256 after) for uninstall.sh.

DRY_RUN=${DRY_RUN:-0}
CHANGES=0
STAMP=$(date +%Y%m%d-%H%M%S)

say() { printf '%s\n' "$*"; }

die() {
  printf '%s: %s\n' "$PROG" "$*" >&2
  exit 1
}

# change <description> <command> [args…]: print the change, then run it unless --dry-run.
change() {
  description=$1
  shift
  CHANGES=$((CHANGES + 1))
  if [ "$DRY_RUN" = 1 ]; then
    say "would: $description"
    return 0
  fi
  say "change: $description"
  record "$description"
  "$@"
}

# record <line>: append to $CC_HOME/install.log once $CC_HOME exists.
record() {
  [ -d "${CC_HOME:-}" ] || return 0
  printf '%s %s: %s\n' "$(date '+%Y-%m-%dT%H:%M:%S%z')" "$PROG" "$1" >>"$CC_HOME/install.log"
}

# summary: the last line of a run.
summary() {
  if [ "$CHANGES" = 0 ]; then
    say "$PROG: nothing to change"
  elif [ "$DRY_RUN" = 1 ]; then
    say "$PROG: dry run, $CHANGES change(s) not made"
  else
    say "$PROG: $CHANGES change(s) made"
  fi
}

sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }

# make_dir <dir> [mode]
make_dir() {
  [ -d "$1" ] && return 0
  change "create directory $1" mkdir -p -m "${2:-755}" "$1"
}

# copy_file <source> <dest> [mode]: copy when the destination is absent or different.
copy_file() {
  if [ -f "$2" ] && cmp -s "$1" "$2"; then
    return 0
  fi
  change "install $2" install_now "$1" "$2" "${3:-644}"
}

install_now() {
  cp "$1" "$2.cc-new"
  chmod "$3" "$2.cc-new"
  mv -f "$2.cc-new" "$2"
}

# link_file <target> <link>: a symbolic link to <target>, made (or replaced) unless it is there.
link_file() {
  [ -L "$2" ] && [ "$(readlink "$2")" = "$1" ] && return 0
  change "link $2 to $1" ln -sfn "$1" "$2"
}

# write_text <dest> <text> [mode]: write <text> plus a newline when the file differs.
write_text() {
  scratch="${TMPDIR:-/tmp}/cc-text.$$"
  printf '%s\n' "$2" >"$scratch"
  copy_file "$scratch" "$1" "${3:-644}"
  rm -f "$scratch"
}

# edit_begin <file>: back up an existing file before an edit (sets EDIT_BACKUP, "-" when absent).
edit_begin() {
  EDIT_BACKUP=-
  [ -e "$1" ] || return 0
  EDIT_BACKUP="$1.cc-backup-$STAMP"
  n=1
  while [ -e "$EDIT_BACKUP" ]; do
    EDIT_BACKUP="$1.cc-backup-$STAMP.$n"
    n=$((n + 1))
  done
  change "back up $1 to $EDIT_BACKUP" cp -p "$1" "$EDIT_BACKUP"
}

# edit_end <file>: record the edit for uninstall.sh.
edit_end() {
  [ "$DRY_RUN" = 1 ] && return 0
  printf '%s\t%s\t%s\n' "$1" "$EDIT_BACKUP" "$(sha256 "$1")" >>"$CC_HOME/state/edits"
}

# state_has <line>: the line is in $CC_HOME/install-state (what the installers added).
state_has() { [ -f "$CC_HOME/install-state" ] && grep -qxF "$1" "$CC_HOME/install-state"; }

# state_add <line>: record it in $CC_HOME/install-state, once (not in a dry run).
state_add() {
  [ "$DRY_RUN" = 1 ] && return 0
  state_has "$1" || printf '%s\n' "$1" >>"$CC_HOME/install-state"
}

# state_remove <line>: drop it from $CC_HOME/install-state (not in a dry run).
state_remove() {
  [ "$DRY_RUN" = 1 ] && return 0
  state_has "$1" || return 0
  grep -vxF "$1" "$CC_HOME/install-state" >"$CC_HOME/install-state.new" || true
  mv -f "$CC_HOME/install-state.new" "$CC_HOME/install-state"
}

# remove_tables <file> <headers, space-separated>: each table and the blank lines just before it.
remove_tables() {
  awk -v list="$2" '
    BEGIN { n = split(list, h, " "); for (i = 1; i <= n; i++) drop[h[i]] = 1 }
    /^\[/ { skip = ($0 in drop); if (skip) { held = 0; next } }
    /^[[:space:]]*$/ { held++; next }
    skip { next }
    { while (held > 0) { print ""; held-- } print }
    END { while (held > 0) { print ""; held-- } }' "$1" >"$1.cc-new"
  cat "$1.cc-new" >"$1"
  rm -f "$1.cc-new"
}

# legacy_server <name>: a PKG-1 install added the MCP server <name> (mempalace, palace-graph)
# and Codex still has it: the last `codex mcp add|remove <name>` in $CC_HOME/install.log is an add.
legacy_server() {
  [ -f "$CC_HOME/install.log" ] || return 1
  grep -E ": codex mcp (add|remove) $1( |\$)" "$CC_HOME/install.log" | tail -1 |
    grep -qE ": codex mcp add $1 " || return 1
  "$CODEX_BIN" mcp get "$1" >/dev/null 2>&1
}

# find_mempalace: MEMPALACE_BIN_DIR, the directory of mempalace-mcp on PATH, else the
# install's own $CC_HOME/uv/bin.
find_mempalace() {
  if found=$(command -v mempalace-mcp 2>/dev/null); then
    MEMPALACE_BIN_DIR=$(dirname "$found")
  else
    MEMPALACE_BIN_DIR="$CC_HOME/uv/bin"
  fi
  safe_path "$MEMPALACE_BIN_DIR"
}

# wing_name <name>: a wing the installers accept (lowercase letters, digits, _ and -).
wing_name() {
  case "$1" in '' | [!a-z0-9]* | *[!a-z0-9_-]*) die "wing $1: lowercase letters, digits, _ and - only, starting with a letter or digit" ;; esac
}

# role_names <person> <assistant>: two different names palace takes as --by (a lowercase
# letter, then lowercase letters, digits, _ or -; at most 32).
role_names() {
  for name in "$1" "$2"; do
    case "$name" in
      [a-z]*) ;;
      *) die "name $name: a lowercase letter, then lowercase letters, digits, _ or -" ;;
    esac
    case "$name" in *[!a-z0-9_-]*) die "name $name: a lowercase letter, then lowercase letters, digits, _ or -" ;; esac
    [ "${#name}" -le 32 ] || die "name $name: at most 32 characters"
  done
  [ "$1" != "$2" ] || die "the person and the assistant are both $1"
}

# recorded <VAR>: its value in $CC_HOME/env.sh (an earlier install's), else nothing.
recorded() {
  sed -n "s/^$1='\(.*\)'\$/\1/p" "$CC_HOME/env.sh" 2>/dev/null || true
}

# login_name: the login name as a --by name (lowercase; any other character becomes -).
login_name() {
  name=$(id -un | LC_ALL=C tr '[:upper:]' '[:lower:]' | LC_ALL=C tr -c 'a-z0-9_\n-' '-' | cut -c1-32)
  case "$name" in [a-z]*) echo "$name" ;; *) echo person ;; esac
}

# release_build <checkout>: palace and palace-mcp as released (PKG-1.2 item 5): without the
# doctors and the owners' default names (cargo features), the home, the checkout, the cargo home
# and the Rust toolchain (PKG-1.2 review L3) remapped out of every compiled path, in
# <checkout>/$RELEASE_TARGET (the development build in target/ is left alone).
RELEASE_TARGET=target/release-dist
release_build() {
  cargo_home=${CARGO_HOME:-$HOME/.cargo}
  sysroot=$(rustc --print sysroot 2>/dev/null) || sysroot=""
  (cd "$1" && CARGO_TARGET_DIR="$1/$RELEASE_TARGET" \
    RUSTFLAGS="--remap-path-prefix=$HOME=~ --remap-path-prefix=$1=. --remap-path-prefix=$cargo_home=cargo${sysroot:+ --remap-path-prefix=$sysroot=rust}" \
    CFLAGS="-ffile-prefix-map=$HOME=~ -ffile-prefix-map=$1=. -ffile-prefix-map=$cargo_home=cargo" \
    cargo build --locked --release --no-default-features -p palace-cli -p palace-mcp)
}

# adapter <dist/common dir>: the scheduler adapter for this OS, <dir>/schedule-<os>.sh (macOS:
# schedule-macos.sh, launchd); an OS without one is refused.
adapter() {
  case "$(uname -s)" in
    Darwin) os=macos ;;
    *) os=$(uname -s | LC_ALL=C tr '[:upper:]' '[:lower:]') ;;
  esac
  [ -f "$1/schedule-$os.sh" ] ||
    die "no scheduler adapter for $(uname -s) (dist/common/schedule-$os.sh): only macOS is supported"
  echo "$1/schedule-$os.sh"
}

# safe_path <path>: refuse paths a hook command or a plist could not carry unquoted.
safe_path() {
  case "$1" in
    *[[:space:]]* | *\'* | *\"* | *\\* | *\$* | *\`* | *\&* | *\<* | *\>*)
      die "unsupported character in path: $1"
      ;;
  esac
}
