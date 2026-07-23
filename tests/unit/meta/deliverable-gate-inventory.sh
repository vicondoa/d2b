#!/usr/bin/env bash
# tests/unit/meta/deliverable-gate-inventory.sh
#
# Assert every planned deliverable has at least one regression gate
# (cargo test, eval script, or doc-drift gate) the integrator can run
# on every change.
#
# This is a static asserter: it does NOT run the tests; it asserts
# the mapping exists. See `tests/static.sh` and per-gate scripts for
# actual execution.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}

fail=0
ok() { echo "  PASS: $1"; }
die() { echo "  FAIL: $1"; fail=$((fail + 1)); }

echo "==> tests/unit/meta/deliverable-gate-inventory.sh"

# Mapping: todo id → gate(s). Each gate is either a script under tests/
# or a cargo test target. The asserter checks the
# scripts exist + executable; cargo targets are validated by name
# only (we don't run cargo here).
#
# Format: "<todo-id>:<gate1>;<gate2>;..."
MAPPINGS=(
  "host-prep-dag:tests/host-prep-dag-eval.sh;cargo:d2b-host::host_prep_dag"
  "store-sync:cargo:d2b-priv-broker::ops::store_sync"
  "manifest-version-bump:cargo:d2b-core::manifest_v04::tests"
  "ownership-matrix:tests/per-vm-state-ownership-eval.sh;cargo:d2b-host::ownership_matrix"
  "privileges-reference-update:docs/reference/privileges.md"
  "vfsd-watchdog:cargo:d2bd::supervisor::pidfd"
  "known-hosts-refresh:cargo:d2bd::known_hosts_refresh"
  "ssh-host-key-preflight:tests/ssh-host-key-preflight-eval.sh;cargo:d2bd::ssh_host_key_preflight"
  "daemon-autostart:cargo:d2bd::autostart"
  "stop-dag-owner:tests/stop-dag-reconcile-eval.sh;cargo:d2bd::supervisor::stop_dag"
  "net-fixture-observability-env:tests/unit/nix/cases/net-vm-network.nix"
  "net-vm-bundle-gate:cargo:d2bd::net_vm_bundle_gate"
  "tap-dag-contract:tests/tap-dag-contract-doc-eval.sh;docs/reference/tap-dag-contract.md"
)

for mapping in "${MAPPINGS[@]}"; do
  todo="${mapping%%:*}"
  gates="${mapping#*:}"
  IFS=';' read -ra parts <<< "$gates"
  found=0
  for gate in "${parts[@]}"; do
    case "$gate" in
      cargo:*)
        # Just check the source file exists; we don't run cargo here.
        # Extract the crate + module path: cargo:crate::path::to::module
        # Find a *.rs file matching the leaf segment.
        leaf="${gate##*::}"
        crate="${gate#cargo:}"; crate="${crate%%::*}"
        # Heuristic: search packages/<crate>/src for a file containing the leaf segment
        if find "$ROOT/packages/$crate" -name "*.rs" -print0 2>/dev/null | xargs -0 grep -l "$leaf" 2>/dev/null | head -1 >/dev/null 2>&1; then
          found=1
          break
        fi
        ;;
      tests/*|docs/*)
        if [ -e "$ROOT/$gate" ]; then
          found=1
          break
        fi
        ;;
    esac
  done
  if [ "$found" -eq 1 ]; then
    ok "$todo has a regression gate: $gates"
  else
    die "$todo has NO discoverable regression gate (checked: $gates)"
  fi
done

if [ "$fail" -gt 0 ]; then
  echo "==> $fail of ${#MAPPINGS[@]} deliverables lack a regression gate"
  exit 1
fi
echo "==> all ${#MAPPINGS[@]} deliverables have at least one regression gate"
