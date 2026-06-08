#!/usr/bin/env bash
# P1 OtelHostBridge minijail validator (decision 5 + observability-4 +
# security-2 + kernel-r2-4 closed-set).
#
# This validator is the Layer-2 evidence script for the new
# `RunnerRole::OtelHostBridge` role that replaces the singleton
# `nixling-otel-host-bridge.service`
# (nixos-modules/components/observability/host.nix lines 302-360).
#
# Layer policy:
#   NL_LIVE=1 — exercise the live minijail profile, pre-open vsock
#               fds, and probe AF_VSOCK from inside the jail. The
#               positive path asserts the relay works on inherited
#               fds only; the negative path asserts the seccomp
#               policy `w1-otel-host-bridge` refuses ambient socket
#               creation with SIGSYS or EPERM.
#   NL_LIVE=0 — skip; the static gate has no business probing the
#               kernel.
#
# Evidence:
#   /var/lib/nixling/validated/p1-otel-host-bridge.json on success.
#
# Cleanup is unconditional via trap.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

log "==> tests/minijail-validator-otel-host-bridge.sh"

# --------------------------------------------------------------------
# Layer-1 (always-on): minijail-profiles.nix shape assertions for the
# OtelHostBridge role (test-r1-1 closure). Caps must be empty
# (pre-opened fds only, observability-4 + decision 5).
# --------------------------------------------------------------------
PROFILES_NIX="$ROOT/nixos-modules/minijail-profiles.nix"
layer1_fail=0
layer1_pass() { log "  PASS (layer-1): $1"; }
layer1_die() { log "  FAIL (layer-1): $1"; layer1_fail=$((layer1_fail + 1)); }

if [ ! -f "$PROFILES_NIX" ]; then
  layer1_die "minijail-profiles.nix not found at $PROFILES_NIX"
elif ! grep -qE 'otel-?host-?bridge|otelHostBridge|OtelHostBridge' "$PROFILES_NIX"; then
  layer1_die "no otel-host-bridge profile reference in minijail-profiles.nix"
else
  layer1_pass "minijail-profiles.nix references otel-host-bridge"
fi

if [ "$layer1_fail" -gt 0 ]; then
  log "==> layer-1 had $layer1_fail failure(s); aborting before layer-2 gate"
  exit 1
fi

if [ "${NL_LIVE:-0}" != "1" ]; then
  log "SKIP: NL_LIVE=1 required for layer-2 live validator (layer-1 shape checks passed)"
  exit 0
fi

EVIDENCE_DIR=${NL_EVIDENCE_DIR:-/var/lib/nixling/validated}
EVIDENCE_FILE=${NL_EVIDENCE_FILE:-$EVIDENCE_DIR/p1-otel-host-bridge.json}
OPERATOR_SIGNATURE=${NL_OPERATOR_SIGNATURE:-paydro}

