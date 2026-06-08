#!/usr/bin/env bash
# tests/nft-foreign-rule-preservation.sh— canary gate.
#
# Pre-seeds foreign iptables-style + nft-style rules into the fake
# nft backend, runs the reconcile against the `inet nixling` table,
# and asserts that:
#
#   1. the rendered nft script NEVER contains the foreign rule body
#      (i.e. nixling does not adopt or reflect foreign rules into its
#      own table);
#   2. repeat-apply of the same batch is byte-stable (hash equality);
#   3. an attempt to flush a foreign rule (modeled here as the foreign
#      rule body leaking into the batch script) fails closed with
#      `nft-foreign-rule-flush-attempted`.
#
# The Rust-level invariants are exercised through
# `nixling-host::nftables::tests::fake_backend_preserves_foreign_rules`.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

nl_activate_rust_toolchain_path || true

manifest="$ROOT/packages/Cargo.toml"

if [ ! -f "$manifest" ]; then
  fail "nft-foreign-rule-preservation: missing $manifest"
  exit 1
fi

if [ -z "${NIXLING_NFT_PRESERVE_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "neither cargo nor nix is on PATH"
    exit 1
  fi
  log "  cargo not on PATH; re-entering via nix shell"
  export NIXLING_NFT_PRESERVE_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi

workspace_target_dir=$(nl_cargo_target_dir workspace)

log "==> tests/nft-foreign-rule-preservation.sh"

log "--> nixling-host fake backend: repeat-apply is byte-stable + foreign rules untouched"
CARGO_TARGET_DIR="$workspace_target_dir" cargo test \
  --manifest-path "$manifest" \
  -p nixling-host \
  --all-features \
  -- \
  nftables::tests::fake_backend_preserves_foreign_rules \
  nftables::tests::drift_detection_strips_volatile_fields \
  nftables::tests::drift_detection_catches_real_change \
  --nocapture
ok "fake backend: foreign rule body not represented in nixling batch; repeat-apply hash stable"

# Statically grep the production sources for the failure-mode marker
# so the fail-closed discriminant cannot be silently renamed.
log "--> static grep: kebab-case discriminants present in the nft surface"
if ! grep -q 'nft-foreign-rule-flush-attempted' "$ROOT/packages/nixling-host/src/nftables.rs"; then
  fail "missing 'nft-foreign-rule-flush-attempted' kebab discriminant in nftables.rs"
  exit 1
fi
if ! grep -q 'foreign-nft-rule-shadows-nixling' "$ROOT/packages/nixling-host/src/nftables.rs"; then
  fail "missing 'foreign-nft-rule-shadows-nixling' kebab discriminant in nftables.rs"
  exit 1
fi
ok "static grep: fail-closed discriminants intact"

ok "tests/nft-foreign-rule-preservation.sh: foreign rules preserved across repeat-apply"
