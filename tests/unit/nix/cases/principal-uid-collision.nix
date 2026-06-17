# nix-unit cases migrated from tests/principal-uid-collision-eval.sh.
#
# Eval-time UID-collision gate for `stablePrincipalId` across every
# principal declared by the multi-env consumer example. stablePrincipalId
# maps a principal name to a deterministic 24-bit UID in [50000, 16827215]
# (50000 + fromHexString(sha256(principal)[0..6])); two distinct principals
# landing on the same UID would share a host_uid_for_zero in the broker
# pre-NS user_namespace mapping (ADR 0021), silently breaking
# least-privilege isolation.
#
# Asserts (faithful to the bash gate's three checks):
#   1. No UID collision across distinct principals.
#   2. Every UID (excluding the root=0 short-circuit) in [50000, 16827215].
#   3. Every rendered nixling-* system user has a unique UID.
# Plus two non-vacuity guards (the bash gate reported these counts):
#   * 32 principal/uid profile entries are extracted.
#   * They resolve to 5 distinct (principal, uid) pairs — one runner per VM
#     (work-app, personal-app, sys-work-net, sys-personal-net) plus
#     nixlingd.
#
# Reads only `.data.{uid,principal}` from each minijail profile entry —
# `.path` (writeText) and `.roleProfile` are never forced. multi-env is
# graphics-free, so the cases contribute on every system and the expected
# values are platform-independent.
{ mkEval, lib, flakeRoot, ... }:

let
  cfg = (mkEval [
    (import (flakeRoot + "/examples/multi-env/configuration.nix"))
  ]).config;

  profiles = cfg.nixling._bundle.minijailProfiles;
  pairs = lib.mapAttrsToList
    (profileId: e: { principal = e.data.principal; uid = e.data.uid; inherit profileId; })
    profiles;

  uniqueUids = lib.unique (map (p: p.uid) pairs);

  # UIDs shared by more than one distinct principal name (a collision).
  collisionUids = lib.filter
    (u: (lib.length (lib.unique
      (map (p: p.principal) (lib.filter (p: p.uid == u) pairs)))) > 1)
    uniqueUids;

  outOfRange = lib.filter
    (p: p.uid != 0 && (p.uid < 50000 || p.uid > 16827215))
    pairs;
  allInRange = lib.all
    (p: p.uid == 0 || (p.uid >= 50000 && p.uid <= 16827215))
    pairs;

  nixlingUsers = lib.mapAttrsToList
    (name: user: { inherit name; uid = user.uid; })
    (lib.filterAttrs (name: _: lib.hasPrefix "nixling-" name) cfg.users.users);
  userUniqueUids = lib.unique (map (u: u.uid) nixlingUsers);
  userCollisionUids = lib.filter
    (uid: (lib.length (lib.filter (u: u.uid == uid) nixlingUsers)) > 1)
    userUniqueUids;
in
{
  "principal-uid-collision/no-uid-collision" = {
    expr = lib.length collisionUids;
    expected = 0;
  };
  "principal-uid-collision/all-uids-in-range" = {
    expr = allInRange;
    expected = true;
  };
  "principal-uid-collision/no-out-of-range" = {
    expr = lib.length outOfRange;
    expected = 0;
  };
  "principal-uid-collision/no-user-uid-collision" = {
    expr = lib.length userCollisionUids;
    expected = 0;
  };
  "principal-uid-collision/principal-count" = {
    expr = lib.length pairs;
    expected = 32;
  };
  "principal-uid-collision/distinct-uid-count" = {
    expr = lib.length uniqueUids;
    expected = 5;
  };
}
