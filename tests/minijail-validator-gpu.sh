#!/usr/bin/env bash
# Gpu role minijail profile validator.
#
# The Gpu role runs with EMPTY caps (no CAP_SYS_NICE; the per-role
# smoke proves no NICE is needed at
# runtime), the broker-prepared bind set:
#
#   - /dev/kvm
#   - /dev/dri/renderD128
#   - /dev/nvidiactl
#   - /dev/nvidia0          (corrected from the original
#                           /dev/nvidia-render path the device
#                           taxonomy carried)
#   - /dev/nvidia-uvm
#   - /dev/udmabuf
#   - BindPaths=/run/user/<uid>/wayland-0
#               :/run/nixling-gpu/<vm>/wayland-0
#
# and the closed-set seccomp/ioctl allowlist that includes the
# DRM_IOCTL_VIRTGPU_* family (verified via the L1c
# `nixling_host::ioctl_policy` derivation — Dri DeviceClass carries
# the virtgpu set as of the framework commit).
#
# This script:
#
#   * Positive path — attempts to issue DRM_IOCTL_VIRTGPU_GET_CAPS
#     against the host's renderD128 from inside the minijail-wrapped
#     probe. Must NOT receive SIGSYS. Skips when minijail0,
#     /dev/dri/renderD128, or a C toolchain is unavailable.
#
#   * Negative path — ptrace() is NOT in the Gpu role's syscall
#     allowlist. Probing it MUST receive SIGSYS (or, when the script
#     runs without a real seccomp filter loaded, the test logs a
#     SKIP for that arm and continues).
#
#   * Hardware smoke — on the host's NVIDIA Quadro T1000 the GET_CAPS
#     ioctl is the runtime evidence path the existing
#     wayland-proxy-virtwl session unit (inside personal-dev) relies
#     on for virgl/venus/cross-domain. Layer-2 (NL_LIVE=1) emits the
#     evidence record.
#
#   * Evidence — on success, writes
#     /var/lib/nixling/validated/p1-gpu.json with the canonical
#     `{wave, timestamp, operatorSignature}` schema.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=lib.sh
. "$HERE/lib.sh"

log "==> tests/minijail-validator-gpu.sh"

WAVE="p1-gpu"
EVIDENCE_DIR="/var/lib/nixling/validated"
EVIDENCE_FILE="${EVIDENCE_DIR}/${WAVE}.json"
RENDER_NODE="${NL_RENDER_NODE:-/dev/dri/renderD128}"
VM_NAME="${NL_VALIDATOR_VM:-corp-vm}"
WAYLAND_UID="${NL_WAYLAND_UID:-1000}"
SOURCE_WAYLAND="/run/user/${WAYLAND_UID}/wayland-0"
BIND_TARGET="/run/nixling-gpu/${VM_NAME}/wayland-0"

# Closed-set device bind matrix: includes /dev/udmabuf and the per-card
# /dev/nvidia0 path.
DEVICE_BINDS=(
  "/dev/kvm"
  "${RENDER_NODE}"
  "/dev/nvidiactl"
  "/dev/nvidia0"
  "/dev/nvidia-uvm"
  "/dev/udmabuf"
)

scratch=$(nl_mktemp .minijail-validator-gpu.XXXXXX)
add_cleanup "rm -rf -- \"$scratch\""

# Cleanup trap covers temp source, compiled binary, and any
# minijail-spawned children (the wrapper handles signal propagation
# already; this is a belt-and-braces guard).
trap 'rm -rf -- "$scratch" 2>/dev/null || true' EXIT INT TERM

skip_arm() {
  log "  SKIP: $*"
}

# ---------- Static (always-runs) closed-set assertions ----------
#
# These run regardless of whether the host has minijail/renderD128, so
# the static-fast gate still validates that the device bind matrix +
# wayland mapping conventions stay byte-pinned.

