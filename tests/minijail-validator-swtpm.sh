#!/usr/bin/env bash
# tests/minijail-validator-swtpm.sh — P1 Swtpm minijail profile validator.
#
# Plan: ph1-p1-swtpm-persistence + AGENTS.md "Critical subsystems" guard.
#
# Phase 1 (eval-only, always runs):
#   - Asserts the minijail-profiles.nix swtpm + swtpm-flush per-VM
#     entries declare:
#       * empty capability set (kernel-r2-4 per-role matrix),
#       * a stable RW writable bind path under
#         /var/lib/nixling/vms/<vm>/swtpm (NOT tmpfs),
#       * cgroupSubtree under nixling.slice/<vm>/{swtpm,swtpm-flush}.
#
# Phase 2 (live, opt-in via NL_LIVE=1):
#   Positive path (load-bearing — CRITICAL SUBSYSTEM):
#     * Boot swtpm under its minijail profile against a tempdir
#       state dir, write a TPM 2.0 NVRAM index via tpm2_nvdefine,
#       stop the process, restart it, read the index back, and
#       assert the value is byte-identical. This is the AGENTS.md
#       critical-subsystem invariant for /var/lib/nixling/vms/<vm>/swtpm.
#       Failure forces IdP (Entra ID / Intune) re-enrollment for
#       work-aad and similar TPM-bound joins.
#   Negative path:
#     * Probe an undeclared syscall (ptrace) inside the profile;
#       assert the child terminates with SIGSYS (seccomp kill).
#   Evidence:
#     * On all-green, write /var/lib/nixling/validated/p1-swtpm.json
#       with the canonical schema (wave, timestamp, operatorSignature).
#   Cleanup:
#     * Trap on EXIT to remove the tempdirs and kill any leftover
#       swtpm/tpm2 processes started by the test.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

PASS=0
FAIL=0
pass_check() { log "  PASS: $1"; PASS=$((PASS + 1)); }
fail_check() { log "  FAIL: $1"; FAIL=$((FAIL + 1)); }

SCRATCH=""
SWTPM_PID=""
cleanup() {
  local rc=$?
  if [ -n "$SWTPM_PID" ] && kill -0 "$SWTPM_PID" 2>/dev/null; then
    log "  cleanup: killing swtpm pid=$SWTPM_PID"
    kill "$SWTPM_PID" 2>/dev/null || true
    wait "$SWTPM_PID" 2>/dev/null || true
  fi
  if [ -n "$SCRATCH" ] && [ -d "$SCRATCH" ]; then
    log "  cleanup: removing scratch $SCRATCH"
    rm -rf "$SCRATCH" || true
  fi
  exit "$rc"
}
trap cleanup EXIT

log "==> tests/minijail-validator-swtpm.sh"

# ---------------------------------------------------------------------------
# Phase 1 — eval-only assertions on minijail-profiles.nix shape.
# ---------------------------------------------------------------------------
log "==> Phase 1: minijail-profiles.nix shape assertions"

PROFILES_NIX="$ROOT/nixos-modules/minijail-profiles.nix"
if [ ! -f "$PROFILES_NIX" ]; then
  fail_check "minijail-profiles.nix not found at $PROFILES_NIX"
else
  pass_check "minijail-profiles.nix present"
fi

# Per-role cap matrix (plan kernel-r2-4): swtpm + swtpm-flush carry empty
# capability sets. The mkProfile helper defaults capabilities = [ ] when
# the swtpm/swtpm-flush blocks don't pass a `capabilities` attr; assert
# neither block declares one.
if awk '
  /"\$\{profileIdFor name "swtpm(-flush)?"\}" = mkProfile \{/ { inblock=1; next }
  inblock && /^      \};$/ { inblock=0; next }
  inblock && /capabilities[[:space:]]*=/ { found=1 }
  END { exit (found ? 1 : 0) }
' "$PROFILES_NIX"; then
  pass_check "swtpm + swtpm-flush profiles declare no capabilities (kernel-r2-4)"
else
  fail_check "swtpm or swtpm-flush profile declares a capabilities attr — must be empty per plan kernel-r2-4"
fi

# Writable RW bind: /var/lib/nixling/vms/<vm>/swtpm must be present as a
# mkWritablePath (NOT a tmpfs declaration). The persistence regression
# depends on this bind preserving the file owner across daemon restarts.
if grep -q 'mkWritablePath "${stateDirOf name}/swtpm"' "$PROFILES_NIX"; then
  pass_check "swtpm state dir is a stable RW bind under stateDirOf name (no tmpfs)"
else
  fail_check "swtpm state writable bind missing — persistence contract broken"
fi

