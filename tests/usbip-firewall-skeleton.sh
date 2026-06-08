#!/usr/bin/env bash
# tests/usbip-firewall-skeleton.sh — W3 s3 L1c canary gate.
#
# Asserts:
#
#   1. `UsbipBindFirewallRule` inserts a per-busid carve-out rule BEFORE
#      the generic allow/drop in `inet nixling`'s `forward` chain
#      (the "specific carve-outs before generic" ordering invariant
#      from plan.md §"W3 broker variant additions");
#   2. the audit row carries `busid` + `rule_hash`;
#   3. `UsbipBind`, `UsbipUnbind`, `UsbipProxyReconcile` are refused
#      with the `unknown-operation` discriminant (W6 scope, not W3).

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

nl_activate_rust_toolchain_path || true

manifest="$ROOT/packages/Cargo.toml"
broker_manifest="$ROOT/packages/nixling-priv-broker/Cargo.toml"

if [ ! -f "$manifest" ] || [ ! -f "$broker_manifest" ]; then
  fail "usbip-firewall-skeleton: missing Cargo manifest"
  exit 1
fi

if [ -z "${NIXLING_USBIP_FW_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "neither cargo nor nix is on PATH"
    exit 1
  fi
  log "  cargo not on PATH; re-entering via nix shell"
  export NIXLING_USBIP_FW_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi

workspace_target_dir=$(nl_cargo_target_dir workspace)
broker_target_dir=$(nl_cargo_target_dir broker)

log "==> tests/usbip-firewall-skeleton.sh"

log "--> nixling-host: carve-out ordering invariant + comment marker"
CARGO_TARGET_DIR="$workspace_target_dir" cargo test \
  --manifest-path "$manifest" \
  -p nixling-host \
  --all-features \
  -- \
  nftables::tests::usbip_carveout_inserted_before_generic \
  nftables::tests::comment_marker_prefix_on_every_managed_rule \
  nftables::tests::fake_backend_refuses_carveout_after_generic \
  --nocapture
ok "host: specific carve-out before generic; nixling-managed marker on every rule"

log "--> broker: UsbipBindFirewallRule audit payload + W6 refusal"
CARGO_TARGET_DIR="$broker_target_dir" cargo test \
  --manifest-path "$broker_manifest" \
  --all-features \
  -- \
  ops::usbip_firewall::tests::bind_firewall_rule_produces_audit_with_busid_and_hash \
  ops::usbip_firewall::tests::carveout_ordering_invariant_via_op \
  ops::usbip_firewall::tests::w6_ops_refused_with_unknown_operation_audit \
  --nocapture
ok "broker: UsbipBind/Unbind/ProxyReconcile refused with unknown-operation"

# Static grep so the W6 boundary cannot be silently elided.
log "--> static grep: W6 USBIP UX variants explicitly refused"
for op in UsbipBind UsbipUnbind UsbipProxyReconcile; do
  if ! grep -q "$op" "$ROOT/packages/nixling-priv-broker/src/ops/usbip_firewall.rs"; then
    fail "missing W6 refusal variant $op in usbip_firewall.rs"
    exit 1
  fi
done
ok "static grep: every W6 USBIP variant has an explicit refusal handler"

ok "tests/usbip-firewall-skeleton.sh: ordering + audit + W6 boundary all enforced"
