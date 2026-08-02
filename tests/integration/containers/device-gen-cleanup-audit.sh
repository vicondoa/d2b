#!/usr/bin/env bash
set -euo pipefail

HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(cd -- "$HERE/../../.." >/dev/null 2>&1 && pwd)}
export ROOT

. "$ROOT/tests/tools/heavy-gate-reexec.sh"
d2b_heavy_gate_reexec "$ROOT" "$0" "$@"

. "$HERE/lib.sh"
cd "$ROOT"

if ! command -v cargo >/dev/null 2>&1; then
  nlc_log "SKIP: cargo unavailable - cleanup audit container smoke needs the Rust test binary"
  exit 0
fi

nlc_require_podman
cargo test --manifest-path packages/Cargo.toml -p d2b-core-controller \
  --test config_cleanup --no-run

binary=$(find packages/target/debug/deps -maxdepth 1 -type f \
  -name 'config_cleanup-*' -perm -111 -print | sort | tail -1)
[ -n "$binary" ] || nlc_fail "config_cleanup test binary was not built"

container_name="d2b-device-cleanup-audit-$$"
cleanup() {
  "${NLC_PODMAN[@]}" rm -f "$container_name" >/dev/null 2>&1 || true
}
trap cleanup EXIT

set +e
output=$("${NLC_PODMAN[@]}" run --rm --name "$container_name" --network none \
  --volume "$binary:/d2b-test:ro" \
  docker.io/library/ubuntu:24.04 \
  /bin/sh -eu -c '/d2b-test final_deletion_is_atomic --exact' 2>&1)
status=$?
set -e

[ "$status" -eq 0 ] || nlc_fail "cleanup audit container exited $status: $output"
nlc_assert_contains "$output" "test result: ok" "cleanup audit"
nlc_ok "final deletion releases its audit proof only after the committed cleanup"
