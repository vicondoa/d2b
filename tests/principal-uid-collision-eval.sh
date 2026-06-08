#!/usr/bin/env bash
# tests/principal-uid-collision-eval.sh — eval-time UID-collision assertion
# for stablePrincipalId across all principals declared in the multi-env
# consumer-flake example.
#
# stablePrincipalId maps a principal name to a deterministic 24-bit UID in
# [50000, 16827215] via:
#   50000 + lib.fromHexString(sha256(principal)[0..6])
# Two principals that land on the same UID would share a host_uid_for_zero
# in the broker-pre-NS user_namespace mapping (ADR 0021), silently breaking
# least-privilege isolation.
#
# Asserts:
#   1. Every stablePrincipalId output is unique across all declared principals
#      (no collision between distinct principal names).
#   2. Every output (excluding root=0 short-circuit) falls in [50000, 16827215].
#   3. Every nixling-* entry in system.users.users has a unique UID (rendered
#      NixOS config collision count is zero).
#
# Does NOT mask genuine collisions: if a collision exists in the current
# consumer flake the test fails with detailed output naming the colliding
# principals and the UID they share.
#
# Wired into tests/static.sh (mid-tier eval pool).

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

SCRATCH=$(nl_mktemp .principal-uid-collision-eval.XXXXXX)

PASS=0
FAIL=0

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; PASS=$((PASS+1)); }
fail() { log "  FAIL: $*"; FAIL=$((FAIL+1)); }

log '==> tests/principal-uid-collision-eval.sh'

# ---------------------------------------------------------------------------
# Build the nix expression.
#
# Evaluates examples/multi-env/configuration.nix against the framework's
# nixosModules.default and extracts:
#   principalUidPairs  — [{ principal, uid, profileId }] from every entry
#                        in config.nixling._bundle.minijailProfiles
#   nixlingUsers       — [{ name, uid }] for every nixling-* system user
#                        declared in config.users.users
#
# Only .data.uid and .data.principal are accessed from each profile entry;
# .path (pkgs.writeText derivation) and .roleProfile are never forced.
# ---------------------------------------------------------------------------

EXPR=$(cat <<EOF
let
  pkgs = import <nixpkgs> { system = "x86_64-linux"; };
  inherit (pkgs) lib;
  flake = builtins.getFlake (toString $ROOT);
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
  pkgsForSystem = import flake.inputs.nixpkgs {
    system = "x86_64-linux";
    config = { allowUnsupportedSystem = true; };
  };
  nixos = nixosSystem {
    system = "x86_64-linux";
    pkgs = pkgsForSystem;
    modules = [
      flake.nixosModules.default
      (import $ROOT/examples/multi-env/configuration.nix)
    ];
  };

  profiles = nixos.config.nixling._bundle.minijailProfiles;

  # Extract principal + uid from each profile entry.
  # Access only .data.{uid,principal} — .path (pkgs.writeText) and
  # .roleProfile (toRoleProfile) are never accessed here and stay unevaluated.
  principalUidPairs = lib.mapAttrsToList
    (profileId: profileEntry: {
      principal = profileEntry.data.principal;
      uid       = profileEntry.data.uid;
      inherit profileId;
    })
    profiles;

  # system.users.users entries for nixling-* accounts only.
  nixlingUsers = lib.mapAttrsToList
    (name: user: { inherit name; uid = user.uid; })
    (lib.filterAttrs (name: _: lib.hasPrefix "nixling-" name)
      nixos.config.users.users);
in {
  inherit principalUidPairs nixlingUsers;
  principalCount = lib.length principalUidPairs;
  userCount      = lib.length nixlingUsers;
}
EOF
)

OUT_FILE="$SCRATCH/out.json"
ERR_FILE="$SCRATCH/err.txt"

log '  --> nix-instantiate --eval --strict --json (multi-env stablePrincipalId extract)'

if ! nix-instantiate --eval --strict --json \
       --expr "$EXPR" \
       > "$OUT_FILE" 2> "$ERR_FILE"; then
  log "  FAIL: nix eval failed — cannot extract stablePrincipalId data"
  log "    --- stderr (tail) ---"
  tail -20 "$ERR_FILE" | sed 's/^/      /' >&2
  exit 1
fi

PRINCIPAL_COUNT=$(jq -r '.principalCount' "$OUT_FILE")
USER_COUNT=$(jq -r '.userCount' "$OUT_FILE")
log "  --> extracted $PRINCIPAL_COUNT principal/uid pairs, $USER_COUNT nixling system users"

# ---------------------------------------------------------------------------
# Assertion 1: every stablePrincipalId output is unique.
# Group principalUidPairs by uid; any group with more than one distinct
# principal name is a collision.
# ---------------------------------------------------------------------------
COLLISIONS=$(jq -r '
  .principalUidPairs
  | group_by(.uid)
  | map(select(
      (map(.principal) | unique | length) > 1
    ))
  | .[]
  | "  UID \(.[0].uid): " + ([.[].principal] | unique | join(", "))
' "$OUT_FILE")

if [ -n "$COLLISIONS" ]; then
  fail "stablePrincipalId UID collision(s) detected — distinct principals share a UID:"
  while IFS= read -r line; do
    log "    $line"
  done <<< "$COLLISIONS"
  log "    This breaks broker-pre-NS user_namespace mapping (ADR 0021)."
  log "    Mitigation: rename a colliding VM or host-singleton principal."
else
  ok "stablePrincipalId — no UID collisions across $PRINCIPAL_COUNT principal/uid pairs"
fi

# ---------------------------------------------------------------------------
# Assertion 2: every UID (excluding the root=0 short-circuit) falls in
# [50000, 16827215].
# Formula: 50000 + fromHexString(sha256(p)[0..6]) → max 50000+16777215=16827215.
# ---------------------------------------------------------------------------
OUT_OF_RANGE=$(jq -r '
  .principalUidPairs
  | map(select(.uid != 0 and (.uid < 50000 or .uid > 16827215)))
  | .[]
  | "  principal \"\(.principal)\" (profile \(.profileId)): uid \(.uid)"
' "$OUT_FILE")

if [ -n "$OUT_OF_RANGE" ]; then
  fail "stablePrincipalId out-of-range UID(s) detected:"
  while IFS= read -r line; do
    log "    $line"
  done <<< "$OUT_OF_RANGE"
  log "    Valid range: [50000, 16827215] (50000 + 24-bit sha256 prefix)."
else
  ok "stablePrincipalId — all UIDs in valid range [50000, 16827215]"
fi

# ---------------------------------------------------------------------------
# Assertion 3: the rendered system.users.users.*uid set has zero collisions.
# Any two distinct nixling-* system users sharing a UID is a passwd mismatch.
# ---------------------------------------------------------------------------
USER_COLLISIONS=$(jq -r '
  .nixlingUsers
  | group_by(.uid)
  | map(select(length > 1))
  | .[]
  | "  UID \(.[0].uid): " + ([.[].name] | unique | join(", "))
' "$OUT_FILE")

if [ -n "$USER_COLLISIONS" ]; then
  fail "system.users.users UID collision(s) detected — distinct nixling users share a UID:"
  while IFS= read -r line; do
    log "    $line"
  done <<< "$USER_COLLISIONS"
  log "    Indicates stablePrincipalId collision propagated to system passwd."
else
  ok "system.users.users — zero nixling UID collisions across $USER_COUNT users"
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
log "==> tests/principal-uid-collision-eval.sh: $PASS PASS, $FAIL FAIL"
log "    ($PRINCIPAL_COUNT principals checked, $USER_COUNT system users verified)"

if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