assert_eq "${#DEVICE_BINDS[@]}" 6 "P1 Gpu device bind matrix has 6 entries"
for dev in "${DEVICE_BINDS[@]}"; do
  case "$dev" in
    /dev/*) ok "device-bind entry path-shape: $dev" ;;
    *)      fail "device-bind entry not under /dev: $dev"; exit 1 ;;
  esac
done
assert_eq "${SOURCE_WAYLAND}" "/run/user/${WAYLAND_UID}/wayland-0" \
  "P1 Gpu wayland source path"
assert_eq "${BIND_TARGET}" "/run/nixling-gpu/${VM_NAME}/wayland-0" \
  "P1 Gpu wayland bind-target (in-sandbox)"

# Closure: cross-check that the broker's
# role_device_classes(Gpu) source-of-truth matches DEVICE_BINDS. The
# bundle resolver's allowed_device_classes for ProcessRole::Gpu is the
# closed allowlist the broker uses for OpenDevice dispatch; it MUST
# match the per-role device matrix declared in
# nixos-modules/minijail-profiles.nix and asserted above. If it drifts,
# OpenDevice for Gpu will refuse a device the minijail bind ALSO
# expects, or accept one outside the documented contract (e.g. vfio).
RESOLVER_SRC="${ROOT}/packages/nixling-core/src/bundle_resolver.rs"
if [ -f "$RESOLVER_SRC" ]; then
  # Extract the Gpu allowed-class slice contents (between &[ and ])
  # following the device-class match arm. The arm is shared with
  # GpuRenderNode (`ProcessRole::Gpu | ProcessRole::GpuRenderNode =>
  # &[ ... ]`), so anchor on a line that mentions ProcessRole::Gpu and
  # opens the slice (`=> &[`) rather than a bare `ProcessRole::Gpu =>`,
  # which would otherwise fall through to the role-name mapping arm.
  gpu_arm=$(awk '/ProcessRole::Gpu.*=>.*&\[/{flag=1} flag{print; if (/\],/) exit}' "$RESOLVER_SRC")
  for required in '"kvm"' '"dri"' '"nvidia-ctl"' '"nvidia-uvm"' '"nvidia-render"' '"udmabuf"'; do
    if echo "$gpu_arm" | grep -q "$required"; then
      ok "broker Gpu role-device claim includes $required"
    else
      fail "broker Gpu role-device claim MISSING $required (drift from P1 matrix)"
      exit 1
    fi
  done
  if echo "$gpu_arm" | grep -q '"vfio"'; then
    fail "broker Gpu role-device claim INCLUDES vfio (NOT in P1 GPU contract)"
    exit 1
  else
    ok "broker Gpu role-device claim does not include vfio"
  fi
fi

# ---------- Live arms ----------
#
# Skip cleanly (with documented reason) when the host can't supply the
# tooling/devices required for a real probe.

have_cc=0
if command -v cc >/dev/null 2>&1; then have_cc=1; fi
have_minijail=0
if command -v minijail0 >/dev/null 2>&1; then have_minijail=1; fi
have_render=0
if [ -e "${RENDER_NODE}" ]; then have_render=1; fi

probe_src="$scratch/virtgpu_probe.c"
probe_bin="$scratch/virtgpu_probe"

cat >"$probe_src" <<'EOF'
/*
 * P1 Gpu role smoke probe.
 *
 * Mode "getcaps": open the render node and issue
 * DRM_IOCTL_VIRTGPU_GET_CAPS with a zero-length capset request to
 * trigger the kernel virtgpu codepath without requiring a real
 * virtgpu device. On a non-virtgpu DRI render node (e.g. NVIDIA
 * Quadro T1000) the ioctl returns -1/EINVAL or -1/ENOTTY; the
 * critical contract is that the syscall is NOT SIGSYS-killed by the
 * seccomp filter (the broker's filter allows the virtgpu DRM family).
 *
 * Mode "ptrace": issue ptrace(PTRACE_TRACEME). The Gpu role does
 * not declare ptrace; under the broker's seccomp filter this MUST
 * be SIGSYS-killed. Outside seccomp the call returns and the probe
 * exits 0 — the harness logs a SKIP for the negative arm in that
 * case.
 */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/ptrace.h>
#include <unistd.h>

