#!/usr/bin/env bash
# P1 usbip byte-parity gate: pins
# tests/golden/runner-shape/usbip-argv-minimal.txt against the
# nixling_host::usbip_argv generator's bind + unbind shapes.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=cli-rust-native-common.sh
. "$HERE/cli-rust-native-common.sh"

log "==> tests/usbip-argv-shape.sh"
scratch=$(nl_mktemp .usbip-argv-shape.XXXXXX)
add_cleanup "rm -rf -- \"$scratch\""
golden="$ROOT/tests/golden/runner-shape/usbip-argv-minimal.txt"
output_file="$scratch/cargo.out"
actual_file="$scratch/actual.txt"
expected_file="$scratch/expected.txt"

[ -f "$golden" ] || {
  fail "missing golden: $golden"
  exit 1
}

nl_cli_toolchain_shell "cd '$ROOT/packages' && CARGO_TARGET_DIR='$(nl_cargo_target_dir workspace)' cargo test -q --manifest-path '$ROOT/packages/Cargo.toml' -p nixling-host --lib usbip_argv::tests::argv_snapshot_lines -- --nocapture --test-threads=1" >"$output_file"

snapshot_count=$(grep -c 'SNAPSHOT: ' "$output_file" || true)
assert_eq "$snapshot_count" 2 "usbip snapshot line count"
sed -n 's/^.*SNAPSHOT: //p' "$output_file" >"$actual_file"
sed '/^#/d' "$golden" >"$expected_file"

if cmp -s "$actual_file" "$expected_file"; then
  ok "usbip argv matches tests/golden/runner-shape/usbip-argv-minimal.txt"
else
  diff -u "$expected_file" "$actual_file" >&2 || true
  fail "usbip argv drifted from tests/golden/runner-shape/usbip-argv-minimal.txt"
  exit 1
fi
