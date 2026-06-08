#!/usr/bin/env bash
# tests/ssh-host-key-preflight-eval.sh—
# integration test for the daemon-side VM-start preflight that refuses
# VM start when `/var/lib/nixling/vms/<vm>/sshd-host-keys` (or one of
# its `ssh_host_*_key` leaves) drifts from the canonical posture.
#
# Strategy (mirrors tests/runner-shape-preflight.sh): drive the pure
# `nixlingd::ssh_host_key_preflight` module via its cargo unit tests.
# The module is purely a filesystem stat check, so a hermetic cargo
# test against a tempdir-built drift fixture covers every failure
# class deterministically without needing root or a live host.
#
# Failure classes asserted (see packages/nixlingd/src/ssh_host_key_preflight.rs):
#   * happy path (canonical posture)               — happy_path_when_running_as_root
#   * missing keys directory                       — missing_directory_is_drift
#   * symlinked keys directory                     — symlink_directory_is_drift
#   * keys directory replaced by a regular file    — non_directory_path_is_drift
#   * empty keys directory tolerated               — empty_keys_dir_is_ok
#   * .pub keys and unrelated files ignored        — ignores_pub_keys_and_unrelated_files
#   * wrong owner on a key file                    — wrong_owner_is_drift
#   * wrong mode on a key file                     — wrong_mode_is_drift
#   * symlinked key file                           — symlink_key_is_drift
#   * drift accessor surfaces offending path       — drift_path_accessor_returns_offending_path
#
# Also asserts the typed-error wiring (`SshdHostKeyDrift` exit code 62
# and its envelope shape).

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

if [ -n "${NL_RUST_TOOLCHAIN_PATH:-}" ]; then
  PATH="$NL_RUST_TOOLCHAIN_PATH:$PATH"
  export PATH
fi

if [ -z "${NIXLING_SSH_HOST_KEY_PREFLIGHT_IN_NIX_SHELL:-}" ] \
   && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    echo "ssh-host-key-preflight-eval: neither cargo nor nix is on PATH" >&2
    exit 1
  fi
  export NIXLING_SSH_HOST_KEY_PREFLIGHT_IN_NIX_SHELL=1
  exec nix --extra-experimental-features 'nix-command flakes' shell \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc \
    --command bash "$0" "$@"
fi

log() { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }

log "==> tests/ssh-host-key-preflight-eval.sh"

export RUSTC_WRAPPER=${RUSTC_WRAPPER:-}

cd "$ROOT/packages/nixlingd"

log "  cargo test --lib ssh_host_key_preflight"
cargo test --lib ssh_host_key_preflight -- --nocapture

log "  cargo test --lib typed_error::tests::sshd_host_key_drift_envelope_shape"
cargo test --lib typed_error::tests::sshd_host_key_drift_envelope_shape -- --nocapture

log "  cargo test --lib typed_error::tests::envelope_kind_matches_expected_discriminant"
cargo test --lib typed_error::tests::envelope_kind_matches_expected_discriminant

log "PASS: ssh-host-key-preflight-eval"
