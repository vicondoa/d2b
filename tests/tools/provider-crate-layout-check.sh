#!/usr/bin/env bash
# Runs the provider-per-crate layout check against the committed tree.
#
# The check is the repo's done-state enforcement (plan unit U19, requirement
# R25): the provider matrix must be closed, shared crates must not carry
# per-resource knowledge, a shared driver must not be parked under a monitored
# source root, and no comment may cite a module the restructure deleted. Its
# own unit tests pin the fixture side; this script runs it over the whole
# repository, so a regression anywhere in the tree fails the L1 policy gate
# rather than a fixture. The allowlist is empty: an entry is never the fix.
set -euo pipefail

runfiles="${TEST_SRCDIR:-}/${TEST_WORKSPACE:-}"
xtask="$runfiles/packages/xtask/xtask"

if [[ ! -x "$xtask" ]]; then
  echo "provider-crate-layout-check: xtask binary unavailable at $xtask" >&2
  exit 1
fi

root="${D2B_REPO_ROOT:-}"
if [[ -z "$root" ]]; then
  echo "provider-crate-layout-check: D2B_REPO_ROOT is not set" >&2
  exit 1
fi

(cd "$root" && D2B_REPO_ROOT="$root" "$xtask" check-provider-crate-layout)