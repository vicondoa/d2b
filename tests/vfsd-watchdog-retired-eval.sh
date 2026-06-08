#!/usr/bin/env bash
# v1.1 invariant gate: assert nixling-vfsd-watchdog@.{service,timer}
# definitions are absent from `nixos-modules/store.nix`. The wedge-
# detection logic moved into the broker's Virtiofsd `SpawnRunner`
# role supervisor (pidfd poll + cgroup.events probe at the same
# 60s cadence; wedge surfaces via the typed `runner-wedged`
# OpAuditRecord) per ADR 0018.
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

store_module="$ROOT/nixos-modules/store.nix"

fail=0

# (a) Productive service template absent (string-key literal in
# attrs scope). Pattern matches lines that DECLARE the unit, not
# comments mentioning it.
if grep -E '^\s*"nixling-vfsd-watchdog@"\s*=' "$store_module" >/dev/null 2>&1; then
  printf 'vfsd-watchdog-retired-eval: FAIL — "nixling-vfsd-watchdog@" service template still declared in %s\n' "$store_module" >&2
  fail=1
fi

# (b) Productive timer template absent.
if grep -E 'systemd\.timers\."nixling-vfsd-watchdog@"\s*=' "$store_module" >/dev/null 2>&1; then
  printf 'vfsd-watchdog-retired-eval: FAIL — systemd.timers."nixling-vfsd-watchdog@" still declared in %s\n' "$store_module" >&2
  fail=1
fi

# (c) Per-VM enabling units absent.
if grep -E '"nixling-vfsd-watchdog-\$\{name\}-enable"' "$store_module" >/dev/null 2>&1; then
  printf 'vfsd-watchdog-retired-eval: FAIL — per-VM enabling unit "nixling-vfsd-watchdog-${name}-enable" still declared in %s\n' "$store_module" >&2
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  exit 1
fi

printf 'vfsd-watchdog-retired-eval: PASS\n'