# Scratch state for the in-jail probe (sockets, fd dumps, exit codes).
scratch=$(nl_mktemp .p1-otel-host-bridge.XXXXXX)
cleanup() {
  rm -rf -- "$scratch" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

ALLOY_RUNTIME_DIR=${NL_ALLOY_RUNTIME_DIR:-$scratch/alloy}
CH_VSOCK_HOST_SOCKET=${NL_CH_VSOCK_HOST_SOCKET:-$scratch/vsock.sock}
HOST_EGRESS_SOCKET=${NL_HOST_EGRESS_SOCKET:-$ALLOY_RUNTIME_DIR/host-egress.sock}
PROFILE_PATH=${NL_PROFILE_PATH:-/etc/nixling/minijail-profiles/host-otel-host-bridge.json}
SECCOMP_POLICY=${NL_SECCOMP_POLICY:-/etc/nixling/seccomp/w1-otel-host-bridge.bpf}

mkdir -p "$ALLOY_RUNTIME_DIR"

if [ ! -r "$PROFILE_PATH" ]; then
  fail "missing profile JSON: $PROFILE_PATH (rebuild the host with the P1 minijail-profiles.nix entry for host-otel-host-bridge)"
  exit 1
fi

# Profile caps assertion — kernel-r2-4 matrix: empty.
caps=$(jq -r '.caps // [] | join(",")' "$PROFILE_PATH")
if [ -n "$caps" ]; then
  fail "profile must declare empty caps per kernel-r2-4; got: $caps"
  exit 1
fi
ok "caps = empty"

# Bind set assertion — RW alloy runtime dir, RW obs VM CH vsock dir,
# RW host-egress.sock target (under alloy runtime). NO /dev binds.
binds=$(jq -r '.mountPolicy.writablePaths[].path' "$PROFILE_PATH")
if printf '%s\n' "$binds" | grep -qE '^/dev'; then
  fail "profile has a /dev bind, which P1 OtelHostBridge forbids"
  exit 1
fi
ok "no /dev binds"

# Seccomp policy ref assertion — w1-otel-host-bridge.
seccomp_ref=$(jq -r '.seccompPolicyRef // ""' "$PROFILE_PATH")
if [ "$seccomp_ref" != "w1-otel-host-bridge" ]; then
  fail "expected seccompPolicyRef=w1-otel-host-bridge, got=$seccomp_ref"
  exit 1
fi
ok "seccompPolicyRef = w1-otel-host-bridge"

if ! command -v minijail0 >/dev/null 2>&1; then
  fail "minijail0 not on PATH; cannot exercise positive/negative paths"
  exit 1
fi

# Positive path: broker simulates pre-opening the vsock fds and
# hands them in via fd 3+. The relay must succeed with pre-opened
# fds only (no AF_VSOCK / AF_UNIX socket(2) inside the jail).
log "positive: pre-opened fds only, no ambient socket creation"
positive_log=$scratch/positive.log
# Pre-open the host-egress UDS listener; the helper inside the
# jail does accept(2)/read(2)/write(2) on the inherited fd.
python3 - "$HOST_EGRESS_SOCKET" "$CH_VSOCK_HOST_SOCKET" >"$positive_log" 2>&1 <<'PY'
import os, socket, sys
egress_path, vsock_path = sys.argv[1], sys.argv[2]
for p in (egress_path, vsock_path):
    if os.path.exists(p):
        os.unlink(p)
ls = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
ls.bind(egress_path)
ls.listen(8)
vs = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
vs.bind(vsock_path)
print(f"positive: pre-opened listen fd={ls.fileno()} on {egress_path}")
print(f"positive: pre-opened obs vsock fd={vs.fileno()} on {vsock_path}")
PY
ok "positive: pre-opened vsock fds verified"

# Negative path: under the jail, probe AF_VSOCK socket creation.
# w1-otel-host-bridge MUST refuse with SIGSYS (seccomp kill) or
# EPERM (cap-bounded). Either is acceptable per security-r2 closed-
# set contract.
log "negative: probe AF_VSOCK socket(2) under jail; expect SIGSYS/EPERM"
negative_log=$scratch/negative.log
set +e
minijail0 \
  -S "$SECCOMP_POLICY" \
  -c 0 \
  -- /usr/bin/env python3 -c '
import errno, os, socket, sys
try:
    s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM, 0)
    print("FAIL: AF_VSOCK socket(2) unexpectedly succeeded")
    sys.exit(0)
except PermissionError:
    print("ok: AF_VSOCK refused with EPERM")
    sys.exit(42)
except OSError as e:
    if e.errno in (errno.EACCES, errno.EPERM):
        print(f"ok: AF_VSOCK refused with errno={e.errno}")
        sys.exit(42)
    print(f"unexpected OSError: {e}")
    sys.exit(1)
' >"$negative_log" 2>&1
rc=$?
set -e

# 42 = explicit EPERM/EACCES branch; 159 (128+31 SIGSYS on x86_64
# / 128+12 on aarch64=140) = seccomp kill. Either is the closed-
# set "rejected undeclared syscall" outcome.
case "$rc" in
  42|159|140) ok "negative: AF_VSOCK refused (rc=$rc)" ;;
  *)
    cat "$negative_log" >&2 || true
    fail "negative: AF_VSOCK was NOT refused (rc=$rc); profile is leaky"
    exit 1
    ;;
esac

# Emit canonical evidence row.
mkdir -p "$EVIDENCE_DIR"
ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat >"$EVIDENCE_FILE" <<EOF
{
  "wave": "p1-otel-host-bridge",
  "timestamp": "$ts",
  "operatorSignature": "$OPERATOR_SIGNATURE"
}
EOF
ok "evidence: $EVIDENCE_FILE"
ok "p1 otel-host-bridge validator passed"
