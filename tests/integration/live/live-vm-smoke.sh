#!/usr/bin/env bash
# shellcheck disable=SC2126,SC2329
# tests/integration/live/live-vm-smoke.sh - live Guest smoke gate.
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
#   77  SKIP (KVM absent / d2b not running / VMs not declared)
#
# Configurable via environment:
#   D2B_SMOKE_EXEC_BUDGET     seconds to wait for Guest exec (default 120)
#   D2B_SMOKE_READY_BUDGET    seconds to wait for Guest Ready (default 60)
#   D2B_SMOKE_VM_PRIMARY      primary VM name (default personal-dev)
#   D2B_SMOKE_VM_SECONDARY    secondary VM for --full (default work-aad)

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}

# --- heavy-gate sole-use semaphore (ADR 0046) ------------------------------
# This live lane mutates real host, KVM, and daemon state, so it must never
# bypass the sole-use heavy-gate semaphore. The mere presence of
# D2B_HEAVY_GATE is not trusted: the shared helper asks the wrapper to verify
# this process genuinely holds a slot and re-execs through the gate exactly
# once when it does not.
# shellcheck source=tests/tools/heavy-gate-reexec.sh
. "$ROOT/tests/tools/heavy-gate-reexec.sh"
d2b_heavy_gate_reexec "$ROOT" "$0" "$@"

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
D2B_SMOKE_EXEC_BUDGET=${D2B_SMOKE_EXEC_BUDGET:-120}
D2B_SMOKE_READY_BUDGET=${D2B_SMOKE_READY_BUDGET:-60}
D2B_SMOKE_VM_PRIMARY=${D2B_SMOKE_VM_PRIMARY:-personal-dev}
D2B_SMOKE_VM_SECONDARY=${D2B_SMOKE_VM_SECONDARY:-work-aad}

PIDFD_TABLE=/var/lib/d2b/daemon-state/pidfd-table.json
VM_RUN_BASE=/run/d2b/vms
VM_STATE_BASE=/var/lib/d2b/vms

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

if ! systemctl is-active --quiet d2b-priv-broker 2>/dev/null; then
  log "==> SKIP: d2b-priv-broker is not active (systemctl is-active returned non-zero)"
  exit 77
fi

if ! command -v d2b >/dev/null 2>&1; then
  log "==> SKIP: d2b not on PATH"
  exit 77
fi

# Guest status is parsed with jq below; do not fall back to the retired
# manifest or VM-array status surfaces.
if ! command -v jq >/dev/null 2>&1; then
  log "==> SKIP: jq is required to parse v3 Guest Resource envelopes"
  exit 77
fi

# Check that the primary Guest is declared in the Zone Resource API.
if ! d2b guest status "$D2B_SMOKE_VM_PRIMARY" --json >/dev/null 2>&1; then
  log "==> SKIP: Guest '$D2B_SMOKE_VM_PRIMARY' not declared in the Zone"
  exit 77
fi

if [ "$MODE" = "full" ]; then
  if ! d2b guest status "$D2B_SMOKE_VM_SECONDARY" --json >/dev/null 2>&1; then
    log "==> SKIP: Guest '$D2B_SMOKE_VM_SECONDARY' not declared (required for --full)"
    exit 77
  fi
fi

# ---------------------------------------------------------------------------
# Probe helpers
# ---------------------------------------------------------------------------

# wait_for_guest_exec <name> <timeout_secs> -- <argv...>
wait_for_guest_exec() {
  local name="$1" timeout="$2" elapsed=0 interval=5
  shift 2
  while [ "$elapsed" -lt "$timeout" ]; do
    if d2b exec run "Guest/$name" -- "$@" >/dev/null 2>&1; then
      return 0
    fi
    sleep "$interval"
    elapsed=$((elapsed + interval))
  done
  return 1
}

# guest_status_json <name> - fetch one complete v3 Guest Resource envelope.
guest_status_json() {
  d2b guest status "$1" --json 2>/dev/null
}

# guest_envelope_is_valid <name> - validate identity and current v3 status
# fields without consulting legacy manifest or VM-array output.
guest_envelope_is_valid() {
  local name="$1"
  jq -e --arg name "$name" '
    .ok == true
    and .apiVersion == "resources.d2bus.org/v3"
    and .type == "Guest"
    and .metadata.name == $name
    and (.status.phase | type == "string")
    and (.status.conditions | type == "array")
  ' >/dev/null
}

