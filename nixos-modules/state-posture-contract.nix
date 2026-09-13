# Declared host posture contract - Nix read-side.
#
# The machine-readable declaration lives at
# `packages/d2b-broker/src/ops/state-posture-contract.json`, so one file is the
# single source for every consumer:
#
#   - `packages/d2b-broker/src/ops/store_view_posture.rs` embeds and applies
#     the `guest-store-view` rows (the broker posture code).
#   - `nixos-modules/host-daemon.nix` derives the `shared-run-dir` and
#     `state-root` tmpfiles lines from the same rows (the Nix provisioning).
#   - `tests/host-integration/state-posture-contract.nix` asserts the live host
#     against every row, as each principal.
#
# Nothing here restates the contract: a missing tree/level or an unsupported
# row field fails closed at evaluation time.
{ lib }:

let
  contract = builtins.fromJSON
    (builtins.readFile ../packages/d2b-broker/src/ops/state-posture-contract.json);

  tree = id:
    let matches = builtins.filter (candidate: candidate.id == id) contract.trees;
    in if matches == [ ] then
      throw "state-posture-contract: tree `${id}` is missing"
    else
      builtins.head matches;

  level = treeId: path:
    let matches = builtins.filter
          (candidate: candidate.path == path)
          (tree treeId).levels;
    in if matches == [ ] then
      throw "state-posture-contract: tree `${treeId}` has no level `${path}`"
    else
      builtins.head matches;

  absolutePath = treeId: path:
    if (level treeId path).path == "." then
      (tree treeId).root
    else
      "${(tree treeId).root}/${(level treeId path).path}";

  # systemd-tmpfiles lines for one declared level: create (+ optional relabel)
  # and the ACL grants this layer applies at boot. ACL rows applied by the
  # broker's spawn preflight (applier "spawn") are asserted by the live
  # validation, never provisioned here.
  tmpfilesRule = treeId: path:
    let
      row = level treeId path;
      target = absolutePath treeId path;
      acls = builtins.filter (acl: acl.applier == "tmpfiles") (row.acl or [ ]);
    in
      [ "d ${target} ${row.mode} ${row.owner} ${row.group} -" ]
      ++ lib.optional (row.relabel or false)
        "z ${target} ${row.mode} ${row.owner} ${row.group} -"
      ++ map (acl: "a+ ${target} - - - - ${acl.spec}") acls;
in
{
  inherit contract;
  inherit tree level absolutePath tmpfilesRule;
}
