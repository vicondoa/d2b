# The offline artifact catalog.
#
# `ADR-046-provider-model-and-packaging`, "Package catalog": Nix authoring
# declares each Provider derivation separately under `d2b.artifacts.<id>`, and
# a Provider ResourceSpec then selects one by `artifactId`. Nix compiles those
# declarations into an offline, sorted, exact-digest catalog.
#
# Three absences are the design and not omissions:
#
#   * There is no runtime marketplace and no download. Every artifact is a
#     store path this evaluation already has.
#   * There is no PATH scan and no directory discovery. An artifact exists in
#     the catalog only because it was authored.
#   * There is no `latest` and no version-range solving. Selection is by exact
#     digest, and an `artifactId` that names nothing is an error rather than a
#     resolution problem.
#
# `artifactId` is a plain bounded ID, not a ResourceRef, and Artifact is not a
# ResourceType. Provider packages and generic NixOS systems are distinct closed
# artifact kinds. The catalog may retain a store path for activation; the public
# projection strips it, because a resource spec, status, or audit record never
# exposes one.

{ config, lib, pkgs, ... }:

let
  types = lib.types;
  cfg = config.d2b;

  # The generated frozen entry shape. Generated rather than written here so
  # this module and any later consumer cannot drift apart silently.
  shape = import ./generated/provider-catalog-shape.nix;

  # `artifactId` grammar: a plain bounded ID. Lowercase alphanumerics and
  # hyphens, starting with a letter, so it can never be confused with a
  # ResourceRef and never needs quoting.
  artifactIdPattern = "[a-z][a-z0-9-]*";
  maxArtifactIdLength = 64;

  # A digest is recorded as an algorithm-qualified lowercase hex string. The
  # shape is pinned here rather than left free-form because exact-digest
  # selection compares these values literally.
  digestPattern = "sha256:[0-9a-f]{64}";

  artifactModule = types.submodule ({ name, config, ... }: {
    options = {
      package = lib.mkOption {
        type = types.package;
        description = ''
          The derivation providing this artifact. Declared by the consumer's
          own Nix authoring, typically from a flake input.
        '';
      };

      type = lib.mkOption {
        type = types.enum [
          "provider"
          "nixos-system"
          "nixos-module-set"
          "config-bundle"
        ];
        default = "provider";
        description = ''
          The artifact kind. Provider packages and generic NixOS systems are
          separate closed kinds; the option is an enum so a new kind remains
          an explicit decision rather than a free string.
        '';
      };

      catalog = lib.mkOption {
        type = types.nullOr (types.attrsOf types.anything);
        default = null;
        description = ''
          The catalog entry for this artifact: the frozen field set from the
          specification's "Package catalog" section. Every field in
          `fields` must be present, and every digest field must carry an
          `sha256:<64 hex>` value, because selection is by exact digest.
        '';
      };

      artifactId = lib.mkOption {
        type = types.str;
        default = name;
        readOnly = true;
        description = "The authored identifier, which is the attribute name.";
      };
    };
  });

  artifacts = cfg.artifacts;
  artifactIds = lib.sort (a: b: a < b) (lib.attrNames artifacts);

  # The catalog: sorted by artifactId, so the emitted order is a function of
  # the identifiers alone and not of the order the consumer happened to declare
  # them in. This is what makes two independent evaluations of the same
  # declarations produce the same bytes.
  entries = map
    (id:
      let artifact = artifacts.${id};
      in {
        inherit id;
        inherit (artifact) type;
        storePath = "${artifact.package}";
        entry =
          if artifact.catalog == null
          then { }
          else lib.filterAttrs (fieldName: _: lib.elem fieldName shape.fields) artifact.catalog;
      })
    artifactIds;

  # The public projection. `storePath` is private catalog data retained for
  # activation and is stripped here, because a resource spec, status, or audit
  # record never exposes a store path.
  publicEntries = map (e: { inherit (e) id type entry; }) entries;

  # The provider catalog is a separate public document.  It carries only the
  # frozen package metadata; private store locations remain in the artifact
  # catalog used by activation.
  providerCatalogEntries = lib.sort
    (left: right:
      let
        leftName = left.entry.providerName or left.id;
        rightName = right.entry.providerName or right.id;
      in leftName < rightName)
    entries;
  providerCatalogData = {
    schemaVersion = "v1";
    entries = map
      (entry: {
        providerName = entry.entry.providerName or entry.id;
        artifactId = entry.id;
      } // entry.entry)
      providerCatalogEntries;
  };
  providerCatalogJson = builtins.toJSON providerCatalogData;
  providerCatalogPath = pkgs.writeText "d2b-provider-catalog.json" providerCatalogJson;

  catalogJson = builtins.toJSON {
    excludedMechanisms = shape.excludedMechanisms;
    entries = publicEntries;
  };

  missingFields = id:
    if artifacts.${id}.catalog == null
    then [ ]
    else lib.filter (field: !(artifacts.${id}.catalog ? ${field})) shape.fields;

  unknownFields = id:
    if artifacts.${id}.catalog == null
    then [ ]
    else lib.filter (field: !(lib.elem field shape.fields))
      (lib.attrNames artifacts.${id}.catalog);

  badDigests = id:
    if artifacts.${id}.catalog == null
    then [ ]
    else lib.filter
      (field:
        let value = artifacts.${id}.catalog.${field} or null;
        in value == null || builtins.match digestPattern (toString value) == null)
      shape.digestFields;

in
{
  options.d2b.artifacts = lib.mkOption {
    type = types.attrsOf artifactModule;
    default = { };
    description = ''
      Artifact declarations. Each entry names a derivation, its closed kind,
      and its catalog metadata. Provider ResourceSpecs select `provider`
      entries with `artifactId`; Guest system fields select `nixos-system`
      entries. There is no runtime discovery of any kind: an artifact that is
      not declared here does not exist.
    '';
    example = lib.literalExpression ''
      {
        provider-wayland = {
          package = inputs.wayland-provider.packages.''${system}.default;
          type = "provider";
        };
      }
    '';
  };

  options.d2b._providerCatalog = lib.mkOption {
    type = types.attrsOf types.anything;
    internal = true;
    visible = false;
    default = {
      inherit entries publicEntries providerCatalogEntries providerCatalogData providerCatalogJson providerCatalogPath;
      json = catalogJson;
      ids = artifactIds;
      shape = shape;
    };
    description = "Internal compiled artifact catalog.";
  };

  config = {
    d2b._providerCatalog = {
      inherit entries publicEntries providerCatalogEntries providerCatalogData providerCatalogJson providerCatalogPath;
      json = catalogJson;
      ids = artifactIds;
      shape = shape;
    };

    d2b._bundle.extraArtifacts.providerCatalog = {
      data = providerCatalogData;
      jsonText = providerCatalogJson;
      path = providerCatalogPath;
      installFileName = "provider-catalog.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };
  };

  config.assertions =
    # The identifier grammar.
    (map
      (id: {
        assertion = builtins.match artifactIdPattern id != null
          && builtins.stringLength id <= maxArtifactIdLength;
        message = ''
          d2b.artifacts."${id}": artifactId must match ${artifactIdPattern}
          and be at most ${toString maxArtifactIdLength} characters. It is a
          plain bounded ID, not a ResourceRef.
        '';
      })
      artifactIds)

    # Every frozen field present.
    ++ (map
      (id: {
        assertion = missingFields id == [ ];
        message = ''
          d2b.artifacts."${id}".catalog is missing required catalog
          field(s): ${lib.concatStringsSep ", " (missingFields id)}.
          The catalog entry shape is frozen by the Package catalog section of
          ADR-046-provider-model-and-packaging.
        '';
      })
      artifactIds)

    # No field outside the frozen set.
    ++ (map
      (id: {
        assertion = unknownFields id == [ ];
        message = ''
          d2b.artifacts."${id}".catalog declares unknown catalog
          field(s): ${lib.concatStringsSep ", " (unknownFields id)}.
          The catalog entry shape is frozen; add a field to the generator, not
          to a consumer declaration.
        '';
      })
      artifactIds)

    # Exact digests, because selection compares them literally.
    ++ (map
      (id: {
        assertion = badDigests id == [ ];
        message = ''
          d2b.artifacts."${id}".catalog has malformed or absent
          digest(s): ${lib.concatStringsSep ", " (badDigests id)}.
          Each must be sha256:<64 lowercase hex>. Selection is by exact digest;
          there is no version-range solving and no latest.
        '';
      })
      artifactIds);
}
