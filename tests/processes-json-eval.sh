#!/usr/bin/env bash
# v1.1 invariant gate: assert `nixos-modules/processes-json.nix`,
# `closures-json.nix`, `minijail-profiles.nix`, and `store.nix` do
# NOT directly read `config.microvm.vms.<name>.config.config.*` —
# all per-VM runner config flows through the nixling-owned helpers
# `nl.vmRunner` / `nl.vmToplevel` / `nl.vmDeclaredRunner` defined
# in `nixos-modules/lib.nix`.
#
# This is the v1.1 partial cut-over: the helper bodies still
# alias to `config.microvm.vms.<name>.config.config.microvm.*`
# under the hood, but the access path is now nixling-owned so
# v1.1-rc2 / v1.1-final can swap the helper bodies to read from
# the new `vm-submodule.nix` evaluator without touching the
# consumer sites.
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

fail=0
for f in processes-json.nix closures-json.nix minijail-profiles.nix store.nix; do
  module="$ROOT/nixos-modules/$f"
  if [ ! -f "$module" ]; then
    printf 'processes-json-eval: FAIL — %s missing\n' "$module" >&2
    fail=1
    continue
  fi
  if grep -E 'config\.microvm\.vms\.\$\{[^}]*\}\.config\.config' "$module" >/dev/null 2>&1; then
    printf 'processes-json-eval: FAIL — %s still reads config.microvm.vms.<name>.config.config.* directly (must route through nl.vmRunner/vmToplevel/vmDeclaredRunner)\n' "$module" >&2
    fail=1
  fi
done

# lib.nix is allowed to contain the helper bodies (which DO read
# from config.microvm.vms.* — that's the point of the helper).
# Explicitly assert the helpers exist in lib.nix.
lib_module="$ROOT/nixos-modules/lib.nix"
for helper in vmRunner vmToplevel vmDeclaredRunner; do
  if ! grep -E "^\s*$helper\s*=" "$lib_module" >/dev/null 2>&1; then
    printf 'processes-json-eval: FAIL — helper %s missing from %s\n' "$helper" "$lib_module" >&2
    fail=1
  fi
done

if [ "$fail" -ne 0 ]; then
  exit 1
fi

printf 'processes-json-eval: PASS (all consumers route through nl.vmRunner/vmToplevel/vmDeclaredRunner)\n'

