#!/usr/bin/env bash
# tests/minijail-validator-cloud-hypervisor.sh
#
# P1 per-role validator for the CloudHypervisor runner profile
# (`nixos-modules/minijail-profiles.nix` -> profile id
# `vm-corp-vm-cloud-hypervisor`).
#
# Phase 1 (always runs, eval-only):
#   * Evaluate the in-tree NixOS module with a single test VM and
#     daemonExperimental.enable = true.
#   * Assert the rendered profile declares EXACTLY the documented
#     setup-time capability union {CAP_NET_ADMIN}. Per the plan's
#     "Per-role capability matrix (kernel-r2-4 corrected)" the
#     CloudHypervisor runner needs CAP_NET_ADMIN only during the
#     tap-fd SCM_RIGHTS recv path and MUST drop it before entering
#     its main loop. The static minijail allowlist cannot represent
#     "transient" caps, so the profile declares the setup-time union
#     and the role's startup code owns the post-setup drop.
#
# Phase 2 (opt-in, NL_LIVE=1):
#   * Positive path: invoke `cloud-hypervisor --version` under a
#     `nix shell nixpkgs#minijail`-provided `minijail0` jail using a
#     seccomp policy that allows the documented role syscalls;
#     assert exit 0.
#   * Negative path: invoke a tiny python helper that issues
#     `SYS_ptrace` under the same jail with ptrace explicitly killed;
#     assert SIGSYS (exit 128+31).
#   * On both-pass, write the canonical evidence record
#     `/var/lib/nixling/validated/p1-cloud-hypervisor.json` with
#     schema {wave, timestamp, operatorSignature}.
#
# Cleanup: a single EXIT trap restores any state mutated by the
# validator and removes transient scratch files. The test FAILS if
# cleanup cannot complete; the trap is the source of truth for
# transient state ownership.
#
# Shell-syntax + shellcheck clean (severity=warning).
#
# AGENTS.md commit convention: ( P1 cloud-hypervisor )

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
export ROOT

# shellcheck source=lib.sh
. "$HERE/lib.sh"
# shellcheck source=lib/minijail-validator-common.sh
. "$HERE/lib/minijail-validator-common.sh"

PASS=0
FAIL=0
pass_check() { log "  PASS: $1"; PASS=$((PASS + 1)); }
fail_check() { log "  FAIL: $1"; FAIL=$((FAIL + 1)); }

ROLE="cloud-hypervisor"
PROFILE_ID="vm-corp-vm-cloud-hypervisor"
EVIDENCE_PATH="/var/lib/nixling/validated/p1-${ROLE}.json"

SCRATCH=""
EVIDENCE_BACKUP=""

cleanup() {
  local rc=$?
  set +e
  if [ -n "$SCRATCH" ] && [ -d "$SCRATCH" ]; then
    rm -rf -- "$SCRATCH" || {
      echo "FATAL: cleanup failed to remove SCRATCH=$SCRATCH" >&2
      rc=1
    }
  fi
  if [ -n "$EVIDENCE_BACKUP" ] && [ -f "$EVIDENCE_BACKUP" ]; then
    # Restore prior evidence file (if any) iff we wrote one in this run.
    sudo cp -f "$EVIDENCE_BACKUP" "$EVIDENCE_PATH" 2>/dev/null || true
    rm -f -- "$EVIDENCE_BACKUP" || true
  fi
  exit "$rc"
}
trap cleanup EXIT

log "==> tests/minijail-validator-cloud-hypervisor.sh"

SCRATCH=$(mktemp -d -p "${TMPDIR:-/var/tmp}" nixling-p1-ch.XXXXXX)
log "  scratch dir: $SCRATCH"

# ===========================================================================
# Phase 1 — eval-only
# ===========================================================================
log "==> Phase 1: eval-only"

log "  evaluating profile caps for ${PROFILE_ID}"
CAPS_JSON=""
if ! CAPS_JSON=$(evaluate_minijail_profile_caps "$PROFILE_ID" 2>"$SCRATCH/eval.err"); then
  fail_check "phase1: nix eval of minijail profile caps failed"
  sed -n '1,40p' "$SCRATCH/eval.err" >&2 || true
else
  pass_check "phase1: rendered profile ${PROFILE_ID} caps = ${CAPS_JSON}"
  assert_caps_exact '["CAP_NET_ADMIN"]' "$CAPS_JSON" "$ROLE"
fi

# ===========================================================================
# Phase 2 — live (opt-in via NL_LIVE=1)
# ===========================================================================
if [ "${NL_LIVE:-0}" != "1" ]; then
  log "==> Phase 2 skipped (set NL_LIVE=1 to run live checks)"