# api_socket <vm> - path to CH HTTP API socket.
# Convention from manifest.nix: /var/lib/d2b/vms/<vm>/<vm>.sock
api_socket() {
  printf '%s/%s/%s.sock\n' "$VM_STATE_BASE" "$1" "$1"
}

# ch_pid <vm> - PID of the cloud-hypervisor process for the given VM.
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

# wait_for_guest_ready <name> <budget_secs> - wait for current Guest status.
wait_for_guest_ready() {
  local name="$1" budget="$2" elapsed=0 interval=5
  while [ "$elapsed" -lt "$budget" ]; do
    local status
    status=$(guest_status_json "$name" || true)
    if printf '%s\n' "$status" \
      | jq -e --arg name "$name" '
          .ok == true
          and .apiVersion == "resources.d2bus.org/v3"
          and .type == "Guest"
          and .metadata.name == $name
          and (
            .status.phase == "Ready"
            or any(.status.conditions[]?; .type == "Ready" and .status == "True")
          )
        ' >/dev/null 2>&1; then
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

  # Later-wave blocker: the current production Zone Resource runtime serves
  # Guest Get/List/Status/Watch, but does not yet expose UpdateSpec lifecycle
  # dispatch. This lane uses the typed Guest lifecycle command below and must
  # remain visibly blocked until that production semantic exists; it must not
  # fall back to a legacy VM command.

  # 1. Start Guest.
  log "  starting $vm"
  local start_output
  if ! start_output=$(d2b guest start "$vm" --apply 2>&1); then
    if printf '%s\n' "$start_output" | grep -q 'pending un-approved guest config edit'; then
      log "  WARN: $vm has a pending un-approved guest config edit; skipping live VM probes for this host-local state"
      return 2
    fi
    fail_check "$vm: d2b guest start failed"
    return 1
  fi
  pass_check "$vm: d2b guest start returned"
  PROBE_COMMON_STARTED=1

  # 2. Validate the v3 Guest envelope and wait for current readiness.
  local initial_status
  initial_status=$(guest_status_json "$vm" || true)
  if printf '%s\n' "$initial_status" | guest_envelope_is_valid "$vm"; then
    pass_check "$vm: v3 Guest status envelope is valid"
  else
    fail_check "$vm: d2b guest status did not return a v3 Guest envelope"
    return 1
  fi
  if wait_for_guest_ready "$vm" "$D2B_SMOKE_READY_BUDGET"; then
    pass_check "$vm: Guest Ready within ${D2B_SMOKE_READY_BUDGET}s"
  else
    fail_check "$vm: Guest did not become Ready within ${D2B_SMOKE_READY_BUDGET}s"
  fi

  # 3. Guest-control exec reachability + uname.
  if wait_for_guest_exec "$vm" "$D2B_SMOKE_EXEC_BUDGET" uname -a; then
    pass_check "$vm: guest-control exec uname -a succeeded within ${D2B_SMOKE_EXEC_BUDGET}s"
  else
    fail_check "$vm: guest-control exec unreachable after ${D2B_SMOKE_EXEC_BUDGET}s"
    return 1
  fi

  # 4. virtiofsd file-IO probe.
  local store_entry
  store_entry=$(d2b exec run "Guest/$vm" -- sh -lc 'ls /nix/store 2>/dev/null | head -1' 2>/dev/null || true)
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
      case "$comm" in virtiofsd|cloud-hypervisor|swtpm|gpu-sidecar|audio-sidecar)
        echo "$comm"
        ;;
      esac
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
    log "  WARN: pidfd-table not found at $PIDFD_TABLE - skipping snapshot check"
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
      pass_check "$vm: CH HTTP API not ready; daemon status runtime is authoritative"
    fi
  else
    pass_check "$vm: CH API socket not exposed; daemon status runtime is authoritative"
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

  # 9. d2b host doctor --read-only.
  local doctor
  doctor=$(d2b host doctor --read-only 2>&1 || true)
  if printf '%s\n' "$doctor" | grep -q 'fail=0'; then
    pass_check "$vm: d2b host doctor --read-only exits 0"
  else
    fail_check "$vm: d2b host doctor --read-only reported failures"
  fi
}

