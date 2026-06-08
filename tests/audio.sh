#!/usr/bin/env bash
# Layer-2 audio tests for nixling.
#
# Each `test_*` is one function. Idempotent. Safe to re-run on the
# live host. Each test cleans up after itself.
#
# Usage:
#   modules/nixling/tests/audio.sh                 # full run
#   modules/nixling/tests/audio.sh --quick         # smoke subset
#   modules/nixling/tests/audio.sh --only test_X   # one test
#   modules/nixling/tests/audio.sh --list
#
# Tests that need a running audio-enabled VM auto-SKIP when none is up.
#
# Why this exists:
#   - The CH v50 -> v52 bump broke host PipeWire / WirePlumber once
#     during this session (the rebuild's user-units reload dropped
#     ALSA card visibility). A pipewire/wireplumber restart recovered.
#     We want regression coverage so the next ambient breakage gets
#     caught by `nixling-test-audio`, not by the user noticing silence.
#
# Layers tested:
#   1. Host audio surface (pipewire/wireplumber)
#       - The host has at least one Audio Device, at least one Sink,
#         at least one Source.
#   2. Audio sidecar lifecycle
#       - `systemctl start nixling-<vm>-snd.service` (system service) creates
#         the listening UDS under /run/user/<uid>/ with group=kvm
#         mode=0660; stop removes it.
#       - The .service auto-activates via socket activation when
#         something connects.
#   3. `nixling audio` CLI smoke
#       - `nixling audio status <vm>` reports a clean baseline
#       - `nixling audio mic on <vm>` -> state file updated, sidecar
#         socket created
#       - `nixling audio speaker off <vm>` -> state file updated
#       - `nixling audio off <vm>` -> state file cleared, socket gone
#   4. Capability matrix
#       - cloud-hypervisor has `--generic-vhost-user` (required for
#         audio attach) AND `--gpu` (required for graphics VMs)
#   5. End-to-end (skip-if-no-running-VM)
#       - For an audio-enabled, currently-running VM with mic OR
#         speaker = on: the guest's ALSA stack reports a virtio-snd
#         sound card (`aplay -l` shows `card 0: VIRT [VIRT virtio]`).

set -uo pipefail

# Scope a safe.directory entry for $ROOT to libgit2 (used by
# `nix eval` below). Same pattern as static.sh/security-baseline.sh.
HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

_AUDIO_GITCFG=$(nl_mktemp .audio-gitcfg.XXXXXX)
install -d -m 0700 "$_AUDIO_GITCFG/git"
printf "[safe]\n\tdirectory = %s\n" "$ROOT" > "$_AUDIO_GITCFG/git/config"
export XDG_CONFIG_HOME="$_AUDIO_GITCFG"
export GIT_CONFIG_COUNT=1
export GIT_CONFIG_KEY_0=safe.directory
export GIT_CONFIG_VALUE_0="$ROOT"

NL_HOST_CONFIG=${NL_HOST_CONFIG:-desktop}

STATE_ROOT=/var/lib/nixling/vms

# Resolve the Wayland session user. Audio tests need a real user-systemd
# manager + PipeWire/WirePlumber session, so we re-exec as that user
# when invoked by root. Resolution order:
#   1. $NL_WAYLAND_USER if explicitly set.
#   2. nix eval of nixling.site.waylandUser on the live host config.
#   3. $SUDO_USER (if running under sudo).
#   4. The invoking non-root user.
NL_WAYLAND_USER=${NL_WAYLAND_USER:-}
if [ -z "$NL_WAYLAND_USER" ]; then
  NL_WAYLAND_USER=$(nix eval --raw \
    "$ROOT#nixosConfigurations.$NL_HOST_CONFIG.config.nixling.site.waylandUser" \
    2>/dev/null || true)
fi
if [ -z "$NL_WAYLAND_USER" ] && [ -n "${SUDO_USER:-}" ]; then
  NL_WAYLAND_USER=$SUDO_USER
fi
if [ -z "$NL_WAYLAND_USER" ] && [ "$(id -u)" != "0" ]; then
  NL_WAYLAND_USER=$(id -un)
fi

# Pick test VMs that have audio.enable=true. The CLI manifest is the
# ground truth.
audio_vms() {
  jq -r '. as $m | to_entries[] | select(.value.audio == true) | .key' \
    /run/current-system/sw/share/nixling/vms.json 2>/dev/null
}
TEST_VMS=$(audio_vms || true)
if [ -z "$TEST_VMS" ]; then
  log "no audio-enabled VMs declared — most tests will be limited to host-side surface."
fi

skip() { log "  SKIP: $*"; }

# Pre-flight: audio tests need a real user session (user-systemd,
# XDG_RUNTIME_DIR, PipeWire/WirePlumber as user services). If the
# runner was invoked as root (e.g. `sudo runner.sh --quick`), step
# DOWN to the resolved Wayland user (`$NL_WAYLAND_USER`) automatically
# so this script stays green in the aggregate suite. If no Wayland
# user can be resolved, or it has no active login session, we SKIP
# cleanly (exit 0) rather than FATAL — that way the suite still
# reports clean on hosts where audio isn't expected to work (e.g. CI
# without a desktop session) instead of breaking the whole runner.
if [ "$(id -u)" = "0" ]; then
  if [ -z "$NL_WAYLAND_USER" ]; then
    printf '%s SKIP: audio tests require NL_WAYLAND_USER (or nixling.site.waylandUser) to be set\n' \
      "$(date +%H:%M:%S)" >&2
    exit 0
  fi
  if ! getent passwd "$NL_WAYLAND_USER" >/dev/null 2>&1; then
    printf '%s SKIP: audio tests require user %s (not present on this host)\n' \
      "$(date +%H:%M:%S)" "$NL_WAYLAND_USER" >&2
    exit 0
  fi
  _wu_uid=$(id -u "$NL_WAYLAND_USER")
  _wu_home=$(getent passwd "$NL_WAYLAND_USER" | cut -d: -f6)
  _wu_runtime="/run/user/$_wu_uid"
  if [ ! -d "$_wu_runtime" ] || [ ! -S "$_wu_runtime/bus" ]; then
    printf '%s SKIP: audio tests require an active %s session (no %s or its DBus bus)\n' \
      "$(date +%H:%M:%S)" "$NL_WAYLAND_USER" "$_wu_runtime" >&2
    exit 0
  fi
  # lib.sh logs via `tee -a "$NL_LOG"`. The runner.sh sets NL_LOG to a
  # root-owned 0700 file under $RUN_DIR, so after we drop privs `tee`
  # would spam permission-denied on every line. Redirect NL_LOG to a
  # user-writable temp file BEFORE re-exec. The runner's per-script
  # log (`$RUN_DIR/audio.log`) is still captured via the runner's
  # FD-level stdout/stderr redirect, which the dropped-priv child
  # inherits across the privilege drop, so nothing is lost.
  reexec_log=$(mktemp -t nixling-audio-reexec.XXXXXX.log)
  chown "$NL_WAYLAND_USER:users" "$reexec_log" 2>/dev/null || true
  chmod 0644 "$reexec_log" 2>/dev/null || true
  printf '%s audio.sh: stepping down root -> %s (NL_LOG=%s, XDG_RUNTIME_DIR=%s)\n' \
    "$(date +%H:%M:%S)" "$NL_WAYLAND_USER" "$reexec_log" "$_wu_runtime" >&2
  exec runuser -u "$NL_WAYLAND_USER" -- env \
    HOME="$_wu_home" \
    USER="$NL_WAYLAND_USER" \
    LOGNAME="$NL_WAYLAND_USER" \
    XDG_RUNTIME_DIR="$_wu_runtime" \
    DBUS_SESSION_BUS_ADDRESS="unix:path=$_wu_runtime/bus" \
    PATH="${PATH:-/run/current-system/sw/bin:/usr/bin}" \
    TERM="${TERM:-dumb}" \
    NL_LOG="$reexec_log" \
    NL_WAYLAND_USER="$NL_WAYLAND_USER" \
    NL_HOST_CONFIG="$NL_HOST_CONFIG" \
    ROOT="$ROOT" \
    bash "$0" "$@"
  # exec failed — keep the old FATAL so the regression is visible
  printf '%s FATAL: runuser -u %s failed; audio tests cannot run as root\n' \
    "$(date +%H:%M:%S)" "$NL_WAYLAND_USER" >&2
  exit 2