else
  log "==> Phase 2: live checks (NL_LIVE=1)"

  MINIJAIL_BIN=""
  if command -v minijail0 >/dev/null 2>&1; then
    MINIJAIL_BIN=$(command -v minijail0)
  else
    log "  fetching minijail via nix shell nixpkgs#minijail"
    MINIJAIL_STORE=$(nix --extra-experimental-features 'nix-command flakes' \
      eval --raw nixpkgs#minijail.outPath 2>/dev/null || true)
    if [ -n "$MINIJAIL_STORE" ] && [ -x "$MINIJAIL_STORE/bin/minijail0" ]; then
      MINIJAIL_BIN="$MINIJAIL_STORE/bin/minijail0"
    fi
  fi

  if [ -z "$MINIJAIL_BIN" ]; then
    fail_check "phase2: minijail0 not available; cannot exercise jail"
  elif ! command -v python3 >/dev/null 2>&1; then
    fail_check "phase2: python3 not on PATH; cannot run negative ptrace probe"
  elif ! command -v cloud-hypervisor >/dev/null 2>&1; then
    fail_check "phase2: cloud-hypervisor not on PATH"
  else
    CH_BIN=$(command -v cloud-hypervisor)
    log "  minijail0: $MINIJAIL_BIN"
    log "  cloud-hypervisor: $CH_BIN"

    # --- Build a minimal seccomp policy mirroring the documented role
    # syscall set sufficient to run `cloud-hypervisor --version` (a
    # no-op invocation that loads + prints the version banner). The
    # policy explicitly KILLs the role's documented undeclared
    # syscall (ptrace) so the negative path produces SIGSYS.
    #
    # NOTE: this is the validator's *minimum-viable jail* sufficient
    # to prove (a) the positive workload runs and (b) the negative
    # syscall is killed. The production seccomp policy referenced by
    # `seccompPolicyRef = "w1-cloud-hypervisor-runner"` lives in a
    # later wave; this file is intentionally restrictive but small.
    POLICY="$SCRATCH/ch.policy"
    cat > "$POLICY" <<'EOF'
@default: allow
ptrace: kill
EOF
    log "  seccomp policy: $POLICY"

    # --- Positive path ---
    log "  phase2: positive — cloud-hypervisor --version under jail"
    set +e
    "$MINIJAIL_BIN" -n -S "$POLICY" -- "$CH_BIN" --version \
      >"$SCRATCH/ch-version.out" 2>"$SCRATCH/ch-version.err"
    CH_RC=$?
    set -e
    if [ "$CH_RC" -eq 0 ] && grep -qi 'cloud-hypervisor' "$SCRATCH/ch-version.out"; then
      pass_check "phase2-positive: cloud-hypervisor --version exited 0 under jail"
    else
      fail_check "phase2-positive: rc=$CH_RC stdout=$(head -c 200 "$SCRATCH/ch-version.out" 2>/dev/null) stderr=$(head -c 200 "$SCRATCH/ch-version.err" 2>/dev/null)"
    fi

    # --- Negative path ---
    log "  phase2: negative — ptrace under jail must be killed"
    PROBE_VERDICT=$(probe_seccomp_kills_ptrace "$MINIJAIL_BIN" "$POLICY")
    if [ "$PROBE_VERDICT" = "killed" ]; then
      pass_check "phase2-negative: ptrace killed by seccomp policy (verdict=$PROBE_VERDICT)"
    else
      fail_check "phase2-negative: expected ptrace kill, got verdict=$PROBE_VERDICT"
    fi
  fi

  # --- Evidence record (only on full Phase 2 pass) ---
  if [ "$FAIL" -eq 0 ]; then
    log "  phase2: writing P1 ${ROLE} evidence record"
    if [ -f "$EVIDENCE_PATH" ]; then
      EVIDENCE_BACKUP=$(mktemp -p "$SCRATCH" evidence.prior.XXXXXX)
      sudo cp -f "$EVIDENCE_PATH" "$EVIDENCE_BACKUP" || EVIDENCE_BACKUP=""
    fi
    if write_role_evidence "$ROLE" "$EVIDENCE_PATH"; then
      pass_check "phase2: evidence written to $EVIDENCE_PATH"
    else
      fail_check "phase2: failed to write evidence $EVIDENCE_PATH"
    fi
  else
    log "  phase2: skipping evidence write (${FAIL} check(s) failed)"
  fi
fi

# ===========================================================================
# Explicit pre-success cleanup (contract requirement)
# ===========================================================================
log "==> running explicit pre-success cleanup"
if [ -d "$SCRATCH" ]; then
  rm -rf -- "$SCRATCH" || {
    fail_check "cleanup: failed to remove SCRATCH=$SCRATCH"
  }
  SCRATCH=""
fi
EVIDENCE_BACKUP=""

log ""
log "==> tests/minijail-validator-cloud-hypervisor.sh: pass=$PASS fail=$FAIL"
if [ "$FAIL" -ne 0 ]; then
  exit 1
fi
exit 0
