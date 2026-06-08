#!/usr/bin/env bash
# tests/nft-coexistence.sh— canary gate.
#
# Asserts the 7-row firewall coexistence detector → policy matrix from
# plan.md §" firewall coexistence policy" by running the
# `nixling-host::nftables::tests::coexistence_matrix_all_7_rows` unit
# test, plus the per-row apply-time refusal checks in
# `nixling-priv-broker::ops::nft`. The fake firewall detector inputs
# live inside the rust unit tests; this shell script is the static gate
# wrapper used by `tests/static.sh`.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

nl_activate_rust_toolchain_path || true

manifest="$ROOT/packages/Cargo.toml"
broker_manifest="$ROOT/packages/nixling-priv-broker/Cargo.toml"

if [ ! -f "$manifest" ] || [ ! -f "$broker_manifest" ]; then
  fail "nft-coexistence: missing Cargo manifest under $ROOT/packages"
  exit 1
fi

if [ -z "${NIXLING_NFT_COEXIST_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "nft-coexistence: neither cargo nor nix is on PATH"
    exit 1
  fi
  log "  cargo not on PATH; re-entering via nix shell"
  export NIXLING_NFT_COEXIST_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi

workspace_target_dir=$(nl_cargo_target_dir workspace)
broker_target_dir=$(nl_cargo_target_dir broker)

log "==> tests/nft-coexistence.sh"

log "--> nixling-host coexistence matrix unit test (7 rows)"
CARGO_TARGET_DIR="$workspace_target_dir" cargo test \
  --manifest-path "$manifest" \
  -p nixling-host \
  --all-features \
  -- \
  nftables::tests::coexistence_matrix_all_7_rows \
  --exact --nocapture
ok "nixling-host: 7-row detector→policy matrix"

log "--> nixling-host detector probe (single vs ambiguous)"
CARGO_TARGET_DIR="$workspace_target_dir" cargo test \
  --manifest-path "$manifest" \
  -p nixling-host \
  --all-features \
  -- \
  nftables::tests::detector_clean_host \
  nftables::tests::detector_single_manager_unambiguous \
  nftables::tests::detector_multiple_managers_unknown \
  --nocapture
ok "nixling-host: detector covers None/single/multiple"

log "--> nixling-priv-broker ApplyNftables refusal cases"
CARGO_TARGET_DIR="$broker_target_dir" cargo test \
  --manifest-path "$broker_manifest" \
  --all-features \
  -- \
  ops::nft::tests::refuses_when_firewalld_declared_coexist \
  ops::nft::tests::applies_on_clean_host_with_coexist \
  ops::nft::tests::nft_script_includes_all_four_chains \
  --nocapture
ok "broker: ApplyNftables matrix wired through to the rendered script"

ok "tests/nft-coexistence.sh: all 7 detector→policy rows enforced"
