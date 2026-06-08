#!/usr/bin/env bash
# v1.1 invariant gate: assert
#   (a) `supervisor = lib.mkOption` is absent from
#       `nixos-modules/options-vms.nix` (productive declaration gone), and
#   (b) the defense-in-depth assertion in `nixos-modules/assertions.nix`
#       catches consumers that still set `nixling.vms.<vm>.supervisor`.
#
# Note: the original v1.1 plan called for a per-submodule
# `mkRemovedOptionModule [ "supervisor" ]` shim in
# `nixos-modules/options-vms-removed.nix`. That shim works for
# top-level removed options but cannot be wired into an
# `attrsOf submodule` per-instance because the per-VM submodule
# layer has no `assertions` option (NixOS assertions live at
# the top-level config root only). The v1.1 implementation
# relies on the top-level fallback assertion in
# `assertions.nix`, which fires on any per-VM supervisor=...
# definition with the friendly ADR-0015 message.
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

options_file="$ROOT/nixos-modules/options-vms.nix"
assertions_file="$ROOT/nixos-modules/assertions.nix"

fail=0

# (a) productive declaration gone
if grep -q -E '^\s*supervisor\s*=\s*lib\.mkOption' "$options_file"; then
  printf 'supervisor-option-absent-eval: FAIL — productive `supervisor = lib.mkOption` still present in %s\n' "$options_file" >&2
  fail=1
fi

# (b) top-level fallback assertion present
if [ ! -f "$assertions_file" ]; then
  printf 'supervisor-option-absent-eval: FAIL — assertions.nix missing\n' >&2
  fail=1
elif ! grep -q -E 'vm \? supervisor|vms\.\$\{name\}\.supervisor' "$assertions_file"; then
  printf 'supervisor-option-absent-eval: FAIL — supervisor-fallback assertion missing from %s\n' "$assertions_file" >&2
  fail=1
fi

# (b) friendly ADR-0015 message present
if ! grep -q -E 'removed in v1\.1.*per ADR 0015|ADR 0015.*daemon-only clean break' "$assertions_file"; then
  printf 'supervisor-option-absent-eval: FAIL — ADR-0015 friendly message text missing from %s\n' "$assertions_file" >&2
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  exit 1
fi

printf 'supervisor-option-absent-eval: PASS\n'

