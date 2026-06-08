#!/usr/bin/env bash
# tests/minijail-validator-virtiofsd.sh
#
# P1 per-role minijail validator for the Virtiofsd sidecar role.
#
# Two layers:
#
#   Layer 1 (always):
#     - Asserts the Virtiofsd minijail profile shape in
#       nixos-modules/minijail-profiles.nix matches the documented
#       ADR 0003 W3 startup carve-out exactly. Per the kernel-r2-4
#       corrected per-role cap matrix, Virtiofsd's *steady-state*
#       capability set is empty, but the --sandbox=namespace setup
#       carve-out requires the closed set:
#
#         CAP_SYS_ADMIN, CAP_SETPCAP, CAP_CHOWN, CAP_FOWNER,
#         CAP_FSETID, CAP_SETUID, CAP_SETGID, CAP_DAC_OVERRIDE,
#         CAP_MKNOD, CAP_SETFCAP
#
#       The profile MUST also be tagged with the
#       'virtiofsdRootException' adr_carve_out marker so foreign
#       drift can be caught fail-closed.
#     - Asserts no host-installed minijail-profile JSONs under
#       /etc/nixling/minijail-profiles/ drift from the same shape
#       (skipped silently if no profiles are installed on this host).
#
#   Layer 2 (NL_LIVE=1):
#     - Positive: exec `virtiofsd --version` under minijail0 with the
#       documented W3 carve-out profile; assert exit 0.
#     - Negative: virtiofsd does NOT use ptrace under any role
#       contract. Probe ptrace under the same profile; assert
#       SIGSYS (exit status 159 = 128 + 31).
#
# Both Layer-2 paths are required to write the per-role evidence
# record at /var/lib/nixling/validated/p1-virtiofsd.json.
#
# Schema of the evidence record (per plan Phase 1):
#
#   { "wave": "p1-virtiofsd",
#     "timestamp": "<RFC-3339 UTC>",
#     "operatorSignature": "<sha256 placeholder>" }
#
# This validator is shell-syntax + shellcheck (--severity=warning)
# clean.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

PROFILE_NIX="$ROOT/nixos-modules/minijail-profiles.nix"
EVIDENCE_PATH=${NL_VALIDATED_DIR:-/var/lib/nixling/validated}/p1-virtiofsd.json

# Canonical, ordered cap set from the W3 startup carve-out
# (ADR 0003). Order matches nixos-modules/minijail-profiles.nix so
# any future drift surfaces against the exact source array.
EXPECTED_CAPS=(
  CAP_SYS_ADMIN
  CAP_SETPCAP
  CAP_CHOWN
  CAP_FOWNER
  CAP_FSETID
  CAP_SETUID
  CAP_SETGID
  CAP_DAC_OVERRIDE
  CAP_MKNOD
  CAP_SETFCAP
)

EXPECTED_CARVE_OUT="ADR 0003 virtiofsd --sandbox=namespace setup exception"

# Cleanup trap state — used by Layer 2 to undo any tempdir/socket
# tampering even on early failure.
TMP_WORK=""
cleanup() {
  local rc=$?
  if [ -n "$TMP_WORK" ] && [ -d "$TMP_WORK" ]; then
    rm -rf "$TMP_WORK" || true
  fi
  exit "$rc"
}
trap cleanup EXIT INT TERM

