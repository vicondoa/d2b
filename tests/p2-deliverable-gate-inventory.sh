#!/usr/bin/env bash
# tests/p2-deliverable-gate-inventory.sh
#
# Assert every planned deliverable has at least one Layer-1 gate
# (cargo test, eval script, or doc-drift gate) the integrator can run
# on every change.
#
# This is a static asserter: it does NOT run the tests; it asserts
# the mapping exists. See `tests/static.sh` and per-gate scripts for
# actual execution.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=$(dirname "$HERE")

fail=0
ok() { echo "  PASS: $1"; }
die() { echo "  FAIL: $1"; fail=$((fail + 1)); }

echo "==> tests/p2-deliverable-gate-inventory.sh"

# Mapping: todo id → gate(s). Each gate is either a script under tests/
# or a cargo test target. The asserter checks the
# scripts exist + executable; cargo targets are validated by name
# only (we don't run cargo here).
#
# Format: "<todo-id>:<gate1>;<gate2>;..."
MAPPINGS=(
  "ph2-dag-host-prep:tests/host-prep-dag-eval.sh;cargo:nixling-host::host_prep_dag"
  "ph2-store-sync:cargo:nixling-priv-broker::ops::store_sync"
  "ph2-p2-manifestversion-bump:cargo:nixling-core::manifest_v04::tests"
  "ph2-p2-ownership-matrix:tests/per-vm-state-ownership-eval.sh;cargo:nixling-host::ownership_matrix"
  "ph2-p2-privileges-md-update:docs/reference/privileges.md"
  "ph2-vfsd-watchdog:cargo:nixlingd::supervisor::pidfd"
  "ph2-known-hosts-refresh:cargo:nixlingd::known_hosts_refresh"
  "ph2-p2-ssh-host-key-preflight:tests/ssh-host-key-preflight-eval.sh;cargo:nixlingd::ssh_host_key_preflight"
  "ph2-p2-daemon-autostart:tests/daemon-autostart-eval.sh;cargo:nixlingd::autostart"
  "ph2-p2-stop-dag-owner:tests/stop-dag-reconcile-eval.sh;cargo:nixlingd::supervisor::stop_dag"
  "ph2-p2-net-fixture-obs-env:tests/net-vm-network-eval.sh"
  "ph2-p2-net-vm-bundle-gate:tests/net-vm-bundle-gate-eval.sh;cargo:nixlingd::net_vm_bundle_gate"
  "ph2-p2-tap-dag-contract:tests/tap-dag-contract-doc-eval.sh;docs/reference/tap-dag-contract.md"
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
    ok "$todo has a Layer-1 gate: $gates"
  else
    die "$todo has NO discoverable Layer-1 gate (checked: $gates)"
  fi
done

if [ "$fail" -gt 0 ]; then
  echo "==> $fail of ${#MAPPINGS[@]} P2 deliverables lack a Layer-1 gate"
  exit 1
fi
echo "==> all ${#MAPPINGS[@]} P2 deliverables have at least one Layer-1 gate"
