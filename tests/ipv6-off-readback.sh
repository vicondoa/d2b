#!/usr/bin/env bash
# tests/ipv6-off-readback.sh — W3 s2 L1c canary.
#
# Drives the 5-step IPv6-off sequence (plan.md §"W3 IPv6-off ordering")
# through the nixling-host fake netlink backend:
#
#   1. pre-create (NM unmanaged + reload command precondition)
#   2. create link with IFF_UP cleared
#   3. write per-link sysctls while link is down
#   4. bring link up
#   5. readback gate (fail closed on drift)
#
# Asserts the canonical canaries `ipv6-sysctl-drift` and
# `nm-reload-required` from plan.md §"W3 pre-merge canary matrix".

set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
# shellcheck source=lib.sh
. "$HERE/lib.sh"

nl_activate_rust_toolchain_path || true

LOG=${TMPDIR:-/tmp}/nixling-ipv6-off-readback.$$.log
: > "$LOG"
exec > >(tee -a "$LOG") 2>&1

cd "$ROOT/packages"

log "W3 s2 :: ipv6-off-readback canary"
log " - five-step sequence runs in order"
CARGO_BUILD_RUSTC_WRAPPER="" cargo test -p nixling-host --all-features --quiet -- \
  ipv6_off_sequence_runs_in_order

log " - ipv6-sysctl-drift fails closed on drift between create and link-up"
CARGO_BUILD_RUSTC_WRAPPER="" cargo test -p nixling-host --all-features --quiet -- \
  drift_after_write_fails_closed

log " - bridge-nf sysctls applied when br_netfilter loaded"
CARGO_BUILD_RUSTC_WRAPPER="" cargo test -p nixling-host --all-features --quiet -- \
  bridge_nf_sysctls_applied_when_loaded

log " - readback failure path: NM unmanaged precondition required"
CARGO_BUILD_RUSTC_WRAPPER="" cargo test -p nixling-host --all-features --quiet -- \
  ipv6_off_sequence_fails_closed_on_nm_precondition

log "OK: ipv6-off-readback canary"
