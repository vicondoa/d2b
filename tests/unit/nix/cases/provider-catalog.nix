# The offline Provider package catalog: authoring, selection shape, and the
# eval-time rules that make selection exact.
#
# Covers the "Package catalog" section of
# ADR-046-provider-model-and-packaging: `d2b.artifacts.<id>` authoring, the
# compiled catalog's sort order, the frozen entry field set, the exact-digest
# requirement, and the private store path being absent from the public
# projection.
{ mkEval, lib, pkgs, ... }:

let
  shape = import ../../../../nixos-modules/generated/provider-catalog-shape.nix;

  base = { ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
    };
    d2b.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
  };

  # A conformant catalog entry: every frozen field, every digest exact.
  entryFor = name:
    let
      digest = field:
        "sha256:" + builtins.hashString "sha256" "${name}/${field}";
      digestFields = lib.listToAttrs
        (map (field: lib.nameValuePair field (digest field)) shape.digestFields);
      plainFields = lib.listToAttrs
        (map (field: lib.nameValuePair field "${name}/${field}")
          (lib.filter (field: !(lib.elem field shape.digestFields)) shape.fields));
    in
    digestFields // plainFields;

  artifactFor = name: {
    package = pkgs.writeText "artifact-${name}" name;
    type = "provider";
    catalog = entryFor name;
  };

  # Declared deliberately out of alphabetical order: the compiled catalog must
  # sort by artifactId rather than preserve the authoring order.
  authored = {
    d2b.artifacts = {
      provider-wayland = artifactFor "provider-wayland";
      provider-audio = artifactFor "provider-audio";
      provider-storage = artifactFor "provider-storage";
    };
  };

  cfg = (mkEval [ base authored ]).config;
  catalog = cfg.d2b._providerCatalog;

  # The same three artifacts, authored in a different order and built from a
  # reversed list rather than a literal attribute set. The compiled catalog
  # must be identical, because sort order is a function of the identifiers.
  reAuthored = {
    d2b.artifacts = lib.listToAttrs
      (map (name: lib.nameValuePair name (artifactFor name))
        [ "provider-storage" "provider-wayland" "provider-audio" ]);
  };
  cfgReAuthored = (mkEval [ base reAuthored ]).config;

  evalArtifacts = artifacts:
    (mkEval [ base ({ ... }: { d2b.artifacts = artifacts; }) ]).config
      .d2b._providerCatalog.ids;

  # Force the assertion list of a configuration that must fail eval.
  failing = artifacts:
    let
      evaluated = (mkEval [ base ({ ... }: { d2b.artifacts = artifacts; }) ]).config;
      broken = lib.filter (a: !a.assertion) evaluated.assertions;
    in
    if broken == [ ] then "no assertion fired" else (lib.head broken).message;
in
{
  # An empty catalog is the default: no artifact exists unless it is authored.
  # This is the "no PATH scan, no directory discovery" rule stated as a value.
  "provider-catalog/empty-by-default" = {
    expr = (mkEval [ base ]).config.d2b._providerCatalog.ids;
    expected = [ ];
  };

  # The catalog is sorted by artifactId, not by authoring order.
  "provider-catalog/sorted-by-artifact-id" = {
    expr = catalog.ids;
    expected = [ "provider-audio" "provider-storage" "provider-wayland" ];
  };

  # Authoring order does not reach the output.
  "provider-catalog/order-independent" = {
    expr = cfgReAuthored.d2b._providerCatalog.json == catalog.json;
    expected = true;
  };

  # The public projection carries the id, the type, and the frozen entry, and
  # never the private store path.
  "provider-catalog/public-entry-shape" = {
    expr = lib.sort (a: b: a < b)
      (lib.attrNames (lib.head catalog.publicEntries));
    expected = [ "entry" "id" "type" ];
  };

  "provider-catalog/public-projection-has-no-store-path" = {
    expr = lib.any (e: e ? storePath) catalog.publicEntries;
    expected = false;
  };

  # The private catalog may retain a store path for activation.
  "provider-catalog/private-entry-retains-store-path" = {
    expr = lib.all (e: e ? storePath) catalog.entries;
    expected = true;
  };

  # The entry field set is exactly the frozen one.
  "provider-catalog/entry-fields-are-the-frozen-set" = {
    expr = lib.sort (a: b: a < b)
      (lib.attrNames (lib.head catalog.publicEntries).entry);
    expected = lib.sort (a: b: a < b) shape.fields;
  };

  # The excluded mechanisms travel with the catalog, so a consumer reading it
  # sees the absences named rather than inferring them.
  "provider-catalog/excluded-mechanisms-recorded" = {
    expr = shape.excludedMechanisms;
    expected = [
      "directory-discovery"
      "latest"
      "path-scan"
      "runtime-download"
      "runtime-marketplace"
      "version-range-solving"
    ];
  };

  # A missing frozen field is rejected, and the message names it.
  "provider-catalog/missing-field-rejected" = {
    expr =
      let
        message = failing {
          incomplete = {
            package = pkgs.writeText "artifact-incomplete" "incomplete";
            catalog = removeAttrs (entryFor "incomplete") [ "supportContact" ];
          };
        };
      in
      lib.hasInfix "supportContact" message;
    expected = true;
  };

  # A field outside the frozen set is rejected.
  "provider-catalog/unknown-field-rejected" = {
    expr =
      let
        message = failing {
          extra = {
            package = pkgs.writeText "artifact-extra" "extra";
            catalog = (entryFor "extra") // { downloadUrl = "https://example.invalid"; };
          };
        };
      in
      lib.hasInfix "downloadUrl" message;
    expected = true;
  };

  # A digest that is not an exact sha256 is rejected. This is the rule that
  # forecloses version-range solving: there is nothing to solve over.
  "provider-catalog/inexact-digest-rejected" = {
    expr =
      let
        message = failing {
          loose = {
            package = pkgs.writeText "artifact-loose" "loose";
            catalog = (entryFor "loose") // { packageDigest = "latest"; };
          };
        };
      in
      lib.hasInfix "packageDigest" message;
    expected = true;
  };

  # `artifactId` is a plain bounded ID, so a ResourceRef-shaped identifier is
  # rejected rather than quietly accepted as one.
  "provider-catalog/resource-ref-shaped-id-rejected" = {
    expr =
      let
        message = failing {
          "Provider/wayland" = {
            package = pkgs.writeText "artifact-ref" "ref";
            catalog = entryFor "ref";
          };
        };
      in
      lib.hasInfix "plain bounded ID" message;
    expected = true;
  };

  # A single authored artifact still compiles, so the sort is not an artefact
  # of having several.
  "provider-catalog/single-artifact" = {
    expr = evalArtifacts { solo = artifactFor "solo"; };
    expected = [ "solo" ];
  };
}