fi
if [ -z "${XDG_RUNTIME_DIR:-}" ]; then
  log "FATAL: XDG_RUNTIME_DIR is unset — run from a Plasma terminal, not a bare SSH."
  exit 2
fi

# Shared scratch VM name we use for sidecar lifecycle tests. Doesn't
# need to be a real VM (the .socket template instantiates per any name).
SCRATCH_VM=__nixling_test_audio__

# ---------------------------------------------------------------------
# Cleanups: at exit, stop the scratch sidecar and reset every
# audio-enabled VM's state file to what we found it as.
# ---------------------------------------------------------------------
declare -A NL_AUDIO_BASELINE=()
for vm in $TEST_VMS; do
  f="$STATE_ROOT/$vm/state/audio-state.json"
  if [ -r "$f" ]; then
    NL_AUDIO_BASELINE[$vm]=$(cat "$f")
  fi
done

cleanup_audio() {
  systemctl stop "nixling-${SCRATCH_VM}-snd.service" 2>/dev/null || true
  systemctl reset-failed "nixling-${SCRATCH_VM}-snd.service" 2>/dev/null || true
  for vm in "${!NL_AUDIO_BASELINE[@]}"; do
    local f="$STATE_ROOT/$vm/state/audio-state.json"
    local baseline="${NL_AUDIO_BASELINE[$vm]}"
    if [ "$(cat "$f" 2>/dev/null)" != "$baseline" ]; then
      log "cleanup: restoring $vm audio state to baseline $baseline"
      tmp=$(mktemp)
      printf '%s\n' "$baseline" > "$tmp"
      sudo -A install -m 0640 -o root -g nixling -- "$tmp" "$f"
      rm -f "$tmp"
      # IMPORTANT: do NOT stop the sidecar of a real VM in cleanup.
      # vhost-user is a one-shot connection: CH binds the socket once
      # at VM start and never reconnects. If we stop the sidecar
      # under a running VM, CH ends up with a dead AF_UNIX peer and
      # any future audio operation in the guest blocks forever
      # (writev() against the closed FD spins inside the kernel; the
      # guest userspace app, e.g. Firefox attempting WebAudio init,
      # hangs uninterruptibly).
      #
      # The state-file change we just restored is enough: the
      # sidecar is one-process-per-VM and its lifecycle is owned by
      # `nixling up` / `nixling down`, not by transient state-file
      # tweaks. If the VM happens to be down, the sidecar is already
      # gone; if it's up, leave it alone.
    fi
  done
}
add_cleanup cleanup_audio

# ---------------------------------------------------------------------
# Pre/post host-audio snapshots.
#
# Catches regressions where running the suite *itself* breaks host
# audio. This has happened twice: the CLI used to SIGHUP WirePlumber
# (which exits on Hangup), and a misplaced WirePlumber stream-rule
# disconnected all ALSA cards. Both showed up as "real_sinks=4 before,
# real_sinks=0/1 after" — and neither was caught because the tests
# only sampled host audio ONCE, near the start, when everything still
# looked fine. The snapshot below forces an end-of-suite re-check.
# ---------------------------------------------------------------------

host_audio_snapshot() {
  # Output: "<n_devices> <n_real_sinks> <n_sources>" on one line.
  local devs sinks sources
  devs=$(wpctl status 2>/dev/null \
    | sed -n '/^Audio$/,/^Video$/p' \
    | sed -n '/├─ Devices:/,/├─ Sinks:/p' \
    | grep -cE '\[alsa\]|\[v4l2\]|\[bluez5\]')
  sinks=$(wpctl status 2>/dev/null \
    | sed -n '/^Audio$/,/^Video$/p' \
    | sed -n '/├─ Sinks:/,/├─ Sources:/p' \
    | grep -E '^ │ ' \
    | grep -vE 'Dummy Output|Dummy Source' \
    | wc -l)
  sources=$(wpctl status 2>/dev/null \
    | sed -n '/^Audio$/,/^Video$/p' \
    | sed -n '/├─ Sources:/,/├─ Filters:/p' \
    | grep -E '^ │ ' \
    | wc -l)
  printf '%s %s %s' "$devs" "$sinks" "$sources"
}

NL_AUDIO_PRE_SNAPSHOT=$(host_audio_snapshot)
log "host audio snapshot at start: $NL_AUDIO_PRE_SNAPSHOT (devices sinks sources)"

# =====================================================================
# Layer 1: host audio surface
# =====================================================================

# Confirm the host PipeWire daemon answers + WirePlumber sees real
# hardware. Catches the failure mode we hit during the CH bump: the
# Plasma audio applet showed "no devices" because the wpctl session
# had lost ALSA card visibility.
test_host_pipewire_alive() {
  log "test_host_pipewire_alive"
  if ! systemctl --user is-active --quiet pipewire.service; then
    fail "host pipewire.service is not active"
    return
  fi
  ok "pipewire.service active"
  if ! systemctl --user is-active --quiet wireplumber.service; then
    fail "host wireplumber.service is not active"
    return
  fi
  ok "wireplumber.service active"

  local status
  status=$(wpctl status 2>&1)
  if [ -z "$status" ] || ! printf '%s' "$status" | grep -q '^PipeWire '; then
    fail "wpctl status produced no header — pipewire connection broken"
    return
  fi
  ok "wpctl status reachable"
}

test_host_has_audio_devices() {
  log "test_host_has_audio_devices"
  local devs
  devs=$(wpctl status 2>/dev/null \
    | awk '/^Audio$/{f=1;next} /^Video$/{f=0} f && /^ ├─ Devices:/{g=1;next} g && /^ ├─/{g=0} g && /^ │ /' \
    | grep -E '\[alsa\]|\[v4l2\]|\[bluez5\]')
  local n
  n=$(printf '%s\n' "$devs" | grep -c -E '\[alsa\]|\[v4l2\]|\[bluez5\]')
  log "  host has $n audio device(s):"
  printf '%s\n' "$devs" | sed 's/^/    /' | tee -a "$NL_LOG" >&2
  assert_ge "$n" 1 "host has >= 1 audio device"
}

