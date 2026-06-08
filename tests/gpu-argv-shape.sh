#!/usr/bin/env bash
# Gpu role byte-parity gate.
#
# Compares the snapshot line emitted by
# `nixling_host::gpu_argv::tests::daemon_input_snapshot_line` against
# `tests/golden/runner-shape/gpu-argv-minimal.txt`. Drift in argv
# ordering, flag spelling, the `--params` JSON layout (cross-domain
# Wayland), or the per-VM wayland-0 bind path fails the wave.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=cli-rust-native-common.sh
. "$HERE/cli-rust-native-common.sh"

log "==> tests/gpu-argv-shape.sh"
scratch=$(nl_mktemp .gpu-argv-shape.XXXXXX)
add_cleanup "rm -rf -- \"$scratch\""
golden="$ROOT/tests/golden/runner-shape/gpu-argv-minimal.txt"
output_file="$scratch/cargo.out"
actual_file="$scratch/actual.txt"
expected_file="$scratch/expected.txt"

[ -f "$golden" ] || {
  fail "missing golden: $golden"
  exit 1
}

nl_cli_toolchain_shell "cd '$ROOT/packages' && CARGO_TARGET_DIR='$(nl_cargo_target_dir workspace)' cargo test -q --manifest-path '$ROOT/packages/Cargo.toml' -p nixling-host --lib gpu_argv -- --nocapture --test-threads=1" >"$output_file"

snapshot_count=$(grep -c 'SNAPSHOT: ' "$output_file" || true)
assert_eq "$snapshot_count" 1 "gpu snapshot line count"
sed -n 's/^.*SNAPSHOT: //p' "$output_file" >"$actual_file"
sed '/^#/d' "$golden" | sed '/^$/d' >"$expected_file"

if cmp -s "$actual_file" "$expected_file"; then
  ok "gpu argv matches tests/golden/runner-shape/gpu-argv-minimal.txt"
else
  diff -u "$expected_file" "$actual_file" >&2 || true
  fail "gpu argv drifted from tests/golden/runner-shape/gpu-argv-minimal.txt"
  exit 1
fi