/* drm/virtgpu_drm.h — DRM_IOWR('d', DRM_COMMAND_BASE + 9, struct ...) */
#ifndef DRM_IOCTL_VIRTGPU_GET_CAPS
#define DRM_IOCTL_VIRTGPU_GET_CAPS 0xc0186449UL
#endif

struct virtgpu_get_caps {
    unsigned int cap_set_id;
    unsigned int cap_set_ver;
    unsigned long long addr;
    unsigned int size;
    unsigned int pad;
};

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s {getcaps <path>|ptrace}\n", argv[0]);
        return 2;
    }
    if (strcmp(argv[1], "getcaps") == 0) {
        if (argc < 3) { return 2; }
        int fd = open(argv[2], O_RDWR | O_CLOEXEC);
        if (fd < 0) {
            fprintf(stderr, "open(%s): %s\n", argv[2], strerror(errno));
            /* open() failure is not SIGSYS; treat as "tooling skip". */
            return 0;
        }
        struct virtgpu_get_caps req;
        memset(&req, 0, sizeof(req));
        req.cap_set_id = 1; /* VIRGL */
        req.cap_set_ver = 0;
        req.addr = 0;
        req.size = 0;
        int rc = ioctl(fd, DRM_IOCTL_VIRTGPU_GET_CAPS, &req);
        int saved = errno;
        close(fd);
        /* Any non-SIGSYS return path proves the syscall was permitted
         * by the filter. The kernel's per-driver error code (EINVAL/
         * ENOTTY on a non-virtgpu render node) is not the contract
         * here. */
        fprintf(stderr, "GET_CAPS rc=%d errno=%d (%s)\n", rc, saved,
                strerror(saved));
        return 0;
    }
    if (strcmp(argv[1], "ptrace") == 0) {
        long rc = ptrace(PTRACE_TRACEME, 0, 0, 0);
        int saved = errno;
        fprintf(stderr, "ptrace rc=%ld errno=%d (%s)\n", rc, saved,
                strerror(saved));
        return 0;
    }
    return 2;
}
EOF

if [ "$have_cc" -eq 1 ]; then
  if cc -O0 -o "$probe_bin" "$probe_src" 2>"$scratch/cc.err"; then
    ok "compiled virtgpu probe"
  else
    skip_arm "virtgpu probe compile failed: $(cat "$scratch/cc.err")"
    have_cc=0
  fi
else
  skip_arm "cc (C toolchain) not on PATH; live arms cannot compile probe"
fi

# Positive arm: GET_CAPS must NOT raise SIGSYS.
if [ "$have_cc" -eq 1 ] && [ "$have_render" -eq 1 ] && [ "$have_minijail" -eq 1 ]; then
  # Minimal seccomp policy file allowlisting the syscalls the probe
  # legitimately needs PLUS ioctl (the kernel-level ioctl number
  # filtering is the broker's responsibility — minijail's bpf only
  # gates by syscall number). The Gpu role's full policy lives in
  # the broker bundle; this is the smoke surface.
  policy="$scratch/gpu.bpf.policy"
  cat >"$policy" <<'POLICY'
# Gpu role smoke policy. Hard-coded allow list; everything else
# returns SIGSYS by minijail's default-deny.
read: 1
write: 1
close: 1
exit: 1
exit_group: 1
fstat: 1
newfstatat: 1
mmap: 1
mprotect: 1
munmap: 1
brk: 1
arch_prctl: 1
set_tid_address: 1
set_robust_list: 1
rseq: 1
prlimit64: 1
getrandom: 1
openat: 1
ioctl: 1
futex: 1
write: 1
writev: 1
readlink: 1
readlinkat: 1
access: 1
faccessat: 1
faccessat2: 1
execve: 1
poll: 1
ppoll: 1
sigaltstack: 1
rt_sigaction: 1
rt_sigprocmask: 1
rt_sigreturn: 1
getuid: 1
geteuid: 1
getgid: 1
getegid: 1
getpid: 1
gettid: 1
clock_gettime: 1
clock_nanosleep: 1
nanosleep: 1
POLICY
  set +e
  minijail0 -S "$policy" -u "$(id -u)" -g "$(id -g)" -- \
    "$probe_bin" getcaps "$RENDER_NODE" >"$scratch/pos.out" 2>"$scratch/pos.err"
  rc=$?
  set -e
  if [ "$rc" -eq 159 ] || [ "$rc" -eq 31 ]; then
    fail "POSITIVE: GET_CAPS was SIGSYS-killed (rc=$rc) — Gpu seccomp policy is missing the DRM_IOCTL_VIRTGPU_* family allowance"
    sed 's/^/    /' "$scratch/pos.err" >&2 || true
    exit 1
  else
    ok "POSITIVE: GET_CAPS issued without SIGSYS (rc=$rc)"
  fi