test_host_has_audio_sinks_and_sources() {
  log "test_host_has_audio_sinks_and_sources"
  # Count any line under Audio › Sinks: (the "Dummy Output" pseudo-sink
  # IS a real PipeWire sink, but if it's the ONLY one then ALSA cards
  # were dropped — this is exactly the failure mode we want to catch).
  local sinks sources real_sinks
  sinks=$(wpctl status 2>/dev/null | awk '/^ ├─ Sinks:/{f=1;next} /^ ├─ Sources:/{f=0} f' \
    | grep -E '\.[0-9]+:' | wc -l 2>/dev/null || true)
  # extract just the sink count from a slightly different filter:
  sinks=$(wpctl status 2>/dev/null \
    | sed -n '/^Audio$/,/^Video$/p' \
    | sed -n '/├─ Sinks:/,/├─ Sources:/p' \
    | grep -E '^ │ ' | wc -l)
  sources=$(wpctl status 2>/dev/null \
    | sed -n '/^Audio$/,/^Video$/p' \
    | sed -n '/├─ Sources:/,/├─ Filters:/p' \
    | grep -E '^ │ ' | wc -l)
  real_sinks=$(wpctl status 2>/dev/null \
    | sed -n '/^Audio$/,/^Video$/p' \
    | sed -n '/├─ Sinks:/,/├─ Sources:/p' \
    | grep -E '^ │ ' \
    | grep -vE 'Dummy Output|Dummy Source' \
    | wc -l)
  log "  sinks=$sinks sources=$sources real_sinks=$real_sinks"
  assert_ge "$sinks"   1 "host has >= 1 sink"
  assert_ge "$sources" 1 "host has >= 1 source"
  assert_ge "$real_sinks" 1 "host has >= 1 REAL sink (not just Dummy Output)"
}

# =====================================================================
# Layer 2: sidecar lifecycle
# =====================================================================

test_sidecar_unit_present() {
  log "test_sidecar_unit_present"
  # nixling-<vm>-snd.service is now a per-VM system service (not
  # a template, not user). Look for at least one such unit.
  if ! systemctl list-unit-files 'nixling-*-snd.service' --no-pager \
       --no-legend 2>/dev/null | grep -q 'nixling-.*-snd.service'; then
    fail "no nixling-<vm>-snd.service unit registered"
  else
    ok "nixling-<vm>-snd.service unit(s) registered"
  fi
}

test_sidecar_socket_lifecycle() {
  log "test_sidecar_socket_lifecycle"
  # nixling-<vm>-snd.service is a per-VM system service with
  # User=nixling-<vm>-snd.  Synthetic VM names (SCRATCH_VM) have no
  # declared system user so systemd refuses to start them ("Access denied").
  # Use the first stopped audio-enabled manifest VM instead so the user
  # exists and the sidecar starts cleanly without disrupting a running VM.
  local test_vm=""
  for _av in $TEST_VMS; do
    if ! vm_running "$_av" 2>/dev/null; then
      test_vm="$_av"
      break
    fi
  done
  if [ -z "$test_vm" ]; then
    log "  SKIP: no stopped audio-enabled VM available for sidecar lifecycle test"
    return 0
  fi

  local sock
  sock="/run/nixling/vms/${test_vm}/snd.sock"
  local svc="nixling-${test_vm}-snd.service"
  systemctl stop "$svc" 2>/dev/null || true
  systemctl reset-failed "$svc" 2>/dev/null || true

  if ! systemctl start "$svc"; then
    fail "could not start $svc"
    return 1
  fi
  ok "systemctl start $svc succeeded"

  # Wait for the service to be fully active and the socket to appear.
  # The sidecar dir is owned by nixling-<vm>-snd and not traversable by
  # the Wayland user, so use sudo -A for socket existence and ownership checks.
  for _ in 1 2 3 4 5; do
    sudo -A test -S "$sock" 2>/dev/null && break
    sleep 0.5
  done
  if ! sudo -A test -S "$sock" 2>/dev/null; then
    fail "socket file $sock did not appear after start"
    systemctl stop "$svc" 2>/dev/null || true
    return 1
  fi
  ok "socket created at $sock"

  local owner
  owner=$(sudo -A stat -c '%U' "$sock" 2>/dev/null)
  assert_eq "$owner" "nixling-${test_vm}-snd" "socket owner nixling-${test_vm}-snd"

  systemctl stop "$svc"
  sleep 0.5
  if systemctl is-active --quiet "$svc"; then
    fail "sidecar still active after stop"
  else
    ok "sidecar inactive after stop"
  fi
}

# =====================================================================
# Layer 3: CLI smoke
# =====================================================================

test_cli_status_smoke() {
  log "test_cli_status_smoke"
  if [ -z "$TEST_VMS" ]; then
    skip "no audio-enabled VMs"
    return
  fi
  local vm
  vm=$(printf '%s\n' "$TEST_VMS" | head -1)
  local out
  out=$(nixling audio status "$vm" 2>&1)
  log "  nixling audio status $vm ->"
  printf '%s\n' "$out" | sed 's/^/    /' | tee -a "$NL_LOG" >&2
  assert_contains "$out" "audio:    enabled" "status reports audio enabled"
  assert_contains "$out" "mic:"             "status has mic line"
  assert_contains "$out" "speaker:"         "status has speaker line"
}

test_cli_grant_revoke() {
  log "test_cli_grant_revoke"
  if [ -z "$TEST_VMS" ]; then
    skip "no audio-enabled VMs"
    return
  fi
  local vm rc=0
  vm=$(printf '%s\n' "$TEST_VMS" | head -1)
  local f="$STATE_ROOT/$vm/state/audio-state.json"
  local sock
  sock="/run/nixling/vms/${vm}/snd.sock"
  local svc="nixling-${vm}-snd.service"

  # baseline reset (also tears down any stale sidecar). reset-failed
  # so the next start isn't blocked by start-limit-hit.
  nixling audio off "$vm" >/dev/null
  systemctl reset-failed "$svc" 2>/dev/null || true

  # grant mic
  nixling audio mic on "$vm" >/dev/null
  local state
  state=$(cat "$f")
  assert_eq "$state" '{"mic":"on","speaker":"off"}' "state after mic on" || rc=1
  # Sidecar should be active; socket file PRESENCE is unreliable because
  # vhost-device-sound v0.2.0 unlinks the listener path on accept (its
  # Drop impl in rust-vmm/vhost). When CH is connected the file is
  # transiently gone but the daemon is still serving. Check the
  # systemd service instead. If no VM is currently connected the
  # daemon's outer loop binds a fresh socket file each iteration, so
  # the file should appear within a couple of seconds.
  # nixling-<vm>-snd is a SYSTEM service (not user); use systemctl without --user.
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    systemctl is-active --quiet "$svc" && break
    sleep 0.5
  done
  if ! systemctl is-active --quiet "$svc"; then
    fail "sidecar not active after mic on (within 5s)"
    rc=1
  else
    ok "sidecar active after mic on"
  fi

  # grant speaker too
  nixling audio speaker on "$vm" >/dev/null
  state=$(cat "$f")
  assert_eq "$state" '{"mic":"on","speaker":"on"}' "state after speaker on" || rc=1

  # mic off, speaker still on -> sidecar should stay up
  nixling audio mic off "$vm" >/dev/null
  state=$(cat "$f")
  assert_eq "$state" '{"mic":"off","speaker":"on"}' "state after mic off" || rc=1
  if ! systemctl is-active --quiet "$svc"; then
    fail "sidecar not active after mic off but speaker still on"
    rc=1
  else
    ok "sidecar retained while speaker still on"
  fi

  # revoke all
  nixling audio off "$vm" >/dev/null
  state=$(cat "$f")
  assert_eq "$state" '{"mic":"off","speaker":"off"}' "state after off" || rc=1
  sleep 0.5
  if systemctl is-active --quiet "$svc"; then
    fail "sidecar still active after audio off"
    rc=1
  else
    ok "sidecar inactive after audio off"
  fi

  return $rc
}

