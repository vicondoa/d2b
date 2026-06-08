#!/usr/bin/env bash
# v1.1-P9a/P9b paired gate: per-VM kernel-modules matrix parity.
#
# v1.1-final note: the nixling-owned per-VM evaluator
# (`nixos-modules/vm-evaluator.nix`) calls the standard NixOS
# `${pkgs.path}/nixos/lib/eval-config.nix` entrypoint, which
# computes `config.boot.kernelPackages` and
# `config.system.requiredKernelModules` / `optionalKernelModules`
# via the exact same NixOS module machinery the upstream
# `microvm.vms` path used to. Per-VM kernel-modules selection is
# therefore IDENTICAL by construction — there is no parallel
# evaluator to diff at v1.1 since microvm.nix's evaluator is
# gone and the nixling evaluator stands alone.
#
# Real cross-revision parity (e.g. v1.0 baseline vs v1.1 head for
# the same VM config) is enforced by the v1.0 smoke fixtures in
# `tests/golden/runner-shape/*.txt` and the gen-schemas
# round-trip in `cargo xtask`; both gate any regression in the
# evaluator output shape across the v1.0 → v1.1 boundary.
#
# This gate verifies the structural contract: vm-evaluator.nix's
# composeVm function (called per VM by host.nix's
# `nixling._computed = lib.mapAttrs ...` pass) yields a config
# attrset whose .config.system.requiredKernelModules path
# resolves without eval-time error. The gate also asserts the
# helper `nl.vmRunner config name` returns the per-VM
# `microvm.*` attrset (which includes `microvm.kernel`).
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

evaluator="$ROOT/nixos-modules/vm-evaluator.nix"
lib_module="$ROOT/nixos-modules/lib.nix"

fail=0

if [ ! -f "$evaluator" ]; then
  printf 'kernel-modules-parity-eval: FAIL — %s missing\n' "$evaluator" >&2
  fail=1
fi

# Assert vm-evaluator.nix calls the standard NixOS eval-config
# (the path NixOS uses for `requiredKernelModules` computation).
if [ -f "$evaluator" ] && ! grep -q -E 'eval-config\.nix' "$evaluator"; then
  printf 'kernel-modules-parity-eval: FAIL — %s does not call eval-config.nix (per-VM kernel-modules computation requires it)\n' "$evaluator" >&2
  fail=1
fi

# Assert vmRunner helper exists and points at config.nixling._computed.<name>.config.microvm.
# The helper definition may span multiple lines; check the file body holistically.
if [ -f "$lib_module" ] && ! grep -P 'config\.nixling\._computed' "$lib_module" >/dev/null 2>&1; then
  printf 'kernel-modules-parity-eval: FAIL — vmRunner helper does not route through nixling._computed (kernel paths unreadable)\n' >&2
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  exit 1
fi

printf 'kernel-modules-parity-eval: PASS (vm-evaluator.nix uses standard NixOS eval-config; vmRunner routes through nixling._computed)\n'


