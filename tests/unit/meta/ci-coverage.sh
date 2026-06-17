#!/usr/bin/env bash
# tests/unit/meta/ci-coverage.sh — structural gate asserting every tests/*.sh is
# wired into at least one CI workflow or test aggregator.
#
# Root-cause gap: static CI set drift.
#
# Exits 0 if all tests are covered, 1 if any are orphaned.
#
# Usage:
#   bash tests/unit/meta/ci-coverage.sh

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}

# ---------------------------------------------------------------------------
# EXCLUDE_FROM_CI — intentional exclusions with rationale.
# Each entry is the basename without leading "tests/" e.g. "lib.sh".
# Keep sorted; add rationale comment above each entry.
# ---------------------------------------------------------------------------
EXCLUDE_FROM_CI=(
  # lib.sh: shared helper library sourced by other tests; not a standalone test.
  "lib.sh"

  # runner.sh: high-level aggregator that invokes static.sh + nixling-store.sh +
  # audio.sh + security tests; it IS a test runner, not an individual test.
  # Not wired into PR CI by design (full suite; takes 10-30 min).
  "runner.sh"

  # cli-rust-native-common.sh: sourced helper (common fixtures for cli-rust-native-*
  # tests); not a standalone runnable test.
  "cli-rust-native-common.sh"

  # static-timing.sh: wall-clock timing analysis tool for maintainer profiling;
  # not a correctness gate.
  "static-timing.sh"

  # hardware-smoke-gpu-yubikey.sh: requires physical GPU + YubiKey hardware;
  # intentionally maintainer-only, never runnable in ephemeral CI.
  "hardware-smoke-gpu-yubikey.sh"

  # static.sh: full panel gate aggregator; invoked manually before panel
  # dispatch / wave-exit gates, and by runner.sh. Not wired into PR CI
  # because it takes 30-60 min and requires a self-hosted NixOS runner.
  "static.sh"

  # audit-forwarding.sh: Layer-2 optional live test for auditd → journald →
  # Alloy → Loki forwarding. Requires nixling installed + live auditd stack.
  # Skips cleanly when manifest absent; deliberately not a PR gate.
  "audit-forwarding.sh"

  # network-isolation.sh: Layer-2 optional live test for host datapath
  # isolation. Requires nixling installed + live networking stack.
  # Skips cleanly when manifest absent; deliberately not a PR gate.
  "network-isolation.sh"

  # nixling-store.sh: Layer-2 integration tests for the per-VM /nix/store
  # hardlink farm + lifecycle CLI. Requires a live host with nixlingd +
  # nixling-priv-broker active. Documented in tests/README.md and AGENTS.md.
  "nixling-store.sh"

  # swtpm-persistence-smoke.sh: Layer-2 persistence regression. Requires
  # NL_LIVE=1, a running nixlingd, and a restartable swtpm. Not runnable in
  # ephemeral CI.
  "swtpm-persistence-smoke.sh"

  # live-vm-smoke.sh: maintainer-side pre-tag live-VM smoke gate.
  # Requires: KVM (/dev/kvm present), systemd-activated nixling-priv-broker,
  # nixling on PATH, and declared VMs (personal-dev + work-aad for --full).
  # Exits 77 (SKIP) cleanly when any prerequisite is absent; never runnable in
  # ephemeral CI. Invoked via `make pre-tag` (--full) or `make smoke-lite` (--lite).
  "live-vm-smoke.sh"
)

# ---------------------------------------------------------------------------
# Reference files to search.
# ---------------------------------------------------------------------------
WORKFLOW_DIR="$ROOT/.github/workflows"
STATIC_SH="$ROOT/tests/static.sh"
STATIC_FAST_SH="$ROOT/tests/static-fast.sh"
LIVE_VM_SMOKE="$ROOT/tests/integration/live/live-vm-smoke.sh"

