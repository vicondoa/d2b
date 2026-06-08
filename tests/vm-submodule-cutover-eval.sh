#!/usr/bin/env bash
# v1.1-P9b invariant gate: assert no production consumer in
# `nixos-modules/` reads `config.microvm.vms.<name>.config.config.*`
# directly — every consumer routes through the nixling-owned
# helpers `nl.vmRunner` / `nl.vmToplevel` / `nl.vmDeclaredRunner`
# defined in `nixos-modules/lib.nix`. The helper bodies are the
# single migration point: at v1.1-rc1 they delegate to upstream
# microvm.vms; at v1.1-final they swap to a nixling-owned
# evaluator without touching consumer sites.
#
# `lib.nix` (where the helpers live), `host.nix` (which still
# writes `microvm.vms = lib.mapAttrs ...` to feed the upstream
# evaluator), and `vm-submodule.nix` (which composes the per-VM
# module merge) are EXEMPT from this gate — they are the
# substrate-side authors, not consumers.
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

exempt_files=(
  "$ROOT/nixos-modules/lib.nix"
  "$ROOT/nixos-modules/host.nix"
  "$ROOT/nixos-modules/vm-submodule.nix"
)

fail=0
while IFS= read -r line; do
  file=$(printf '%s' "$line" | cut -d: -f1)
  for exempt in "${exempt_files[@]}"; do
    if [ "$file" = "$exempt" ]; then
      file=""
      break
    fi
  done
  if [ -n "$file" ]; then
    printf 'vm-submodule-cutover-eval: FAIL — %s\n' "$line" >&2
    fail=1
  fi
done < <(grep -rEn 'config\.microvm\.vms\.\$\{[^}]*\}\.config\.config' "$ROOT/nixos-modules/" 2>/dev/null || true)

if [ "$fail" -ne 0 ]; then
  printf 'vm-submodule-cutover-eval: FAIL — production consumers must route through nl.vmRunner/vmToplevel/vmDeclaredRunner\n' >&2
  exit 1
fi

printf 'vm-submodule-cutover-eval: PASS (production consumers route through nixling-owned helpers)\n'

