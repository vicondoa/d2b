#!/usr/bin/env bash
# W3 s4 L1c canary: device-node matrix mode/group/ACL validators + Open*
# pre-open decision + SCM_RIGHTS fd return.
#
# Drives nixling-host::devices and nixling-priv-broker::ops::device. The
# fd-passing layer itself is already exercised by tests/broker-scm-rights-
# fd-lifecycle.sh (W2 gate); this canary focuses on the typed device
# matrix decisions.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

if [ -n "${NL_RUST_TOOLCHAIN_PATH:-}" ]; then
  PATH="$NL_RUST_TOOLCHAIN_PATH:$PATH"
  export PATH
fi

if [ -z "${NIXLING_W3_S4_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    echo "device-node-matrix: neither cargo nor nix is on PATH" >&2
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

log "==> tests/device-node-matrix.sh"

cd "$ROOT/packages"

log "  canary: mode/group/kind validation"
for t in \
  ok_when_mode_and_group_match \
  missing_required_when_absent \
  missing_optional_when_absent_and_not_required \
  wrong_kind_when_directory_seen_for_char_device \
  loose_mode_when_world_bit_set \
  wrong_group_when_group_name_differs \
  validate_with_collects_fail_closed_classes; do
  cargo test -p nixling-host --all-features --lib devices::tests::$t 2>&1 | tail -3
done

cd "$ROOT/packages/nixling-priv-broker"
log "  canary: Open* pre-open decision"
for t in \
  allowed_when_role_declares_class_and_matrix_validates \
  denied_role_class_mismatch \
  denied_not_in_matrix_for_undeclared_path \
  denied_validation_propagates \
  open_device_fd_against_dev_null_succeeds; do
  cargo test --all-features --lib ops::device::tests::$t 2>&1 | tail -3
done

ok "tests/device-node-matrix.sh: every W3 device-node canary passed"
