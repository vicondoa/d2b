# v3 artifact and Provider catalog declarations.
#
# The older provider-catalog module remains the compatibility owner of
# `d2b.artifacts` and its frozen metadata fields. This module adds the v3
# Provider selection catalog without putting derivations or store paths into
# ResourceSpecs.
{ config, lib, ... }:

let
  cfg = config.d2b;
  idPattern = "^[a-z][a-z0-9-]*$";
  providerCatalogEntry = lib.types.submodule {
    freeformType = null;
    options = {
      artifactId = lib.mkOption {
        type = lib.types.strMatching idPattern;
        description = "Provider artifact selected by this catalog entry.";
      };
      trust = lib.mkOption {
        type = lib.types.submodule {
          freeformType = null;
          options.publisherRef = lib.mkOption {
            type = lib.types.strMatching idPattern;
            default = "d2b-official";
          };
        };
        default = { };
      };
    };
  };

  catalog = cfg.providerCatalog or { };
  entries = lib.mapAttrsToList
    (name: entry: {
      inherit name entry;
    })
    catalog;
  artifactFor = id:
    if builtins.hasAttr id (cfg.artifacts or { })
    then cfg.artifacts.${id}
    else null;
  duplicateIds = lib.unique (lib.filter
    (id: lib.length (lib.filter (entry: entry.entry.artifactId == id) entries) > 1)
    (map (entry: entry.entry.artifactId) entries));
  duplicateNames = id:
    map (entry: entry.name)
      (lib.filter (entry: entry.entry.artifactId == id) entries);

  trustKnown = zone: publisher:
    publisher == "d2b-official"
    || builtins.hasAttr publisher (zone.trustedPublishers or { });

  assertions = lib.concatMap
    (entry:
      let
        path = "d2b.providerCatalog.${entry.name}";
        artifact = artifactFor entry.entry.artifactId;
        trustedZones = lib.filter
          (zone: trustKnown zone entry.entry.trust.publisherRef)
          (lib.attrValues cfg.zones);
      in [
        {
          assertion = artifact != null && (artifact.type or null) == "provider";
          message = "${path}.artifactId must resolve to a provider artifact.";
        }
        {
          assertion = duplicateIds == [ ]
            || !(builtins.elem entry.entry.artifactId duplicateIds);
          message = "${path}: provider-catalog-duplicate-artifact-id (${lib.concatStringsSep ", " (duplicateNames entry.entry.artifactId)} share ${entry.entry.artifactId}).";
        }
        {
          assertion = entry.entry.trust.publisherRef == "d2b-official"
            || trustedZones != [ ];
          message = "${path}.trust.publisherRef is not a trusted publisher for any declared Zone.";
        }
      ])
    entries;
in
{
  options.d2b.providerCatalog = lib.mkOption {
    type = lib.types.attrsOf providerCatalogEntry;
    default = { };
    description = ''
      Offline Provider catalog. The attribute name is an operator-facing
      catalog key; artifactId is the unique Provider identity. Entries select
      declared provider artifacts and never carry a package derivation.
    '';
  };

  options.d2b._artifactCatalogConfig = lib.mkOption {
    type = lib.types.attrsOf lib.types.anything;
    default = { };
    internal = true;
    visible = false;
  };

  config = {
    assertions = assertions;
    d2b._artifactCatalogConfig = {
      ids = lib.sort lib.lessThan (map (entry: entry.entry.artifactId) entries);
      entries = entries;
      duplicateArtifactIds = duplicateIds;
    };
  };
}
