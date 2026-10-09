#!/bin/sh
# shellcheck source-path=SCRIPTDIR
# build-release.sh — pack the release tarball the installers run from.
#
# Usage: dist/build-release.sh [--dry-run] [--bin-dir <dir>] [--out <dir>]
#   --bin-dir   take palace and palace-mcp from <dir> (default: built as released: lib.sh
#               release_build, `cargo build --locked --release --no-default-features` with the
#               doctors off and the build paths remapped, in target/release-dist)
#   --out       output directory (default: target/dist)
# Writes <out>/context-compiler-<git describe>-aarch64-apple-darwin.tar.gz holding bin/ (palace,
# palace-mcp), scripts/ (palace-cron.sh, mempalace-check.sh), prompts/, share/canaries.json (an
# empty list: fixtures/canaries.json holds private canaries and is never shipped), dist/ (without
# dist/test), LICENSE and VERSION. No fixture, palace or diary is shipped. dist/test/deny-list.sh
# (no owner's name, build path, canary or fixture text) runs three times, LICENSE's copyright line
# the one allowed occurrence: over the binaries (a hit stops the release before anything is
# staged), over every staged file (a hit removes the staging directory: nothing packed), and over
# the packed tarball, its headers included (a hit removes the tarball). The entries are owned by
# root:wheel (0/0) and carry no extended attributes or macOS metadata. Prints DENY-LIST OK …
# (three times) and RELEASE <tarball>. Outside a git checkout the version is "unversioned". A tree
# without dist/test/deny-list.sh (the public export: the list is private) skips the three checks,
# saying so once.
set -eu
PROG=build-release
ROOT=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=common/lib.sh
. "$ROOT/dist/common/lib.sh"

BIN_DIR="" OUT="$ROOT/target/dist"
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --bin-dir) [ $# -ge 2 ] || die "missing value for $1"; BIN_DIR=$2; shift ;;
    --out) [ $# -ge 2 ] || die "missing value for $1"; OUT=$2; shift ;;
    -h | --help) sed -n '/^set -eu/q;3,$p' "$0"; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
  shift
done

VERSION=$(git -C "$ROOT" describe --tags --always --dirty 2>/dev/null) || VERSION=unversioned
NAME="context-compiler-$VERSION-aarch64-apple-darwin"
STAGE="$OUT/$NAME"
TARBALL="$OUT/$NAME.tar.gz"
# The one allowed occurrence of the owner's name: LICENSE's copyright line, read from LICENSE.
ALLOWED=$(sed -n '/^Copyright (c) /{p;q;}' "$ROOT/LICENSE")
DENY_LIST="$ROOT/dist/test/deny-list.sh"

main() {
  [ -x "$DENY_LIST" ] || say "DENY-LIST skipped: no $DENY_LIST in this tree (the list is private)"
  binaries
  deny_list
  stage
  deny_list_staged
  change "pack $TARBALL" pack
  deny_list_packed
  summary
  [ "$DRY_RUN" = 1 ] || say "RELEASE $TARBALL"
}

binaries() {
  if [ -z "$BIN_DIR" ]; then
    change "cargo build --locked --release --no-default-features in $ROOT/$RELEASE_TARGET" release_build "$ROOT"
    BIN_DIR="$ROOT/$RELEASE_TARGET/release"
  fi
  [ "$DRY_RUN" = 1 ] && return 0
  for bin in palace palace-mcp; do
    [ -x "$BIN_DIR/$bin" ] || die "no executable $BIN_DIR/$bin"
  done
}

# deny_list: the shipped binaries' strings hold no private text (PKG-1.2 item 5, review M3).
deny_list() {
  if [ "$DRY_RUN" = 1 ] || [ ! -x "$DENY_LIST" ]; then return 0; fi
  "$DENY_LIST" "$BIN_DIR/palace" "$BIN_DIR/palace-mcp" ||
    die "a shipped binary carries private text (above): nothing packed"
}

# deny_list_staged: no staged file holds private text (PKG-1.3 item 2, review M1).
deny_list_staged() {
  if [ "$DRY_RUN" = 1 ] || [ ! -x "$DENY_LIST" ]; then return 0; fi
  find "$STAGE" -type f ! -path "$STAGE/LICENSE" | LC_ALL=C sort >"$STAGE.files"
  set --
  while IFS= read -r file; do set -- "$@" "$file"; done <"$STAGE.files"
  rm -f "$STAGE.files"
  status=0
  "$DENY_LIST" --allow "$ALLOWED" "$STAGE/LICENSE" || status=1
  "$DENY_LIST" "$@" || status=1
  [ "$status" = 0 ] && return 0
  rm -rf "$STAGE"
  die "a staged file carries private text (above): nothing packed"
}

# deny_list_packed: nor does the tarball, headers included; a hit removes it.
deny_list_packed() {
  if [ "$DRY_RUN" = 1 ] || [ ! -x "$DENY_LIST" ]; then return 0; fi
  gzip -dc "$TARBALL" >"$OUT/$NAME.tar"
  "$DENY_LIST" --allow "$ALLOWED" "$OUT/$NAME.tar" || {
    rm -f "$TARBALL" "$OUT/$NAME.tar"
    die "the tarball carries private text (above): removed"
  }
  rm -f "$OUT/$NAME.tar"
}

stage() {
  [ -e "$STAGE" ] && change "remove the previous staging directory $STAGE" rm -rf "$STAGE"
  change "stage $STAGE" stage_now
}

stage_now() {
  mkdir -p "$STAGE/bin" "$STAGE/scripts" "$STAGE/share"
  cp "$BIN_DIR/palace" "$BIN_DIR/palace-mcp" "$STAGE/bin/"
  cp "$ROOT/scripts/palace-cron.sh" "$ROOT/scripts/mempalace-check.sh" "$STAGE/scripts/"
  cp -R "$ROOT/prompts" "$STAGE/prompts"
  printf '[]\n' >"$STAGE/share/canaries.json"
  cp -R "$ROOT/dist" "$STAGE/dist"
  rm -rf "$STAGE/dist/test"
  cp "$ROOT/LICENSE" "$STAGE/"
  printf '%s\n' "$VERSION" >"$STAGE/VERSION"
}

# pack: entries owned by root:wheel, without the build account, xattrs or macOS metadata.
pack() {
  COPYFILE_DISABLE=1 tar --uid 0 --gid 0 --uname root --gname wheel --no-xattrs \
    --no-mac-metadata -czf "$TARBALL" -C "$OUT" "$NAME"
}

main
