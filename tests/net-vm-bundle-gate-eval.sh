#!/usr/bin/env bash
# tests/net-vm-bundle-gate-eval.sh — P2 ph2-p2-net-vm-bundle-gate
# integration test for the daemon-side VM-start preflight that refuses
# to bring up a `sys-<env>-net` VM when the on-disk dnsmasq.conf hash
# diverges from the bundle's nft/route/hosts intent hash for that env.
#
# Strategy (mirrors tests/ssh-host-key-preflight-eval.sh): drive the
# pure `nixlingd::net_vm_bundle_gate` module via its cargo unit tests.
# The module is purely a filesystem + bundle-resolver check, so a
# hermetic cargo test against a tempdir-built fixture covers every
# failure class deterministically without needing root or a live host.
#
# Failure classes asserted (see packages/nixlingd/src/net_vm_bundle_gate.rs):
#   * workload VM short-circuits to NotANetVm        — workload_vm_short_circuits_to_not_a_net_vm
#   * unknown VM short-circuits to NotANetVm         — unknown_vm_short_circuits_to_not_a_net_vm
#   * missing on-disk dnsmasq.conf is drift          — missing_dnsmasq_conf_is_drift
#   * happy path with matching hash                  — matching_hash_is_ok
#   * divergent bytes surface hash mismatch          — divergent_bytes_surface_hash_mismatch
#   * expected-hash function is deterministic        — expected_hash_is_deterministic
#   * drift reason redacts host paths                — drift_reason_redacts_paths
#   * drift accessor surfaces offending path         — drift_path_accessor_returns_offending_path
#
# Also asserts the typed-error wiring (`BundleDnsmasqDrift` exit code 63
# and its envelope shape).

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

if [ -n "${NL_RUST_TOOLCHAIN_PATH:-}" ]; then
  PATH="$NL_RUST_TOOLCHAIN_PATH:$PATH"
  export PATH
fi

if [ -z "${NIXLING_NET_VM_BUNDLE_GATE_IN_NIX_SHELL:-}" ] \
   && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    echo "net-vm-bundle-gate-eval: neither cargo nor nix is on PATH" >&2
    exit 1
  fi
  export NIXLING_NET_VM_BUNDLE_GATE_IN_NIX_SHELL=1
  exec nix --extra-experimental-features 'nix-command flakes' shell \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc \
    --command bash "$0" "$@"
fi

log() { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }

log "==> tests/net-vm-bundle-gate-eval.sh"

export RUSTC_WRAPPER=${RUSTC_WRAPPER:-}

cd "$ROOT/packages/nixlingd"

log "  cargo test --lib net_vm_bundle_gate"
cargo test --lib net_vm_bundle_gate -- --nocapture

log "  cargo test --lib typed_error::tests::bundle_dnsmasq_drift_envelope_shape"
cargo test --lib typed_error::tests::bundle_dnsmasq_drift_envelope_shape -- --nocapture

log "  cargo test --lib typed_error::tests::envelope_kind_matches_expected_discriminant"
cargo test --lib typed_error::tests::envelope_kind_matches_expected_discriminant

log "PASS: net-vm-bundle-gate-eval"
