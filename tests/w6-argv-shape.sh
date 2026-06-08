#!/usr/bin/env bash
# Static gate: vsock-relay + USBIP argv-generator parity.
# Drives the unit-test surface pinned by name.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

if [ -n "${NL_RUST_TOOLCHAIN_PATH:-}" ]; then
  PATH="$NL_RUST_TOOLCHAIN_PATH:$PATH"
  export PATH
fi

if [ -z "${NIXLING_W6_H3_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    echo "vsock-usbip-argv-shape: neither cargo nor nix is on PATH" >&2
    exit 1
  fi
  export NIXLING_W6_H3_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc nixpkgs#sccache nixpkgs#jq \
    --command bash "$0" "$@"
fi

# shellcheck source=lib.sh
. "$HERE/lib.sh"

export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$(nl_cargo_target_dir workspace)}
export RUSTC_WRAPPER=${RUSTC_WRAPPER:-}

log "==> tests/vsock-usbip-argv-shape.sh"

cd "$ROOT/packages"

vsock_relay_tests=(
  vsock_relay_argv::tests::stack_vsock_in_parity
  vsock_relay_argv::tests::guest_egress_parity
  vsock_relay_argv::tests::exec_arg0_round_trip
  vsock_relay_argv::tests::rejects_non_absolute_socat
  vsock_relay_argv::tests::rejects_empty_socat
  vsock_relay_argv::tests::rejects_empty_relay_name
  vsock_relay_argv::tests::rejects_source_that_is_connect
  vsock_relay_argv::tests::rejects_source_vsock_connect
  vsock_relay_argv::tests::rejects_empty_endpoint_path_in_source
  vsock_relay_argv::tests::rejects_empty_endpoint_path_in_sink
  vsock_relay_argv::tests::omits_max_children_when_absent
  vsock_relay_argv::tests::unix_listen_renders_mode_octal
  vsock_relay_argv::tests::extra_args_appended_in_order
  vsock_relay_argv::tests::argv_is_round_trip_serializable
  vsock_relay_argv::tests::rejects_unix_listen_path_with_comma_injection
  vsock_relay_argv::tests::rejects_unix_connect_path_with_semicolon_injection
  vsock_relay_argv::tests::rejects_path_with_whitespace
  vsock_relay_argv::tests::rejects_path_with_quote
  vsock_relay_argv::tests::rejects_path_with_colon
  vsock_relay_argv::tests::rejects_path_with_brackets
  vsock_relay_argv::tests::rejects_path_with_nul
  vsock_relay_argv::tests::unix_listen_to_unix_connect_shape_is_supported
)

usbip_tests=(
  usbip_argv::tests::bind_argv_has_expected_shape
  usbip_argv::tests::unbind_argv_has_expected_shape
  usbip_argv::tests::accepts_chained_hub_bus_id
  usbip_argv::tests::accepts_deeply_chained_hub_bus_id
  usbip_argv::tests::accepts_root_only_bus_id
  usbip_argv::tests::accepts_multi_digit_bus_number
  usbip_argv::tests::rejects_invalid_binary_path
  usbip_argv::tests::rejects_empty_bus_id
  usbip_argv::tests::rejects_shell_metachar_bus_id
  usbip_argv::tests::rejects_bus_id_with_letters
  usbip_argv::tests::rejects_bus_id_with_empty_port
  usbip_argv::tests::rejects_bus_id_with_empty_chain_segment
  usbip_argv::tests::rejects_bus_id_with_leading_dot
  usbip_argv::tests::rejects_bus_id_with_trailing_dot
  usbip_argv::tests::rejects_unicode_digits
  usbip_argv::tests::rejects_leading_zero_in_bus_segment
  usbip_argv::tests::rejects_leading_zero_in_chained_segment
  usbip_argv::tests::accepts_literal_zero_segment
  usbip_argv::tests::rejects_bus_id_over_sysfs_max_length
  usbip_argv::tests::accepts_bus_id_at_sysfs_max_length
  usbip_argv::tests::rejects_bus_id_with_slash
  usbip_argv::tests::rejects_bus_id_with_space
  usbip_argv::tests::subcommand_string_round_trip
  usbip_argv::tests::argv_input_round_trip_serializable
)

log "  canary: vsock_relay_argv (${#vsock_relay_tests[@]} cases)"
for t in "${vsock_relay_tests[@]}"; do
  cargo test -p nixling-host --lib "$t" 2>&1 | tail -3
done

log "  canary: usbip_argv (${#usbip_tests[@]} cases)"
for t in "${usbip_tests[@]}"; do
  cargo test -p nixling-host --lib "$t" 2>&1 | tail -3
done

ok "tests/vsock-usbip-argv-shape.sh: every vsock/USBIP argv-generator canary passed"
