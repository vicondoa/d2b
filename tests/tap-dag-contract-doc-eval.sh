#!/usr/bin/env bash
# doc/code drift gate.
#
# Asserts that docs/reference/tap-dag-contract.md matches the
# implementation it claims to document: the derived-ifname scheme in
# nixling_host::ifname, the tap broker ops in nixling-priv-broker, the
# host-prep DAG variant + ordering edges in nixling_host::host_prep_dag,
# and the ChNetHandoffMode enum in nixling-core.
#
# Static-only gate: no nixpkgs eval, no rust build. Pure grep over
# committed sources.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DOC="docs/reference/tap-dag-contract.md"
IFNAME="packages/nixling-host/src/ifname.rs"
TAP_OPS="packages/nixling-priv-broker/src/ops/tap.rs"
DAG="packages/nixling-host/src/host_prep_dag.rs"
HOST_DTO="packages/nixling-core/src/host.rs"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

ok() {
    echo "  ok: $*"
}

for f in "$DOC" "$IFNAME" "$TAP_OPS" "$DAG" "$HOST_DTO"; do
    [ -f "$f" ] || fail "missing $f"
done

echo "==> doc references existing source files"
# Every relative path the doc points at must resolve.
while IFS= read -r relpath; do
    [ -e "$relpath" ] || fail "doc references missing path: $relpath"
    ok "doc path $relpath"
done < <(
    grep -oE '\.\./\.\./[a-zA-Z0-9._/-]+' "$DOC" \
        | sed 's|\.\./\.\./||' \
        | sort -u
)

echo "==> ifname derivation contract"
# Default prefix.
grep -q 'pub const DEFAULT_PREFIX: &str = "nl-";' "$IFNAME" \
    || fail "ifname.rs DEFAULT_PREFIX is not \"nl-\""
grep -qF '`nl-`' "$DOC" \
    || fail "doc must mention default prefix \`nl-\`"
ok "default prefix nl-"

# Role tag chars.
grep -q "pub const BRIDGE_TAG: char = 'b';" "$IFNAME" \
    || fail "ifname.rs BRIDGE_TAG is not 'b'"
grep -q "pub const TAP_TAG: char = 't';" "$IFNAME" \
    || fail "ifname.rs TAP_TAG is not 't'"
grep -qE '`t` for taps' "$DOC" \
    || fail "doc must document tap role tag 't'"
grep -qE '`b` for' "$DOC" \
    || fail "doc must document bridge role tag 'b'"
ok "role tag chars t/b"

# Hash length.
grep -q 'pub const HASH_SUFFIX_LEN: usize = 8;' "$IFNAME" \
    || fail "ifname.rs HASH_SUFFIX_LEN is not 8"
grep -qE '(8 chars|HASH8|8-char)' "$DOC" \
    || fail "doc must document 8-char hash suffix"
ok "8-char hash suffix"

# Derivation function name appears in doc.
grep -q "derive_from_env_vm" "$DOC" \
    || fail "doc must reference derive_from_env_vm"
grep -q "pub fn derive_from_env_vm" "$IFNAME" \
    || fail "ifname.rs missing pub fn derive_from_env_vm"
ok "derive_from_env_vm referenced"

# Reverse-lookup helper.
grep -q "looks_nixling_owned" "$DOC" \
    || fail "doc must reference looks_nixling_owned"
grep -q "pub fn looks_nixling_owned" "$IFNAME" \
    || fail "ifname.rs missing pub fn looks_nixling_owned"
ok "looks_nixling_owned referenced"

echo "==> tap broker ops contract"
for op in CreateTapFd CreatePersistentTap SetBridgePortFlags; do
    grep -qF "$op" "$DOC" || fail "doc must mention broker op $op"
    grep -qF "$op" "$TAP_OPS" || fail "tap.rs missing $op"
    ok "broker op $op"
done

# NM unmanaged pre-create gate.
grep -q "nm-unmanaged-pre-create-required" "$DOC" \
    || fail "doc must document nm-unmanaged-pre-create-required error"
grep -q "nm-unmanaged-pre-create-required" "$TAP_OPS" \
    || fail "tap.rs missing nm-unmanaged-pre-create-required error string"
ok "NM unmanaged gate"

# TUNSETPERSIST / TUNSETOWNER / TUNSETGROUP for persistent mode.
for sym in TUNSETPERSIST TUNSETOWNER TUNSETGROUP; do
    grep -qF "$sym" "$DOC" \
        || fail "doc must document $sym"
done
grep -qF "TUNSETOWNER" "$TAP_OPS" \
    || fail "tap.rs missing TUNSETOWNER reference"
ok "persistent tap ioctls"

echo "==> host-prep DAG step + ordering"
grep -q "BringUpTapInterface" "$DOC" \
    || fail "doc must reference BringUpTapInterface step"
grep -qE '^\s+BringUpTapInterface,?$' "$DAG" \
    || fail "host_prep_dag.rs missing BringUpTapInterface variant"
ok "BringUpTapInterface variant"

grep -qE '"bring-up-tap-interface"' "$DAG" \
    || fail "host_prep_dag.rs missing bring-up-tap-interface step_id slug"
grep -q "bring-up-tap-interface" "$DOC" \
    || fail "doc must reference bring-up-tap-interface step_id slug"
ok "step_id slug"

# Documented broker op name for the step.
grep -qE 'Self::BringUpTapInterface\s*=>\s*"CreateTapFd"' "$DAG" \
    || fail "host_prep_dag.rs BringUpTapInterface.broker_op_name must be \"CreateTapFd\""
ok "broker_op_name == CreateTapFd"

# Ordering edges: apply-nftables-rules -> bring-up -> pre-open-vhost-net-fd.
grep -q "apply-nftables-rules" "$DOC" \
    || fail "doc must document upstream gate apply-nftables-rules"
grep -q "pre-open-vhost-net-fd" "$DOC" \
    || fail "doc must document downstream consumer pre-open-vhost-net-fd"
ok "ordering edges documented"

# Failure envelope.
grep -q "HostPrepStepFailed" "$DOC" \
    || fail "doc must reference HostPrepStepFailed"
grep -q "pub struct HostPrepStepFailed" "$DAG" \
    || fail "host_prep_dag.rs missing HostPrepStepFailed"
ok "HostPrepStepFailed"

echo "==> ChNetHandoffMode enum"
for variant in TapFd PersistentTap; do
    grep -qF "$variant" "$DOC" \
        || fail "doc must document ChNetHandoffMode::$variant"
    grep -qE "^\s+${variant},?$|^\s+${variant}\b" "$HOST_DTO" \
        || fail "host.rs missing ChNetHandoffMode::$variant"
    ok "ChNetHandoffMode::$variant"
done

grep -q "pub enum ChNetHandoffMode" "$HOST_DTO" \
    || fail "host.rs missing pub enum ChNetHandoffMode"
grep -q "ChNetHandoffMode" "$DOC" \
    || fail "doc must reference ChNetHandoffMode"
ok "ChNetHandoffMode enum"

echo "==> launcher group naming (daemon-only canonical)"
# The doc claims the broker public socket sits behind nixling
# (plural — declared by host-daemon.nix). Sanity check that's still true.
grep -q "nixling" "$DOC" \
    || fail "doc must reference daemon-only nixling group"
grep -q "users.groups.nixling" nixos-modules/host-daemon.nix \
    || fail "host-daemon.nix no longer declares nixling group"
ok "nixling group"

echo "OK: tap-dag-contract doc matches implementation"
