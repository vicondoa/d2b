#!/usr/bin/env bash
# P1 byte-parity gate for the daemon-spawned vhost-user-media (video)
# sidecar. Drives the `audit_parity_snapshot_line` test in
# `packages/nixling-host/src/video_argv.rs` and byte-compares the captured
# argv + kernel-8 wire-contract pins against
# `tests/golden/runner-shape/video-argv-minimal.txt`.
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=cli-rust-native-common.sh
. "$HERE/cli-rust-native-common.sh"

log "==> tests/video-argv-shape.sh"
scratch=$(nl_mktemp .video-argv-shape.XXXXXX)
add_cleanup "rm -rf -- \"$scratch\""
golden="$ROOT/tests/golden/runner-shape/video-argv-minimal.txt"
output_file="$scratch/cargo.out"
actual_file="$scratch/actual.txt"
expected_file="$scratch/expected.txt"

[ -f "$golden" ] || {
  fail "missing golden: $golden"
  exit 1
}

nl_cli_toolchain_shell "cd '$ROOT/packages' && CARGO_TARGET_DIR='$(nl_cargo_target_dir workspace)' RUSTC_WRAPPER='' cargo test -q --manifest-path '$ROOT/packages/Cargo.toml' -p nixling-host --lib video_argv::tests::audit_parity_snapshot_line -- --nocapture --test-threads=1" >"$output_file"

snapshot_count=$(grep -c '^SNAPSHOT: ' "$output_file" || true)
wire_count=$(grep -c '^WIRE: ' "$output_file" || true)
assert_eq "$snapshot_count" 1 "video snapshot line count"
assert_eq "$wire_count" 1 "video wire-contract line count"

{
  sed -n 's/^SNAPSHOT: //p' "$output_file"
  sed -n 's/^WIRE: //p' "$output_file"
} >"$actual_file"
sed '/^#/d;/^[[:space:]]*$/d' "$golden" >"$expected_file"

if cmp -s "$actual_file" "$expected_file"; then
  ok "video argv + wire-contract matches tests/golden/runner-shape/video-argv-minimal.txt"
else
  diff -u "$expected_file" "$actual_file" >&2 || true
  fail "video argv/wire-contract drifted from tests/golden/runner-shape/video-argv-minimal.txt"
  exit 1
fi
