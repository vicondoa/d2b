#!/usr/bin/env bash
# tests/host-prepare-network.sh — W3 s2 L1c canary.
#
# Drives the fake-backend happy path + the W3 plan's network-reconcile
# fail-closed canaries through cargo tests:
#
#   - nm-managed-foreign-conflict
#   - nm-reload-required (`general reload conf` vs `connection reload`)
#   - bridge-port-flag-drift (every flag, every role)
#   - route-preflight-no-default-route
#   - route-preflight-foreign-default-route
#   - dnsmasq-not-bound
#   - host-lan-cidr-ambiguous
#   - ch-net-handoff-not-supported (asserted via the docs anchor; the
#     wire test lives in W3 s4's device probe scope).
#
# All scratch state lands outside $ROOT per AGENTS.md disk-hygiene
# contract (W2fu4 H8/H9/H14/H15).

set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
# shellcheck source=lib.sh
. "$HERE/lib.sh"

nl_activate_rust_toolchain_path || true

LOG=${TMPDIR:-/tmp}/nixling-host-prepare-network.$$.log
: > "$LOG"
exec > >(tee -a "$LOG") 2>&1

log "W3 s2 :: host-prepare-network canary"

run_focused_host() {
  local label="$1"; shift
  log " - $label"
  ( cd "$ROOT/packages" && CARGO_BUILD_RUSTC_WRAPPER="" cargo test -p nixling-host \
      --all-features --quiet -- "$@" )
}

run_focused_broker() {
  local label="$1"; shift
  log " - $label"
  ( cd "$ROOT/packages/nixling-priv-broker" && CARGO_BUILD_RUSTC_WRAPPER="" cargo test \
      --all-features --quiet -- "$@" )
}

# Happy path: ipv6_off_sequence runs in order + readback matches.
run_focused_host "happy path: ipv6_off_sequence_runs_in_order" \
  ipv6_off_sequence_runs_in_order

# nm-managed-foreign-conflict
run_focused_broker "nm-managed-foreign-conflict" managed_foreign_conflict_detected

# nm-reload-required: assert general reload conf for NM >= 1.20
run_focused_broker "nm-reload-required:general-reload-conf" reload_command_select_by_version

# bridge-port-flag-drift: every flag, every role
run_focused_broker "bridge-port-flag-drift" set_bridge_port_flags_readback_drift_fails_closed
run_focused_host "bridge-port-readback-defaults-pass" readback_matches_defaults

# route-preflight predicates
run_focused_host "route-preflight-no-default-route" no_default_route_fails_closed
run_focused_host "route-preflight-foreign-default-route" foreign_default_route_fails_closed
run_focused_host "dnsmasq-not-bound" dnsmasq_not_bound_fails_closed
run_focused_host "host-lan-cidr-ambiguous" host_lan_cidr_ambiguous_for_vpn

# ch-net-handoff-not-supported: documented in host-prepare.d/network.md;
# enforced by the broker's CreateTap variant guard. We assert the
# guard exists via a grep on the produced docs fragment so future
# rewordings don't silently drop the failure mode from operator docs.
HANDOFF_DOC="$ROOT/docs/how-to/host-prepare.d/network.md"
if ! grep -q "ch-net-handoff-not-supported" "$HANDOFF_DOC"; then
  fail "ch-net-handoff-not-supported failure mode missing from $HANDOFF_DOC"
fi
ok "ch-net-handoff-not-supported documented"

log "OK: host-prepare-network canary"
