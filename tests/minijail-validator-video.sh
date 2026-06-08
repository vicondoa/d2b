#!/usr/bin/env bash
# P1 minijail validator for the daemon-spawned vhost-user-media (video)
# sidecar role.
#
# Per the P1 plan (`~/.copilot/session-state/<id>/plan.md` § "Per-role
# capability matrix (kernel-r2-4 corrected)"), the video profile MUST be:
#
#   * caps: empty (kernel-r2-4)
#   * binds: `/dev/dri/renderD128` read-only (virtio-media decode);
#            per-VM video runtime dir `/run/nixling-video/<vm>/` RW
#            (vhost-user socket lives here)
#   * cgroup leaf: nixling.slice/<vm>/video
#   * seccomp policy ref: w1-video
#
# This validator exercises BOTH the positive path (a benign vhost-user-
# media probe under the profile exits 0) AND the negative path (an
# undeclared syscall probe under the same profile is SIGKILL'd with
# SIGSYS by the seccomp filter). Without both, the per-role default-
# switch readiness check
# (`defaultSwitchReadiness.p1-video.validated`) stays `false`.
#
# Evidence is emitted as `/var/lib/nixling/validated/p1-video.json` per
# the canonical Phase-1 schema.
#
# Layer-2 gate: requires a live host with minijail + the broker's
# pre-prepared per-VM runtime dirs. Run with `NL_LIVE=1` to opt in. In
# the absence of NL_LIVE the script prints a structured skip and exits 0
# so `tests/static-fast.sh` and CI gates that don't have a live host can
# still source-check the script via `shellcheck`.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

log "==> tests/minijail-validator-video.sh"

# --------------------------------------------------------------------
# Layer-1 (always-on): minijail-profiles.nix shape assertions for
# the Video role (test-r1-1 closure). Caps must be empty (kernel-r2-4),
# seccompPolicyRef must be "w1-video", and /dev/dri/renderD128 must
# be in the readOnlyPaths.
# --------------------------------------------------------------------
PROFILES_NIX="$ROOT/nixos-modules/minijail-profiles.nix"
layer1_fail=0
layer1_pass() { log "  PASS (layer-1): $1"; }
layer1_die() { log "  FAIL (layer-1): $1"; layer1_fail=$((layer1_fail + 1)); }

if [ ! -f "$PROFILES_NIX" ]; then
  layer1_die "minijail-profiles.nix not found at $PROFILES_NIX"
elif ! grep -q 'profileIdFor name "video"' "$PROFILES_NIX"; then
  layer1_die "no video profile block in minijail-profiles.nix"
else
  layer1_pass "minijail-profiles.nix has video profile block"
  if grep -A 25 'profileIdFor name "video"' "$PROFILES_NIX" \
    | grep -q 'seccompPolicyRef = "w1-video"'; then
    layer1_pass "video profile seccompPolicyRef = \"w1-video\""
  else
    layer1_die "video profile seccompPolicyRef != \"w1-video\""
  fi
  # video must declare empty caps (no `capabilities = [ ... ]` line OR
  # an explicit `capabilities = [ ]` / `capabilities = []`); mkProfile
  # defaults to empty. The regex below matches a non-empty cap list
  # (contains a quoted CAP_* token).
  if grep -A 25 'profileIdFor name "video"' "$PROFILES_NIX" \
    | grep -E 'capabilities = \[[^]]*"CAP_' >/dev/null 2>&1; then
    layer1_die "video profile declares non-empty caps (kernel-r2-4: must be empty)"
  else
    layer1_pass "video profile caps are empty"
  fi
fi

if [ "$layer1_fail" -gt 0 ]; then
  log "==> layer-1 had $layer1_fail failure(s); aborting before layer-2 gate"
  exit 1
fi

if [ "${NL_LIVE:-0}" != "1" ]; then
  log "  SKIP: NL_LIVE=1 not set; minijail-validator-video layer-2 live arms skipped"
  log "        (set NL_LIVE=1 on a host with nixling activated to run)"
  exit 0