test_cli_rejects_audio_disabled_vm() {
  log "test_cli_rejects_audio_disabled_vm"
  # Pick a VM that does NOT have audio enabled.
  local non
  non=$(jq -r '. as $m | to_entries[] | select(.value.audio != true) | .key' \
    /run/current-system/sw/share/nixling/vms.json 2>/dev/null | head -1)
  if [ -z "$non" ]; then
    skip "every VM has audio enabled — no negative case to test"
    return
  fi
  local out rc=0
  out=$(nixling audio mic on "$non" 2>&1) || rc=$?
  if [ "$rc" -eq 0 ]; then
    fail "CLI accepted mic on for audio-disabled VM $non (exit 0)"
    return
  fi
  assert_contains "$out" "does not have audio.enable=true" "error mentions audio.enable"
  ok "CLI rejected mic on for audio-disabled VM $non (exit $rc)"
}

# =====================================================================
# Layer 4: capability matrix
# =====================================================================

test_cloud_hypervisor_capabilities() {
  log "test_cloud_hypervisor_capabilities"
  # Resolve the actual CH binary the runner uses (not whatever's on $PATH).
  # Any declared microvm.vms.<vm> entry works for the capability probe;
  # pick the first one so we don't hardcode a maintainer-specific VM name.
  local ch probe_vm
  probe_vm=$(nix eval --json \
    "$ROOT#nixosConfigurations.$NL_HOST_CONFIG.config.microvm.vms" 2>/dev/null \
    | jq -r 'keys[0] // empty')
  if [ -z "$probe_vm" ]; then
    skip "no microvm.vms entries declared — cannot probe CH binary"
    return 0
  fi
  ch=$(nix eval --raw \
    "$ROOT#nixosConfigurations.$NL_HOST_CONFIG.config.microvm.vms.$probe_vm.config.config.microvm.cloud-hypervisor.package.outPath" \
    2>/dev/null)/bin/cloud-hypervisor
  if [ ! -x "$ch" ]; then
    fail "cloud-hypervisor binary not found at $ch"
    return
  fi
  local v
  v=$("$ch" --version 2>&1)
  log "  $v"
  if ! printf '%s\n' "$v" | grep -qE 'cloud-hypervisor v(5[2-9]|[6-9][0-9]|[1-9][0-9]{2,})\.'; then
    fail "CH version is older than v52 — vulnerable to CVE-2026-45782 and audio is not unblocked"
    return
  fi
  ok "CH version >= v52 (CVE-2026-45782 fixed)"

  local help
  help=$("$ch" --help 2>&1)
  if ! printf '%s' "$help" | grep -q -- '--generic-vhost-user'; then
    fail "CH lacks --generic-vhost-user (audio path won't work)"
  else
    ok "CH has --generic-vhost-user (audio attach supported)"
  fi
  if ! printf '%s' "$help" | grep -q -- '--gpu'; then
    fail "CH lacks --gpu (spectrum graphics patches missing — graphics VMs WILL break)"
  else
    ok "CH has --gpu (spectrum graphics patches present)"
  fi
}

# =====================================================================
# Layer 5: in-guest end-to-end
# =====================================================================

# For an audio-enabled, currently-running VM: ssh in and confirm the
# guest sees a virtio-snd card. Skip if no audio-enabled VM is up.
test_guest_sees_virtio_snd() {
  log "test_guest_sees_virtio_snd"
  if [ -z "$TEST_VMS" ]; then
    skip "no audio-enabled VMs declared"
    return
  fi

  local vm running=""
  for vm in $TEST_VMS; do
    if vm_running "$vm"; then running=$vm; break; fi
  done
  if [ -z "$running" ]; then
    skip "no audio-enabled VM currently running"
    return
  fi

  log "  checking $running"
  # Don't gate on state file (test_cli_grant_revoke may have left it
  # at off,off). Check the CH process command line for
  # --generic-vhost-user; if it's there, the VM was booted with audio
  # attached and the guest MUST see the device.
  if ! pgrep -af "cloud-hypervisor.*microvm-${running}|nixos-system-${running}" 2>/dev/null \
       | grep -q -- '--generic-vhost-user'; then
    skip "$running is running but was NOT booted with --generic-vhost-user (audio off at boot time)"
    return
  fi

  # Probe the guest. ssh_vm uses nixling status to find creds.
  local out
  out=$(ssh_vm "$running" 'cat /proc/asound/cards 2>/dev/null' 2>&1) || {
    skip "$running: SSH probe failed — VM may not yet be sshable"
    return
  }
  if printf '%s' "$out" | grep -qE 'VIRT|virtio'; then
    ok "$running: guest /proc/asound/cards reports virtio-snd"
    printf '%s\n' "$out" | sed 's/^/    /' | tee -a "$NL_LOG" >&2
  else
    fail "$running: guest /proc/asound/cards does NOT report virtio-snd"
    log "    /proc/asound/cards output:"
    printf '%s\n' "$out" | sed 's/^/      /' | tee -a "$NL_LOG" >&2
  fi
}

# ---------------------------------------------------------------------
# Always-last regression guard: confirm we didn't degrade host audio.
# ---------------------------------------------------------------------
test_zzz_host_audio_unchanged() {
  log "test_zzz_host_audio_unchanged"
  # Brief settle window: some tests stop sidecars right before this.
  sleep 1
  local post
  post=$(host_audio_snapshot)
  log "  pre: '$NL_AUDIO_PRE_SNAPSHOT'  post: '$post'"

  # Parse "<devs> <sinks> <sources>"
  local pre_d pre_s pre_so post_d post_s post_so
  read -r pre_d  pre_s  pre_so  <<<"$NL_AUDIO_PRE_SNAPSHOT"
  read -r post_d post_s post_so <<<"$post"

  if [ "$post_d" -lt "$pre_d" ]; then
    fail "host AUDIO DEVICE count regressed during test run: $pre_d -> $post_d (running the test killed host audio)"
  else
    ok "host audio devices: $pre_d -> $post_d (no regression)"
  fi
  if [ "$post_s" -lt "$pre_s" ]; then
    fail "host REAL-SINK count regressed during test run: $pre_s -> $post_s"
  else
    ok "host real sinks: $pre_s -> $post_s (no regression)"
  fi
  if [ "$post_so" -lt "$pre_so" ]; then
    fail "host SOURCE count regressed during test run: $pre_so -> $post_so"
  else
    ok "host sources: $pre_so -> $post_so (no regression)"
  fi
}


