#!/usr/bin/env bash
# shellcheck disable=SC2016
# tests/unit/meta/pr-checklist-gate.sh — W0 policy gate for the mandatory PR checklist.
#
# W0 validates the template (default) or a provided PR body (file arg or `-` for
# stdin). A later wave wires this against live PR bodies in CI.

set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}

usage() {
  echo "usage: pr-checklist-gate.sh [pr-body-file|-]" >&2
}

if [ "$#" -gt 1 ]; then
  usage
  exit 2
fi

source_label=".github/PULL_REQUEST_TEMPLATE.md"
if [ "$#" -eq 1 ]; then
  source_label="$1"
elif [ -p /dev/stdin ] || [ -f /dev/stdin ]; then
  source_label="-"
fi

if [ "$source_label" = "-" ]; then
  body=$(cat)
else
  path="$source_label"
  case "$path" in
    /*) ;;
    *) path="$ROOT/$path" ;;
  esac
  [ -f "$path" ] || { echo "pr-checklist-gate: missing PR body/template: $source_label" >&2; exit 1; }
  body=$(cat "$path")
fi

fail=0
check_item() {
  local label="$1" pattern="$2"
  if grep -Eq -- "$pattern" <<<"$body"; then
    printf 'PASS: PR checklist contains %s\n' "$label"
  else
    printf 'FAIL: PR checklist missing %s\n' "$label" >&2
    fail=1
  fi
}

check_item 'make check checkbox' '^- \[[ xX]\] \*\*`make check` passes locally\*\*'
check_item 'make test-integration checkbox' '^- \[[ xX]\] \*\*`make test-integration` passes on the host before PR creation\*\*'
check_item 'make test-host-integration checkbox' '^- \[[ xX]\] \*\*`make test-host-integration` passes on the host before PR creation\*\*'
check_item 'manual test-hardware checkbox' '^- \[[ xX]\] \*\*Manual `make test-hardware` run\*\*'
check_item 'make-target wiring checkbox' '^- \[[ xX]\] \*\*New/changed tests are wired into a `make` target\*\*'
check_item 'docs and CI lockstep checkbox' '^- \[[ xX]\] \*\*Docs \+ CI updated in lockstep\*\*'

if [ "$fail" -ne 0 ]; then
  echo "pr-checklist-gate: mandatory checklist is incomplete" >&2
  exit 1
fi

echo "pr-checklist-gate: mandatory checklist present"
