# The host contract.
#
# One document aggregates the host-side rows the platform layers agree on:
#
#   - `bindings` - the operator RoleBinding rows authored in Nix. Each row
#     carries the Zone it was authored in, its name, and the owner reference
#     it was authored with, so a reader can attribute a grant without walking
#     the Zone configuration.
#   - `principals` - the committed principal allocation: the name-to-uid/gid
#     map the foundation seed resolves and `host-users.nix` materializes.
#   - `storageRoots` - the declared host storage trees and their ACL rows, as
#     the state-posture contract declares them, each carrying the code that
#     postures it.
#
# The document is framed and hashed the way the Zone resource bundle is: one
# preimage, one `d2b-digest/v1` frame, one digest beside it. A consumer that
# needs to prove it read the same declaration compares the digest, and the
# golden digest in the unit cases fails the moment a row moves.
{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  resourcesBundle = import ./resources-bundle.nix { inherit lib; };
  allocation = builtins.fromJSON
    (builtins.readFile ../docs/reference/policy/principal-allocation.json);
  posture = builtins.fromJSON
    (builtins.readFile ../packages/d2b-broker/src/ops/state-posture-contract.json);

  attrOr = attrs: name: fallback:
    if builtins.isAttrs attrs && builtins.hasAttr name attrs
    then builtins.getAttr name attrs
    else fallback;

  # The operator RoleBinding rows, in Zone then name order.
  bindingRows = zoneName:
    let
      resources = attrOr (attrOr (cfg.zones or { }) zoneName { }) "resources" { };
      rows = lib.mapAttrsToList
        (resourceName: resource:
          lib.optional (attrOr resource "type" null == "RoleBinding") {
            zone = zoneName;
            name = resourceName;
            owner = attrOr (attrOr resource "metadata" { }) "ownerRef"
              (attrOr (attrOr resource "spec" { }) "providerRef" null);
            spec = attrOr resource "spec" { };
          })
        resources;
    in
    builtins.filter (row: row != null) (lib.concatLists rows);

  bindings = lib.concatMap bindingRows
    (lib.sort lib.lessThan (builtins.attrNames (cfg.zones or { })));

  # The committed allocation, in name order.
  principals = lib.mapAttrsToList
    (name: entry: { inherit name; uid = entry.uid; gid = entry.gid; })
    allocation.principals;

  # The declared storage trees, in declaration order, with their ACL rows.
  storageRoots = map
    (tree: {
      inherit (tree) id root creator postureOwner;
      levels = map
        (level: {
          inherit (level) path owner group mode;
          acls = attrOr level "acl" [ ];
        })
        tree.levels;
    })
    posture.trees;

  # The preimage is the document without its own digest: the digest covers
  # exactly the rows a consumer compares.
  preimage = {
    contractVersion = 1;
    inherit bindings principals storageRoots;
  };
  preimageJson = builtins.toJSON preimage;
  digest = "sha256:${resourcesBundle.framedDigest "d2b:v3:host-contract" preimageJson}";
  data = preimage // { inherit digest; };
  jsonText = builtins.toJSON data;
  path = pkgs.writeText "d2b-host-contract.json" "${jsonText}\n";
in
{
  options.d2b._hostContract = lib.mkOption {
    type = lib.types.attrsOf lib.types.anything;
    default = { };
    internal = true;
    visible = false;
    description = ''
      Internal host-contract projection: one preimage and its framed digest
      over the operator bindings, the committed principal allocation, and the
      declared storage roots. Consumers compare the digest instead of
      restating the rows.
    '';
  };

  config.d2b._hostContract = {
    inherit
      bindings
      data
      digest
      jsonText
      path
      preimage
      preimageJson
      principals
      storageRoots
      ;
  };
}
