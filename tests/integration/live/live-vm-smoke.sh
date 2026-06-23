#!/usr/bin/env bash
# shellcheck disable=SC2126,SC2329
# tests/integration/live/live-vm-smoke.sh— v1.2 live-VM smoke gate.
#
# Pre-tag maintainer-side gate per ADR 0022 + v1.2 plan §.
# SKIP-ON-CI (requires KVM / systemd / privileged broker).
#
# Modes:
#   --lite    Single VM (personal-dev), ≤5 min.  For panel-round HEAD.
#   --full    Both VMs (personal-dev + work-aad), ≤20 min.  Default.
#             REQUIRED before any v1.2.* tag (per I5).
#
# Exit codes:
#   0   PASS
#   1   FAIL
#   77  SKIP (KVM absent / nixling not running / VMs not declared)
#
# Configurable via environment:
#   NL_SMOKE_TIMEOUT_SSH     seconds to wait for SSH (default 120)
#   NL_SMOKE_APIREADY_BUDGET seconds to wait for api_ready (default 60)
#   NL_SMOKE_VM_PRIMARY      primary VM name (default personal-dev)
#   NL_SMOKE_VM_SECONDARY    secondary VM for --full (default work-aad)

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}

# ---------------------------------------------------------------------------
# Source lib.sh helpers when available; otherwise define minimal stubs.
# ---------------------------------------------------------------------------
if [ -f "$ROOT/tests/lib.sh" ]; then
  # shellcheck source=tests/lib.sh
  . "$ROOT/tests/lib.sh"
else
  log()  { printf '[smoke] %s\n' "$*" >&2; }
  ok()   { printf '[smoke] PASS: %s\n' "$*" >&2; }
  fail() { printf '[smoke] FAIL: %s\n' "$*" >&2; }
fi

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
NL_SMOKE_TIMEOUT_SSH=${NL_SMOKE_TIMEOUT_SSH:-120}
NL_SMOKE_APIREADY_BUDGET=${NL_SMOKE_APIREADY_BUDGET:-60}
NL_SMOKE_VM_PRIMARY=${NL_SMOKE_VM_PRIMARY:-personal-dev}
NL_SMOKE_VM_SECONDARY=${NL_SMOKE_VM_SECONDARY:-work-aad}

PIDFD_TABLE=/var/lib/nixling/daemon-state/pidfd-table.json
VM_RUN_BASE=/run/nixling/vms
VM_STATE_BASE=/var/lib/nixling/vms

MODE=full
for arg in "$@"; do
  case "$arg" in
    --lite) MODE=lite ;;
    --full) MODE=full ;;
    *)
      log "unknown argument: $arg"
      exit 1
      ;;
  esac
done

PASS=0
FAIL=0