# =====================================================================
# M7 fail-closed: audio-state.json edge-case tests (Finding M7)
# =====================================================================
#
# These tests exercise nixling_read_audio_state (from lib.nix) for all
# the fail-closed edge cases mandated by the security audit. They create
# a scratch state directory with controlled content, invoke the helper,
# and assert correct behaviour.
#
# Tests run as $NL_WAYLAND_USER (after re-exec). Writing to /var/lib/nixling/
# requires sudo -A install to set root:nixling 0640 ownership.

_M7_SCRATCH_VM="__nixling_m7test__"
_M7_SCRATCH_DIR="$STATE_ROOT/$_M7_SCRATCH_VM"
_M7_STATE_FILE="$_M7_SCRATCH_DIR/state/audio-state.json"

_m7_write_state() {
  sudo -A install -d -m 0755 -o root -g root "$_M7_SCRATCH_DIR" 2>/dev/null || true
  sudo -A install -d -m 0750 -o root -g nixling "$_M7_SCRATCH_DIR/state" 2>/dev/null || true
  local tmp
  tmp=$(mktemp)
  printf '%s\n' "$1" > "$tmp"
  sudo -A install -m 0640 -o root -g nixling "$tmp" "$_M7_STATE_FILE"
  rm -f "$tmp"
}

_m7_remove_state() {
  sudo -A rm -f "$_M7_STATE_FILE" 2>/dev/null || true
}

_m7_cleanup() { sudo -A rm -rf "$_M7_SCRATCH_DIR" 2>/dev/null || true; }
add_cleanup _m7_cleanup

# Run nixling_read_audio_state for the scratch VM by sourcing the
# helper from the Nix store. Returns 77 if the helper is not built yet
# (pre-rebuild) so callers can SKIP cleanly.
_m7_run_helper() {
  local helper_path
  # The `nixling.audioStateHelperPath`
  # internal option was retired together with the bash CLI surface,
  # so we no longer eval it out of the live config. Instead, locate
  # the helper by greping the nixling binary in the current system
  # generation (the daemon-managed `nixling` references it directly).
  local nixling_bin
  nixling_bin=$(command -v nixling 2>/dev/null || echo /run/current-system/sw/bin/nixling)
  helper_path=$(grep -o '/nix/store/[^ ]*nixling-read-audio-state\.sh' \
    "$nixling_bin" 2>/dev/null | head -1) || helper_path=""
  if [ -z "$helper_path" ] || [ ! -f "$helper_path" ]; then
    return 77
  fi
  bash -c ". \"$helper_path\"; nixling_read_audio_state \"$_M7_SCRATCH_VM\""
}

test_audio_state_fail_closed_missing() {
  log "test_audio_state_fail_closed_missing"
  _m7_remove_state
  local out rc=0
  out=$(_m7_run_helper); rc=$?
  if [ "$rc" -ne 0 ]; then
    if [ "$rc" -eq 77 ]; then skip "nixling-read-audio-state.sh not in store (rebuild needed)"; return 0; fi
  fi
  assert_eq "$out" "mic=off speaker=off" "missing state file -> mic=off speaker=off" || rc=1
  return $rc
}

test_audio_state_fail_closed_garbage() {
  log "test_audio_state_fail_closed_garbage"
  _m7_write_state "not-json"
  local out rc=0
  out=$(_m7_run_helper); rc=$?
  if [ "$rc" -ne 0 ]; then
    if [ "$rc" -eq 77 ]; then skip "nixling-read-audio-state.sh not in store (rebuild needed)"; return 0; fi
  fi
  assert_eq "$out" "mic=off speaker=off" "garbage JSON -> mic=off speaker=off" || rc=1
  return $rc
}

test_audio_state_fail_closed_unexpected_value() {
  log "test_audio_state_fail_closed_unexpected_value"
  _m7_write_state '{"mic":"true","speaker":1}'
  local out rc=0
  out=$(_m7_run_helper); rc=$?
  if [ "$rc" -ne 0 ]; then
    if [ "$rc" -eq 77 ]; then skip "nixling-read-audio-state.sh not in store (rebuild needed)"; return 0; fi
  fi
  assert_eq "$out" "mic=off speaker=off" 'unexpected field values {"mic":"true","speaker":1} -> mic=off speaker=off' || rc=1
  return $rc
}

test_audio_state_open_when_valid() {
  log "test_audio_state_open_when_valid"
  _m7_write_state '{"mic":"on","speaker":"on"}'
  local out rc=0
  out=$(_m7_run_helper); rc=$?
  if [ "$rc" -ne 0 ]; then
    if [ "$rc" -eq 77 ]; then skip "nixling-read-audio-state.sh not in store (rebuild needed)"; return 0; fi
  fi
  assert_eq "$out" "mic=on speaker=on" 'valid on/on -> mic=on speaker=on' || rc=1
  return $rc
}

test_audio_state_file_mode() {
  log "test_audio_state_file_mode"
  if [ -z "$TEST_VMS" ]; then
    skip "no audio-enabled VMs declared"
    return 0
  fi
  local rc=0 vm
  for vm in $TEST_VMS; do
    local f="$STATE_ROOT/$vm/state/audio-state.json"
    if [ ! -e "$f" ]; then
      log "  SKIP: $vm: state file not yet created (VM never started)"
      continue
    fi
    local mode_owner
    if ! mode_owner=$(stat -c '%a %U:%G' "$f" 2>/dev/null); then
      fail "$vm: stat failed on $f"
      rc=1
      continue
    fi
    assert_eq "$mode_owner" "640 root:nixling" "$vm: audio-state.json is mode 640 owner root:nixling" || rc=1
  done
  return $rc
}

# ---------------------------------------------------------------------
# security-r8-audio: end-to-end signal-path tests.
#
# History: the refactor (audio sidecar moved from user service to
# system service with nixling-<vm>-snd system user + ACL-gated socket)
# silently broke the audio path in three distinct ways that all
# slipped past the existing tests because none of them verified the
# LIVE signal:
#
#   1. ExecStartPost timeout was 4s but vhost-device-sound takes >4s
#      to create its listen socket → ACL never applied → CH bailed
#      with EACCES on connect (security-r8-audio-1).
#   2. audioArgsScript inside microvm-run tried to `systemctl start`
#      the sidecar as nixling-<vm>-gpu, which triggered polkit
#      password prompts on every VM boot (security-r8-audio-2/-3).
#   3. A broad WirePlumber stream rule null-targeted EVERY
#      `nixling-*` capture stream forever, so mic=on had no effect
#      even after audio.nix correctly emitted --generic-vhost-user
#      (security-r8-audio-6).
#
# The new tests below verify the LIVE audio path end-to-end:
#   - test_e2e_ch_has_generic_vhost_user      (#1 detector)
#   - test_e2e_sidecar_socket_acl             (#1 detector)
#   - test_e2e_node_name_per_vm               (cosmetic - per-VM identification in pavucontrol)
#   - test_e2e_mic_routes_when_on             (#3 detector + new req)
#   - test_e2e_speaker_routes_when_on         (#3 detector)
#   - test_e2e_guest_can_record               (full guest-side check)
#   - test_e2e_guest_can_play                 (full guest-side check)
#
# All tests run against a RUNNING audio-enabled graphics VM. They
# auto-SKIP when no such VM exists (e.g. on a headless CI host).
# ---------------------------------------------------------------------