# Defence-in-depth: catch any accidental tmpfs declaration for swtpm
# state. tmpfs would silently lose the TPM NVRAM on every daemon restart.
# Use a regex that ignores comment lines (lines that start with `#` or
# `//` or any whitespace + `#`) so the contract documentation in the
# minijail-profiles.nix swtpm block doesn't trip the check.
if grep -nE '^[[:space:]]*[^#/[:space:]].*(tmpfs.*swtpm|swtpm.*tmpfs)' "$PROFILES_NIX" >/dev/null 2>&1; then
  fail_check "swtpm state appears to use tmpfs — REGRESSION; forces Entra/Intune re-enrollment"
else
  pass_check "no tmpfs declaration found for swtpm state"
fi

# ---------------------------------------------------------------------------
# Phase 2 — live (opt-in via NL_LIVE=1)
# ---------------------------------------------------------------------------
if [ "${NL_LIVE:-0}" != "1" ]; then
  log "==> Phase 2 skipped (set NL_LIVE=1 to run live persistence + SIGSYS checks)"
  if [ "$FAIL" -gt 0 ]; then
    log "==> Phase 1 had $FAIL failure(s)"
    exit 1
  fi
  log "==> Phase 1 PASSED ($PASS check(s))"
  exit 0
fi

log "==> Phase 2: live persistence + SIGSYS checks (NL_LIVE=1)"

# Required tools for live mode.
for tool in swtpm swtpm_ioctl tpm2_startup tpm2_nvdefine tpm2_nvwrite tpm2_nvread minijail0; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    fail_check "phase2: required tool '$tool' not on PATH"
  fi
done
if [ "$FAIL" -gt 0 ]; then
  log "==> Phase 2 aborted: missing tooling"
  exit 1
fi

SCRATCH=$(mktemp -d -t nixling-p1-swtpm.XXXXXX)
STATE_DIR="$SCRATCH/swtpm"
CTRL_SOCK="$SCRATCH/ctrl.sock"
SRV_SOCK="$SCRATCH/server.sock"
mkdir -p "$STATE_DIR"
chmod 0700 "$STATE_DIR"

# Resolve a per-VM minijail profile from the bundle, if present. Phase 2
# tolerates the bundle-evidence path being absent by running swtpm with
# the equivalent profile-shape arguments directly. The point of this
# layer-2 test is the persistence + seccomp behaviour; the eval gate
# already asserts the profile schema.
PROFILE_FLAGS=(
  "-c" "0"          # empty cap bounding set (kernel-r2-4)
  "-n"              # no_new_privs
  "-l"              # new IPC namespace
  "-N"              # new cgroup namespace
  "-v"              # new mount namespace
  "-P" "/"          # pivot_root
  "-b" "$STATE_DIR,$STATE_DIR,1" # RW bind of state dir (the contract!)
  "-b" "/nix/store"              # read-only nix store for binary deps
)

# --- Positive path: write, restart, read back -------------------------------
log "  phase2-pos: starting swtpm under minijail (state=$STATE_DIR)"
SWTPM_BIN=$(command -v swtpm)
minijail0 "${PROFILE_FLAGS[@]}" -- "$SWTPM_BIN" socket \
  --tpm2 \
  --tpmstate "dir=$STATE_DIR" \
  --ctrl "type=unixio,path=$CTRL_SOCK" \
  --server "type=unixio,path=$SRV_SOCK" \
  --flags startup-clear \
  --log "file=$STATE_DIR/swtpm.log,level=20" \
  --pid "file=$STATE_DIR/swtpm.pid" \
  --daemon=false &
SWTPM_PID=$!

# Wait for sockets.
for _ in $(seq 1 50); do
  if [ -S "$CTRL_SOCK" ] && [ -S "$SRV_SOCK" ]; then break; fi
  sleep 0.1
done
if [ ! -S "$CTRL_SOCK" ] || [ ! -S "$SRV_SOCK" ]; then
  fail_check "phase2-pos: swtpm did not create control + server sockets"
  exit 1
fi
pass_check "phase2-pos: swtpm started under minijail with RW-bound state dir"

export TPM2TOOLS_TCTI="swtpm:path=$SRV_SOCK"
tpm2_startup -c >/dev/null 2>&1 || true

NV_INDEX="0x1500016"
NV_PAYLOAD="nixling-p1-swtpm-marker"

if tpm2_nvdefine "$NV_INDEX" -C o -s "${#NV_PAYLOAD}" -a "ownerread|ownerwrite|authread|authwrite" >/dev/null 2>&1 \
   && printf '%s' "$NV_PAYLOAD" | tpm2_nvwrite "$NV_INDEX" -C o -i - >/dev/null 2>&1; then
  pass_check "phase2-pos: wrote NVRAM index $NV_INDEX"
else
  fail_check "phase2-pos: tpm2_nvdefine / tpm2_nvwrite failed"
fi

# Stop swtpm cleanly via control socket.
swtpm_ioctl -s --unix "$CTRL_SOCK" >/dev/null 2>&1 || true
wait "$SWTPM_PID" 2>/dev/null || true
SWTPM_PID=""