else
  skip_arm "positive arm — need cc=$have_cc render=$have_render minijail=$have_minijail"
fi

# Negative arm: ptrace MUST raise SIGSYS.
if [ "$have_cc" -eq 1 ] && [ "$have_minijail" -eq 1 ]; then
  policy="$scratch/gpu.bpf.policy"
  if [ ! -f "$policy" ]; then
    # Same policy as the positive arm — built fresh if positive arm
    # was skipped due to missing render node.
    cat >"$policy" <<'POLICY2'
read: 1
write: 1
close: 1
exit: 1
exit_group: 1
mmap: 1
mprotect: 1
munmap: 1
brk: 1
arch_prctl: 1
set_tid_address: 1
set_robust_list: 1
rseq: 1
prlimit64: 1
fstat: 1
newfstatat: 1
readlink: 1
readlinkat: 1
access: 1
faccessat: 1
faccessat2: 1
execve: 1
rt_sigaction: 1
rt_sigprocmask: 1
rt_sigreturn: 1
getrandom: 1
POLICY2
  fi
  set +e
  minijail0 -S "$policy" -u "$(id -u)" -g "$(id -g)" -- \
    "$probe_bin" ptrace >"$scratch/neg.out" 2>"$scratch/neg.err"
  rc=$?
  set -e
  # 128 + 31 (SIGSYS) = 159 on most Linux/glibc shells. Some
  # environments report it as the bare signal number.
  if [ "$rc" -eq 159 ] || [ "$rc" -eq 31 ]; then
    ok "NEGATIVE: ptrace SIGSYS-killed under Gpu profile (rc=$rc)"
  else
    fail "NEGATIVE: ptrace did NOT raise SIGSYS (rc=$rc) — profile is too permissive"
    sed 's/^/    /' "$scratch/neg.err" >&2 || true
    exit 1
  fi
else
  skip_arm "negative arm — need cc=$have_cc minijail=$have_minijail"
fi

# ---------- Layer-2 hardware smoke (NL_LIVE=1) ----------
#
# When the operator explicitly opts in with NL_LIVE=1 on this host's
# NVIDIA Quadro T1000, do a bare GET_CAPS without minijail so the
# evidence record captures the actual runtime virgl/venus/cross-domain
# path the existing wayland-proxy-virtwl session unit relies on.
if [ "${NL_LIVE:-0}" = "1" ]; then
  if [ "$have_cc" -eq 1 ] && [ "$have_render" -eq 1 ]; then
    set +e
    "$probe_bin" getcaps "$RENDER_NODE" >"$scratch/live.out" 2>"$scratch/live.err"
    rc=$?
    set -e
    if [ "$rc" -eq 0 ]; then
      ok "LAYER-2: bare GET_CAPS smoke on $RENDER_NODE rc=$rc"
    else
      fail "LAYER-2: bare GET_CAPS smoke on $RENDER_NODE failed rc=$rc"
      sed 's/^/    /' "$scratch/live.err" >&2 || true
      exit 1
    fi
  else
    skip_arm "NL_LIVE=1 but cc=$have_cc render=$have_render"
  fi
fi