# Returns 0 + sets RUNNING_AUDIO_VM if an audio-enabled VM is currently
# up, otherwise 1.
_e2e_pick_running_vm() {
  RUNNING_AUDIO_VM=""
  if [ -z "${TEST_VMS:-}" ]; then
    return 1
  fi
  local vm
  for vm in $TEST_VMS; do
    if vm_running "$vm"; then
      RUNNING_AUDIO_VM="$vm"
      return 0
    fi
  done
  return 1
}

test_e2e_ch_has_generic_vhost_user() {
  log "test_e2e_ch_has_generic_vhost_user"
  if ! _e2e_pick_running_vm; then
    skip "no audio-enabled VM currently running"
    return 0
  fi
  local vm="$RUNNING_AUDIO_VM"
  local mic spk
  read -r mic spk < <(nixling audio status "$vm" 2>/dev/null \
    | awk '/^mic:/{m=$2} /^speaker:/{s=$2} END{print m, s}')
  if [ "$mic" != "on" ] && [ "$spk" != "on" ]; then
    skip "$vm: mic=$mic spk=$spk; --generic-vhost-user not expected"
    return 0
  fi
  local cmdline
  cmdline=$(pgrep -af "cloud-hypervisor.*${vm}" 2>/dev/null | head -1)
  if [ -z "$cmdline" ]; then
    fail "$vm: no cloud-hypervisor process found despite vm_running"
    return 1
  fi
  if printf '%s' "$cmdline" | grep -q -- '--generic-vhost-user'; then
    ok "$vm: CH cmdline includes --generic-vhost-user (mic=$mic spk=$spk)"
  else
    fail "$vm: CH was launched WITHOUT --generic-vhost-user despite mic=$mic spk=$spk; audio is dead (regression of security-r8-audio-1/2)"
    return 1
  fi
}

test_e2e_sidecar_socket_acl() {
  log "test_e2e_sidecar_socket_acl"
  if ! _e2e_pick_running_vm; then
    skip "no audio-enabled VM currently running"
    return 0
  fi
  local vm="$RUNNING_AUDIO_VM"
  local mic spk
  read -r mic spk < <(nixling audio status "$vm" 2>/dev/null \
    | awk '/^mic:/{m=$2} /^speaker:/{s=$2} END{print m, s}')
  if [ "$mic" != "on" ] && [ "$spk" != "on" ]; then
    skip "$vm: mic=$mic spk=$spk; sidecar socket not expected to exist"
    return 0
  fi
  if ! sudo -A test -d "/run/nixling/vms/$vm"; then
    fail "$vm: /run/nixling/vms/$vm does not exist despite mic=$mic spk=$spk"
    return 1
  fi
  # Sidecar's vhost-user listen socket gets CONSUMED on CH connect
  # (vhost-device-sound v0.3.0 is single-connection). If CH is
  # already attached the socket file is gone but its FDs live on
  # inside both processes. The post-connect invariant we CAN check:
  # CH process holds a socket FD AND --generic-vhost-user was on its
  # cmdline (the latter is test_e2e_ch_has_generic_vhost_user). We
  # check the directory ACL for nixling-<vm>-gpu:--x which is set
  # by ExecStartPost and is durable.
  #
  # If the socket FILE still exists (e.g. CH hasn't connected yet, or
  # restart raced) verify both ACLs.
  local dacl sacl
  dacl=$(sudo -A getfacl -p "/run/nixling/vms/$vm" 2>/dev/null) || dacl=""
  if ! printf '%s' "$dacl" | grep -qE "^user:nixling-${vm}-gpu:--x"; then
    fail "$vm: socket dir ACL missing nixling-${vm}-gpu:x (regression of security-r8-audio-1)"
    log "    getfacl: $dacl"
    return 1
  fi
  ok "$vm: socket dir ACL grants nixling-${vm}-gpu:x (security-r8-audio-1 invariant)"
  if sudo -A test -S "/run/nixling/vms/$vm/snd.sock"; then
    sacl=$(sudo -A getfacl -p "/run/nixling/vms/$vm/snd.sock" 2>/dev/null) || sacl=""
    if printf '%s' "$sacl" | grep -qE "^user:nixling-${vm}-gpu:rw"; then
      ok "$vm: snd.sock present with nixling-${vm}-gpu:rw ACL (CH not yet attached)"
    else
      fail "$vm: snd.sock present but ACL missing nixling-${vm}-gpu:rw"
      log "    getfacl: $sacl"
      return 1
    fi
  else
    ok "$vm: snd.sock already consumed by CH (vhost-user single-connection mode)"
  fi
}