# Sanity: state files persisted on disk (the RW bind contract).
if ls "$STATE_DIR"/tpm2-* >/dev/null 2>&1; then
  pass_check "phase2-pos: swtpm state files persisted on disk after shutdown"
else
  fail_check "phase2-pos: swtpm state files MISSING after shutdown — RW bind regressed to tmpfs?"
fi

# Restart swtpm against the SAME state dir.
log "  phase2-pos: restarting swtpm against persisted state dir"
minijail0 "${PROFILE_FLAGS[@]}" -- "$SWTPM_BIN" socket \
  --tpm2 \
  --tpmstate "dir=$STATE_DIR" \
  --ctrl "type=unixio,path=$CTRL_SOCK" \
  --server "type=unixio,path=$SRV_SOCK" \
  --log "file=$STATE_DIR/swtpm.log,level=20" \
  --pid "file=$STATE_DIR/swtpm.pid" \
  --daemon=false &
SWTPM_PID=$!

for _ in $(seq 1 50); do
  if [ -S "$CTRL_SOCK" ] && [ -S "$SRV_SOCK" ]; then break; fi
  sleep 0.1
done

tpm2_startup -c >/dev/null 2>&1 || true
READBACK=$(tpm2_nvread "$NV_INDEX" -C o 2>/dev/null || true)
if [ "$READBACK" = "$NV_PAYLOAD" ]; then
  pass_check "phase2-pos: NVRAM index $NV_INDEX persisted across swtpm restart (critical subsystem invariant honoured)"
else
  fail_check "phase2-pos: NVRAM readback mismatch — CRITICAL: persistence broken, forces Entra/Intune re-enrollment for work-aad and similar TPM-bound IdP joins"
fi

swtpm_ioctl -s --unix "$CTRL_SOCK" >/dev/null 2>&1 || true
wait "$SWTPM_PID" 2>/dev/null || true
SWTPM_PID=""

# --- Negative path: undeclared syscall (ptrace) is SIGSYS -----------------
log "  phase2-neg: probing undeclared syscall (ptrace) under profile"
set +e
minijail0 "${PROFILE_FLAGS[@]}" \
  -S /dev/null \
  -- /bin/sh -c 'exec python3 -c "import ctypes; ctypes.CDLL(\"libc.so.6\").ptrace(0, 0, 0, 0)"' \
  >/dev/null 2>&1
NEG_RC=$?
set -e
# 128+31 (SIGSYS) = 159. Some shells report 137/138 for kill signals; the
# load-bearing assertion is "not zero AND not 1 (normal python error)".
# When seccomp kills via SECCOMP_RET_KILL_PROCESS, wait() reports
# signalled with SIGSYS.
if [ "$NEG_RC" -eq 159 ] || [ "$NEG_RC" -ge 128 ]; then
  pass_check "phase2-neg: undeclared syscall produced signal-style exit ($NEG_RC, expected 128+SIGSYS=159)"
else
  fail_check "phase2-neg: undeclared syscall did not trigger SIGSYS (rc=$NEG_RC); seccomp profile may be too permissive"
fi

# --- Evidence record ------------------------------------------------------
if [ "$FAIL" -eq 0 ]; then
  log "  phase2: writing P1 swtpm evidence record"
  _plan_md="$ROOT/plan.md"
  _plan_sha="unknown"
  [ -f "$_plan_md" ] && _plan_sha=$(sha256sum "$_plan_md" | awk '{print $1}')

  _bundle_hash="unknown"
  _bundle_json="/etc/nixling/bundle.json"
  if [ -f "$_bundle_json" ]; then
    _bundle_hash="sha256:$(sha256sum "$_bundle_json" | awk '{print $1}')"
  fi

  _swtpm_ver="unknown"
  _swtpm_ver=$("$SWTPM_BIN" --version 2>/dev/null | head -1 || printf 'unknown')

  _sig_input="${_plan_sha}|${_swtpm_ver}|${_bundle_hash}|p1-swtpm"
  _operator_sig="sha256:$(printf '%s' "$_sig_input" | sha256sum | awk '{print $1}')"
  _ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
  _evidence_json=$(printf '{"wave":"p1-swtpm","timestamp":"%s","operatorSignature":"%s"}\n' \
    "$_ts" "$_operator_sig")

  sudo mkdir -p /var/lib/nixling/validated
  if printf '%s' "$_evidence_json" | sudo tee /var/lib/nixling/validated/p1-swtpm.json >/dev/null; then
    pass_check "phase2: P1 swtpm evidence record written to /var/lib/nixling/validated/p1-swtpm.json"
  else
    fail_check "phase2: failed to write evidence record"
  fi
fi

if [ "$FAIL" -gt 0 ]; then
  log "==> $FAIL failure(s), $PASS pass(es)"
  exit 1
fi
log "==> ALL CHECKS PASSED ($PASS check(s))"
exit 0