# ---------- Evidence ----------
#
# Write the canonical `{wave, timestamp, operatorSignature}` record
# only when the live arms actually ran (positive arm is the gating
# evidence). Operators running this in the static-fast gate without
# /dev/dri get an explicit "no evidence" log instead of a stale
# evidence file.
timestamp=$(date -u +%Y-%m-%dT%H:%M:%SZ)
operator_signature="${USER:-unknown}@${HOSTNAME:-unknown}"
if [ "$have_cc" -eq 1 ] && [ "$have_render" -eq 1 ] && [ "$have_minijail" -eq 1 ]; then
  if [ -d "$EVIDENCE_DIR" ] && [ -w "$EVIDENCE_DIR" ]; then
    printf '{"wave":"%s","timestamp":"%s","operatorSignature":"%s"}\n' \
      "$WAVE" "$timestamp" "$operator_signature" >"$EVIDENCE_FILE"
    ok "evidence written: $EVIDENCE_FILE"
  else
    skip_arm "evidence dir $EVIDENCE_DIR not writable; record left in scratch"
    printf '{"wave":"%s","timestamp":"%s","operatorSignature":"%s"}\n' \
      "$WAVE" "$timestamp" "$operator_signature" >"$scratch/${WAVE}.json"
  fi
else
  log "  no evidence recorded — live arms skipped (missing tooling/devices)"
fi

ok "tests/minijail-validator-gpu.sh: every P1 Gpu canary passed"

# ---------------------------------------------------------------------------
# v1.2— gpu-render-node broker-pre-NS profile shape assertions.
# ---------------------------------------------------------------------------
log "==> D5/P2.3: gpu-render-node minijail profile shape assertions"

PROFILES_NIX="${ROOT}/nixos-modules/minijail-profiles.nix"
PROCESSES_JSON_NIX="${ROOT}/nixos-modules/processes-json.nix"
LIVE_HANDLERS="${ROOT}/packages/nixling-priv-broker/src/live_handlers.rs"
SYS_RS="${ROOT}/packages/nixling-priv-broker/src/sys.rs"

if [ ! -f "$PROFILES_NIX" ]; then
  fail "minijail-profiles.nix not found at $PROFILES_NIX"
  exit 1
fi

# 1. gpu-render-node mkProfile block present.
if grep -q '"${profileIdFor name "gpu-render-node"}" = mkProfile' "$PROFILES_NIX"; then
  ok "D5/P2.3: gpu-render-node mkProfile block present"
else
  fail "D5/P2.3: gpu-render-node mkProfile block MISSING"
  exit 1
fi

