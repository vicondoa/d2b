#!/usr/bin/env bash
# OtelHostBridge argv byte-parity gate.
#
# Diffs `cargo test -p nixling-host --lib otel_host_bridge_argv`'s
# SNAPSHOT line against
# tests/golden/runner-shape/otel-host-bridge-argv-minimal.txt. Catches
# drift between the broker-spawned runner argv and the singleton
# nixling-otel-host-bridge.service ExecStart line being retired in v1.0.
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=cli-rust-native-common.sh
. "$HERE/cli-rust-native-common.sh"

log "==> tests/otel-host-bridge-argv-shape.sh"
scratch=$(nl_mktemp .otel-host-bridge-argv-shape.XXXXXX)
add_cleanup "rm -rf -- \"$scratch\""
golden="$ROOT/tests/golden/runner-shape/otel-host-bridge-argv-minimal.txt"
output_file="$scratch/cargo.out"
actual_file="$scratch/actual.txt"
expected_file="$scratch/expected.txt"

[ -f "$golden" ] || {
  fail "missing golden: $golden"
  exit 1
}

nl_cli_toolchain_shell "cd '$ROOT/packages' && CARGO_TARGET_DIR='$(nl_cargo_target_dir workspace)' cargo test -q --manifest-path '$ROOT/packages/Cargo.toml' -p nixling-host --lib otel_host_bridge_argv -- --nocapture --test-threads=1" >"$output_file"

snapshot_count=$(grep -c 'SNAPSHOT: ' "$output_file" || true)
assert_eq "$snapshot_count" 1 "otel-host-bridge snapshot line count"
sed -n 's/^.*SNAPSHOT: //p' "$output_file" >"$actual_file"
sed '/^#/d' "$golden" >"$expected_file"

if cmp -s "$actual_file" "$expected_file"; then
  ok "otel-host-bridge argv matches tests/golden/runner-shape/otel-host-bridge-argv-minimal.txt"
else
  diff -u "$expected_file" "$actual_file" >&2 || true
  fail "otel-host-bridge argv drifted from tests/golden/runner-shape/otel-host-bridge-argv-minimal.txt"
  exit 1
fi
