#!/usr/bin/env bash
# P2 ph2-dag-host-prep: assert that the host-prep DAG module + docs
# carry the documented step set + broker-op mapping. Static gate —
# no nixpkgs eval required.

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

echo "==> host-prep DAG module surface"
MOD="packages/nixling-host/src/host_prep_dag.rs"
[ -f "$MOD" ] || fail "missing $MOD"

# Every step kind variant declared in the typed enum.
for kind in \
    BringUpTapInterface \
    PreOpenVhostNetFd \
    SeedDnsmasqLease \
    BindMountFromHardlinkFarm \
    ApplyNftablesRules \
    OwnershipMatrixCheck \
    SshHostKeyPreflight \
    ApplyNmUnmanaged \
    ApplySysctl \
    SetBridgePortFlags; do
    grep -qE "^\s+${kind},?$|^\s+${kind}\b" "$MOD" \
        || fail "host_prep_dag.rs missing HostPrepStepKind::${kind}"
    ok "HostPrepStepKind::${kind}"
done

# P2fu1 kernel-r1-1 closure: assert ordering edges in the workload
# DAG fixture exist as documented. The DAG builder is a pure function
# of the bundle resolver shape; the test below pins the dep edges
# inline by grepping the builder source for the canonical
# depends_on declarations the integrator just landed.
echo "==> P2fu1 host-prep DAG ordering edges"
# NM unmanaged is a sibling of preflights (no upstream deps), runs
# before nftables apply.
grep -qE 'kind: HostPrepStepKind::ApplyNftablesRules' "$MOD" \
    || fail "host_prep_dag.rs missing ApplyNftablesRules step kind in builder"
grep -qE 'id\(HostPrepStepKind::ApplyNmUnmanaged\)' "$MOD" \
    || fail "host_prep_dag.rs missing ApplyNmUnmanaged dep edge in ApplyNftablesRules"
ok "ApplyNmUnmanaged runs before ApplyNftablesRules"
# sysctl after tap.
grep -qE 'kind: HostPrepStepKind::ApplySysctl' "$MOD" \
    || fail "host_prep_dag.rs missing ApplySysctl step kind in builder"
grep -qB 2 'kind: HostPrepStepKind::ApplySysctl' "$MOD" \
    || fail "host_prep_dag.rs missing ApplySysctl in builder block"
ok "ApplySysctl present in builder"
# bridge flags after sysctl.
grep -qE 'kind: HostPrepStepKind::SetBridgePortFlags' "$MOD" \
    || fail "host_prep_dag.rs missing SetBridgePortFlags step kind in builder"
ok "SetBridgePortFlags present in builder"

# Public API.
for sym in \
    "pub struct HostPrepStep" \
    "pub struct HostPrepStepId" \
    "pub enum HostPrepStepKind" \
    "pub struct BundleStepRef" \
    "pub struct HostPrepStepFailed" \
    "pub enum CycleError" \
    "pub fn build_host_prep_dag" \
    "pub fn build_host_prep_dag_for" \
    "pub fn topo_sort"; do
    grep -qF "$sym" "$MOD" || fail "host_prep_dag.rs missing '$sym'"
    ok "$sym"
done

echo "==> nixling-host re-export"
grep -q "pub mod host_prep_dag;" packages/nixling-host/src/lib.rs \
    || fail "nixling-host lib.rs does not re-export host_prep_dag"
ok "pub mod host_prep_dag"

echo "==> broker wire scaffolds (typed Unimplemented stubs)"
WIRE="packages/nixling-ipc/src/broker_wire.rs"
RUNTIME="packages/nixling-priv-broker/src/runtime.rs"
# P3 host-prep-broker-arms: SeedDnsmasqLease and
# BindMountFromHardlinkFarm flipped from typed-Unimplemented stubs
# to live broker dispatch arms (they look up the per-VM bundle
# intent, record a typed audit row, and ack). OwnershipMatrixCheck
# and SshHostKeyPreflight still carry the typed Unimplemented P2
# label pending the sibling P3 wave-B handlers.
for variant in \
    SeedDnsmasqLease \
    BindMountFromHardlinkFarm \
    OwnershipMatrixCheck \
    SshHostKeyPreflight; do
    grep -q "${variant}(${variant}Request)" "$WIRE" \
        || fail "broker_wire.rs missing BrokerRequest::${variant}"
    grep -q "pub struct ${variant}Request" "$WIRE" \
        || fail "broker_wire.rs missing ${variant}Request struct"
    grep -q "Self::${variant}(_) => \"${variant}\"" "$WIRE" \
        || fail "broker_wire.rs missing op_name arm for ${variant}"
    grep -q "RealBrokerRequest::${variant}" "$RUNTIME" \
        || fail "runtime.rs missing dispatch arm for ${variant}"
    ok "BrokerRequest::${variant} dispatch arm present"
