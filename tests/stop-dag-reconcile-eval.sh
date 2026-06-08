#!/usr/bin/env bash
# Assert the stop-DAG owner module + docs
# carry the documented surface and that the planner only dispatches
# through existing broker ops (no new wire variants).

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

ok() {
    echo "  ok: $*"
}

echo "==> stop-DAG owner module surface"
MOD="packages/nixlingd/src/supervisor/stop_dag.rs"
[ -f "$MOD" ] || fail "missing $MOD"

for sym in \
    "pub struct StopDagOwner" \
    "pub struct ObservedHostState" \
    "pub struct ReconcileReport" \
    "pub struct NftablesReconcileAction" \
    "pub struct UsbipReconcileAction" \
    "pub enum NftablesDriftReason" \
    "pub enum UsbipDriftReason" \
    "pub fn reconcile_on_restart" \
    "pub fn reconcile("; do
    grep -qF "$sym" "$MOD" || fail "stop_dag.rs missing '$sym'"
    ok "$sym"
done

echo "==> supervisor mod wires stop_dag"
grep -q "pub mod stop_dag;" packages/nixlingd/src/supervisor/mod.rs \
    || fail "supervisor/mod.rs does not declare stop_dag module"
ok "pub mod stop_dag"

echo "==> planner uses only existing broker ops (no new wire variants)"
WIRE="packages/nixling-ipc/src/broker_wire.rs"
# The planner must not introduce a new BrokerRequest variant; assert
# the three ops it composes against are present.
for variant in ApplyNftables UsbipBind UsbipUnbind; do
    grep -q "${variant}(${variant}Request)" "$WIRE" \
        || fail "broker_wire.rs missing pre-existing BrokerRequest::${variant}"
    ok "BrokerRequest::${variant} (reused, not redeclared)"
done

# Negative: the stop_dag module must not declare a `pub enum` /
# `pub struct` that ends in `Request` (that would be a wire-shape
# addition smuggled in via the planner).
if grep -E "pub (struct|enum) [A-Za-z]+Request\b" "$MOD" >/dev/null; then
    fail "stop_dag.rs declares a *Request type; it must dispatch through existing broker wire variants"
fi
ok "no new *Request types declared in stop_dag.rs"

echo "==> documentation"
DOC="docs/reference/stop-dag-reconcile.md"
[ -f "$DOC" ] || fail "missing $DOC"
for marker in \
    "stop-dag-reconcile" \
    "StopDagOwner" \
    "ApplyNftables" \
    "UsbipBind" \
    "UsbipUnbind" \
    "reconcile_on_restart" \
    "ObservedHostState"; do
    grep -qF "$marker" "$DOC" || fail "stop-dag-reconcile.md missing '$marker'"
    ok "doc references '$marker'"
done

echo "PASS stop-dag-reconcile-eval.sh"