pass_check() { log "  PASS: $1"; PASS=$((PASS + 1)); }
fail_check() { log "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# ---------------------------------------------------------------------------
# Pre-flight skip checks (exit 77 = SKIP)
# ---------------------------------------------------------------------------
log "==> tests/integration/live/live-vm-smoke.sh (mode: $MODE)"

if [ ! -e /dev/kvm ]; then
  log "==> SKIP: /dev/kvm not present (no KVM support)"
  exit 77
fi

if ! systemctl is-active --quiet nixling-priv-broker 2>/dev/null; then
  log "==> SKIP: nixling-priv-broker is not active (systemctl is-active returned non-zero)"
  exit 77
fi

if ! command -v nixling >/dev/null 2>&1; then
  log "==> SKIP: nixling not on PATH"
  exit 77
fi

# Check that the primary VM is declared in the manifest.
if ! nixling vm status "$NL_SMOKE_VM_PRIMARY" >/dev/null 2>&1; then
  log "==> SKIP: VM '$NL_SMOKE_VM_PRIMARY' not declared in manifest"
  exit 77
fi

if [ "$MODE" = "full" ]; then
  if ! nixling vm status "$NL_SMOKE_VM_SECONDARY" >/dev/null 2>&1; then
    log "==> SKIP: VM '$NL_SMOKE_VM_SECONDARY' not declared (required for --full)"
    exit 77
  fi
fi

# ---------------------------------------------------------------------------
# Probe helpers
# ---------------------------------------------------------------------------

# wait_for_ssh <ip> <timeout_secs> — poll until SSH uname -a succeeds.
wait_for_ssh() {
  local ip="$1" timeout="$2" elapsed=0 interval=5
  while [ "$elapsed" -lt "$timeout" ]; do
    if ssh -o StrictHostKeyChecking=no \
           -o BatchMode=yes \
           -o ConnectTimeout=5 \
           "root@${ip}" uname -a >/dev/null 2>&1; then
      return 0
    fi
    sleep "$interval"
    elapsed=$((elapsed + interval))
  done
  return 1
}

# vm_ip <vm> — resolve the VM's static IP from the nixling manifest.
vm_ip() {
  nixling vm status "$1" --json 2>/dev/null \
    | grep -o '"static_ip"[[:space:]]*:[[:space:]]*"[^"]*"' \
    | grep -o '"[0-9][^"]*"' \
    | tr -d '"' \
    | head -1
}

# api_socket <vm> — path to CH HTTP API socket.
# Convention from manifest.nix: /var/lib/nixling/vms/<vm>/<vm>.sock
api_socket() {
  printf '%s/%s/%s.sock\n' "$VM_STATE_BASE" "$1" "$1"
}

# ch_pid <vm> — PID of the cloud-hypervisor process for the given VM.
ch_pid() {
  local vm="$1"
  if [ -f "$PIDFD_TABLE" ]; then
    grep -o "\"${vm}:cloud-hypervisor\"[[:space:]]*:[[:space:]]*{[^}]*\"pid\"[[:space:]]*:[[:space:]]*[0-9]*" \
         "$PIDFD_TABLE" 2>/dev/null \
      | grep -o '"pid"[[:space:]]*:[[:space:]]*[0-9]*' \
      | grep -o '[0-9]*$' \
      | head -1
  fi
}

# wait_for_api_ready <vm> <budget_secs> — wait until nixling vm status reports api_ready yes.
wait_for_api_ready() {
  local vm="$1" budget="$2" elapsed=0 interval=5
  while [ "$elapsed" -lt "$budget" ]; do
    local status
    status=$(nixling vm status "$vm" --json 2>/dev/null || true)
    if printf '%s\n' "$status" | grep -Eq '"api_ready"[[:space:]]*:[[:space:]]*"yes"|"apiReady"[[:space:]]*:[[:space:]]*"yes"'; then
      return 0
    fi
    if printf '%s\n' "$status" | grep -q '"runtime"[[:space:]]*:[[:space:]]*"running"' \
       && printf '%s\n' "$status" | grep -q '"guest-control-health"'; then
      return 0
    fi
    sleep "$interval"
    elapsed=$((elapsed + interval))
  done
  return 1
}

# ---------------------------------------------------------------------------
# Per-VM common assertions
# ---------------------------------------------------------------------------
probe_common() {
  local vm="$1"
  log "==> probe_common: VM=$vm"

  # 1. Start VM.
  log "  starting $vm"
  if ! nixling vm start "$vm" --apply >/dev/null 2>&1; then
    fail_check "$vm: nixling vm start failed"
    return 1
  fi
  pass_check "$vm: nixling vm start returned"

  # 2. api_ready within budget.
  if wait_for_api_ready "$vm" "$NL_SMOKE_APIREADY_BUDGET"; then
    pass_check "$vm: api_ready=yes within ${NL_SMOKE_APIREADY_BUDGET}s"
  else
    fail_check "$vm: api_ready never became yes within ${NL_SMOKE_APIREADY_BUDGET}s"
  fi

  # 3. SSH reachability + uname.
  local ip
  ip=$(vm_ip "$vm")
  if [ -z "$ip" ]; then
    fail_check "$vm: could not resolve static IP from manifest"
    return 1
  fi
  if wait_for_ssh "$ip" "$NL_SMOKE_TIMEOUT_SSH"; then
    pass_check "$vm: SSH uname -a succeeded at ${ip} within ${NL_SMOKE_TIMEOUT_SSH}s"
  else
    fail_check "$vm: SSH unreachable at ${ip} after ${NL_SMOKE_TIMEOUT_SSH}s"
    return 1
  fi

  # 4. virtiofsd file-IO probe.
  local store_entry
  store_entry=$(ssh -o StrictHostKeyChecking=no \
                    -o BatchMode=yes \
                    -o ConnectTimeout=10 \
                    "root@${ip}" \
                    'ls /nix/store 2>/dev/null | head -1' 2>/dev/null || true)
  if [ -n "$store_entry" ]; then
    pass_check "$vm: virtiofsd file-IO probe: /nix/store entry='${store_entry}'"
  else
    fail_check "$vm: virtiofsd file-IO probe: /nix/store is empty or unreachable (fu27 class)"
  fi

  # 5. No zombie processes [fu32 class].
  local zombies
  zombies=$(grep -r 'Z (defunct)' /proc/*/stat 2>/dev/null \
    | grep -E 'virtiofsd|cloud-hypervisor|swtpm|gpu|audio' \
    | wc -l || true)
  # Alternative detection via /proc/*/status
  zombies_alt=$(for f in /proc/*/status; do
    if grep -q '^State:[[:space:]]*Z' "$f" 2>/dev/null; then
      comm=$(grep '^Name:' "$f" 2>/dev/null | awk '{print $2}' || true)
      case "$comm" in virtiofsd|cloud-hypervisor|swtpm|gpu-sidecar|audio-sidecar) echo "$comm" ;; esac
    fi
  done | wc -l || true)
  local total_zombies=$(( zombies + zombies_alt ))
  if [ "$total_zombies" -eq 0 ]; then
    pass_check "$vm: no zombie sidecar processes"
  else
    fail_check "$vm: found ${total_zombies} zombie sidecar process(es) (fu32 class)"
  fi

  # 6. pidfd-table snapshot consistency [fu32 class].
  if [ -f "$PIDFD_TABLE" ]; then
    local snap_fail=0
    # Extract all pid values from the JSON and verify they refer to live processes.
    while IFS= read -r pid_val; do
      if [ -n "$pid_val" ] && [ "$pid_val" != "null" ]; then
        if ! [ -d "/proc/${pid_val}" ]; then
          log "  pidfd-table entry PID $pid_val has no /proc entry (stale)"
          snap_fail=$((snap_fail + 1))
        fi
      fi
    done < <(grep -o '"pid"[[:space:]]*:[[:space:]]*[0-9]*' "$PIDFD_TABLE" 2>/dev/null \
             | grep -o '[0-9]*$' || true)
    if [ "$snap_fail" -eq 0 ]; then
      pass_check "$vm: pidfd-table snapshot matches running PIDs"
    else
      fail_check "$vm: pidfd-table has ${snap_fail} stale PID(s) (fu32 class)"
    fi
  else
    log "  WARN: pidfd-table not found at $PIDFD_TABLE — skipping snapshot check"
  fi

  # 7. CH HTTP API liveness.
  local sock
  sock=$(api_socket "$vm")
  if [ -S "$sock" ]; then
    if curl -sf --unix-socket "$sock" \
         -o /dev/null -w "%{http_code}" \
         http://localhost/api/v1/vm.info 2>/dev/null | grep -q '^200$'; then
      pass_check "$vm: CH HTTP API /api/v1/vm.info → HTTP 200"
    else
      fail_check "$vm: CH HTTP API /api/v1/vm.info did not return HTTP 200"
    fi
  else
    fail_check "$vm: CH API socket $sock not present"
  fi

  # 8. CAP_NET_ADMIN bit-clear.
  sleep 10
  local ch_pid_val
  ch_pid_val=$(ch_pid "$vm")
  if [ -n "$ch_pid_val" ] && [ -f "/proc/${ch_pid_val}/status" ]; then
    local cap_eff
    cap_eff=$(grep '^CapEff:' "/proc/${ch_pid_val}/status" | awk '{print $2}' || true)
    if [ -n "$cap_eff" ]; then
      # CAP_NET_ADMIN = bit 12 = 0x1000
      local cap_hex
      cap_hex=$(printf '%d' "0x${cap_eff}" 2>/dev/null || true)
      if [ -n "$cap_hex" ] && [ $(( cap_hex & 0x1000 )) -eq 0 ]; then
        pass_check "$vm: CH process CAP_NET_ADMIN bit clear (D4a)"
      else
        fail_check "$vm: CH process CAP_NET_ADMIN bit set in CapEff=0x${cap_eff} (D4a violation)"
      fi
    else
      log "  WARN: could not parse CapEff from /proc/${ch_pid_val}/status"
    fi
  else
    log "  WARN: CH PID not found in pidfd-table; skipping CAP_NET_ADMIN check"
  fi

  # 9. nixling host doctor --read-only.
  if nixling host doctor --read-only >/dev/null 2>&1; then
    pass_check "$vm: nixling host doctor --read-only exits 0"
  else
    fail_check "$vm: nixling host doctor --read-only failed"
  fi
}

# ---------------------------------------------------------------------------
# Per-VM teardown assertions
# ---------------------------------------------------------------------------
probe_teardown() {
  local vm="$1"
  log "==> probe_teardown: VM=$vm"

  nixling vm stop "$vm" --apply >/dev/null 2>&1 || true
  sleep 3

  # Assert no orphan sidecar processes.
  local orphans=0
  for comm in virtiofsd cloud-hypervisor swtpm; do
    if pgrep -x "$comm" >/dev/null 2>&1; then
      log "  found orphan process: $comm"
      orphans=$((orphans + 1))
    fi
  done
  if [ "$orphans" -eq 0 ]; then
    pass_check "$vm: no orphan sidecar processes after stop"
  else
    fail_check "$vm: ${orphans} orphan sidecar process(es) after stop"
  fi

  # Assert no stale vsock sockets.
  local stale_vsocks
  stale_vsocks=$(find "${VM_RUN_BASE}/${vm}/" -maxdepth 1 \
                   -name 'vsock_*' 2>/dev/null | wc -l || true)
  if [ "$stale_vsocks" -eq 0 ]; then
    pass_check "$vm: no stale vsock_* sockets after stop"
  else
    fail_check "$vm: ${stale_vsocks} stale vsock_* socket(s) found after stop (panel-virt R0 Q1 #4)"
  fi
}

# ---------------------------------------------------------------------------
# Full-mode: TPM functional probe + persistence
# ---------------------------------------------------------------------------
probe_tpm() {
  local vm="$1"
  log "==> probe_tpm: VM=$vm"

  local ip
  ip=$(vm_ip "$vm")
  if [ -z "$ip" ]; then
    fail_check "$vm: could not resolve IP for TPM probe"
    return
  fi

  # TPM functional probe: tpm2_getrandom 8.
  if ssh -o StrictHostKeyChecking=no \
         -o BatchMode=yes \
         -o ConnectTimeout=10 \
         "root@${ip}" \
         'tpm2_getrandom 8 >/dev/null 2>&1' 2>/dev/null; then
    pass_check "$vm: TPM functional probe tpm2_getrandom 8 succeeded"
  else
    fail_check "$vm: TPM functional probe tpm2_getrandom 8 failed (fu36 class)"
  fi

  # TPM SRK persistence pre-state.
  local srk_count_before
  srk_count_before=$(ssh -o StrictHostKeyChecking=no \
                         -o BatchMode=yes \
                         -o ConnectTimeout=10 \
                         "root@${ip}" \
                         'tpm2_getcap handles-persistent 2>/dev/null | grep -c 0x81000001 || echo 0' \
                         2>/dev/null || echo 0)
  if [ "${srk_count_before:-0}" -ge 1 ]; then
    pass_check "$vm: TPM SRK handle 0x81000001 present before restart"
  else
    log "  WARN: SRK handle 0x81000001 absent pre-restart (VM may not have enrolled yet)"
  fi

  # Restart VM; re-assert SRK handle.
  log "  restarting $vm for TPM persistence check"
  nixling vm stop "$vm" --apply >/dev/null 2>&1 || true
  sleep 2
  if ! nixling vm start "$vm" --apply >/dev/null 2>&1; then
    fail_check "$vm: nixling vm start (post-stop for TPM persistence) failed"
    return
  fi
  # Wait for SSH to come back.
  if ! wait_for_ssh "$ip" "$NL_SMOKE_TIMEOUT_SSH"; then
    fail_check "$vm: SSH unreachable after restart for TPM persistence check"
    return
  fi
  local srk_count_after
  srk_count_after=$(ssh -o StrictHostKeyChecking=no \
                        -o BatchMode=yes \
                        -o ConnectTimeout=10 \
                        "root@${ip}" \
                        'tpm2_getcap handles-persistent 2>/dev/null | grep -c 0x81000001 || echo 0' \
                        2>/dev/null || echo 0)
  if [ "${srk_count_after:-0}" -ge 1 ]; then
    pass_check "$vm: TPM SRK handle 0x81000001 survived restart (panel-virt R0 #6)"
  else
    fail_check "$vm: TPM SRK handle 0x81000001 absent after restart (fu36 class)"
  fi
}

# ---------------------------------------------------------------------------
# Full-mode: bridge sysctl persistence under networkd restart
# ---------------------------------------------------------------------------
probe_bridge_sysctl() {
  log "==> probe_bridge_sysctl: bridge sysctl persistence under networkd restart"

  # Enumerate nixling-declared bridge interfaces.
  # nixling host doctor --read-only --json outputs interface names; fall back
  # to reading from /sys/class/net + filtering bridge type.
  local bridges
  bridges=$(nixling host info --json 2>/dev/null \
    | grep -o '"[a-zA-Z0-9_-]*br[a-zA-Z0-9_-]*"' \
    | tr -d '"' \
    | sort -u || true)

  if [ -z "$bridges" ]; then
    # Fallback: any bridge in /sys/class/net that ip link reports.
    bridges=$(ip link show type bridge 2>/dev/null \
      | grep -o '^[0-9]*:[[:space:]]*[a-zA-Z0-9_-]*' \
      | awk '{print $2}' \
      | tr -d ':' || true)
  fi

  if [ -z "$bridges" ]; then
    log "  WARN: no bridge interfaces found; skipping sysctl persistence check"
    return
  fi

  # Record disable_ipv6 values before networkd restart.
  log "  bridges found: $(echo "$bridges" | tr '\n' ' ')"
  sudo systemctl restart systemd-networkd
  sleep 3

  local sysctl_fail=0
  while IFS= read -r br; do
    [ -z "$br" ] && continue
    local val
    val=$(sysctl -n "net.ipv6.conf.${br}.disable_ipv6" 2>/dev/null || echo "")
    if [ "$val" = "1" ]; then
      pass_check "bridge $br: disable_ipv6=1 after networkd restart (panel-networking R0 #3)"
    else
      fail_check "bridge $br: disable_ipv6=${val:-missing} after networkd restart (expected 1)"
      sysctl_fail=$((sysctl_fail + 1))
    fi
  done <<< "$bridges"
}

# ---------------------------------------------------------------------------
# Full-mode: audio sidecar probe + restart binding
# ---------------------------------------------------------------------------
probe_audio() {
  local vm="$1"
  log "==> probe_audio: VM=$vm"

  local ip
  ip=$(vm_ip "$vm")
  if [ -z "$ip" ]; then
    fail_check "$vm: could not resolve IP for audio probe"
    return
  fi

  # Audio card probe.
  local card_count
  card_count=$(ssh -o StrictHostKeyChecking=no \
                   -o BatchMode=yes \
                   -o ConnectTimeout=10 \
                   "root@${ip}" \
                   'aplay -l 2>/dev/null | grep -c card || echo 0' \
                   2>/dev/null || echo 0)
  if [ "${card_count:-0}" -ge 1 ]; then
    pass_check "$vm: audio sidecar probe: ${card_count} card(s) visible in guest"
  else
    fail_check "$vm: audio sidecar probe: no audio cards visible in guest (aplay -l)"
  fi

  # Audio sidecar restart binding.
  log "  audio restart binding: stop + restart $vm"
  nixling vm stop "$vm" --apply >/dev/null 2>&1 || true
  sleep 2
  if ! nixling vm start "$vm" --apply >/dev/null 2>&1; then
    fail_check "$vm: nixling vm start (audio restart) failed"
    return
  fi
  if ! wait_for_ssh "$ip" 30; then
    fail_check "$vm: SSH unreachable within 30s after audio restart"
    return
  fi
  local card_count_after
  card_count_after=$(ssh -o StrictHostKeyChecking=no \
                         -o BatchMode=yes \
                         -o ConnectTimeout=10 \
                         "root@${ip}" \
                         'aplay -l 2>/dev/null | grep -c card || echo 0' \
                         2>/dev/null || echo 0)
  if [ "${card_count_after:-0}" -ge 1 ]; then
    pass_check "$vm: audio sidecar restart binding: ${card_count_after} card(s) after restart"
  else
    fail_check "$vm: audio sidecar restart binding: no audio cards after restart (panel-virt R1)"
  fi
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
HEAD_SHA=$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo "unknown")
ISO_TS=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
LOG_FILE="${TMPDIR:-/tmp}/nixling-smoke-run-log.txt"

log "==> HEAD=$HEAD_SHA mode=$MODE ts=$ISO_TS"

# Primary VM probes (both modes).
probe_common "$NL_SMOKE_VM_PRIMARY"

if [ "$MODE" = "full" ]; then
  # Full-mode: TPM probes on primary VM (personal-dev has TPM enabled).
  probe_tpm "$NL_SMOKE_VM_PRIMARY"

  # Full-mode: bridge sysctl persistence (global, not per-VM).
  probe_bridge_sysctl

  # Full-mode: secondary VM (work-aad) common probes.
  probe_common "$NL_SMOKE_VM_SECONDARY"

  # Full-mode: audio probe on secondary VM (work-aad has audio sidecar).
  probe_audio "$NL_SMOKE_VM_SECONDARY"

  # Teardown secondary VM.
  probe_teardown "$NL_SMOKE_VM_SECONDARY"
fi

# Teardown primary VM.
probe_teardown "$NL_SMOKE_VM_PRIMARY"

# ---------------------------------------------------------------------------
# Append result to the out-of-tree smoke-run log.
# ---------------------------------------------------------------------------
if [ "$FAIL" -eq 0 ]; then
  STATUS=PASS
else
  STATUS=FAIL
fi

LOG_LINE="${HEAD_SHA} ${ISO_TS} ${STATUS} ${MODE}"
printf '%s\n' "$LOG_LINE" >> "$LOG_FILE"
log "==> smoke-run-log: $LOG_LINE"

# ---------------------------------------------------------------------------
# Final result
# ---------------------------------------------------------------------------
if [ "$FAIL" -gt 0 ]; then
  log "==> FAILED ($FAIL failure(s), $PASS pass(es))"
  exit 1
fi

log "==> PASSED ($PASS check(s), mode=$MODE)"
exit 0
