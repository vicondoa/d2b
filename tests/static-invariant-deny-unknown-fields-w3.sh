#!/usr/bin/env bash
# W3 static invariant: every W3-added security-sensitive DTO under
# `nixling-core::host_w3` carries `#[serde(deny_unknown_fields)]` (per
# AGENTS.md "Manifest bundle" policy + plan §"W3 schema/version bump
# rules" + plan §"W3 host.json per-field schema gold-file drift gate").
#
# This is the W3 extension of `tests/static-invariant-deny-unknown-
# fields.sh`: that gate covers the v1 schemas (bundle / host /
# processes / privileges / closures); this one verifies the W3 DTO
# attribute is present in the Rust source for every named type, so a
# regression that silently drops `deny_unknown_fields` from a W3 DTO
# fails the static gate.
#
# Owner: H3 (Nix emitters + DTO additions). H4 wires this script into
# the top-level `tests/static.sh` driver in a separate commit.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

W3_DTOS=(
  # host_w3.rs — security-sensitive types per plan §"W3 broker variant
  # additions" + plan §"W3 host.json per-field schema gold-file drift
  # gate":
  "IfNameMapping"
  "BridgePortFlagsW3"
  "KernelModuleEntry"
  "RouteIntent"
  "SysctlIntent"
  "HostsEntry"
  "NmUnmanagedEntry"
  "FirewallCoexistencePolicy"
  # host.rs — W3 additions to HostJson (networking-2 + virt-1):
  "HostChConfig"
)

HOST_W3_SRC="$ROOT/packages/nixling-core/src/host_w3.rs"
HOST_SRC="$ROOT/packages/nixling-core/src/host.rs"

if [ ! -f "$HOST_W3_SRC" ] || [ ! -f "$HOST_SRC" ]; then
  log "nixling-core W3 sources absent — skipping W3 deny-unknown-fields gate (W3 unstaged)"
  exit 0
fi

fail_dto() {
  local dto=$1 reason=$2
  fail "W3 DTO '$dto' is missing #[serde(deny_unknown_fields)] ($reason)"
}

# For each named struct, require a `deny_unknown_fields` attribute on
# the line preceding `pub struct <DTO>` (within the previous 10 lines,
# tolerant of multi-line `#[serde(rename_all = "...", deny_unknown_fields)]`).
for dto in "${W3_DTOS[@]}"; do
  src=""
  if grep -qE "^pub struct $dto\b" "$HOST_W3_SRC"; then
    src=$HOST_W3_SRC
  elif grep -qE "^pub struct $dto\b" "$HOST_SRC"; then
    src=$HOST_SRC
  else
    fail "W3 DTO '$dto' not found in host_w3.rs nor host.rs"
  fi
  # Look at the 8 lines immediately preceding the struct decl for the
  # serde attribute. This is conservative enough to catch the canonical
  # `#[serde(rename_all = "camelCase", deny_unknown_fields)]` pattern.
  if ! awk -v dto="pub struct $dto" '
    /^pub struct/ && $0 ~ dto { found=1; exit }
    { lines[NR % 10] = $0 }
    END {
      if (!found) exit 1
      for (i = 0; i < 10; i++) {
        if (lines[i] ~ /deny_unknown_fields/) exit 0
      }
      exit 1
    }
  ' "$src"; then
    fail_dto "$dto" "no deny_unknown_fields attribute within the 10 lines preceding the struct declaration in $src"
  fi
done

log "W3 deny-unknown-fields static gate passed for ${#W3_DTOS[@]} DTOs"
