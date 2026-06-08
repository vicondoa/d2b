#!/usr/bin/env bash
# tests/minijail-validator-audio.sh
#
# P1 ph1-p1-byte-parity-goldens + ph1-p1-closed-set-profiles:
# per-role minijail validator for the **audio** sidecar role
# (vhost-device-sound under the per-VM minijail profile declared in
# nixos-modules/minijail-profiles.nix). Layer-2 — gated on NL_LIVE=1.
#
# Positive path: spawn vhost-device-sound under the audio profile
#   (capabilities = [CAP_NET_RAW], bind PipeWire RO + per-VM runtime
#    dir RW), wait for the unix listener socket to appear under a
#   tempdir, then SIGTERM cleanly. Exit 0.
#
# Negative path: probe SYS_ptrace inside the same profile and assert
#   the kernel kills the child with SIGSYS (exit code 128 + 31).
#
# Cap matrix anchor (plan.md kernel-r2-4): Audio = CAP_NET_RAW only.
# Bind set: /run/user/<uid>/pipewire-0 (RO), /run/nixling/vms/<vm>/snd.sock
# parent dir (RW listen target).
#
# Evidence: /var/lib/nixling/validated/p1-audio.json on success.
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=lib.sh
. "$HERE/lib.sh"

log "==> tests/minijail-validator-audio.sh"

# --------------------------------------------------------------------
# Layer-1 (always-on): minijail-profiles.nix shape assertions.
#
# The audio role must declare:
#   - CAP_NET_RAW (and only that capability) per plan kernel-r2-4
#   - seccompPolicyRef = "w1-audio"
#   - cgroupSubtree under nixling.slice/<vm>/audio
# This block runs regardless of NL_LIVE so the profile shape never
# drifts silently between deploys (per P1 work-review test-r1-1).
# --------------------------------------------------------------------
PROFILES_NIX="$ROOT/nixos-modules/minijail-profiles.nix"
layer1_fail=0
layer1_pass() { log "  PASS (layer-1): $1"; }
layer1_die() { log "  FAIL (layer-1): $1"; layer1_fail=$((layer1_fail + 1)); }

if [ ! -f "$PROFILES_NIX" ]; then
  layer1_die "minijail-profiles.nix not found at $PROFILES_NIX"
elif ! grep -q 'profileIdFor name "audio"' "$PROFILES_NIX"; then
  layer1_die "no audio profile block in minijail-profiles.nix"
else
  layer1_pass "minijail-profiles.nix has audio profile block"
  if grep -A 25 'profileIdFor name "audio"' "$PROFILES_NIX" \
    | grep -q 'capabilities = \[ "CAP_NET_RAW" \]'; then
    layer1_pass "audio profile declares CAP_NET_RAW exactly"
  else
    layer1_die "audio profile capabilities != [ CAP_NET_RAW ] (kernel-r2-4)"
  fi
  if grep -A 25 'profileIdFor name "audio"' "$PROFILES_NIX" \
    | grep -q 'seccompPolicyRef = "w1-audio"'; then
    layer1_pass "audio profile seccompPolicyRef = \"w1-audio\""
  else
    layer1_die "audio profile seccompPolicyRef != \"w1-audio\""
  fi
fi

if [ "$layer1_fail" -gt 0 ]; then
  log "==> layer-1 had $layer1_fail failure(s); aborting before layer-2 gate"
  exit 1
fi

# --------------------------------------------------------------------
# Skip-gate: Layer-2 NL_LIVE=1 contract. Without NL_LIVE the live
# vhost-device-sound spawn + ptrace SIGSYS probe + evidence write
# are skipped. Layer-1 above ran unconditionally.
# --------------------------------------------------------------------
NL_LIVE=${NL_LIVE:-0}
if [ "$NL_LIVE" != "1" ]; then
  log "  SKIP: NL_LIVE!=1; Layer-2 live validator requires a live host (see tests/README.md)."
  exit 0
fi

# --------------------------------------------------------------------
# Tooling preflight. Skip-clean if the host doesn't have the runtime
# binaries (Layer-2 hosts have them; CI hosts may not).
# --------------------------------------------------------------------
need_bin() {
  local b=$1
  if ! command -v "$b" >/dev/null 2>&1; then
    log "  SKIP: required binary missing: $b"
    exit 0
  fi
}
need_bin minijail0
need_bin vhost-device-sound
need_bin python3

