#!/bin/sh
# codex-llm.sh — the `command:` model of palace rollup and the judge (--llm, --judge) for Codex.
#
# Usage: <prompt on stdin> | $CC_HOME/bin/codex-llm.sh [--dry-run]
# Becomes (exec), in the empty directory $CC_HOME/state/codex-llm, with the prompt on stdin and
# the answer on stdout:
#   codex exec --ignore-user-config --ignore-rules --disable hooks --disable apps
#     --disable plugins --disable multi_agent --disable image_generation -c web_search="disabled"
#     --sandbox read-only --skip-git-repo-check --ephemeral --color never [-m $CODEX_LLM_MODEL] -
# An unattended run with untrusted diary text as its prompt, so it is cut off from what the
# user's Codex can do: no config.toml (no MCP servers, notify program, profiles or model
# settings; auth still comes from CODEX_HOME), no execpolicy rules, no hooks, no account tools
# (apps: the ChatGPT account's codex_apps server, which --ignore-user-config alone leaves on;
# plugins, multi_agent, image_generation), no web search, a read-only shell. The model is pinned at install (CODEX_LLM_MODEL), since config.toml's is not
# read. exec, so that palace's deadline, which ends only its direct child, ends Codex itself.
# CONTEXT_COMPILER_NO_HOOKS=1 silences our own hooks as well. CODEX_BIN, CODEX_HOME and
# CODEX_LLM_MODEL come from $CC_HOME/codex-env.sh (dist/codex/install.sh).
# --ignore-user-config, --ignore-rules, --disable, -c, -m and -o are in `codex exec --help` of
# codex-cli 0.154.0; the feature name `hooks` and the key `web_search` are not shown there.
# Shown in the maintainers' test on Codex 0.154 (2026-10-03): with --ignore-user-config alone the model listed
# 72 tools of codex_apps (21 Gmail); with the four account features disabled it listed only
# functions/clock/collaboration and still answered. UNVERIFIED with a real model: whether
# $CODEX_HOME/AGENTS.md is still loaded, and that the last message alone reaches stdout.
set -eu
CC_HOME=$(cd "$(dirname "$0")/.." && pwd)
CODEX_BIN=codex
CODEX_HOME=${CODEX_HOME:-$HOME/.codex}
CODEX_LLM_MODEL=""
# shellcheck source=/dev/null
[ ! -f "$CC_HOME/codex-env.sh" ] || . "$CC_HOME/codex-env.sh"
export CODEX_HOME
export CONTEXT_COMPILER_NO_HOOKS=1

DRY_RUN=0
[ "${1:-}" != --dry-run ] || DRY_RUN=1
set -- exec --ignore-user-config --ignore-rules --disable hooks --disable apps \
  --disable plugins --disable multi_agent --disable image_generation -c 'web_search="disabled"' \
  --sandbox read-only --skip-git-repo-check --ephemeral --color never \
  ${CODEX_LLM_MODEL:+-m "$CODEX_LLM_MODEL"} -
if [ "$DRY_RUN" = 1 ]; then
  echo "would run in $CC_HOME/state/codex-llm: CODEX_HOME=$CODEX_HOME $CODEX_BIN $*"
  exit 0
fi
mkdir -p "$CC_HOME/state/codex-llm"
cd "$CC_HOME/state/codex-llm"
exec "$CODEX_BIN" "$@"