test_e2e_node_name_per_vm() {
  log "test_e2e_node_name_per_vm"
  if ! _e2e_pick_running_vm; then
    skip "no audio-enabled VM currently running"
    return 0
  fi
  local vm="$RUNNING_AUDIO_VM"
  if ! systemctl is-active --quiet "nixling-${vm}-snd.service"; then
    skip "$vm: sidecar not active"
    return 0
  fi

  local dump
  dump=$(_e2e_pw_dump) || dump=""
  if [ -z "$dump" ]; then
    skip "$vm: pw-dump produced no output (Wayland session not reachable)"
    return 0
  fi

  # ---------------------------------------------------------------------
  # CLIENT-level check (what pavucontrol / KDE audio applet display)
  # ---------------------------------------------------------------------
  # The PipeWire client for the sidecar must show application.name =
  # "nixling-<vm>" — NOT the generic "vhost-device-sound" inherited from
  # argv[0]. In pavucontrol the
  # Applications tab labels each stream by its CLIENT application.name,
  # so two VMs both showing "vhost-device-sound" are indistinguishable.
  #
  # security-r8-audio-7: libpipewire derives application.name from
  # argv[0]'s basename (program_invocation_short_name). PIPEWIRE_PROPS
  # does NOT override this for clients (it only covers streams/filters).
  # The unit wraps ExecStart with `bash -c 'exec -a nixling-<vm> ...'`
  # so the kernel-visible argv[0] is the per-VM name.
  local client_app
  client_app=$(printf '%s' "$dump" \
    | jq -r --arg user "nixling-$vm-snd" '
        first(.[]
          | select(.type == "PipeWire:Interface:Client"
                   and .info.props["application.process.user"] == $user)
          | .info.props["application.name"]) // empty' 2>/dev/null)
  if [ "$client_app" = "nixling-$vm" ]; then
    ok "$vm: PipeWire Client application.name=\"nixling-${vm}\" (visible in pavucontrol/wpctl)"
  elif [ -z "$client_app" ]; then
    fail "$vm: no PipeWire Client owned by nixling-$vm-snd found"
    return 1
  else
    fail "$vm: PipeWire Client application.name is \"$client_app\", expected \"nixling-${vm}\" (regression of security-r8-audio-7 exec-a wrapping)"
    return 1
  fi

  # ---------------------------------------------------------------------
  # STREAM-level check (set via PIPEWIRE_PROPS env file)
  # ---------------------------------------------------------------------
  # Streams are created LAZILY: vhost-device-sound only opens PipeWire
  # streams when the guest opens its virtio-snd device. Trigger a
  # brief 0.2s in-guest aplay to force the output stream into
  # existence (speaker direction always auto-creates regardless of
  # mic state).
  ssh_vm "$vm" 'set -e
    if command -v aplay >/dev/null 2>&1; then
      head -c 19200 /dev/zero | timeout 3 aplay -q -D plughw:0,0 -f S16_LE -c 1 -r 48000 - >/dev/null 2>&1 || true
    fi' >/dev/null 2>&1 || true
  # Re-dump after the stream got created.
  dump=$(_e2e_pw_dump) || dump=""

  local found
  found=$(printf '%s' "$dump" \
    | jq -r --arg an "nixling-$vm" '[.[]
        | select(.type == "PipeWire:Interface:Node"
                 and .info.props["application.name"] == $an
                 and .info.props["node.name"] == $an)] | length' 2>/dev/null)
  if [ -n "$found" ] && [ "$found" -ge 1 ]; then
    ok "$vm: PipeWire stream node.name=\"nixling-${vm}\" (per-VM identity, $found stream(s))"
  else
    # Fall back: maybe streams weren't created. Verify static
    # config intent so we still catch a missing env file.
    local env_file="/run/nixling/vms/$vm/snd.env"
    if sudo -A test -r "$env_file" \
       && sudo -A grep -q "\"node.name\":\"nixling-$vm\"" "$env_file"; then
      ok "$vm: env file requests node.name=\"nixling-${vm}\" (no active streams to verify live)"
    else
      fail "$vm: PIPEWIRE_PROPS env file missing per-VM node.name request"
      return 1
    fi
  fi
}

# Helper: run pw-dump in the Wayland user's PipeWire session, regardless of
# whether the caller is that user or root.
_e2e_pw_dump() {
  local wu_uid
  wu_uid=$(id -u "$NL_WAYLAND_USER" 2>/dev/null) || wu_uid="1000"
  if [ "$(id -un)" = "$NL_WAYLAND_USER" ]; then
    env XDG_RUNTIME_DIR="/run/user/$wu_uid" pw-dump 2>/dev/null
  else
    sudo -A -u "$NL_WAYLAND_USER" env XDG_RUNTIME_DIR="/run/user/$wu_uid" pw-dump 2>/dev/null
  fi
}

# Helper: assert that a vhost-device-sound stream of the given media.class
# IS / IS NOT linked to a hardware peer.
_e2e_stream_linked() {
  local vm="$1" class="$2"  # class: Stream/Input/Audio | Stream/Output/Audio
  local dump wu_uid
  wu_uid=$(id -u "$NL_WAYLAND_USER" 2>/dev/null) || wu_uid="1000"
  dump=$(runuser -u "$NL_WAYLAND_USER" -- env XDG_RUNTIME_DIR="/run/user/$wu_uid" pw-dump 2>/dev/null) || return 2
  # Find the node id whose props contain BOTH application.name=nixling-<vm>
  # AND media.class=<class>. Then look for any link involving that node.
  printf '%s' "$dump" | jq -re --arg an "nixling-$vm" --arg mc "$class" '
    [.[] | select(.type == "PipeWire:Interface:Node"
                  and .info.props["application.name"] == $an
                  and .info.props["media.class"] == $mc)] as $nodes
    | if ($nodes | length) == 0 then "no-node"
      else
        ($nodes[0].id) as $nid
        | [.[] | select(.type == "PipeWire:Interface:Link"
                        and (.info["input-node-id"] == $nid or .info["output-node-id"] == $nid))]
          as $links
        | if ($links | length) >= 1 then "linked" else "unlinked" end
      end' 2>/dev/null
}

test_e2e_speaker_routes_when_on() {
  log "test_e2e_speaker_routes_when_on"
  if ! _e2e_pick_running_vm; then
    skip "no audio-enabled VM currently running"
    return 0
  fi
  local vm="$RUNNING_AUDIO_VM" spk
  spk=$(nixling audio status "$vm" 2>/dev/null | awk '/^speaker:/{print $2}')
  if [ "$spk" != "on" ]; then
    skip "$vm: speaker=$spk; auto-route test not applicable"
    return 0
  fi
  # Even before any in-guest playback, the sidecar's output stream
  # should NOT carry node.dont-fallback (which would force "-1"
  # routing). Check the live node props.
  local nd dump
  dump=$(_e2e_pw_dump) || dump=""
  nd=$(printf '%s' "$dump" | jq -re --arg an "nixling-$vm" '
    .[] | select(.type == "PipeWire:Interface:Node"
                 and .info.props["application.name"] == $an
                 and .info.props["media.class"] == "Stream/Output/Audio")
    | .info.props["node.dont-fallback"] // "false"' 2>/dev/null | head -1)
  if [ "$nd" = "true" ]; then
    fail "$vm: speaker=on but output stream has node.dont-fallback=true; WP rule is over-broad (regression of security-r8-audio-6)"
    return 1
  fi
  ok "$vm: speaker=on, output stream does NOT carry node.dont-fallback"
}

test_e2e_mic_routes_when_on() {
  log "test_e2e_mic_routes_when_on"
  if ! _e2e_pick_running_vm; then
    skip "no audio-enabled VM currently running"
    return 0
  fi
  local vm="$RUNNING_AUDIO_VM" mic
  mic=$(nixling audio status "$vm" 2>/dev/null | awk '/^mic:/{print $2}')
  if [ "$mic" != "on" ]; then
    skip "$vm: mic=$mic; auto-route test not applicable"
    return 0
  fi
  # When mic=on, the input stream MUST NOT carry node.dont-fallback,
  # otherwise WirePlumber refuses to auto-link it to the host's
  # default source and arecord in the guest returns I/O error.
  local nd dump
  dump=$(_e2e_pw_dump) || dump=""
  nd=$(printf '%s' "$dump" | jq -re --arg an "nixling-$vm" '
    .[] | select(.type == "PipeWire:Interface:Node"
                 and .info.props["application.name"] == $an
                 and .info.props["media.class"] == "Stream/Input/Audio")
    | .info.props["node.dont-fallback"] // "false"' 2>/dev/null | head -1)
  if [ "$nd" = "true" ]; then
    fail "$vm: mic=on but input stream has node.dont-fallback=true; WP rule blocked the auto-route (regression of security-r8-audio-6)"
    return 1
  fi
  ok "$vm: mic=on, input stream does NOT carry node.dont-fallback"
}

test_e2e_guest_can_play() {
  log "test_e2e_guest_can_play"
  if ! _e2e_pick_running_vm; then
    skip "no audio-enabled VM currently running"
    return 0
  fi
  local vm="$RUNNING_AUDIO_VM" spk
  spk=$(nixling audio status "$vm" 2>/dev/null | awk '/^speaker:/{print $2}')
  if [ "$spk" != "on" ]; then
    skip "$vm: speaker=$spk; playback test not applicable"
    return 0
  fi
  # Generate 1 second of silence inside the guest and feed it to the
  # virtio-snd output. If --generic-vhost-user is wired up and the
  # output is auto-routed, `aplay` returns 0. If the device is dead
  # (no socket, EACCES, sidecar panic, or wrong format negotiation),
  # aplay returns non-zero or hangs.
  local out rc=0
  out=$(ssh_vm "$vm" 'set -e
    if ! command -v aplay >/dev/null 2>&1; then
      echo "MISSING aplay"; exit 77
    fi
    # 1s of silence at 48kHz mono S16 = 96000 bytes
    head -c 96000 /dev/zero \
      | timeout 5 aplay -q -D plughw:0,0 -f S16_LE -c 1 -r 48000 - >/dev/null 2>&1
    echo "aplay rc=$?"' 2>&1) || rc=$?
  if [ "$rc" -ne 0 ]; then
    skip "$vm: SSH probe failed (rc=$rc)"
    return 0
  fi
  if printf '%s' "$out" | grep -q "MISSING aplay"; then
    skip "$vm: alsa-utils not installed in guest"
    return 0
  fi
  if printf '%s' "$out" | grep -q "aplay rc=0"; then
    ok "$vm: guest aplay (1s silence) succeeded; --generic-vhost-user signal path is live"
  else
    fail "$vm: guest aplay failed: $out"
    return 1
  fi
}

test_e2e_guest_can_record() {
  log "test_e2e_guest_can_record"
  if ! _e2e_pick_running_vm; then
    skip "no audio-enabled VM currently running"
    return 0
  fi
  local vm="$RUNNING_AUDIO_VM" mic
  mic=$(nixling audio status "$vm" 2>/dev/null | awk '/^mic:/{print $2}')
  if [ "$mic" != "on" ]; then
    skip "$vm: mic=$mic; capture test not applicable"
    return 0
  fi
  # Capture 1s of audio from the guest's virtio-snd input. With
  # mic=on the sidecar's input stream auto-links to the host's
  # default source, so `arecord` should successfully read 96kB.
  local out rc=0
  out=$(ssh_vm "$vm" 'set -e
    if ! command -v arecord >/dev/null 2>&1; then
      echo "MISSING arecord"; exit 77
    fi
    tmp=$(mktemp /tmp/nixling-cap.XXXXXX.wav)
    if timeout 5 arecord -q -D plughw:0,0 -d 1 -f S16_LE -c 1 -r 48000 "$tmp" >/dev/null 2>&1; then
      sz=$(stat -c "%s" "$tmp")
      rm -f "$tmp"
      echo "arecord rc=0 sz=$sz"
    else
      arc=$?
      rm -f "$tmp"
      echo "arecord rc=$arc"
    fi' 2>&1) || rc=$?
  if [ "$rc" -ne 0 ]; then
    skip "$vm: SSH probe failed (rc=$rc)"
    return 0
  fi
  if printf '%s' "$out" | grep -q "MISSING arecord"; then
    skip "$vm: alsa-utils not installed in guest"
    return 0
  fi
  local sz
  sz=$(printf '%s' "$out" | sed -n 's/.*sz=\([0-9]*\).*/\1/p')
  if printf '%s' "$out" | grep -q "arecord rc=0" \
     && [ -n "$sz" ] && [ "$sz" -ge 90000 ]; then
    ok "$vm: guest arecord (1s) succeeded; mic=on routing path is live (got ${sz}B)"
  else
    fail "$vm: guest arecord failed (regression of security-r8-audio-6 mic-routing): $out"
    return 1
  fi
}

test_audio_off_no_virtio_snd() {
  log "test_audio_off_no_virtio_snd"
  if [ -z "$TEST_VMS" ]; then
    skip "no audio-enabled VMs declared"
    return 0
  fi
  local vm running=""
  for vm in $TEST_VMS; do
    if vm_running "$vm"; then running="$vm"; break; fi
  done
  if [ -z "$running" ]; then
    skip "no audio-enabled VM currently running"
    return 0
  fi
  # Check current audio state for the running VM.
  local state_out mic spk
  state_out=$(nixling audio status "$running" 2>/dev/null) || state_out=""
  mic=$(printf '%s\n' "$state_out" | awk '/^mic:/{print $2}')
  spk=$(printf '%s\n' "$state_out" | awk '/^speaker:/{print $2}')
  if [ "$mic" = "on" ] || [ "$spk" = "on" ]; then
    skip "$running: audio is on (mic=$mic speaker=$spk); virtio-snd expected, not testing off-check"
    return 0
  fi
  local out rc=0
  if ! out=$(ssh_vm "$running" 'cat /proc/asound/cards 2>/dev/null' 2>&1); then
    skip "$running: SSH probe failed — VM not yet sshable"
    return 0
  fi
  if printf '%s' "$out" | grep -qiE 'virtio|VIRT'; then
    fail "$running: /proc/asound/cards shows virtio-snd but audio state is off"
    rc=1
  else
    ok "$running: no virtio-snd in /proc/asound/cards (audio correctly disabled)"
  fi
  return $rc
}

# ---------------------------------------------------------------------
# Dispatch
# ---------------------------------------------------------------------
# shellcheck disable=SC2034  # QUICK_TESTS consumed via `local -n SET=QUICK_TESTS` in main()
QUICK_TESTS=(
  test_host_pipewire_alive
  test_host_has_audio_devices
  test_host_has_audio_sinks_and_sources
  test_sidecar_unit_present
  test_cloud_hypervisor_capabilities
  test_audio_state_fail_closed_missing
  test_audio_state_fail_closed_garbage
  test_audio_state_fail_closed_unexpected_value
  test_audio_state_open_when_valid
  # security-r8-audio: live signal-path regression guards. These are
  # cheap (no nixos-rebuild, no VM bring-up) and catch the exact
  # failure modes that slipped past --quick during this hardening pass.
  test_e2e_ch_has_generic_vhost_user
  test_e2e_sidecar_socket_acl
  test_e2e_node_name_per_vm
  test_e2e_mic_routes_when_on
  test_e2e_speaker_routes_when_on
  test_zzz_host_audio_unchanged
)
ALL_TESTS=(
  test_host_pipewire_alive
  test_host_has_audio_devices
  test_host_has_audio_sinks_and_sources
  test_sidecar_unit_present
  test_sidecar_socket_lifecycle
  test_cli_status_smoke
  test_cli_grant_revoke
  test_cli_rejects_audio_disabled_vm
  test_cloud_hypervisor_capabilities
  test_guest_sees_virtio_snd
  test_audio_state_fail_closed_missing
  test_audio_state_fail_closed_garbage
  test_audio_state_fail_closed_unexpected_value
  test_audio_state_open_when_valid
  test_audio_state_file_mode
  test_audio_off_no_virtio_snd
  # security-r8-audio: live signal-path regression guards (live SSH).
  test_e2e_ch_has_generic_vhost_user
  test_e2e_sidecar_socket_acl
  test_e2e_node_name_per_vm
  test_e2e_mic_routes_when_on
  test_e2e_speaker_routes_when_on
  test_e2e_guest_can_play
  test_e2e_guest_can_record
  test_zzz_host_audio_unchanged
)

main() {
  local mode=${1:-full} only=""
  case "$mode" in
    --quick)        local -n SET=QUICK_TESTS ;;
    --only)         only="${2:-}"; local -n SET=ALL_TESTS ;;
    --list)         printf '%s\n' "${ALL_TESTS[@]}"; return 0 ;;
    full|*)         local -n SET=ALL_TESTS ;;
  esac

  log "nixling audio test suite — log: $NL_LOG"
  local pass=0 fail_count=0
  for t in "${SET[@]}"; do
    if [ -n "$only" ] && [ "$t" != "$only" ]; then continue; fi
    if "$t"; then
      pass=$((pass+1))
    else
      fail_count=$((fail_count+1))
    fi
  done
  log "==="
  log "Summary: $pass passed, $fail_count failed"
  [ "$fail_count" -eq 0 ]
}

main "$@"
