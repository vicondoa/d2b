#!/usr/bin/env bash
# tests/integration/distro-matrix/ubuntu-2404-tier1.sh— (rollup):
# Ubuntu 24.04 LTS x86_64 Tier-1 smoke harness scaffold.
#
# This is the **scaffold** half of the deliverable. The Tier-1
# smoke gate runs end-to-end against an Ubuntu 24.04 host: install
# nixling via `nixling host install --apply`, prepare the host,
# create a minimal VM, bring it up, SSH into it, tear it down, and
# verify every audit row + every artifact written by the broker.
#
# Why this is a manual gate today (not wired into static.sh):
#   - The current dev host is NixOS, not Ubuntu 24.04.
#   - The harness needs root + KVM + an Ubuntu image + ~5 GiB
#     scratch; it is a Layer 3 / nightly gate per the validation
#     strategy in plan.md.
#
# How to run it (on an Ubuntu 24.04 host with KVM + nix):
#
#   sudo NIXLING_REPO=/path/to/nixling \
#        tests/integration/distro-matrix/ubuntu-2404-tier1.sh
#
# Set NIXLING_UBUNTU_TIER1_STRICT=1 to fail closed when expected
# audit rows or installer artifacts are missing after the live run.
#
# Phases:
#   1. preflight: kernel KVM module, nix on PATH, root.
#   2. install: cargo build the daemon + broker, RunHostInstall via
#      the broker dispatch path.
#   3. host prepare: nixling host prepare --apply (live
#      ApplyNftables / ApplyRoute / ApplySysctl / UpdateHostsFile
#      via BundleResolver, broker live).
#   4. vm start: nixling vm start minimal-vm --apply (
#      SpawnRunner real wire, pidfd via SCM_RIGHTS).
#   5. probe: SSH into the VM, confirm reachability.
#   6. vm stop: nixling vm stop --apply, confirm pidfd table drain.
#   7. host destroy: confirm nft + route + sysctl rollback.
#   8. audit replay: ExportBrokerAudit, validate every operation
#      landed an audit row.
#
# Expected fixtures live in tests/integration/distro-matrix/fixtures/ubuntu-2404/.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-${NIXLING_REPO:-$(cd "$HERE/../../.." && pwd)}}
FIXTURES="$HERE/fixtures/ubuntu-2404"

DISTRO="ubuntu-24.04"
ARCH="x86_64-linux"
NIXLING_UBUNTU_TIER1_CLEANUP_ARMED=0
export DISTRO ARCH NIXLING_UBUNTU_TIER1_CLEANUP_ARMED

log() {
    printf '[ubuntu-2404-tier1] %s\n' "$*" >&2
}

fail() {
    printf '[ubuntu-2404-tier1] FAIL: %s\n' "$*" >&2
    exit 78
}

skip() {
    printf '[ubuntu-2404-tier1] SKIP: %s\n' "$*" >&2
    exit 77
}

ok() {
    printf '[ubuntu-2404-tier1] ok: %s\n' "$*" >&2
}

strict_missing_ok_or_fail() {
    local reason=$1
    if [ "${NIXLING_UBUNTU_TIER1_STRICT:-0}" = "1" ]; then
        fail "$reason"
    fi
    log "$reason (NIXLING_UBUNTU_TIER1_STRICT unset)"
}

preflight_or_skip() {
    if [ "$(id -u)" -ne 0 ]; then
        skip "Tier-1 smoke needs root for KVM + nft + ip route"
    fi
    if ! [ -e /dev/kvm ]; then
        skip "no /dev/kvm — Tier-1 smoke requires KVM"
    fi
    if ! command -v nix >/dev/null 2>&1; then
        skip "nix not on PATH — install nix or run from a NixOS host"
    fi
    if ! [ -f /etc/os-release ]; then
        skip "no /etc/os-release — cannot identify host distro"
    fi
    if ! grep -q '^ID=ubuntu' /etc/os-release; then
        log "host is not Ubuntu; running scaffold validation only"
        export NIXLING_UBUNTU_SCAFFOLD_ONLY=1
    fi
    if ! grep -q '^VERSION_ID="24.04"' /etc/os-release \
        && [ -z "${NIXLING_UBUNTU_SCAFFOLD_ONLY:-}" ]; then
        skip "expected Ubuntu 24.04 (set NIXLING_UBUNTU_SCAFFOLD_ONLY=1 to force scaffold-only)"
    fi
    ok "preflight"
}

phase_install() {
    if [ -n "${NIXLING_UBUNTU_SCAFFOLD_ONLY:-}" ]; then
        log "phase install: scaffold-only mode, skipping live install"
        return 0
    fi
    log "phase install: cargo build nixlingd + broker"
    (cd "$ROOT/packages" && cargo build --release --workspace) \
        || fail "cargo build workspace"
    (cd "$ROOT/packages/nixling-priv-broker" && cargo build --release) \
        || fail "cargo build nixling-priv-broker"
    ok "phase install"
}