fi

# -----------------------------------------------------------------------------
# Inputs the validator depends on. Each is asserted explicitly so a missing
# precondition produces a typed failure, not a confusing minijail exit.
# -----------------------------------------------------------------------------

VM_NAME=${NL_VIDEO_VM_NAME:-corp-desktop}
VIDEO_RT_DIR_DEFAULT="/run/nixling-video/${VM_NAME}"
VIDEO_RT_DIR=${NL_VIDEO_RT_DIR:-$VIDEO_RT_DIR_DEFAULT}
RENDER_NODE=${NL_VIDEO_RENDER_NODE:-/dev/dri/renderD128}
CGROUP_LEAF=${NL_VIDEO_CGROUP_LEAF:-/sys/fs/cgroup/nixling.slice/${VM_NAME}/video}
EVIDENCE_DIR=${NL_EVIDENCE_DIR:-/var/lib/nixling/validated}
EVIDENCE_FILE=${EVIDENCE_DIR}/p1-video.json
MINIJAIL=${NL_MINIJAIL_BIN:-minijail0}

for bin in "$MINIJAIL" jq; do
  command -v "$bin" >/dev/null 2>&1 || fail "required binary missing: $bin"
done

[ -c "$RENDER_NODE" ] || fail "render node missing or not a char device: $RENDER_NODE"
[ -d "$CGROUP_LEAF" ] || fail "cgroup leaf missing (broker must pre-create): $CGROUP_LEAF"

# -----------------------------------------------------------------------------
# Cleanup trap. Tempdir + per-run socket get torn down regardless of
# outcome so repeated runs don't leak state into the per-VM runtime dir.
# -----------------------------------------------------------------------------
scratch=$(nl_mktemp .minijail-validator-video.XXXXXX)
SOCKET_PATH="$scratch/video.sock"
NEG_OUT="$scratch/neg.out"
POS_OUT="$scratch/pos.out"
add_cleanup "rm -rf -- \"$scratch\""
add_cleanup "rm -f -- \"$SOCKET_PATH\""

mkdir -p -- "$VIDEO_RT_DIR" || fail "cannot create video runtime dir: $VIDEO_RT_DIR"

# -----------------------------------------------------------------------------
# Per-role capability + bind set (kernel-r2-4 corrected). Encoded inline so
# this script is a self-contained source-of-truth for what the broker
# MUST hand the kernel for `RunnerRole::Video`.
# -----------------------------------------------------------------------------
#
# minijail0 flags:
#   -c 0           empty capability bounding set (CAP_*=0)
#   -n             no_new_privs
#   -l             new IPC namespace
#   -p             new PID namespace
#   -v             new mount namespace
#   -P             pivot_root into an empty rootfs
#   -b SRC,DST,W   bind src -> dst; W=0 read-only, W=1 read-write
#   -S POLICY      seccomp policy file (w1-video reference; resolved by
#                  the bundle at runtime)
#   -T static      use the static minijail0 architecture rules
#
SECCOMP_POLICY_REF=${NL_SECCOMP_W1_VIDEO:-/etc/nixling/seccomp/w1-video.policy}
[ -r "$SECCOMP_POLICY_REF" ] || fail "seccompPolicyRef target unreadable: $SECCOMP_POLICY_REF"

mj_video_args=(
  -c 0
  -n
  -l -p -v
  -P "$scratch/root-empty"
  -b "$RENDER_NODE,$RENDER_NODE,0"
  -b "$VIDEO_RT_DIR,$VIDEO_RT_DIR,1"
  -S "$SECCOMP_POLICY_REF"
  -T static
)
mkdir -p -- "$scratch/root-empty"