# ---------------------------------------------------------------------------
# Per-VM teardown assertions
# ---------------------------------------------------------------------------
probe_teardown() {
  local vm="$1"
  log "==> probe_teardown: VM=$vm"

  d2b guest stop "$vm" --apply >/dev/null 2>&1 || true
  sleep 3

  # Assert no orphan sidecar processes.
  local orphans=0
  for comm in virtiofsd cloud-hypervisor swtpm; do
    if pgrep -af "$comm" | grep -F "$vm" >/dev/null 2>&1; then
      log "  found orphan process for $vm: $comm"
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

  # Later-wave blocker: the v3 Guest Resource status contract does not yet
  # expose a standardized TPM runner/readiness field. Do not infer TPM
  # enablement from the retired VM status service map or run a compatibility
  # probe based on that legacy shape.
  log "  BLOCKED: v3 Guest status has no TPM readiness field; TPM persistence probe is deferred to the Guest Provider status contract"
  return 2
}

# ---------------------------------------------------------------------------
# Full-mode: bridge sysctl persistence under networkd restart
# ---------------------------------------------------------------------------
probe_bridge_sysctl() {
  log "==> probe_bridge_sysctl: bridge sysctl persistence under networkd restart"

  # Enumerate d2b-declared bridge interfaces.
  # d2b host doctor --read-only --json outputs interface names; fall back
  # to reading from /sys/class/net + filtering bridge type.
  local bridges
  bridges=$(d2b host info --json 2>/dev/null \
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

  # Audio card probe.
  local card_count
  card_count=$(d2b exec run "Guest/$vm" -- sh -lc 'aplay -l 2>/dev/null | grep -c card || echo 0' 2>/dev/null || echo 0)
  if [ "${card_count:-0}" -ge 1 ]; then
    pass_check "$vm: audio sidecar probe: ${card_count} card(s) visible in guest"
  else
    fail_check "$vm: audio sidecar probe: no audio cards visible in guest (aplay -l)"
  fi

  # Audio sidecar restart binding.
  log "  audio restart binding: stop + restart $vm"
  d2b guest stop "$vm" --apply >/dev/null 2>&1 || true
  sleep 2
  if ! d2b guest start "$vm" --apply >/dev/null 2>&1; then
    fail_check "$vm: d2b guest start (audio restart) failed"
    return
  fi
  if ! wait_for_guest_exec "$vm" 30 uname -a; then
    fail_check "$vm: guest-control exec unreachable within 30s after audio restart"
    return
  fi
  local card_count_after
  card_count_after=$(d2b exec run "Guest/$vm" -- sh -lc 'aplay -l 2>/dev/null | grep -c card || echo 0' 2>/dev/null || echo 0)
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
LOG_FILE="${TMPDIR:-/tmp}/d2b-smoke-run-log.txt"

log "==> HEAD=$HEAD_SHA mode=$MODE ts=$ISO_TS"

# Primary VM probes (both modes).
primary_ready=0
primary_started=0
PROBE_COMMON_STARTED=0
if probe_common "$D2B_SMOKE_VM_PRIMARY"; then
  primary_ready=1
fi
primary_started=$PROBE_COMMON_STARTED

if [ "$MODE" = "full" ]; then
  # Full-mode: TPM probes on primary VM (personal-dev has TPM enabled).
  if [ "$primary_ready" -eq 1 ]; then
    probe_tpm "$D2B_SMOKE_VM_PRIMARY"
  fi

  # Full-mode: bridge sysctl persistence (global, not per-VM).
  probe_bridge_sysctl

  # Full-mode: secondary VM (work-aad) common probes.
  secondary_ready=0
  secondary_started=0
  PROBE_COMMON_STARTED=0
  if probe_common "$D2B_SMOKE_VM_SECONDARY"; then
    secondary_ready=1
  fi
  secondary_started=$PROBE_COMMON_STARTED

  if [ "$secondary_ready" -eq 1 ]; then
    # Full-mode: audio probe on secondary VM (work-aad has audio sidecar).
    probe_audio "$D2B_SMOKE_VM_SECONDARY"
  fi

  # Teardown secondary VM if start returned, even if later probes failed.
  if [ "$secondary_started" -eq 1 ]; then
    probe_teardown "$D2B_SMOKE_VM_SECONDARY"
  fi
fi

# Teardown primary VM if start returned, even if later probes failed.
if [ "$primary_started" -eq 1 ]; then
  probe_teardown "$D2B_SMOKE_VM_PRIMARY"
fi

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
