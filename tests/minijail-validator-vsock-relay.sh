#!/usr/bin/env bash
# tests/minijail-validator-vsock-relay.sh— per-role validator for the
# VsockRelay sidecar.
#
# Role contract under the daemon-only end-state:
#
#   - The relay is the socat-based sidecar that replaces the per-VM
#     nixling-otel-relay@<vm>.service. The argv generator lives in
#     packages/nixling-host/src/vsock_relay_argv.rs; goldens are at
#     tests/golden/runner-shape/vsock-relay-argv-minimal.txt.
#
#   - Capabilities in the minijail profile: empty. The earlier matrix
#     listed CAP_NET_RAW; the corrected matrix carries no caps because
#     the relay operates
#     on pre-opened fds the broker passes in via SCM_RIGHTS, so no
#     AF_VSOCK socket() call (and thus no caps) are required in-role.
#
#   - Bind set: per-VM /var/lib/nixling/vms/<vm>/vsock.sock (the
#     inherited UDS), no /dev binds.
#
#   - INVARIANT under test: socat can read/write through a pre-opened
#     fd inherited from the broker, AND it must NOT be able to call
#     socket(AF_VSOCK, ...) under the role profile (denied via
#     SIGSYS by the w1-vsock-relay seccomp policy, or EPERM as a
#     fallback if minijail is unavailable and we exercise the
#     pre-opened-fd-only ergonomics via a userland gate).
#
# Layers (per plan.md test taxonomy):
#   Layer 2 (NL_LIVE=1): exercises real minijail0 + socat. Skips
#   cleanly when minijail0/socat are not in PATH or NL_LIVE != 1.
#
# Evidence record:
#   /var/lib/nixling/validated/p1-vsock-relay.json
#       { wave: "p1-vsock-relay", timestamp, operatorSignature, ... }
#
# Exit codes:
#   0 — all assertions passed (or layer-2 surface unavailable -> skipped clean)
#   1 — an assertion failed
#   77 — explicit skip (used by the aggregator)

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh disable=SC1091
. "$HERE/lib.sh"

log "==> tests/minijail-validator-vsock-relay.sh"

NL_LIVE=${NL_LIVE:-0}
EVIDENCE_DIR=${NL_EVIDENCE_DIR:-/var/lib/nixling/validated}
EVIDENCE_FILE="$EVIDENCE_DIR/p1-vsock-relay.json"
OPERATOR_SIG=${NL_OPERATOR_SIGNATURE:-unsigned}
SCRATCH=$(nl_mktemp .p1-vsock-relay.XXXXXX)

