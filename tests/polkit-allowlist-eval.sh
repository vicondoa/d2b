#!/usr/bin/env bash
# tests/polkit-allowlist-eval.sh
#
# P6 ph6-p6-polkit-retire — asserts nixos-modules/host-polkit.nix
# names ONLY daemon-only singleton units in its launcher allowlist:
#
#   * nixlingd.service
#   * nixling-priv-broker.service
#   * nixling-priv-broker.socket
#
# and contains NO references to retired per-VM / per-env unit shapes
# (the W2-followup C1 exact-unit allowlist that pre-P6 generated entries
# for `nixling@<vm>`, `nixling-<vm>-{gpu,snd,swtpm,store-sync}`,
# `nixling-sys-<env>-usbipd-{proxy,backend}.{service,socket}`,
# `nixling-audit-check`, and the per-VM `nixling-<vm>-gpu` → `-snd`
# fallback rule).
#
# Layer-1, eval-only (no flake build, no daemon, no host state).

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

polkit="$ROOT/nixos-modules/host-polkit.nix"

fail() {
  printf 'tests/polkit-allowlist-eval: %s\n' "$*" >&2
  exit 1
}

[ -f "$polkit" ] || fail "missing $polkit"

# ---------------------------------------------------------------------------
# Required daemon-only singletons present in the allowlist.
# ---------------------------------------------------------------------------
for unit in \
  '"nixlingd.service"' \
  '"nixling-priv-broker.service"' \
  '"nixling-priv-broker.socket"'
do
  grep -qF "$unit" "$polkit" \
    || fail "$polkit allowlist missing required singleton $unit"
done

# ---------------------------------------------------------------------------
# Forbidden patterns: any reference to the retired per-VM / per-env
# unit shapes the bash CLI used to drive via systemctl.
#
# Scoped to the executable region only — the `let` bindings + the
# `security.polkit.extraConfig` JS body. Documentation prose in the
# leading comment block is allowed to name the retired shapes (and
# does, so that operators reading the module can see what was
# retired) — but the executable code MUST NOT.
# ---------------------------------------------------------------------------
exec_region=$(awk '/^in$/,0' "$polkit")

forbidden_patterns=(
  'nixling@'
  'microvm@'
  'microvm-virtiofsd@'
  'nixling-<vm>-'
  'nixling-sys-<env>-usbipd'
  'nixling-audit-check'
  'perVmUnits'
  'perEnvUnits'
  'config\.nixling\.vms'
  'config\.nixling\.envs'
  'cfg\.vms'
  'cfg\.envs'
  'enabledVms'
  'mapAttrsToList'
)

for pat in "${forbidden_patterns[@]}"; do
  if printf '%s\n' "$exec_region" | grep -qE "$pat"; then
    fail "host-polkit.nix executable region references retired per-VM/per-env shape: $pat"
  fi
done

# ---------------------------------------------------------------------------
# Structural invariants on the surviving polkit grant:
#
#   * still scoped to org.freedesktop.systemd1.manage-units
#   * still scoped to the nixling-launcher group
#   * verb allowlist still restricts to start/stop/restart
# ---------------------------------------------------------------------------
grep -qF 'org.freedesktop.systemd1.manage-units' "$polkit" \
  || fail "$polkit lost the manage-units action-id guard"

grep -qF 'isInGroup("nixling-launcher")' "$polkit" \
  || fail "$polkit lost the nixling-launcher group guard"

grep -qE 'verb !== "start".*verb !== "stop".*verb !== "restart"' "$polkit" \
  || fail "$polkit lost the start/stop/restart verb allowlist"

# ---------------------------------------------------------------------------
# Exactly one polkit.addRule callback should remain (the per-VM
# `nixling-<vm>-gpu` → `nixling-<vm>-snd` fallback rule must be gone).
# ---------------------------------------------------------------------------
addrule_count=$(grep -cF 'polkit.addRule(' "$polkit" || true)
if [ "$addrule_count" -ne 1 ]; then
  fail "$polkit must declare exactly one polkit.addRule callback (found $addrule_count); the per-VM gpu→snd fallback rule must be retired"
fi

printf 'tests/polkit-allowlist-eval: OK\n'
