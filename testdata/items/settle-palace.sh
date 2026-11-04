#!/bin/sh
# Synthetic settle transport: records calls and never invokes a service or model.
set -eu
printf '%s\n' 'CALL' "$@" >>"$HOOK_CALL_LOG"
mode="" transcript=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    distill | fidelity | items) mode=$1 ;;
    --transcript) shift; transcript=$1 ;;
  esac
  shift
done
case "$mode" in
  distill)
    [ "${HOOK_DISTILL_EXIT:-0}" = 0 ] || exit "$HOOK_DISTILL_EXIT"
    printf 'DISTILL skipped exchanges=1 wing=%s position=%s\n' "${HOOK_WING:-memory}" "$(wc -l <"$transcript" | tr -d ' ')"
    ;;
  items) exit "${HOOK_ITEMS_EXIT:-0}" ;;
esac