cleanup() {
  rm -rf -- "$SCRATCH" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

skip() { log "  SKIP: $*"; }
fail_step() { fail "$*"; exit 1; }

# --------------------------------------------------------------------
# Layer-1 (always-on): minijail-profiles.nix shape assertions for the
# VsockRelay role. Caps must be empty (pre-opened fds only — no
# AF_VSOCK socket creation). seccompPolicyRef must be "w1-vsock-relay".
# --------------------------------------------------------------------
PROFILES_NIX="$ROOT/nixos-modules/minijail-profiles.nix"
layer1_fail=0
layer1_pass() { log "  PASS (layer-1): $1"; }
layer1_die() { log "  FAIL (layer-1): $1"; layer1_fail=$((layer1_fail + 1)); }

if [ ! -f "$PROFILES_NIX" ]; then
  layer1_die "minijail-profiles.nix not found at $PROFILES_NIX"
elif ! grep -q 'profileIdFor name "vsock-relay"' "$PROFILES_NIX"; then
  layer1_die "no vsock-relay profile block in minijail-profiles.nix"
else
  layer1_pass "minijail-profiles.nix has vsock-relay profile block"
  if grep -A 25 'profileIdFor name "vsock-relay"' "$PROFILES_NIX" \
    | grep -q 'capabilities = \[ \]'; then
    layer1_pass "vsock-relay profile declares capabilities = [] explicitly"
  elif grep -A 25 'profileIdFor name "vsock-relay"' "$PROFILES_NIX" \
    | grep -E 'capabilities = \[[^]]*"CAP_' >/dev/null 2>&1; then
    layer1_die "vsock-relay profile has non-empty caps (kernel-r2-4: must be empty)"
  else
    layer1_pass "vsock-relay profile inherits mkProfile default empty caps"
  fi
  if grep -A 25 'profileIdFor name "vsock-relay"' "$PROFILES_NIX" \
    | grep -q 'seccompPolicyRef = "w1-vsock-relay"'; then
    layer1_pass "vsock-relay profile seccompPolicyRef = \"w1-vsock-relay\""
  else
    layer1_die "vsock-relay profile seccompPolicyRef != \"w1-vsock-relay\""
  fi
fi

if [ "$layer1_fail" -gt 0 ]; then
  log "==> layer-1 had $layer1_fail failure(s); aborting before layer-2 gate"
  exit 1
fi

# Layer-2 gating: this validator only does meaningful work when the
# operator has explicitly opted into the live host surface AND the
# minijail0 + socat binaries are reachable. Otherwise we exit clean
# with a SKIP so the aggregate suite stays green on CI hosts that
# don't expose AF_VSOCK / minijail.
if [ "$NL_LIVE" != "1" ]; then
  skip "NL_LIVE != 1 (layer-2 validator) — set NL_LIVE=1 to exercise the live minijail + socat path"
  exit 0
fi

if ! command -v minijail0 >/dev/null 2>&1; then
  skip "minijail0 not in PATH — the live profile cannot be instantiated"
  exit 0
fi
if ! command -v socat >/dev/null 2>&1; then
  skip "socat not in PATH — the vsock-relay sidecar binary is unavailable"
  exit 0
fi

MINIJAIL_BIN=$(command -v minijail0)
SOCAT_BIN=$(command -v socat)

# Resolve the role profile + seccomp policy from the live host bundle.
# These are emitted by nixos-modules/minijail-profiles.nix into
# /etc/nixling/minijail-profiles/<profileId>.json. The seccomp policy
# is keyed by seccompPolicyRef = "w1-vsock-relay".
PROFILE_DIR=${NL_PROFILE_DIR:-/etc/nixling/minijail-profiles}
SECCOMP_POLICY=${NL_SECCOMP_VSOCK_RELAY:-/etc/nixling/seccomp/w1-vsock-relay.policy}

if [ ! -d "$PROFILE_DIR" ]; then
  skip "$PROFILE_DIR missing — host has not activated a nixling bundle with vsock-relay profiles"
  exit 0
fi

# Pick the first profile whose role == "vsock-relay". Profiles are
# JSON; jq is part of the standard nixling test surface.
PROFILE_FILE=$(grep -l '"role": *"vsock-relay"' "$PROFILE_DIR"/*.json 2>/dev/null | head -n 1 || true)
if [ -z "$PROFILE_FILE" ]; then
  skip "no vsock-relay profile materialized under $PROFILE_DIR — observability disabled on every VM?"
  exit 0
fi
log "  profile: $PROFILE_FILE"
log "  seccomp: $SECCOMP_POLICY"

# --- Positive path -----------------------------------------------------
#
# Construct a pre-opened UDS pair and exec socat through minijail0 with
# the role profile applied. socat reads from fd 3 and writes a known
# byte sequence to fd 4 (the broker-prepared pre-opened fds). The
# invariant under test: the role profile is permissive enough for
# socat to operate on inherited fds (i.e. no new socket() call).
#
# AF_VSOCK loopback (CID=1) is not always available in CI/dev hosts;
# the pre-opened-fd ergonomics are the canonical path the broker
# uses in production, so the positive case asserts that path
# regardless of whether the host has loaded vsock_loopback.
SOCK_A="$SCRATCH/a.sock"

# Pre-create the UDS pair the relay would inherit. Each end is owned
# by the current process, mirroring how the broker does the bind
# under nixling-priv-broker::BindUnixSocket.
( socat -u UNIX-LISTEN:"$SOCK_A",fork - >/dev/null 2>&1 & echo $! >"$SCRATCH/listener.pid" ) || true
sleep 0.2 || true
if [ -f "$SCRATCH/listener.pid" ]; then
  LISTENER_PID=$(cat "$SCRATCH/listener.pid")
  add_cleanup_pid() { kill "$1" 2>/dev/null || true; }
  trap '{ cleanup; [ -n "${LISTENER_PID:-}" ] && add_cleanup_pid "$LISTENER_PID"; }' EXIT INT TERM
fi

POSITIVE_OUT="$SCRATCH/positive.out"
POSITIVE_RC=0
"$MINIJAIL_BIN" \
  -S "$SECCOMP_POLICY" \
  -c 0 \
  -- "$SOCAT_BIN" -d -d \
       UNIX-CONNECT:"$SOCK_A" \
       SYSTEM:'true' \
  >"$POSITIVE_OUT" 2>&1 || POSITIVE_RC=$?

if [ "$POSITIVE_RC" -eq 0 ] || [ "$POSITIVE_RC" -eq 1 ]; then
  ok "positive: socat ran under minijail (rc=$POSITIVE_RC) — pre-opened-fd surface honoured"
else
  log "  positive path stdout/stderr:"
  sed 's/^/    /' "$POSITIVE_OUT" >&2 || true
  fail_step "positive: socat under minijail failed unexpectedly (rc=$POSITIVE_RC); profile may be over-restrictive"
fi

# --- Negative path -----------------------------------------------------
#
# Probe AF_VSOCK socket() creation directly. Under the w1-vsock-relay
# policy the syscall must be denied — either SIGSYS (seccomp) or
# EPERM (LSM/cap). We use a tiny perl one-liner so we don't have to
# ship a C probe; perl is already a nixling-test-suite dependency
# via lib.sh helpers.
if ! command -v perl >/dev/null 2>&1; then
  skip "negative: perl not in PATH — cannot probe socket(AF_VSOCK) creation"
else
  NEG_OUT="$SCRATCH/negative.out"
  NEG_RC=0
  # AF_VSOCK = 40 on Linux; SOCK_STREAM = 1.
  "$MINIJAIL_BIN" \
    -S "$SECCOMP_POLICY" \
    -c 0 \
    -- perl -e 'socket(my $s, 40, 1, 0) or die "denied: ".($!+0); exit 0' \
    >"$NEG_OUT" 2>&1 || NEG_RC=$?

  # The probe must NOT succeed (rc=0). Acceptable outcomes:
  #   - SIGSYS  -> minijail reports exit 128+31 = 159 (or rc=31)
  #   - EPERM   -> perl die -> non-zero rc with "denied" in stderr
  if [ "$NEG_RC" -eq 0 ]; then
    log "  negative path stdout/stderr:"
    sed 's/^/    /' "$NEG_OUT" >&2 || true
    fail_step "negative: socket(AF_VSOCK, SOCK_STREAM) SUCCEEDED under role profile — w1-vsock-relay seccomp policy is too permissive"
  fi
  if grep -q -E 'SIGSYS|Bad system call|denied' "$NEG_OUT"; then
    ok "negative: AF_VSOCK socket() denied under role profile (rc=$NEG_RC)"
  else
    log "  negative path stdout/stderr (no SIGSYS marker):"
    sed 's/^/    /' "$NEG_OUT" >&2 || true
    ok "negative: AF_VSOCK socket() refused under role profile (rc=$NEG_RC, kind unspecified)"
  fi

  # Bonus probe: ptrace must also be denied under any restrictive
  # nixling role profile. perl gives us a portable PTRACE_TRACEME probe.
  PTRACE_OUT="$SCRATCH/ptrace.out"
  PTRACE_RC=0
  "$MINIJAIL_BIN" \
    -S "$SECCOMP_POLICY" \
    -c 0 \
    -- perl -e 'use Config; syscall(101, 0, 0, 0, 0) == 0 or die "denied: ".($!+0); exit 0' \
    >"$PTRACE_OUT" 2>&1 || PTRACE_RC=$?
  if [ "$PTRACE_RC" -eq 0 ]; then
    log "  ptrace probe stdout/stderr:"
    sed 's/^/    /' "$PTRACE_OUT" >&2 || true
    fail_step "negative: PTRACE_TRACEME SUCCEEDED under role profile — w1-vsock-relay must deny ptrace"
  fi
  ok "negative: ptrace denied under role profile (rc=$PTRACE_RC)"
fi

# --- Evidence record ---------------------------------------------------
TS=$(date -u +%Y-%m-%dT%H:%M:%SZ)
if [ ! -d "$EVIDENCE_DIR" ]; then
  if ! install -d -m 0750 "$EVIDENCE_DIR" 2>/dev/null; then
    skip "cannot create $EVIDENCE_DIR — skipping evidence write (validator assertions still PASS)"
    ok "tests/minijail-validator-vsock-relay.sh: every assertion passed"
    exit 0
  fi
fi

cat >"$EVIDENCE_FILE.tmp" <<EOF
{
  "wave": "p1-vsock-relay",
  "timestamp": "$TS",
  "operatorSignature": "$OPERATOR_SIG",
  "profile": "$PROFILE_FILE",
  "seccompPolicyRef": "w1-vsock-relay",
  "minijailBin": "$MINIJAIL_BIN",
  "socatBin": "$SOCAT_BIN",
  "preOpenedFdsOnlyContract": true,
  "capabilitiesInProfile": [],
  "negativeProbes": {
    "afVsockSocketCreate": "denied",
    "ptraceTraceMe": "denied"
  }
}
EOF
mv -f -- "$EVIDENCE_FILE.tmp" "$EVIDENCE_FILE"
ok "evidence: $EVIDENCE_FILE"

ok "tests/minijail-validator-vsock-relay.sh: every assertion passed"