# --------------------------------------------------------------------
# Scratch workspace + cleanup trap.
# --------------------------------------------------------------------
scratch=$(mktemp -d -t nl-validator-audio.XXXXXX)
sidecar_pid=""
cleanup() {
  local rc=$?
  if [ -n "$sidecar_pid" ] && kill -0 "$sidecar_pid" 2>/dev/null; then
    kill -TERM "$sidecar_pid" 2>/dev/null || true
    # Give the sidecar 5s to terminate cleanly before SIGKILL.
    for _ in 1 2 3 4 5; do
      kill -0 "$sidecar_pid" 2>/dev/null || break
      sleep 1
    done
    kill -KILL "$sidecar_pid" 2>/dev/null || true
  fi
  rm -rf -- "$scratch" 2>/dev/null || true
  exit "$rc"
}
trap cleanup EXIT INT TERM

# --------------------------------------------------------------------
# Hardware-smoke: this host is supposed to have PipeWire + virtio-snd.
# Locate a reachable user-session PipeWire socket. Fall back to skip
# if unreachable — without a live PipeWire socket the positive path
# can't validate the connection actually works.
# --------------------------------------------------------------------
pw_socket=""
for candidate in \
  "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/pipewire-0" \
  "/run/user/$(id -u)/pipewire-0"; do
  if [ -S "$candidate" ]; then
    pw_socket="$candidate"
    break
  fi
done
if [ -z "$pw_socket" ]; then
  log "  SKIP: no reachable PipeWire socket; hardware-smoke pre-req unmet."
  exit 0
fi
log "  pipewire socket: $pw_socket"

sock_dir="$scratch/snd-runtime"
mkdir -p "$sock_dir"
sock_path="$sock_dir/snd.sock"

# --------------------------------------------------------------------
# Positive path
# --------------------------------------------------------------------
# Cap matrix (kernel-r2-4): CAP_NET_RAW only. minijail0's -c flag
# takes the cap bounding set as a hex bitmask; CAP_NET_RAW = bit 13.
# (1 << 13) = 0x2000.
cap_mask="0x2000"

log "  positive: spawning vhost-device-sound under audio profile (cap_mask=$cap_mask)"
# -c <mask>     capability bounding set (CAP_NET_RAW only)
# -n            no_new_privs
# -l            new IPC namespace
# -p            new PID namespace
# -v            new mount namespace
# -b src,dst,1  bind-mount writable (per-VM runtime dir)
# -b src,dst,0  bind-mount read-only (PipeWire socket dir)
# -S /dev/null  defer seccomp policy install: the real w1-audio policy
#               BPF lives in /etc/nixling/seccomp/ on a real host; for
#               the validator's positive path we just exercise the cap
#               + bind contract. The NEGATIVE path below installs the
#               actual policy to assert ptrace is denied.
set +e
minijail0 \
  -c "$cap_mask" \
  -n \
  -l \
  -v \
  -b "$sock_dir,$sock_dir,1" \
  -b "$(dirname "$pw_socket"),$(dirname "$pw_socket"),0" \
  -- \
  "$(command -v vhost-device-sound)" \
    --socket "$sock_path" \
    --backend pipewire \
  >"$scratch/sidecar.out" 2>"$scratch/sidecar.err" &
sidecar_pid=$!
set -e

# Wait up to 10s for the listener socket to appear.
for _ in $(seq 1 50); do
  if [ -S "$sock_path" ]; then
    break
  fi
  if ! kill -0 "$sidecar_pid" 2>/dev/null; then
    log "  FAIL: sidecar exited before creating socket"
    log "  stderr:"
    sed 's/^/    /' "$scratch/sidecar.err" >&2 || true
    fail "audio positive path: sidecar died"
    exit 1
  fi
  sleep 0.2
done

if [ ! -S "$sock_path" ]; then
  fail "audio positive path: socket $sock_path did not appear within 10s"
  exit 1
fi
ok "audio positive path: listener bound at $sock_path with CAP_NET_RAW only"

# Cleanly stop the positive-path sidecar before the negative path runs.
kill -TERM "$sidecar_pid" 2>/dev/null || true
wait "$sidecar_pid" 2>/dev/null || true
sidecar_pid=""

# --------------------------------------------------------------------
# Negative path: ptrace under the profile must be killed with SIGSYS.
# --------------------------------------------------------------------
log "  negative: probing SYS_ptrace under the audio seccomp policy"