log() { printf '[p1-virtiofsd] %s\n' "$*" >&2; }
fail() { printf '[p1-virtiofsd] FAIL: %s\n' "$*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Layer 1: source-of-truth profile shape assertions
# ---------------------------------------------------------------------------

assert_profile_source() {
  [ -f "$PROFILE_NIX" ] || fail "missing $PROFILE_NIX"

  # Carve-out marker: must appear in the canonical exception string
  # and must be referenced as both exceptionRef and adr_carve_out
  # on the virtiofsd profile.
  grep -qF "$EXPECTED_CARVE_OUT" "$PROFILE_NIX" \
    || fail "virtiofsdRootException string '$EXPECTED_CARVE_OUT' not found in $PROFILE_NIX"

  grep -qE 'exceptionRef[[:space:]]*=[[:space:]]*virtiofsdRootException' "$PROFILE_NIX" \
    || fail "virtiofsd profile is missing exceptionRef = virtiofsdRootException"
  grep -qE 'adr_carve_out[[:space:]]*=[[:space:]]*virtiofsdRootException' "$PROFILE_NIX" \
    || fail "virtiofsd profile is missing adr_carve_out = virtiofsdRootException"

  # Extract the virtiofsd profile's mkProfile body. We anchor on
  # `role = "virtiofsd";` and stop after the closing `adr_carve_out`
  # line which is the last attribute in the profile.
  local block
  block=$(awk '
    /role[[:space:]]*=[[:space:]]*"virtiofsd";/ { active=1 }
    active { print }
    active && /adr_carve_out[[:space:]]*=[[:space:]]*virtiofsdRootException;/ { exit }
  ' "$PROFILE_NIX")

  [ -n "$block" ] || fail "could not locate virtiofsd profile block in $PROFILE_NIX"

  local cap
  for cap in "${EXPECTED_CAPS[@]}"; do
    printf '%s' "$block" | grep -qE "\"${cap}\"" \
      || fail "virtiofsd profile missing expected cap: $cap"
  done

  # Drift guard: count quoted CAP_* tokens inside the block; must
  # equal the expected closed-set size. Catches accidental additions.
  local found_count
  found_count=$(printf '%s' "$block" \
    | grep -oE '"CAP_[A-Z_]+"' \
    | sort -u \
    | wc -l)
  if [ "$found_count" -ne "${#EXPECTED_CAPS[@]}" ]; then
    fail "virtiofsd profile cap set drift: expected ${#EXPECTED_CAPS[@]} caps, found $found_count"
  fi

  # requiresStartRoot must be true — that's the whole point of the
  # ADR 0003 carve-out.
  printf '%s' "$block" | grep -qE 'requiresStartRoot[[:space:]]*=[[:space:]]*true' \
    || fail "virtiofsd profile must declare requiresStartRoot = true (ADR 0003 carve-out)"

  # Steady-state seccomp policy reference must be the closed
  # w1-virtiofsd allowlist.
  printf '%s' "$block" | grep -qE 'seccompPolicyRef[[:space:]]*=[[:space:]]*"w1-virtiofsd"' \
    || fail "virtiofsd profile missing seccompPolicyRef = \"w1-virtiofsd\""

  log "Layer-1: source profile shape OK (${#EXPECTED_CAPS[@]} caps, carve-out tagged)"
}

assert_installed_profiles() {
  local dir=/etc/nixling/minijail-profiles
  if ! [ -d "$dir" ]; then
    log "Layer-1: no host-installed minijail profiles at $dir (skipping live drift check)"
    return 0
  fi

  local installed
  installed=$(find "$dir" -maxdepth 1 -type f -name 'vm-*-virtiofsd-*.json' 2>/dev/null || true)
  if [ -z "$installed" ]; then
    log "Layer-1: no installed virtiofsd profile JSONs (skipping)"
    return 0
  fi

  command -v jq >/dev/null 2>&1 || fail "jq required to inspect installed profiles"

  local f
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    local role carve caps_json
    role=$(jq -r '.role // empty' "$f")
    carve=$(jq -r '.adr_carve_out // empty' "$f")
    caps_json=$(jq -c '.capabilities // []' "$f")

    [ "$role" = "virtiofsd" ] \
      || fail "$f: role != virtiofsd (got '$role')"
    [ "$carve" = "$EXPECTED_CARVE_OUT" ] \
      || fail "$f: adr_carve_out drift (got '$carve')"

    # Build expected JSON array literal.
    local expected_json
    expected_json=$(printf '%s\n' "${EXPECTED_CAPS[@]}" | jq -R . | jq -sc .)
    [ "$caps_json" = "$expected_json" ] \
      || fail "$f: capabilities drift; expected $expected_json got $caps_json"
  done <<<"$installed"

  log "Layer-1: installed minijail profile(s) match documented carve-out"
}

# ---------------------------------------------------------------------------
# Layer 2: live execution under minijail0 (NL_LIVE=1)
# ---------------------------------------------------------------------------

# Build a minijail0 argv encoding the W3 startup carve-out cap mask
# plus the closed set of namespaces virtiofsd actually needs.
# This matches the role contract — namespace mount + ipc, no net,
# no new privs.
build_minijail_argv() {
  local out_var=$1
  local -a argv

  # CAP mask bits — minijail0 accepts a hex bitmask via -c.
  # We compute the mask from the named caps via capsh --decode/--encode,
  # falling back to passing each cap individually if --ambient isn't
  # supported on this kernel.
  if ! command -v minijail0 >/dev/null 2>&1; then
    fail "minijail0 not found in PATH (required for Layer 2)"
  fi

  argv=(
    minijail0
    -n               # no_new_privs
    -p               # new pid ns
    -l               # new ipc ns
    -v               # new mount ns
    -N               # new cgroup ns
    --uts            # new uts ns
  )

  # Whitelist exactly the carve-out cap set. minijail0 -c expects
  # a hex bitmask; build it via capsh if available, else use the
  # named-list form via --add-suppl-group=...; for robustness across
  # minijail versions we use the bitmask form computed via Python
  # (no extra deps — Python is in the validator env via nix shell).
  local bitmask
  bitmask=$(python3 - <<'PY'
caps = {
    "CAP_CHOWN": 0, "CAP_DAC_OVERRIDE": 1, "CAP_DAC_READ_SEARCH": 2,
    "CAP_FOWNER": 3, "CAP_FSETID": 4, "CAP_KILL": 5,
    "CAP_SETGID": 6, "CAP_SETUID": 7, "CAP_SETPCAP": 8,
    "CAP_LINUX_IMMUTABLE": 9, "CAP_NET_BIND_SERVICE": 10,
    "CAP_NET_BROADCAST": 11, "CAP_NET_ADMIN": 12, "CAP_NET_RAW": 13,
    "CAP_IPC_LOCK": 14, "CAP_IPC_OWNER": 15, "CAP_SYS_MODULE": 16,
    "CAP_SYS_RAWIO": 17, "CAP_SYS_CHROOT": 18, "CAP_SYS_PTRACE": 19,
    "CAP_SYS_PACCT": 20, "CAP_SYS_ADMIN": 21, "CAP_SYS_BOOT": 22,
    "CAP_SYS_NICE": 23, "CAP_SYS_RESOURCE": 24, "CAP_SYS_TIME": 25,
    "CAP_SYS_TTY_CONFIG": 26, "CAP_MKNOD": 27, "CAP_LEASE": 28,
    "CAP_AUDIT_WRITE": 29, "CAP_AUDIT_CONTROL": 30, "CAP_SETFCAP": 31,
}
want = [
    "CAP_SYS_ADMIN","CAP_SETPCAP","CAP_CHOWN","CAP_FOWNER","CAP_FSETID",
    "CAP_SETUID","CAP_SETGID","CAP_DAC_OVERRIDE","CAP_MKNOD","CAP_SETFCAP",
]
mask = 0
for c in want:
    mask |= 1 << caps[c]
print(f"0x{mask:x}")
PY
)
  argv+=( -c "$bitmask" )

  # Export the constructed argv via nameref.
  # shellcheck disable=SC2178,SC2034
  declare -n out_ref="$out_var"
  # shellcheck disable=SC2034
  out_ref=("${argv[@]}")
}

layer2_positive() {
  command -v virtiofsd >/dev/null 2>&1 \
    || fail "virtiofsd not found in PATH (required for Layer 2 positive path)"

  local -a mj
  build_minijail_argv mj

  log "Layer-2 positive: $(printf '%q ' "${mj[@]}") -- virtiofsd --version"
  if ! "${mj[@]}" -- "$(command -v virtiofsd)" --version >/dev/null 2>&1; then
    fail "Layer-2 positive: virtiofsd --version under minijail0 carve-out profile did not exit 0"
  fi
  log "Layer-2 positive: OK"
}

layer2_negative() {
  command -v python3 >/dev/null 2>&1 \
    || fail "python3 required for Layer 2 negative path (ptrace probe)"

  TMP_WORK=$(mktemp -d -t p1-virtiofsd-XXXXXX)
  local probe="$TMP_WORK/ptrace_probe.py"
  cat >"$probe" <<'PY'
# ptrace(PTRACE_TRACEME, 0, 0, 0). Virtiofsd never calls ptrace; the
# w1-virtiofsd seccomp policy MUST kill us with SIGSYS. This probe
# must be reached only AFTER minijail has applied the policy.
import ctypes, sys
libc = ctypes.CDLL("libc.so.6", use_errno=True)
SYS_ptrace = 101  # x86_64
PTRACE_TRACEME = 0
# Use raw syscall to bypass any glibc wrapper.
libc.syscall.restype = ctypes.c_long
libc.syscall.argtypes = [ctypes.c_long] + [ctypes.c_long] * 4
rc = libc.syscall(SYS_ptrace, PTRACE_TRACEME, 0, 0, 0)
# If we reach here, the syscall was NOT blocked by seccomp -> negative
# probe failed.
sys.exit(0)
PY

  local -a mj
  build_minijail_argv mj

  # NOTE: minijail0 -S <policy> would apply the actual seccomp BPF
  # blob if we had one materialized. The plan defers the per-role
  # seccomp blob to W1; here we approximate by relying on the
  # capability-only profile to demonstrate Layer-2 mechanics. When
  # the w1-virtiofsd seccomp policy file is materialized, swap in
  # `mj+=( -S "$policy_path" )` here so the negative probe binds to
  # the real allowlist.
  local seccomp_policy=${NL_VIRTIOFSD_SECCOMP_POLICY:-}
  if [ -n "$seccomp_policy" ] && [ -f "$seccomp_policy" ]; then
    mj+=( -S "$seccomp_policy" )
  else
    log "Layer-2 negative: NL_VIRTIOFSD_SECCOMP_POLICY unset; ptrace probe under cap-only profile (will not SIGSYS without seccomp blob)"
    log "Layer-2 negative: SKIPPED (seccomp blob not materialized — gated on W1 deliverable)"
    return 2
  fi

  log "Layer-2 negative: probing ptrace under w1-virtiofsd seccomp"
  set +e
  "${mj[@]}" -- "$(command -v python3)" "$probe" >/dev/null 2>&1
  local rc=$?
  set -e

  # SIGSYS = 31 → exit status 128 + 31 = 159.
  if [ "$rc" -eq 159 ]; then
    log "Layer-2 negative: OK (SIGSYS as required)"
    return 0
  fi
  fail "Layer-2 negative: ptrace probe was not killed by SIGSYS (rc=$rc)"
}

write_evidence() {
  local dir
  dir=$(dirname "$EVIDENCE_PATH")
  if ! mkdir -p "$dir" 2>/dev/null; then
    log "evidence: cannot mkdir $dir (need root); skipping write"
    return 0
  fi

  local ts sig
  ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  # operatorSignature is a placeholder per plan §"evidence files".
  # Integrator may swap in a real signature once the per-host
  # operator-key material is wired in.
  sig=$(printf 'p1-virtiofsd:%s' "$ts" | sha256sum | awk '{print $1}')

  local tmp
  tmp=$(mktemp -p "$dir" .p1-virtiofsd.XXXXXX.json)
  printf '{"wave":"p1-virtiofsd","timestamp":"%s","operatorSignature":"%s"}\n' \
    "$ts" "$sig" >"$tmp"
  mv -f "$tmp" "$EVIDENCE_PATH"
  log "evidence: wrote $EVIDENCE_PATH"
}

main() {
  assert_profile_source
  assert_installed_profiles

  if [ "${NL_LIVE:-0}" != "1" ]; then
    log "Layer-2 skipped (NL_LIVE!=1); evidence record NOT written"
    return 0
  fi

  layer2_positive

  local neg_rc=0
  layer2_negative || neg_rc=$?
  if [ "$neg_rc" -eq 2 ]; then
    log "Layer-2 negative skipped (seccomp blob not materialized); evidence record NOT written"
    return 0
  fi

  write_evidence
}

main "$@"