# ---------------------------------------------------------------------------
# Build a combined blob of all reference content for fast grep.
# ---------------------------------------------------------------------------
ref_files=()
for wf in "$WORKFLOW_DIR"/*.yml; do
  [ -f "$wf" ] && ref_files+=("$wf")
done
[ -f "$STATIC_SH" ]      && ref_files+=("$STATIC_SH")
[ -f "$STATIC_FAST_SH" ] && ref_files+=("$STATIC_FAST_SH")
[ -f "$LIVE_VM_SMOKE" ]  && ref_files+=("$LIVE_VM_SMOKE")

if [ ${#ref_files[@]} -eq 0 ]; then
  echo "ERROR: no reference files found (no .github/workflows/*.yml and no tests/static*.sh)" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Helper: is_excluded <basename>
# ---------------------------------------------------------------------------
is_excluded() {
  local name="$1"
  for ex in "${EXCLUDE_FROM_CI[@]}"; do
    [ "$ex" = "$name" ] && return 0
  done
  return 1
}

# ---------------------------------------------------------------------------
# Helper: is_referenced <basename>
# Returns 0 if the test is mentioned in any reference file.
# Searches for:
#   (a) the exact string "tests/<name>"  (direct path reference)
#   (b) the bare stem  (e.g. "bundle-drift") to catch loop-based references
#       in static-fast.sh where tests are listed as bare names in arrays.
# ---------------------------------------------------------------------------
is_referenced() {
  local name="$1"                       # e.g. "bundle-drift.sh"
  local rel="${2:-tests/$name}"         # e.g. "tests/unit/gates/bundle-drift.sh"
  local stem="${name%.sh}"              # e.g. "bundle-drift"
  # Use grep -qF (fixed-string) for speed; search all reference files at once.
  if grep -qF "$rel" "${ref_files[@]}" 2>/dev/null; then
    return 0
  fi
  # Bare stem match — covers loop entries like "  bundle-drift \" in static-fast.sh
  # and "  vm-submodule-eval; do" (last item in a for loop) in static.sh.
  # Require the stem be preceded by whitespace/quote and followed by
  # whitespace, backslash, quote, semicolon, or end-of-field.
  if grep -qE "(^|[ \t'\"])(${stem})([ \t'\"\\\\;]|$)" "${ref_files[@]}" 2>/dev/null; then
    return 0
  fi
  return 1
}

# ---------------------------------------------------------------------------
# Main scan
# ---------------------------------------------------------------------------
orphans=()
covered=0
excluded_count=${#EXCLUDE_FROM_CI[@]}

for script in "$HERE"/*.sh; do
  name=$(basename "$script")
  rel=${script#"$ROOT"/}

  # Skip self.
  [ "$name" = "ci-coverage.sh" ] && continue

  # Skip _helper scripts.
  [[ "$name" == _* ]] && continue

  # Skip excluded.
  if is_excluded "$name"; then
    continue
  fi

  if is_referenced "$name" "$rel"; then
    covered=$((covered + 1))
  else
    orphans+=("$rel")
  fi
done

# ---------------------------------------------------------------------------
# Report
# ---------------------------------------------------------------------------
if [ ${#orphans[@]} -eq 0 ]; then
  echo "PASS: $covered tests referenced; $excluded_count intentionally excluded"
  exit 0
else
  echo "FAIL: the following tests/*.sh are not referenced in any CI workflow or test aggregator:" >&2
  for o in "${orphans[@]}"; do
    echo "  $o" >&2
  done
  echo "" >&2
  echo "Remediation: wire each orphan into tests/static.sh (or another aggregator/workflow)" >&2
  echo "OR add it to the EXCLUDE_FROM_CI list in tests/unit/meta/ci-coverage.sh with a rationale comment." >&2
  echo "" >&2
  echo "FAIL: $covered tests referenced; ${#orphans[@]} orphaned; $excluded_count intentionally excluded" >&2
  exit 1
fi
