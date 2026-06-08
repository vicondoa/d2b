#!/usr/bin/env bash
# Asserts no live references to legacy nixling-launcher{,s} group
# names remain in source. Allowlist uses anchored full-line regex
# matched against rg's "path:lineno:content" output (NOT substring).
# Patterns are kept as array entries so literal spaces stay intact.
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}
cd "$ROOT"

search_legacy_refs() {
  if command -v rg >/dev/null 2>&1; then
    rg -n 'nixling-launcher(s)?' "$@"
  else
    grep -RInE 'nixling-launcher(s)?' "$@"
  fi
}

allowlist=(
  'nixos-modules/host-activation\.nix:[0-9]+:[[:space:]]*(legacyLauncherGid|legacyLaunchersGid|getent group|for legacy_name in nixling-launcher nixling-launchers; do).*'
  'nixos-modules/host-activation-helper/.*'
  'packages/nixling-host-activation-helper/.*'
  'nixos-modules/host-users\.nix:[0-9]+:[[:space:]]*# DEPRECATED v1\.2: kept as migration tombstone for the[[:space:]]*'
  'nixos-modules/host-users\.nix:[0-9]+:[[:space:]]*# nixling-launcher\{,s\} → nixling rename\. No module references the[[:space:]]*'
  'nixos-modules/host-users\.nix:[0-9]+:[[:space:]]*nixling-launcher = \{ \};[[:space:]]*'
  'nixos-modules/host-daemon\.nix:[0-9]+:[[:space:]]*# DEPRECATED v1\.2: kept as migration tombstone for the[[:space:]]*'
  'nixos-modules/host-daemon\.nix:[0-9]+:[[:space:]]*# nixling-launcher\{,s\} → nixling rename\. No module references the[[:space:]]*'
  'nixos-modules/host-daemon\.nix:[0-9]+:[[:space:]]*users\.groups\.nixling-launchers = \{ \};[[:space:]]*'
  'packages/nixling-core/src/privileges\.rs:[0-9]+:.*nixling-launcher.*'
  'packages/nixling-ipc/src/broker_wire\.rs:[0-9]+:.*nixling-launcher.*'
  'packages/nixling-priv-broker/src/bootstrap\.rs:[0-9]+:.*nixling-launcher.*'
  'nixos-modules/privileges-json\.nix:[0-9]+:.*nixling-launcher.*'
  'tests/legacy-group-name-denylist(-self-test)?\.sh:[0-9]+:.*'
  'tests/group-rename-semantic-eval\.sh:[0-9]+:.*'
)

allowlist_regex="^($(IFS='|'; echo "${allowlist[*]}"))$"
violations=$(search_legacy_refs ${NL_LEGACY_GROUP_DENYLIST_PATHS:-nixos-modules packages tests} \
  | grep -vE "$allowlist_regex" || true)
if [ -n "$violations" ]; then
  echo "FAIL: legacy nixling-launcher{,s} references found:"
  echo "$violations"
  exit 1
fi
