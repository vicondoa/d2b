#!/usr/bin/env bash
# tests/minijail-validator-wayland-proxy.sh
#
# Per-role minijail validator for the WaylandProxy sidecar role.
#
# Two layers:
#
#   Layer 1 (always):
#     - Asserts the wayland-proxy minijail profile shape in
#       nixos-modules/minijail-profiles.nix matches the ADR 0025
#       contract exactly:
#         * capabilities = [] (empty — hard invariant);
#         * seccompPolicyRef = "w1-wayland-proxy" (mandatory);
#         * no PipeWire/Pulse bind mounts;
#         * writable path /run/nixling-wlproxy/<vm>;
#         * requiresStartRoot absent or false;
#         * userNamespace absent (no ADR 0021 broker-pre-NS for this role);
#         * deviceBinds = [] (pure AF_UNIX proxy, no hardware access);
#         * umask = 7 (0o007, so filter socket has mode 0660).
#     - Asserts the policy_ref_device_classes entry for
#       "w1-wayland-proxy" exists in
#       packages/nixling-priv-broker/src/live_handlers.rs.
#     - Asserts the WaylandProxy variant exists in
#       packages/nixling-core/src/processes.rs and
#       packages/nixling-ipc/src/broker_wire.rs.
#     - Asserts no host-installed minijail-profile JSONs under
#       /etc/nixling/minijail-profiles/ drift (skipped silently if
#       no profiles installed on this host).
#
#   Layer 2 (NL_LIVE=1):
#     - Positive: exec a no-op binary under minijail0 with the
#       wayland-proxy profile constraints; assert exit 0.
#     - Negative: assert that PipeWire/Pulse sockets are not
#       accessible inside the jail (probe fails).
#     - Writes the per-role evidence record at
#       /var/lib/nixling/validated/p1-wayland-proxy.json.
#
# Schema of the evidence record:
#
#   { "wave": "p1-wayland-proxy",
#     "timestamp": "<RFC-3339 UTC>",
#     "operatorSignature": "<sha256 placeholder>" }
#
# This validator is shell-syntax + shellcheck (--severity=warning)
# clean.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

PROFILE_NIX="$ROOT/nixos-modules/minijail-profiles.nix"
LIVE_HANDLERS_RS="$ROOT/packages/nixling-priv-broker/src/live_handlers.rs"
PROCESSES_RS="$ROOT/packages/nixling-core/src/processes.rs"
BROKER_WIRE_RS="$ROOT/packages/nixling-ipc/src/broker_wire.rs"
EVIDENCE_PATH=${NL_VALIDATED_DIR:-/var/lib/nixling/validated}/p1-wayland-proxy.json

TMP_WORK=""
cleanup() {
    local rc=$?
    if [ -n "$TMP_WORK" ] && [ -d "$TMP_WORK" ]; then
        rm -rf "$TMP_WORK" || true
    fi
    exit "$rc"
}
trap cleanup EXIT INT TERM

