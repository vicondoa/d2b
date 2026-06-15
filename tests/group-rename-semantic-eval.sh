#!/usr/bin/env bash
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

require() {
  local pattern="$1" file="$2" msg="$3"
  if ! grep -Eq -- "$pattern" "$file"; then
    printf 'group-rename-semantic-eval: FAIL — %s\n' "$msg" >&2
    exit 1
  fi
}

require 'nixling = \{ \};' "$ROOT/nixos-modules/host-users.nix" 'users.groups.nixling declaration missing'
require 'extraGroups = \[ "nixling" \];' "$ROOT/nixos-modules/host-users.nix" 'launcherUsers are not added to nixling'
require 'publicSocketGroup = "nixling";' "$ROOT/nixos-modules/host-daemon.nix" 'daemon publicSocketGroup is not nixling'
require 'LAUNCHER_GROUP=nixling' "$ROOT/nixos-modules/host-activation.nix" 'state-dir ACL launcher group is not nixling'
require 'g:\$LAUNCHER_GROUP:--x' "$ROOT/nixos-modules/host-activation.d/state-dir-acl.sh" 'ACL helper does not render g:<launcher-group>:'
require 'nixling-launcher = \{ \};' "$ROOT/nixos-modules/host-users.nix" 'nixling-launcher tombstone missing'
require 'nixling-launchers = \{ \};' "$ROOT/nixos-modules/host-daemon.nix" 'nixling-launchers tombstone missing'
if grep -R -nE 'extraGroups = \[[^]]*"nixling-launcher(s)?"' "$ROOT/nixos-modules" >/dev/null 2>&1; then
  printf 'group-rename-semantic-eval: FAIL — legacy group still appears in extraGroups\n' >&2
  exit 1
fi
# The privileges Nix<->Rust matrix parity (which this gate previously
# piggybacked via tests/privileges-json-rust-vs-nix-eval.sh) is now covered
# independently by the gated contract test
# packages/nixling-contract-tests/tests/privileges_parity.rs (run from
# tests/rust-workspace-checks.sh with NL_FIXTURES). This gate retains only
# its own group-rename source invariants.
printf 'group-rename-semantic-eval: PASS\n'
