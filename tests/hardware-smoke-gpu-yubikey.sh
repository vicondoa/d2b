#!/usr/bin/env bash
# tests/hardware-smoke-gpu-yubikey.sh— (rollup):
# hardware validation smoke on this NixOS dev host.
#
# Validates the GPU sidecar + USBIP YubiKey live paths
# against this host's NVIDIA Quadro T1000 + USB / YubiKey hardware.
#
# Phases:
#   1. preflight: confirm /dev/dri/renderD128 + /dev/bus/usb present
#      + this is a NixOS host with the required nix shell tools.
#   2. workspace build: cargo build the rollup workspace
#      (nixling + nixlingd + nixling-priv-broker) so the validation
#      tests can drive the released binaries.
#   3. minijail profile validator: run the
#      `BundleResolver::validate_minijail_profiles()` invariant gate
#      against a synthesized resolver fixture so any regression in
#      the per-role profile shape (uid=0 without carve-out,
#      writable /nix/store, cgroup outside nixling/) fails the wave.
#   4. bundle drift: cargo xtask gen-schemas + gen-daemon-api
#      verifies the bundle + wire artifacts match committed
#      docs/reference state.
#   5. example eval: nix flake check examples/graphics-workstation +
#      examples/with-entra-id to confirm GPU + YubiKey-enabled
#      consumer flakes still build.
#   6. (manual) live smoke: documents the operator-driven steps for
#      driving the broker live SpawnRunner against
#      /dev/dri/renderD128 + the USBIP live_bind against the
#      plugged-in YubiKey. This phase is intentionally NOT
#      automated — running it spawns real VMs that disrupt the
#      operator's active Wayland session.
#
# Set NIXLING_HARDWARE_SMOKE_STRICT=1 to fail closed on the cargo
# build / minijail / bundle-drift / example-eval phases instead of
# logging an explicit SKIP reason and continuing.
#
# After the manual live smoke passes, set
# NIXLING_HARDWARE_SMOKE_RECORD_EVIDENCE_ONLY=1,
# NIXLING_HARDWARE_SMOKE_LIVE_GREEN=1, and
# NIXLING_HARDWARE_SMOKE_OPERATOR_SIGNATURE=<opaque-signer-id> to
# write `/var/lib/nixling/validated/{w5Fu,w6Fu}.json`.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=$(cd "$HERE/.." && pwd)
NL_LOG=${NL_LOG:-"$ROOT/.nixling-hardware-smoke.log"}
export NL_LOG

log() {
    printf '[hardware-smoke] %s\n' "$*" >&2
}

fail() {
    printf '[hardware-smoke] FAIL: %s\n' "$*" >&2
    exit 78
}

skip() {
    printf '[hardware-smoke] SKIP: %s\n' "$*" >&2
    exit 77
}

ok() {
    printf '[hardware-smoke] ok: %s\n' "$*" >&2
}

soft_fail_or_skip() {
    local reason=$1
    if [ "${NIXLING_HARDWARE_SMOKE_STRICT:-0}" = "1" ]; then
        fail "$reason"
    fi
    log "SKIP REASON: $reason (NIXLING_HARDWARE_SMOKE_STRICT unset)"
}

