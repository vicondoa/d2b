#!/usr/bin/env bash
# tests/swtpm-persistence-smoke.sh — P1 critical-subsystem regression.
#
# Plan: ph1-p1-swtpm-persistence.
#
# This is the END-TO-END regression for the AGENTS.md "Critical
# subsystems" guard on /var/lib/nixling/vms/<vm>/swtpm. The narrower
# tests/minijail-validator-swtpm.sh exercises a tempdir under the same
# minijail profile, but this script drives a real per-VM swtpm sidecar
# through the daemon, restarts the daemon (not just the VM, not just
# the sidecar), restarts the VM, and reads the TPM NVRAM index back.
#
# It MUST be run NL_LIVE=1 on a host with nixling activated and at
# least one TPM-enabled VM declared. Default target VM is "corp-vm";
# override with NL_VM=<name>.
#
# !!! WARNING !!!
# Failure of this test means the swtpm state bind regressed (most
# likely to a tmpfs or a non-stable owner across restarts). Such a
# failure on a production host forces Entra/Intune re-enrollment for
# work-aad and similar TPM-bound IdP joins because the IdP will see a
# fresh EK seed and a wiped NVRAM and refuse the device. Do NOT mark
# this test "expected to fail" — fix the bind contract instead.
#
# Layer 2; opt-in via NL_LIVE=1. Cleanup trap restores all changes
# (best-effort, since the destructive step is the daemon restart).

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

VM="${NL_VM:-corp-vm}"
NV_INDEX="${NL_NV_INDEX:-0x1500017}"
NV_PAYLOAD="${NL_NV_PAYLOAD:-nixling-persistence-smoke}"

PASS=0
FAIL=0
pass_check() { log "  PASS: $1"; PASS=$((PASS + 1)); }
fail_check() { log "  FAIL: $1"; FAIL=$((FAIL + 1)); }

cleanup() {
  local rc=$?
  log "  cleanup: best-effort restore of nixlingd"
  sudo systemctl start nixlingd.service 2>/dev/null || true
  exit "$rc"
}
trap cleanup EXIT

log "==> tests/swtpm-persistence-smoke.sh"

if [ "${NL_LIVE:-0}" != "1" ]; then
  log "==> SKIP: set NL_LIVE=1 to run this Layer-2 persistence regression"
  exit 0
fi

log "==> WARNING: This test exercises the TPM NVRAM persistence contract."
log "==> WARNING: Failure here forces Entra/Intune re-enrollment for work-aad"
log "==> WARNING: and any other TPM-bound IdP joins on the target host."

# Sanity: required tools + nixling on PATH.
for tool in nixling jq systemctl tpm2_nvdefine tpm2_nvwrite tpm2_nvread tpm2_startup; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    fail_check "required tool '$tool' not on PATH"
  fi
done
[ "$FAIL" -gt 0 ] && exit 1

# Start the VM (idempotent).
log "==> Step 1: start VM $VM"
if ! sudo nixling vm start "$VM" >/dev/null 2>&1; then
  fail_check "nixling vm start $VM failed"
  exit 1
fi
pass_check "VM $VM running"

# Resolve TCTI for the running VM's swtpm server socket. Path follows
# the swtpm_argv generator: /run/nixling/vms/<vm>/swtpm.sock.
SRV_SOCK="/run/nixling/vms/${VM}/swtpm.sock"
for _ in $(seq 1 100); do
  [ -S "$SRV_SOCK" ] && break
  sleep 0.2
done
if [ ! -S "$SRV_SOCK" ]; then
  fail_check "swtpm server socket $SRV_SOCK never appeared"
  exit 1
fi
export TPM2TOOLS_TCTI="swtpm:path=$SRV_SOCK"

# Write the NVRAM marker (guest-side path requires SSH into VM; we
# write host-side via the server socket since the swtpm server speaks
# to whoever can reach the socket — the daemon and root).
log "==> Step 2: write NVRAM index $NV_INDEX = '$NV_PAYLOAD'"
tpm2_startup -c >/dev/null 2>&1 || true
if sudo -E tpm2_nvdefine "$NV_INDEX" -C o -s "${#NV_PAYLOAD}" \
     -a "ownerread|ownerwrite|authread|authwrite" >/dev/null 2>&1; then
  pass_check "tpm2_nvdefine $NV_INDEX"
elif sudo -E tpm2_nvread "$NV_INDEX" -C o >/dev/null 2>&1; then
  pass_check "NVRAM index $NV_INDEX already defined (reusing)"
else
  fail_check "tpm2_nvdefine $NV_INDEX failed"
  exit 1
fi
if printf '%s' "$NV_PAYLOAD" | sudo -E tpm2_nvwrite "$NV_INDEX" -C o -i - >/dev/null 2>&1; then
  pass_check "tpm2_nvwrite $NV_INDEX"
else
  fail_check "tpm2_nvwrite $NV_INDEX failed"
  exit 1
fi

# Stop the VM.
log "==> Step 3: stop VM $VM"
sudo nixling vm stop "$VM" >/dev/null 2>&1 || true
pass_check "nixling vm stop $VM returned"

# Restart the daemon — the load-bearing step. swtpm state must survive
# a daemon restart, not merely a sidecar restart.
log "==> Step 4: restart nixlingd.service (daemon-level restart)"
sudo systemctl restart nixlingd.service
sleep 2
if systemctl is-active --quiet nixlingd.service; then
  pass_check "nixlingd restarted cleanly"
else
  fail_check "nixlingd failed to come back up after restart"
  exit 1
fi

# Start the VM again, on the same per-VM state dir.
log "==> Step 5: start VM $VM again"
if ! sudo nixling vm start "$VM" >/dev/null 2>&1; then
  fail_check "nixling vm start $VM (post-restart) failed"
  exit 1
fi
for _ in $(seq 1 100); do
  [ -S "$SRV_SOCK" ] && break
  sleep 0.2
done
if [ ! -S "$SRV_SOCK" ]; then
  fail_check "swtpm server socket $SRV_SOCK never reappeared after restart"
  exit 1
fi
pass_check "VM $VM + swtpm sidecar back up against persisted state"

# Read NVRAM back. THIS is the AGENTS.md critical-subsystem invariant.
log "==> Step 6: read NVRAM index $NV_INDEX back"
tpm2_startup -c >/dev/null 2>&1 || true
READBACK=$(sudo -E tpm2_nvread "$NV_INDEX" -C o 2>/dev/null || true)
if [ "$READBACK" = "$NV_PAYLOAD" ]; then
  pass_check "NVRAM survived daemon restart — critical-subsystem invariant honoured"
else
  log "  observed = '$READBACK' expected = '$NV_PAYLOAD'"
  fail_check "WARNING: this failure forces Entra/Intune re-enrollment for work-aad and similar TPM-bound IdP joins. NVRAM did NOT survive the daemon restart — the /var/lib/nixling/vms/${VM}/swtpm bind is not preserving state across daemon restarts."
fi

if [ "$FAIL" -gt 0 ]; then
  log "==> $FAIL failure(s), $PASS pass(es)"
  exit 1
fi
log "==> PASSED ($PASS check(s))"
exit 0