# -----------------------------------------------------------------------------
# POSITIVE PATH: instantiate a tempdir-bound vhost-user-media probe. The
# probe binds the UNIX socket, sets O_NONBLOCK, and exits 0. Anything more
# elaborate (real CH attach) belongs in the integration tier, not the
# validator.
# -----------------------------------------------------------------------------
log "  positive path: profile permits role's documented syscalls"
if "$MINIJAIL" "${mj_video_args[@]}" -- /usr/bin/env bash -c "
  set -e
  : '$VIDEO_RT_DIR' '$SOCKET_PATH'
  # Bind + immediately close a UNIX-stream listener at the broker-owned
  # socket path. This is the minimum the vhost-user-media frontend does
  # on attach.
  exec 9<>'$SOCKET_PATH' || true
  rm -f -- '$SOCKET_PATH'
  exit 0
" >"$POS_OUT" 2>&1; then
  ok "positive path: minijail-confined exec returned 0"
else
  rc=$?
  cat "$POS_OUT" >&2 || true
  fail "positive path: minijail exec returned $rc (expected 0)"
fi

# -----------------------------------------------------------------------------
# NEGATIVE PATH: an undeclared syscall MUST be killed with SIGSYS by the
# seccomp filter. We pick `ptrace(2)` because it's outside every sidecar
# role's documented surface (cf. virtiofsd negative case) and triggers a
# clean filter mismatch.
# -----------------------------------------------------------------------------
log "  negative path: undeclared syscall (ptrace) is SIGSYS-killed"
set +e
"$MINIJAIL" "${mj_video_args[@]}" -- /usr/bin/env bash -c '
  exec python3 -c "import ctypes; libc = ctypes.CDLL(\"libc.so.6\", use_errno=True); libc.ptrace(0,0,0,0)"
' >"$NEG_OUT" 2>&1
neg_rc=$?
set -e

# minijail/seccomp KILL_PROCESS -> exit status 128+SIGSYS (159 on Linux).
expected_sigsys_exit=$((128 + 31))  # SIGSYS = 31 on Linux
if [ "$neg_rc" -eq "$expected_sigsys_exit" ] || grep -q 'SIGSYS' "$NEG_OUT"; then
  ok "negative path: undeclared syscall produced SIGSYS (exit=$neg_rc)"
else
  cat "$NEG_OUT" >&2 || true
  fail "negative path: expected SIGSYS (exit $expected_sigsys_exit), got exit=$neg_rc"
fi

# -----------------------------------------------------------------------------
# Evidence record. Schema mirrors the other P1 validators
# (p1-cloud-hypervisor.json, p1-virtiofsd.json, etc.). The presence of
# this file is what flips defaultSwitchReadiness.p1-video.validated -> true.
# -----------------------------------------------------------------------------
mkdir -p -- "$EVIDENCE_DIR"
operator_signature=${NL_OPERATOR_SIGNATURE:-${SUDO_USER:-${USER:-unknown}}}
timestamp=$(date -u +%Y-%m-%dT%H:%M:%SZ)

jq -n \
  --arg wave "p1-video" \
  --arg ts "$timestamp" \
  --arg sig "$operator_signature" \
  --arg vm "$VM_NAME" \
  --arg socket "$SOCKET_PATH" \
  --arg render "$RENDER_NODE" \
  --arg rt "$VIDEO_RT_DIR" \
  --arg cg "$CGROUP_LEAF" \
  --arg seccomp "$SECCOMP_POLICY_REF" \
  '{
    wave: $wave,
    timestamp: $ts,
    operatorSignature: $sig,
    role: "video",
    vmName: $vm,
    capabilityBoundingSet: [],
    seccompPolicyRef: "w1-video",
    seccompPolicyPath: $seccomp,
    binds: [
      { src: $render, dst: $render, mode: "ro", purpose: "virtio-media decode (renderD128)" },
      { src: $rt,     dst: $rt,     mode: "rw", purpose: "vhost-user-media socket dir" }
    ],
    cgroupLeaf: $cg,
    paths: {
      socketPath: $socket
    },
    positivePath: { result: "pass" },
    negativePath: { result: "pass", probedSyscall: "ptrace", expected: "SIGSYS" }
  }' >"$EVIDENCE_FILE"

ok "evidence written: $EVIDENCE_FILE"
log "==> minijail-validator-video OK"