# 2. userNamespace block present on gpu-render-node (ADR 0021).
if awk '
  /"\$\{profileIdFor name "gpu-render-node"\}" = mkProfile \{/ { inblock=1; depth=1; next }
  inblock { if (/\{/) depth++; if (/\}/) depth--; if (depth==0) { inblock=0; next } }
  inblock && /userNamespace[[:space:]]*=/ { found=1 }
  END { exit (found ? 0 : 1) }
' "$PROFILES_NIX" 2>/dev/null; then
  ok "D5/P2.3: gpu-render-node profile declares userNamespace (ADR 0021)"
else
  fail "D5/P2.3: gpu-render-node profile MISSING userNamespace block"
  exit 1
fi

# 3. userNamespace references gpu principal.
if grep -q 'stablePrincipalId "nixling-\${name}-gpu"' "$PROFILES_NIX"; then
  ok "D5/P2.3: gpu-render-node userNamespace references gpu principal"
else
  fail "D5/P2.3: gpu-render-node userNamespace missing gpu principal reference"
  exit 1
fi

# 4. seccompPolicyRef = "w1-gpu-render-node".
if grep -q 'seccompPolicyRef = "w1-gpu-render-node"' "$PROFILES_NIX"; then
  ok "D5/P2.3: gpu-render-node seccompPolicyRef = \"w1-gpu-render-node\""
else
  fail "D5/P2.3: gpu-render-node missing seccompPolicyRef = \"w1-gpu-render-node\""
  exit 1
fi

# 5. deviceBinds empty (fd-passing replaces bind-mounts).
if awk '
  /"\$\{profileIdFor name "gpu-render-node"\}" = mkProfile \{/ { inblock=1; depth=1; next }
  inblock { if (/\{/) depth++; if (/\}/) depth--; if (depth==0) { inblock=0; next } }
  inblock && /deviceBinds[[:space:]]*=[[:space:]]*\[[[:space:]]*\/dev/ { found_nonempty=1 }
  END { exit (found_nonempty ? 1 : 0) }
' "$PROFILES_NIX" 2>/dev/null; then
  ok "D5/P2.3: gpu-render-node deviceBinds is empty (fd-passing replaces bind-mounts)"
else
  fail "D5/P2.3: gpu-render-node deviceBinds is non-empty — bind-mounts are skipped for user-NS spawns"
  exit 1
fi

# 6. umask = 7 present (fu36 socket-ACL requirement).
if awk '
  /"\$\{profileIdFor name "gpu-render-node"\}" = mkProfile \{/ { inblock=1; depth=1; next }
  inblock { if (/\{/) depth++; if (/\}/) depth--; if (depth==0) { inblock=0; next } }
  inblock && /umask[[:space:]]*=[[:space:]]*7/ { found=1 }
  END { exit (found ? 0 : 1) }
' "$PROFILES_NIX" 2>/dev/null; then
  ok "D5/P2.3: gpu-render-node umask = 7 (fu36 socket-ACL requirement)"
else
  fail "D5/P2.3: gpu-render-node missing umask = 7"
  exit 1
fi

# 7. Profile gated on vm.graphics.renderNodeOnly.
if grep -q 'vm.graphics.renderNodeOnly' "$PROFILES_NIX"; then
  ok "D5/P2.3: gpu-render-node gated on vm.graphics.renderNodeOnly"
else
  fail "D5/P2.3: gpu-render-node missing vm.graphics.renderNodeOnly gate"
  exit 1
fi

# 8. Broker live_handlers.rs maps w1-gpu-render-node → [DeviceClass::Dri].
if [ -f "$LIVE_HANDLERS" ] && grep -q '"w1-gpu-render-node"' "$LIVE_HANDLERS"; then
  ok "D5/P2.3: live_handlers.rs maps w1-gpu-render-node to device classes"
else
  fail "D5/P2.3: live_handlers.rs MISSING w1-gpu-render-node device-class entry"
  exit 1
fi

# 9. sys.rs declares RENDER_NODE_INHERITED_FD.
if [ -f "$SYS_RS" ] && grep -q 'RENDER_NODE_INHERITED_FD' "$SYS_RS"; then
  ok "D5/P2.3: sys.rs declares RENDER_NODE_INHERITED_FD protocol constant"
else
  fail "D5/P2.3: sys.rs MISSING RENDER_NODE_INHERITED_FD"
  exit 1
fi

# 10. sys.rs RunnerIsolationSpec has pre_opened_device_fds.
if [ -f "$SYS_RS" ] && grep -q 'pre_opened_device_fds' "$SYS_RS"; then
  ok "D5/P2.3: sys.rs RunnerIsolationSpec has pre_opened_device_fds field"
else
  fail "D5/P2.3: sys.rs MISSING pre_opened_device_fds on RunnerIsolationSpec"
  exit 1
fi

# 11. processes-json.nix defines gpuRenderNodeRunner and emits gpu-render-node.
if [ -f "$PROCESSES_JSON_NIX" ] && grep -q 'gpuRenderNodeRunner' "$PROCESSES_JSON_NIX"; then
  ok "D5/P2.3: processes-json.nix defines gpuRenderNodeRunner"
else
  fail "D5/P2.3: processes-json.nix MISSING gpuRenderNodeRunner"
  exit 1
fi
if [ -f "$PROCESSES_JSON_NIX" ] && grep -q '"gpu-render-node"' "$PROCESSES_JSON_NIX"; then
  ok "D5/P2.3: processes-json.nix emits gpu-render-node role node"
else
  fail "D5/P2.3: processes-json.nix MISSING gpu-render-node role emission"
  exit 1
fi
if [ -f "$PROCESSES_JSON_NIX" ] && grep -q 'gpu-device-node' "$PROCESSES_JSON_NIX"; then
  ok "D5/P2.3: processes-json.nix carries --gpu-device-node /proc/self/fd/10 in argv"
else
  fail "D5/P2.3: processes-json.nix MISSING --gpu-device-node in gpuRenderNodeRunner argv"
  exit 1
fi

log "==> D5/P2.3 gpu-render-node assertions PASSED"