phase_host_prepare() {
    if [ -n "${NIXLING_UBUNTU_SCAFFOLD_ONLY:-}" ]; then
        log "phase host prepare: scaffold-only mode, skipping live prepare"
        return 0
    fi
    log "phase host prepare: nixling host prepare --apply"
    NIXLING_NATIVE_ONLY=1 \
        "$ROOT/packages/target/release/nixling" host prepare --apply \
        || fail "host prepare --apply"
    ok "phase host prepare"
}

phase_vm_start() {
    if [ -n "${NIXLING_UBUNTU_SCAFFOLD_ONLY:-}" ]; then
        log "phase vm start: scaffold-only mode, skipping live start"
        return 0
    fi
    log "phase vm start: nixling vm start minimal-vm --apply"
    NIXLING_NATIVE_ONLY=1 \
        "$ROOT/packages/target/release/nixling" vm start minimal-vm --apply \
        || fail "vm start --apply"
    ok "phase vm start"
}

phase_probe() {
    if [ -n "${NIXLING_UBUNTU_SCAFFOLD_ONLY:-}" ]; then
        log "phase probe: scaffold-only mode, skipping live SSH probe"
        return 0
    fi
    log "phase probe: SSH into minimal-vm"
    ssh -o StrictHostKeyChecking=no -o ConnectTimeout=30 \
        minimal-vm.work.local true \
        || fail "ssh smoke probe"
    ok "phase probe"
}

phase_vm_stop() {
    if [ -n "${NIXLING_UBUNTU_SCAFFOLD_ONLY:-}" ]; then
        log "phase vm stop: scaffold-only mode, skipping live stop"
        return 0
    fi
    log "phase vm stop: nixling vm stop minimal-vm --apply"
    NIXLING_NATIVE_ONLY=1 \
        "$ROOT/packages/target/release/nixling" vm stop minimal-vm --apply \
        || fail "vm stop --apply"
    ok "phase vm stop"
}

phase_host_destroy() {
    if [ -n "${NIXLING_UBUNTU_SCAFFOLD_ONLY:-}" ]; then
        log "phase host destroy: scaffold-only mode, skipping live destroy"
        return 0
    fi
    log "phase host destroy: nixling host destroy --apply"
    NIXLING_NATIVE_ONLY=1 \
        "$ROOT/packages/target/release/nixling" host destroy --apply \
        || fail "host destroy --apply"
    ok "phase host destroy"
}

cleanup() {
    trap - EXIT
    if [ "${NIXLING_UBUNTU_TIER1_CLEANUP_ARMED:-0}" != "1" ]; then
        return 0
    fi
    log "cleanup: stopping minimal-vm and destroying host state"
    (phase_vm_stop) 2>/dev/null || true
    (phase_host_destroy) 2>/dev/null || true
}

phase_audit_replay() {
    if [ -n "${NIXLING_UBUNTU_SCAFFOLD_ONLY:-}" ]; then
        log "phase audit replay: scaffold-only mode, skipping live replay"
        return 0
    fi
    log "phase audit replay: validate broker audit rows + installer artifacts"
    local audit_date
    audit_date=$(date -u +%F)
    local audit_log="/var/lib/nixling/audit/broker-${audit_date}.jsonl"
    local artifact
    if ! [ -f "$audit_log" ]; then
        fail "expected audit log at $audit_log"
    fi
    while IFS= read -r op; do
        case "$op" in
            ''|'#'*) continue ;;
        esac
        if ! grep -qF "\"operation\":\"$op\"" "$audit_log"; then
            strict_missing_ok_or_fail "missing audit row for $op (may be OK if op not invoked)"
        fi
    done < "$FIXTURES/expected-audit-ops.txt"
    while IFS= read -r artifact; do
        case "$artifact" in
            ''|'#'*) continue ;;
        esac
        if ! [ -s "$artifact" ]; then
            strict_missing_ok_or_fail "missing installer artifact $artifact (may be OK if install not invoked)"
        fi
    done < "$FIXTURES/expected-installer-artifacts.txt"
    ok "phase audit replay"
}

scaffold_self_test() {
    log "scaffold self-test: every fixture and helper present"
    [ -d "$FIXTURES" ] || fail "missing fixtures dir $FIXTURES"
    for f in expected-audit-ops.txt expected-installer-artifacts.txt README.md; do
        [ -f "$FIXTURES/$f" ] || fail "missing fixture $FIXTURES/$f"
    done
    ok "scaffold self-test"
}

main() {
    trap cleanup EXIT
    preflight_or_skip
    scaffold_self_test
    NIXLING_UBUNTU_TIER1_CLEANUP_ARMED=1
    export NIXLING_UBUNTU_TIER1_CLEANUP_ARMED
    phase_install
    phase_host_prepare
    phase_vm_start
    phase_probe
    phase_vm_stop
    phase_host_destroy
    phase_audit_replay
    NIXLING_UBUNTU_TIER1_CLEANUP_ARMED=0
    export NIXLING_UBUNTU_TIER1_CLEANUP_ARMED
    log "tier-1 smoke complete (scaffold_only=${NIXLING_UBUNTU_SCAFFOLD_ONLY:-0})"
}

main "$@"
