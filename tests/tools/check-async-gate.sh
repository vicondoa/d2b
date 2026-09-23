#!/usr/bin/env bash
# Runs the async-gate source hygiene check against the committed tree.
#
# The gate scans the broker, the daemon, and every provider crate for denied
# blocking calls inside async contexts (plan unit U13): a non-yielding handler
# starves the whole envelope. Since U6 (issue #590) it also flags the
# conservative method-call lock shape - a `lock()`/`read()`/`write()` method
# call inside an async context (`async fn` or async block) not followed by
# `.await` - because the production
# lock shape is the method-call form, invisible to the deny list's qualified
# paths. Its own unit tests pin the fixture side; this script runs it over the
# whole repository, so a regression anywhere in the tree fails the L1 policy
# gate rather than a fixture. The deny list lives in clippy.toml. The escape
# hatch is the source-level marker `// async-gate-allow: <reason>` on the
# call's own line, recorded in packages/xtask/data/async-gate-inventory.json:
# a marker without an inventory entry fails the gate, and an inventory entry
# without a marked site fails it too (as does an entry whose file no longer
# exists in the tree), so the hatch cannot drift into an allowlist. The
# inventory keys sites by (file, line): a line-shifting edit above a marked
# call turns the gate red, and the repair is to regenerate the ledger from
# the run's marker sites - `cargo xtask check-async-gate --write-inventory`
# from the repo root - rather than hand-editing line numbers.
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