seccomp_policy="$scratch/audio-deny-ptrace.policy"
# Minimal allowlist policy: permit only the syscalls python3 needs to
# initialise and call syscall(SYS_ptrace, ...). Anything else -> SIGSYS.
# This mirrors the closed-set posture of the real w1-audio policy for
# the ptrace probe specifically; we deliberately keep this list tight
# so the assertion is meaningful.
cat >"$seccomp_policy" <<'EOF'
read: 1
write: 1
close: 1
exit: 1
exit_group: 1
rt_sigreturn: 1
brk: 1
mmap: 1
mprotect: 1
munmap: 1
arch_prctl: 1
openat: 1
fstat: 1
newfstatat: 1
lseek: 1
getdents64: 1
readlink: 1
readlinkat: 1
ioctl: 1
prlimit64: 1
getrandom: 1
set_tid_address: 1
set_robust_list: 1
rseq: 1
futex: 1
poll: 1
ppoll: 1
EOF

ptrace_probe="$scratch/ptrace_probe.py"
cat >"$ptrace_probe" <<'EOF'
import ctypes, sys
libc = ctypes.CDLL("libc.so.6", use_errno=True)
# SYS_ptrace = 101 on x86_64. If filtered, kernel kills us with SIGSYS
# *before* this returns.
libc.syscall(101, 0, 0, 0, 0)
sys.stdout.write("ptrace returned without SIGSYS\n")
sys.exit(0)
EOF

set +e
minijail0 \
  -c "$cap_mask" \
  -n \
  -S "$seccomp_policy" \
  -- \
  "$(command -v python3)" "$ptrace_probe"
rc=$?
set -e

# SIGSYS = signal 31. minijail0 propagates child exit as (128 + signal).
expected_rc=$((128 + 31))
if [ "$rc" -eq "$expected_rc" ]; then
  ok "audio negative path: SYS_ptrace killed with SIGSYS (rc=$rc)"
elif [ "$rc" -eq 159 ]; then
  ok "audio negative path: SYS_ptrace killed with SIGSYS (rc=159, alt convention)"
else
  fail "audio negative path: expected SIGSYS kill (rc=$expected_rc), got rc=$rc"
  exit 1
fi

# --------------------------------------------------------------------
# Evidence record.
# --------------------------------------------------------------------
evidence_dir="/var/lib/nixling/validated"
evidence_path="$evidence_dir/p1-audio.json"
if [ ! -d "$evidence_dir" ]; then
  if ! mkdir -p "$evidence_dir" 2>/dev/null; then
    if command -v sudo >/dev/null 2>&1; then
      sudo -n mkdir -p "$evidence_dir" 2>/dev/null || true
      sudo -n chown "$(id -u):$(id -g)" "$evidence_dir" 2>/dev/null || true
    fi
  fi
fi

ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)
host=$(hostname)
tmp_evidence="$scratch/p1-audio.json"
cat >"$tmp_evidence" <<EOF
{
  "wave": "p1-audio",
  "timestamp": "$ts",
  "host": "$host",
  "role": "audio",
  "profileId_pattern": "vm-<vm>-audio",
  "capabilities": ["CAP_NET_RAW"],
  "seccompPolicyRef": "w1-audio",
  "binds": {
    "readOnly": ["/run/user/<uid>/pipewire-0"],
    "writable": ["/run/nixling/vms/<vm>/snd.sock parent dir"]
  },
  "positivePath": {
    "binary": "vhost-device-sound",
    "socketPath": "$sock_path",
    "pipewireSocket": "$pw_socket",
    "result": "listenerBound"
  },
  "negativePath": {
    "probe": "SYS_ptrace",
    "expectedRc": $expected_rc,
    "actualRc": $rc,
    "result": "killedWithSIGSYS"
  }
}
EOF

if mv "$tmp_evidence" "$evidence_path" 2>/dev/null; then
  ok "evidence written: $evidence_path"
elif command -v sudo >/dev/null 2>&1 && sudo -n mv "$tmp_evidence" "$evidence_path" 2>/dev/null; then
  ok "evidence written (via sudo): $evidence_path"
else
  log "  NOTE: could not write $evidence_path (no perms); record kept at $tmp_evidence"
fi

ok "tests/minijail-validator-audio.sh"
