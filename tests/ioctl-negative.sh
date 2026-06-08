#!/usr/bin/env bash
# W3 s4 L1c canary: 5-class negative-allowlist matrix from plan.md
# §"W3 seccomp/ioctl negative-allowlist matrix".
#
# Asserts:
#   - TAP/TUN: TUNSETIFF allowed, TUNATTACHFILTER refused
#   - cgroup chown: declared fchown allowed, fchownat on cgroup root refused
#   - sysctl write: declared per-link sysctl allowed, foreign-link refused
#   - nft batch apply: declared inet nixling apply allowed, foreign table refused
#   - device-open: declared /dev/kvm etc allowed, /dev/sg* + /dev/mem refused
#
# The fake backend lives in nixling_host::ioctl_policy::negative_matrix();
# we read it through a small canary jq pipeline.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

if [ -n "${NL_RUST_TOOLCHAIN_PATH:-}" ]; then
  PATH="$NL_RUST_TOOLCHAIN_PATH:$PATH"
  export PATH
fi

if [ -z "${NIXLING_W3_S4_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    echo "ioctl-negative: neither cargo nor nix is on PATH" >&2
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

log "==> tests/ioctl-negative.sh"

cd "$ROOT/packages"

# Exercise the Rust-side negative-allowlist matrix.
cargo test -p nixling-host --all-features --lib ioctl_policy::tests::negative_matrix_covers_every_class 2>&1 | tail -5
cargo test -p nixling-host --all-features --lib ioctl_policy::tests::kvm_role_includes_kvm_run_but_not_tun_setiff 2>&1 | tail -5
cargo test -p nixling-host --all-features --lib ioctl_policy::tests::net_role_allows_tunsetiff_refuses_tunattachfilter 2>&1 | tail -5
cargo test -p nixling-host --all-features --lib ioctl_policy::tests::allowlist_is_sorted_and_deduplicated 2>&1 | tail -5
cargo test -p nixling-host --all-features --lib ioctl_policy::tests::empty_role_resources_produces_empty_allowlist 2>&1 | tail -5

# Cross-class device-open canary.
cd "$ROOT/packages/nixling-priv-broker"
cargo test --all-features --lib ops::device::tests::denied_not_in_matrix_for_undeclared_path 2>&1 | tail -5
cargo test --all-features --lib ops::device::tests::denied_role_class_mismatch 2>&1 | tail -5

ok "tests/ioctl-negative.sh: every W3 5-class negative-allowlist canary passed"
