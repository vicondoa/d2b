#!/usr/bin/env bash
# v1.1-P9a invariant gate: assert `nixos-modules/vm-submodule.nix`
# exists with the expected `composeVm` ownership shape. The
# full toplevel-hash parity test (vm-submodule.nix vs upstream
# microvm.vms evaluation) lands at v1.1-final when the submodule's
# `composeVm` switches to a nixling-owned `lib.evalModules` call.
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

submodule="$ROOT/nixos-modules/vm-submodule.nix"

if [ ! -f "$submodule" ]; then
  printf 'vm-submodule-eval: FAIL — %s missing\n' "$submodule" >&2
  exit 1
fi

if ! grep -q -E 'composeVm\s*=' "$submodule"; then
  printf 'vm-submodule-eval: FAIL — composeVm function not found in %s\n' "$submodule" >&2
  exit 1
fi

printf 'vm-submodule-eval: PASS (v1.1-P9a structural ownership move; toplevel-hash parity tested at v1.1-final)\n'

