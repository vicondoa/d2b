#!/usr/bin/env bash
# W3 s4 L1c canary: kernel-module 4-step probe + modules_disabled refusal +
# modprobe-denied-not-in-matrix + br_netfilter sysctl tightening.
#
# Drives the deterministic helpers in nixling-host::modules and
# nixling-priv-broker::ops::modprobe (no /proc access needed). The shell
# wrapper just exercises the cargo-test surface and asserts the canary
# row names from plan.md §"W3 pre-merge canary matrix" pass.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

if [ -n "${NL_RUST_TOOLCHAIN_PATH:-}" ]; then
  PATH="$NL_RUST_TOOLCHAIN_PATH:$PATH"
  export PATH
fi

if [ -z "${NIXLING_W3_S4_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    echo "kernel-module-matrix: neither cargo nor nix is on PATH" >&2
    exit 1
  fi
  export NIXLING_W3_S4_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi

# shellcheck source=lib.sh
. "$HERE/lib.sh"

export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$(nl_cargo_target_dir workspace)}
export RUSTC_WRAPPER=${RUSTC_WRAPPER:-}

log "==> tests/kernel-module-matrix.sh"

cd "$ROOT/packages"

# modules-disabled-sysctl-locked canary (plan.md §"W3 pre-merge canary matrix").
log "  canary: modules-disabled-sysctl-locked"
cargo test -p nixling-host --all-features --lib modules::tests::modules_disabled_locks_required_absent_module 2>&1 | tail -5
cargo test -p nixling-host --all-features --lib modules::tests::loaded_module_passes_even_with_modules_disabled 2>&1 | tail -5
cargo test -p nixling-host --all-features --lib modules::tests::builtin_module_passes_even_with_modules_disabled 2>&1 | tail -5

# modprobe-denied-not-in-matrix canary.
log "  canary: modprobe-denied-not-in-matrix"
cd "$ROOT/packages/nixling-priv-broker"
cargo test --all-features --lib ops::modprobe::tests::unknown_module_denied_not_in_matrix 2>&1 | tail -5
cargo test --all-features --lib ops::modprobe::tests::matrix_row_with_load_allowed_false_refuses_silently 2>&1 | tail -5
cargo test --all-features --lib ops::modprobe::tests::modules_disabled_blocks_loadable_request 2>&1 | tail -5

# br_netfilter sysctl tightening canary.
log "  canary: br_netfilter sysctl tightening"
cd "$ROOT/packages"
cargo test -p nixling-host --all-features --lib modules::tests::br_netfilter_loaded_triggers_bridge_nf_recommendations 2>&1 | tail -5

# Parser surface canaries.
cargo test -p nixling-host --all-features --lib modules::tests::parse_proc_modules_extracts_names 2>&1 | tail -5
cargo test -p nixling-host --all-features --lib modules::tests::parse_modules_builtin_strips_path_and_compression_suffix 2>&1 | tail -5
cargo test -p nixling-host --all-features --lib modules::tests::kernel_config_skips_not_set_comments 2>&1 | tail -5

ok "tests/kernel-module-matrix.sh: every W3 kernel-module canary passed"