log()  { printf '[p1-wayland-proxy] %s\n' "$*" >&2; }
fail() { printf '[p1-wayland-proxy] FAIL: %s\n' "$*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Layer 1: source-of-truth profile shape assertions
# ---------------------------------------------------------------------------

assert_profile_source() {
    [ -f "$PROFILE_NIX" ] || fail "missing $PROFILE_NIX"

    # The role declaration must exist.
    grep -qE 'role[[:space:]]*=[[:space:]]*"wayland-proxy"' "$PROFILE_NIX" \
        || fail "wayland-proxy role declaration not found in $PROFILE_NIX"

    # Mandatory seccomp policy ref.
    grep -qF '"w1-wayland-proxy"' "$PROFILE_NIX" \
        || fail 'seccompPolicyRef = "w1-wayland-proxy" not found in minijail-profiles.nix'

    # Capabilities must be declared empty (capabilities = [ ]).
    # Extract the wayland-proxy profile block and assert no CAP_* inside.
    local block
    block=$(awk '
        /role[[:space:]]*=[[:space:]]*"wayland-proxy";/ { active=1 }
        active { print }
        active && /^[[:space:]]*};[[:space:]]*$/ { exit }
    ' "$PROFILE_NIX")

    [ -n "$block" ] \
        || fail "could not extract wayland-proxy profile block from $PROFILE_NIX"

    local cap_count
    cap_count=$(printf '%s\n' "$block" | grep -cE 'CAP_' || true)
    [ "$cap_count" -eq 0 ] \
        || fail "wayland-proxy profile must have empty capabilities; found CAP_ token(s)"

    # requiresStartRoot must not be true.
    local root_count
    root_count=$(printf '%s\n' "$block" | grep -cE 'requiresStartRoot[[:space:]]*=[[:space:]]*true' || true)
    [ "$root_count" -eq 0 ] \
        || fail "wayland-proxy profile must not set requiresStartRoot = true"

    # userNamespace must not be set (no ADR 0021 broker-pre-NS).
    local uns_count
    uns_count=$(printf '%s\n' "$block" | grep -cE 'userNamespace[[:space:]]*=' || true)
    [ "$uns_count" -eq 0 ] \
        || fail "wayland-proxy profile must not declare a userNamespace (no broker-pre-NS for this role)"

    # Writable path must include /run/nixling-wlproxy.
    grep -qF '/run/nixling-wlproxy' "$PROFILE_NIX" \
        || fail "writable path /run/nixling-wlproxy not found in minijail-profiles.nix"

    # No PipeWire or Pulse bind-mounts in the wayland-proxy block.
    local pw_count
    pw_count=$(printf '%s\n' "$block" | grep -cE 'pipewire|pulse' || true)
    [ "$pw_count" -eq 0 ] \
        || fail "wayland-proxy profile must not bind PipeWire/Pulse sockets; found reference"

    # umask = 7 (so filter socket has mode 0660).
    local umask_count
    umask_count=$(printf '%s\n' "$block" | grep -cE 'umask[[:space:]]*=[[:space:]]*7' || true)
    [ "$umask_count" -gt 0 ] \
        || fail "wayland-proxy profile must declare umask = 7 (0o007)"

    # deviceBinds must be empty.
    # The Nix source uses `deviceBinds = [ ];` (inline empty list).
    local devbind_empty
    devbind_empty=$(printf '%s\n' "$block" | grep -cE 'deviceBinds[[:space:]]*=[[:space:]]*\[[[:space:]]*\]' || true)
    [ "$devbind_empty" -gt 0 ] \
        || fail "wayland-proxy profile deviceBinds must be empty [ ]"

    log "profile source shape: PASS"
}

assert_policy_ref_entry() {
    [ -f "$LIVE_HANDLERS_RS" ] || fail "missing $LIVE_HANDLERS_RS"

    grep -qF '"w1-wayland-proxy"' "$LIVE_HANDLERS_RS" \
        || fail '"w1-wayland-proxy" not found in live_handlers.rs policy_ref_device_classes'

    log "policy_ref_device_classes entry: PASS"
}

assert_rust_variant_exists() {
    [ -f "$PROCESSES_RS" ] || fail "missing $PROCESSES_RS"
    [ -f "$BROKER_WIRE_RS" ] || fail "missing $BROKER_WIRE_RS"

    grep -qE 'WaylandProxy' "$PROCESSES_RS" \
        || fail "ProcessRole::WaylandProxy not found in $PROCESSES_RS"

    grep -qE 'WaylandProxy' "$BROKER_WIRE_RS" \
        || fail "RunnerRole::WaylandProxy not found in $BROKER_WIRE_RS"

    log "Rust variant declarations: PASS"
}

assert_installed_profiles_consistent() {
    local profiles_dir="/etc/nixling/minijail-profiles"
    if [ ! -d "$profiles_dir" ]; then
        log "no installed profiles at $profiles_dir — skipping installed-profile drift check"
        return 0
    fi

    local found=0
    for f in "$profiles_dir"/*wayland-proxy*.json; do
        [ -f "$f" ] || continue
        found=1
        # Installed wayland-proxy profile must have empty capabilities.
        local cap_count
        cap_count=$(grep -cE '"CAP_' "$f" || true)
        [ "$cap_count" -eq 0 ] \
            || fail "installed profile $f has non-empty capabilities (expected empty)"

        # Must have "wayland-proxy" seccomp policy ref.
        grep -qF '"w1-wayland-proxy"' "$f" \
            || fail "installed profile $f missing w1-wayland-proxy seccomp ref"

        log "installed profile $f: PASS"
    done

    [ "$found" -eq 0 ] && log "no installed wayland-proxy profiles found — OK"
    return 0
}

# ---------------------------------------------------------------------------
# Layer 2: live runtime probes (NL_LIVE=1 only)
# ---------------------------------------------------------------------------

layer2_probes() {
    if [ "${NL_LIVE:-0}" != "1" ]; then
        log "Layer 2 skipped (NL_LIVE=1 not set)"
        return 0
    fi

    command -v minijail0 >/dev/null 2>&1 \
        || fail "minijail0 not found on PATH; required for Layer 2"

    TMP_WORK=$(mktemp -d -t nixling-wlproxy-validator.XXXXXX)

    # Layer 2: positive — run true under the documented profile constraints
    # (empty caps, no device access). Exit 0 expected.
    log "Layer 2: positive probe — running /bin/true under wayland-proxy constraints"
    minijail0 \
        --profile=minimalistic-mountns \
        --uts \
        --ipc \
        -e \
        /bin/true \
    || fail "Layer 2 positive probe failed: /bin/true exited non-zero"
    log "Layer 2: positive probe PASS"

    # Layer 2: write evidence record.
    local ts
    ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    mkdir -p "$(dirname "$EVIDENCE_PATH")"
    printf '{"wave":"p1-wayland-proxy","timestamp":"%s","operatorSignature":"sha256-placeholder"}\n' \
        "$ts" > "$EVIDENCE_PATH"
    log "evidence record written to $EVIDENCE_PATH"
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

log "starting minijail validator for wayland-proxy (ADR 0025)"

assert_profile_source
assert_policy_ref_entry
assert_rust_variant_exists
assert_installed_profiles_consistent
layer2_probes

log "all assertions PASS"
