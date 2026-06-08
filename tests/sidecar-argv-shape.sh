#!/usr/bin/env bash
# GPU / audio / video sidecar argv-generator
# parity. Drives the unit-test surface to assert
# each sidecar argv generator emits the documented audit-parity shape
# AND every input-validation rejection still fires.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

if [ -n "${NL_RUST_TOOLCHAIN_PATH:-}" ]; then
  PATH="$NL_RUST_TOOLCHAIN_PATH:$PATH"
  export PATH
fi

if [ -z "${NIXLING_W5_H4_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    echo "sidecar-argv-shape: neither cargo nor nix is on PATH" >&2
    exit 1
  fi
  export NIXLING_W5_H4_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc nixpkgs#sccache nixpkgs#jq \
    --command bash "$0" "$@"
fi

# shellcheck source=lib.sh
. "$HERE/lib.sh"

export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$(nl_cargo_target_dir workspace)}
export RUSTC_WRAPPER=${RUSTC_WRAPPER:-}

log "==> tests/sidecar-argv-shape.sh"

cd "$ROOT/packages"

gpu_argv_tests=(
  gpu_argv::tests::audit_parity_minimal
  gpu_argv::tests::exec_arg0_matches_systemd_unit_name
  gpu_argv::tests::rejects_invalid_binary_path
  gpu_argv::tests::rejects_empty_vm_name
  gpu_argv::tests::rejects_empty_socket_path
  gpu_argv::tests::rejects_empty_wayland_sock
  gpu_argv::tests::rejects_empty_context_types
  gpu_argv::tests::rejects_empty_displays
  gpu_argv::tests::extra_args_appended_in_order
  gpu_argv::tests::params_renders_multi_display
  gpu_argv::tests::params_renders_subset_context_types
  gpu_argv::tests::params_omits_egl_when_false
  gpu_argv::tests::context_type_string_round_trip
  gpu_argv::tests::context_type_string_is_json_safe
  gpu_argv::tests::argv_is_round_trip_serializable
)

audio_argv_tests=(
  audio_argv::tests::audit_parity_minimal
  audio_argv::tests::exec_arg0_matches_systemd_unit_name
  audio_argv::tests::rejects_non_absolute_binary
  audio_argv::tests::rejects_nix_store_direct_path
  audio_argv::tests::rejects_run_current_system_symlink
  audio_argv::tests::rejects_other_vms_per_vm_copy
  audio_argv::tests::rejects_empty_binary
  audio_argv::tests::rejects_empty_vm_name
  audio_argv::tests::rejects_empty_socket_path
  audio_argv::tests::exec_arg0_rejects_empty_vm_name
  audio_argv::tests::extra_args_appended_in_order
  audio_argv::tests::backend_string_round_trip
  audio_argv::tests::argv_is_round_trip_serializable
)

video_argv_tests=(
  video_argv::tests::audit_parity_minimal
  video_argv::tests::exec_arg0_matches_systemd_unit_name
  video_argv::tests::rejects_non_absolute_binary
  video_argv::tests::rejects_empty_binary
  video_argv::tests::rejects_empty_vm_name
  video_argv::tests::rejects_empty_socket_path
  video_argv::tests::exec_arg0_rejects_empty_vm_name
  video_argv::tests::rejects_unknown_extra_args_field
  video_argv::tests::backend_string_round_trip
  video_argv::tests::argv_is_round_trip_serializable
)

log "  canary: gpu_argv (${#gpu_argv_tests[@]} cases)"
for t in "${gpu_argv_tests[@]}"; do
  cargo test -p nixling-host --lib "$t" 2>&1 | tail -3
done

log "  canary: audio_argv (${#audio_argv_tests[@]} cases)"
for t in "${audio_argv_tests[@]}"; do
  cargo test -p nixling-host --lib "$t" 2>&1 | tail -3
done

log "  canary: video_argv (${#video_argv_tests[@]} cases)"
for t in "${video_argv_tests[@]}"; do
  cargo test -p nixling-host --lib "$t" 2>&1 | tail -3
done

ok "tests/sidecar-argv-shape.sh: every W5-H1/H2/H3 sidecar argv-generator canary passed"
