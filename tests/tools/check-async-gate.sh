#!/usr/bin/env bash
# Runs the async-gate source hygiene check against the committed tree.
#
# The gate scans the broker, the daemon, and every provider crate for denied
# blocking calls inside async contexts (plan unit U13): a non-yielding handler
# starves the whole envelope. Its own unit tests pin the fixture side; this
# script runs it over the whole repository, so a regression anywhere in the
# tree fails the L1 policy gate rather than a fixture. The deny list lives in
# clippy.toml; there is no violation allowlist - an entry is never the fix.
set -euo pipefail

runfiles="${TEST_SRCDIR:-}/${TEST_WORKSPACE:-}"
xtask="$runfiles/packages/xtask/xtask"

if [[ ! -x "$xtask" ]]; then
  echo "check-async-gate: xtask binary unavailable at $xtask" >&2
  exit 1
fi

root="${D2B_REPO_ROOT:-}"
if [[ -z "$root" ]]; then
  echo "check-async-gate: D2B_REPO_ROOT is not set" >&2
  exit 1
fi

(cd "$root" && D2B_REPO_ROOT="$root" "$xtask" check-async-gate)