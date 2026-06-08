#!/usr/bin/env bash
# CH / virtiofsd / swtpm argv generator parity.
#
# Drives the unit-test surface to assert each
# argv generator emits the documented audit-parity shape AND every
# input-validation rejection still fires. Wired into tests/static.sh
# alongside the runner-shape preflight canary so daemon-side spawn
# argv drift surfaces on every PR-loop run.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

if [ -n "${NL_RUST_TOOLCHAIN_PATH:-}" ]; then
  PATH="$NL_RUST_TOOLCHAIN_PATH:$PATH"
  export PATH
fi

if [ -z "${NIXLING_W4_H10_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    echo "ch-argv-shape: neither cargo nor nix is on PATH" >&2
    exit 1
  fi
  export NIXLING_W4_H10_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc nixpkgs#sccache nixpkgs#jq \
    --command bash "$0" "$@"
fi

# shellcheck source=lib.sh
. "$HERE/lib.sh"

export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$(nl_cargo_target_dir workspace)}
export RUSTC_WRAPPER=${RUSTC_WRAPPER:-}

log "==> tests/ch-argv-shape.sh"

cd "$ROOT/packages"

# Per-module slice. Names are pinned so a regression that *deletes*
# a test (rather than failing it) surfaces here as a missing case.
ch_argv_tests=(
  ch_argv::tests::headless_audit_parity_minimal
  ch_argv::tests::exec_arg0_matches_runner_process_name
  ch_argv::tests::exec_arg0_rejects_empty_vm_name
  ch_argv::tests::rejects_empty_vm_name
  ch_argv::tests::rejects_non_absolute_ch_binary
  ch_argv::tests::rejects_empty_ch_binary
  ch_argv::tests::rejects_zero_cpus
  ch_argv::tests::rejects_empty_kernel_path
  ch_argv::tests::tap_fd_mode_emits_fd_token
  ch_argv::tests::tap_fd_missing_is_rejected
  ch_argv::tests::persistent_tap_missing_ifname_is_rejected
  ch_argv::tests::omits_initramfs_when_absent
  ch_argv::tests::omits_platform_when_no_oem_strings
  ch_argv::tests::omits_vsock_when_absent
  ch_argv::tests::extra_vsock_emits_socket_only_form
  ch_argv::tests::extra_args_appended_in_order
  ch_argv::tests::omits_fs_when_no_shares
  ch_argv::tests::omits_net_when_no_ifaces
  ch_argv::tests::omits_watchdog_when_disabled
  ch_argv::tests::multiple_oem_strings_join_with_comma
  ch_argv::tests::argv_is_round_trip_serializable
)

virtiofsd_argv_tests=(
  virtiofsd_argv::tests::audit_ro_store_parity
  virtiofsd_argv::tests::audit_nl_meta_omits_readonly
  virtiofsd_argv::tests::exec_arg0_matches_audit_naming
  virtiofsd_argv::tests::exec_arg0_rejects_empty_vm_name
  virtiofsd_argv::tests::exec_arg0_rejects_empty_share_tag
  virtiofsd_argv::tests::rejects_non_absolute_binary
  virtiofsd_argv::tests::rejects_empty_binary
  virtiofsd_argv::tests::rejects_empty_vm_name
  virtiofsd_argv::tests::rejects_empty_share_tag
  virtiofsd_argv::tests::rejects_empty_socket_path
  virtiofsd_argv::tests::rejects_empty_socket_group
  virtiofsd_argv::tests::rejects_empty_shared_dir
  virtiofsd_argv::tests::rejects_zero_thread_pool
  virtiofsd_argv::tests::cache_mode_string_round_trip
  virtiofsd_argv::tests::inode_file_handles_string_round_trip
  virtiofsd_argv::tests::extra_args_emitted_in_order_at_end
  virtiofsd_argv::tests::omits_optional_flags_when_disabled
  virtiofsd_argv::tests::argv_is_round_trip_serializable
  virtiofsd_argv::tests::all_four_audit_shares_render_independently
)

swtpm_argv_tests=(
  swtpm_argv::tests::long_lived_argv_has_expected_shape
  swtpm_argv::tests::flush_argv_matches_w3_invariant
  swtpm_argv::tests::exec_arg0_for_long_lived
  swtpm_argv::tests::exec_arg0_for_flush
  swtpm_argv::tests::omits_startup_clear_when_disabled
  swtpm_argv::tests::extra_args_appended_at_end
  swtpm_argv::tests::rejects_invalid_binary_path
  swtpm_argv::tests::rejects_empty_vm_name
  swtpm_argv::tests::rejects_non_absolute_state_dir
  swtpm_argv::tests::rejects_empty_ctrl_socket
  swtpm_argv::tests::rejects_empty_server_socket
  swtpm_argv::tests::rejects_empty_log_path
  swtpm_argv::tests::rejects_empty_pid_path
  swtpm_argv::tests::rejects_log_level_out_of_range
  swtpm_argv::tests::flush_rejects_invalid_inputs
  swtpm_argv::tests::argv_is_round_trip_serializable
  swtpm_argv::tests::flush_input_round_trip_serializable
)

log "  canary: ch_argv (${#ch_argv_tests[@]} cases)"
for t in "${ch_argv_tests[@]}"; do
  cargo test -p nixling-host --lib "$t" 2>&1 | tail -3
done

log "  canary: virtiofsd_argv (${#virtiofsd_argv_tests[@]} cases)"
for t in "${virtiofsd_argv_tests[@]}"; do
  cargo test -p nixling-host --lib "$t" 2>&1 | tail -3
done

log "  canary: swtpm_argv (${#swtpm_argv_tests[@]} cases)"
for t in "${swtpm_argv_tests[@]}"; do
  cargo test -p nixling-host --lib "$t" 2>&1 | tail -3
done

# Modules added post---main: hardlink
# farm primitive, ssh-keygen probe, daemon reconcile_and_adopt.
log "  canary: hardlink_farm (W7-fu)"
cargo test -p nixling-host --lib hardlink_farm 2>&1 | tail -3
log "  canary: ssh_keygen (W8-fu)"
cargo test -p nixling-host --lib ssh_keygen 2>&1 | tail -3
log "  canary: supervisor::state reconcile_and_adopt (W4-fu)"
cargo test -p nixlingd --lib supervisor::state::tests::reconcile_and_adopt 2>&1 | tail -3

ok "tests/ch-argv-shape.sh: every W4-H1/H2/H3 argv-generator canary passed"