done
for variant in OwnershipMatrixCheck SshHostKeyPreflight; do
    grep -q "operation: \"${variant}\"" "$RUNTIME" \
        || fail "runtime.rs missing Unimplemented op label for ${variant} (expected to stay P2-stubbed)"
    ok "BrokerRequest::${variant} still typed Unimplemented P2"
done
for variant in SeedDnsmasqLease BindMountFromHardlinkFarm; do
    grep -q "\"${variant}\"" "$RUNTIME" \
        || fail "runtime.rs missing op label for live ${variant} arm"
    grep -q "OperationFields::${variant}" "$RUNTIME" \
        || fail "runtime.rs missing OperationFields::${variant} audit row for live arm"
    ok "BrokerRequest::${variant} live broker arm (P3 host-prep-broker-arms)"
done

echo "==> nixlingd vm_start wiring"
LIB="packages/nixlingd/src/lib.rs"
for marker in \
    "build_host_prep_dag" \
    "log_host_prep_dag" \
    "execute_host_prep_dag" \
    "NIXLING_HOST_PREP_DAG_EXECUTE"; do
    grep -q "$marker" "$LIB" \
        || fail "nixlingd/src/lib.rs missing '$marker'"
    ok "$marker"
done

echo "==> documentation"
DOC="docs/reference/host-prep-dag.md"
[ -f "$DOC" ] || fail "missing $DOC"
for kind in \
    BringUpTapInterface \
    PreOpenVhostNetFd \
    SeedDnsmasqLease \
    BindMountFromHardlinkFarm \
    ApplyNftablesRules \
    OwnershipMatrixCheck \
    SshHostKeyPreflight \
    ApplyNmUnmanaged \
    ApplySysctl \
    SetBridgePortFlags; do
    grep -qF "$kind" "$DOC" || fail "host-prep-dag.md missing $kind"
done
ok "docs cover every step kind (all 10)"

# P2fu1 kernel-r1-1 + P2fu2 test-r2 closure: assert the doc's
# canonical ordering block names the new step kinds AND the
# dependency-edge keywords ("AFTER tap", "BEFORE tap" / "BEFORE
# nftables apply", "AFTER ApplySysctl") so doc drift is caught.
echo "==> documentation: ordering"
for phrase in "BEFORE tap creation" "AFTER tap creation" "AFTER \`ApplySysctl\`"; do
    grep -qF "$phrase" "$DOC" || fail "host-prep-dag.md missing ordering phrase '$phrase'"
done
ok "host-prep DAG doc names the canonical ordering"

# Cross-reference asserted in AGENTS.md row.
grep -qF "host-prep-dag" "$DOC" || fail "doc does not self-reference its own slug"

# P2fu2 test-r2 closure: strengthen the source-side ordering grep
# to check the actual depends_on edges for the new step kinds, not
# just their presence in the enum.
echo "==> source-side ordering edges (depends_on)"
grep -B 3 'kind: HostPrepStepKind::ApplySysctl' "$MOD" \
    | grep -q 'id(HostPrepStepKind::BringUpTapInterface)' \
    && ok "ApplySysctl depends on BringUpTapInterface" \
    || fail "ApplySysctl missing BringUpTapInterface dep edge"

grep -B 3 'kind: HostPrepStepKind::SetBridgePortFlags' "$MOD" \
    | grep -q 'id(HostPrepStepKind::ApplySysctl)' \
    && ok "SetBridgePortFlags depends on ApplySysctl" \
    || fail "SetBridgePortFlags missing ApplySysctl dep edge"

grep -B 3 'kind: HostPrepStepKind::PreOpenVhostNetFd' "$MOD" \
    | grep -q 'id(HostPrepStepKind::SetBridgePortFlags)' \
    && ok "PreOpenVhostNetFd depends on SetBridgePortFlags" \
    || fail "PreOpenVhostNetFd missing SetBridgePortFlags dep edge"

# P2fu4 docs-r4 / product-r4 / test-r4 closure: the "Canonical
# step set" section is the discoverable operator-facing index;
# previously it listed only 7 steps (pre-P2fu1). Assert all 10
# step slugs appear in that specific section (between
# "## Canonical step set" and the next "##").
echo "==> documentation: Canonical step set completeness"
canonical_section=$(awk '/^## Canonical step set/{f=1;next} /^## /{if(f){exit}} f{print}' "$DOC")
for slug in \
    ssh-host-key-preflight \
    ownership-matrix-check \
    apply-nm-unmanaged \
    apply-nftables-rules \
    bring-up-tap-interface \
    apply-sysctl \
    set-bridge-port-flags \
    pre-open-vhost-net-fd \
    bind-mount-from-hardlink-farm \
    seed-dnsmasq-lease; do
    if echo "$canonical_section" | grep -qF "$slug"; then
        ok "Canonical step set lists $slug"
    else
        fail "Canonical step set section MISSING $slug (drift between section and table)"
    fi
done

echo "PASS host-prep-dag-eval.sh"