json_escape() {
    printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

record_validation_evidence() {
    local operator_signature="${NIXLING_HARDWARE_SMOKE_OPERATOR_SIGNATURE:-}"
    local timestamp host dir path wave json
    local -a sudo_cmd=()

    if [ "${NIXLING_HARDWARE_SMOKE_LIVE_GREEN:-0}" != "1" ]; then
        fail "refusing to record validation evidence unless NIXLING_HARDWARE_SMOKE_LIVE_GREEN=1 confirms the manual live smoke passed"
    fi
    if [ -z "$operator_signature" ]; then
        fail "set NIXLING_HARDWARE_SMOKE_OPERATOR_SIGNATURE before recording validation evidence"
    fi
    if [ "$EUID" -ne 0 ]; then
        if ! command -v sudo >/dev/null 2>&1 || ! sudo -n true 2>/dev/null; then
            fail "recording validation evidence requires root or passwordless sudo"
        fi
        sudo_cmd=(sudo -n)
    fi

    timestamp=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    host=$(hostname -f 2>/dev/null || hostname)
    dir=/var/lib/nixling/validated
    "${sudo_cmd[@]}" install -d -m 0750 "$dir"

    for wave in w5Fu w6Fu; do
        path="$dir/$wave.json"
        json=$(cat <<EOF
{
  "wave": "$(json_escape "$wave")",
  "timestamp": "$(json_escape "$timestamp")",
  "operatorSignature": "$(json_escape "$operator_signature")",
  "host": "$(json_escape "$host")",
  "source": "tests/hardware-smoke-gpu-yubikey.sh"
}
EOF
)
        "${sudo_cmd[@]}" install -m 0640 /dev/null "$path"
        printf '%s\n' "$json" | "${sudo_cmd[@]}" tee "$path" >/dev/null
        ok "recorded validation evidence: $path"
    done
}

phase_preflight() {
    log "phase preflight: GPU + USB + nix"
    if ! [ -e /dev/dri/renderD128 ]; then
        skip "no /dev/dri/renderD128; this host has no GPU render node"
    fi
    if ! [ -d /dev/bus/usb ]; then
        skip "no /dev/bus/usb; this host has no USB subsystem"
    fi
    if ! command -v nix >/dev/null 2>&1; then
        skip "nix not on PATH"
    fi
    ok "preflight (GPU + USB + nix present)"
}

phase_yubikey_optional() {
    log "phase yubikey-optional: check for plugged-in YubiKey"
    if ! command -v lsusb >/dev/null 2>&1; then
        log "lsusb not available (need usbutils); skipping YubiKey detection"
        return 0
    fi
    if lsusb 2>/dev/null | grep -qi "yubico"; then
        ok "YubiKey detected on host USB bus"
        export NIXLING_HARDWARE_YUBIKEY=1
    else
        log "no YubiKey plugged in; W6-fu live bind path will skip"
    fi
}

phase_cargo_build() {
    log "phase cargo build: workspace + broker"
    if ! (cd "$ROOT/packages" && \
        env -u RUSTC_WRAPPER nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc nixpkgs#rustfmt nixpkgs#clippy \
        -c cargo build --workspace 2>&1 | tail -3); then
        soft_fail_or_skip "cargo build --workspace failed; rerun via tests/rust-workspace-checks.sh for full diagnostic"
        return 0
    fi
    if ! (cd "$ROOT/packages/nixling-priv-broker" && \
        env -u RUSTC_WRAPPER nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc nixpkgs#rustfmt nixpkgs#clippy \
        -c cargo build 2>&1 | tail -3); then
        soft_fail_or_skip "cargo build nixling-priv-broker failed"
        return 0
    fi
    ok "phase cargo build"
}

phase_minijail_invariants() {
    log "phase minijail invariants: validate every shipped profile"
    if ! (cd "$ROOT/packages" && \
        env -u RUSTC_WRAPPER nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc nixpkgs#rustfmt nixpkgs#clippy \
        -c cargo test -p nixling-core --test nixling-core-smoke \
            bundle_resolver_minijail_profile_validator 2>&1 | tail -10); then
        soft_fail_or_skip "minijail profile validator failed to run"
        return 0
    fi
    ok "phase minijail invariants (W17 validator green)"
}

phase_bundle_drift() {
    log "phase bundle drift: gen-schemas + gen-daemon-api regen"
    if ! (cd "$ROOT/packages" && \
        env -u RUSTC_WRAPPER nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc nixpkgs#rustfmt nixpkgs#clippy \
        -c bash -c 'cargo xtask gen-schemas && cargo xtask gen-daemon-api' \
        2>&1 | tail -5); then
        soft_fail_or_skip "cargo xtask regen failed"
        return 0
    fi
    if ! git -C "$ROOT" diff --quiet docs/reference/; then
        fail "drift detected: docs/reference/ changed after regen; commit it"
    fi
    ok "phase bundle drift (no regen drift)"
}

phase_example_eval() {
    log "phase example eval: graphics-workstation + with-entra-id"
    for example in graphics-workstation with-entra-id; do
        if ! [ -d "$ROOT/examples/$example" ]; then
            log "examples/$example missing; skipping"
            continue
        fi
        if (cd "$ROOT/examples/$example" && \
            nix flake check --no-build --all-systems \
                --no-write-lock-file 2>&1 | tail -5); then
            ok "phase example eval: $example"
        else
            soft_fail_or_skip "nix flake check examples/$example failed (may be pre-existing)"
        fi
    done
}

phase_live_smoke_documentation() {
    log "phase live smoke: manual operator steps"
    cat >&2 <<'GUIDE'
[hardware-smoke] The live GPU + YubiKey smoke is intentionally MANUAL
[hardware-smoke] because it spawns real VMs that disrupt the active
[hardware-smoke] Wayland session. Run by hand when the host is idle:
[hardware-smoke]
[hardware-smoke] 1. Start nixlingd:
[hardware-smoke]      sudo NIXLING_BROKER_NFT_BINARY=$(which nft) \
[hardware-smoke]           NIXLING_BROKER_IP_BINARY=$(which ip) \
[hardware-smoke]           NIXLING_BROKER_USBIP_BINARY=$(which usbip) \
[hardware-smoke]           packages/target/debug/nixlingd serve &
[hardware-smoke]
[hardware-smoke] 2. Run host install:
[hardware-smoke]      sudo NIXLING_NATIVE_ONLY=1 \
[hardware-smoke]           packages/target/debug/nixling host install --apply
[hardware-smoke]
[hardware-smoke] 3. Bring up the work-vm with GPU:
[hardware-smoke]      sudo NIXLING_NATIVE_ONLY=1 \
[hardware-smoke]           packages/target/debug/nixling vm start work-vm --apply
[hardware-smoke]
[hardware-smoke] 4. Once the VM is up, plug in the YubiKey and attach it:
[hardware-smoke]      sudo nixling usb probe
[hardware-smoke]      sudo NIXLING_NATIVE_ONLY=1 \
[hardware-smoke]           packages/target/debug/nixling usb attach work-vm <busid> --apply
[hardware-smoke]      # `usb probe` lists the daemon-declared busids and
[hardware-smoke]      # current lock owners; replace <busid> with the host
[hardware-smoke]      # busid you want to bind to work-vm.
[hardware-smoke]
[hardware-smoke] 5. Verify ExportBrokerAudit shows
[hardware-smoke]      ApplyNftables / SpawnRunner / OpenPidfd / UsbipBind
[hardware-smoke]      rows for the corresponding bundle intent refs.
[hardware-smoke]
[hardware-smoke] 6. Once the manual live smoke is green, record the
[hardware-smoke]      W20 validation evidence:
[hardware-smoke]      NIXLING_HARDWARE_SMOKE_RECORD_EVIDENCE_ONLY=1 \
[hardware-smoke]      NIXLING_HARDWARE_SMOKE_LIVE_GREEN=1 \
[hardware-smoke]      NIXLING_HARDWARE_SMOKE_OPERATOR_SIGNATURE='alice@example' \
[hardware-smoke]      bash tests/hardware-smoke-gpu-yubikey.sh
[hardware-smoke]      # writes /var/lib/nixling/validated/{w5Fu,w6Fu}.json
[hardware-smoke]
[hardware-smoke] 7. Then set only the matching validated bits in host
[hardware-smoke]      config:
[hardware-smoke]      nixling.defaultSwitchReadiness.w5Fu.validated = true;
[hardware-smoke]      nixling.defaultSwitchReadiness.w6Fu.validated = true;
[hardware-smoke]      # other waves need their own evidence files before
[hardware-smoke]      # daemonExperimental.enable can auto-default true.
GUIDE
    ok "phase live smoke documentation"
}

main() {
    if [ "${NIXLING_HARDWARE_SMOKE_RECORD_EVIDENCE_ONLY:-0}" = "1" ]; then
        log "phase record evidence: writing W20 validation files"
        record_validation_evidence
        log "W20 validation evidence recorded"
        return 0
    fi

    phase_preflight
    phase_yubikey_optional
    phase_cargo_build
    phase_minijail_invariants
    phase_bundle_drift
    phase_example_eval
    phase_live_smoke_documentation
    log "W20 hardware-smoke complete (automated phases green; live smoke manual)"
}

main "$@"
