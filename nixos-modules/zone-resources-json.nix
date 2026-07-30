# Per-Zone resource bundle emitter (ADR 0046 zone routing).
#
# Renders one monolithic `/etc/d2b/zones/<zone>/resource-bundle.json` per
# declared Zone from `d2b.zones.<zone>.resources.*`, plus the private sealed
# child-to-parent topology rows the local-root allocator bootstrap consumes.
#
# The canonical spec projection comes from the generated
# `./generated/zone-spec-canonical.nix` table, so the emitted field set for a
# ResourceType cannot drift from the committed JSON Schema.
{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;

  specCanonical = import ./generated/zone-spec-canonical.nix;

  apiVersion = "resources.d2bus.org/v3";

  localRootZoneName = "local-root";

  # The distinguished Zone self-resource is controller-created and is never
  # emitted into any bundle.
  emittedResources = resources:
    lib.filterAttrs (_: resource: resource.type != "Zone") resources;

  # Structural-base defaults injected by the shared `spec` submodule in
  # nixos-modules/options-zones.nix. They are the canonical ExecutionPolicy for
  # Host and Guest, so they are kept there; for any other ResourceType a key
  # still holding its structural default was not authored and is dropped rather
  # than fabricated into that type's canonical spec.
  executionPolicyBaseDefaults = {
    providerRef = null;
    defaultDomain = "system";
    allowedDomains = [ "system" ];
    defaultUserRef = null;
    budget = {
      cpu = { request = null; limit = null; };
      memory = { request = null; limit = null; };
      pids = { limit = null; };
      fds = { limit = null; };
      ioWeight = null;
      networkEgressBps = null;
      threadLimit = null;
    };
    networkAttachments = [ ];
    deviceAttachments = [ ];
    volumeAttachmentDefaults = [ ];
  };

  executionPolicyTypes = [ "Host" "Guest" ];

  # Project an authored spec onto the generated canonical default tree: only
  # declared fields survive, every declared field is emitted, and an unauthored
  # field takes its generated default.
  projectOntoDefaults = defaults: authored:
    if builtins.isAttrs defaults && defaults != { }
    then
      lib.mapAttrs
        (fieldName: fieldDefault:
          if builtins.isAttrs authored && builtins.hasAttr fieldName authored
          then projectOntoDefaults fieldDefault authored.${fieldName}
          else fieldDefault)
        defaults
    else authored;

  # Fallback for a ResourceType that has no committed canonical schema yet.
  stripUnauthoredBaseDefaults = spec:
    let
      unauthored = lib.filter
        (fieldName:
          builtins.hasAttr fieldName spec
          && spec.${fieldName} == executionPolicyBaseDefaults.${fieldName})
        (lib.attrNames executionPolicyBaseDefaults);
    in
    builtins.removeAttrs spec unauthored;

  canonicalSpec = resource:
    if builtins.hasAttr resource.type specCanonical
    then projectOntoDefaults specCanonical.${resource.type}.defaults resource.spec
    else if builtins.elem resource.type executionPolicyTypes
    then resource.spec
    else stripUnauthoredBaseDefaults resource.spec;

  optionalMetadata = resource:
    (lib.optionalAttrs (resource.metadata.ownerRef != null) {
      inherit (resource.metadata) ownerRef;
    })
    // (lib.optionalAttrs
      (resource.metadata ? labels && resource.metadata.labels != { })
      { inherit (resource.metadata) labels; })
    // (lib.optionalAttrs
      (resource.metadata ? annotations && resource.metadata.annotations != { })
      { inherit (resource.metadata) annotations; });

  canonicalResource = zoneName: resourceName: resource: {
    inherit apiVersion;
    inherit (resource) type;
    metadata = {
      name = resourceName;
      zone = zoneName;
    } // optionalMetadata resource;
    spec = canonicalSpec resource;
  };

  # Canonical order is (type, zone, name). Within one Zone bundle the zone
  # component is constant, so the comparison reduces to (type, name).
  sortResources = resources:
    lib.sort
      (left: right:
        if left.type != right.type
        then left.type < right.type
        else if left.metadata.zone != right.metadata.zone
        then left.metadata.zone < right.metadata.zone
        else left.metadata.name < right.metadata.name)
      resources;

  zoneResourceList = zoneName: zone:
    sortResources (lib.mapAttrsToList
      (resourceName: resource: canonicalResource zoneName resourceName resource)
      (emittedResources zone.resources));

  # Provider settings-schema digests are a digest-chain member sourced from the
  # Provider package catalog, which this emitter does not yet have. The field is
  # emitted as the empty map rather than being omitted from the envelope.
  providerSchemaDigests = { };

  bundleDerivation = zoneName: zone:
    pkgs.runCommand "d2b-zone-${zoneName}-bundle"
      {
        resourcesJson = builtins.toJSON (zoneResourceList zoneName zone);
        providerSchemaDigestsJson = builtins.toJSON providerSchemaDigests;
        zoneJson = builtins.toJSON zoneName;
        passAsFile = [ "resourcesJson" "providerSchemaDigestsJson" ];
      }
      ''
        set -euo pipefail

        # D101 digest: SHA-256(domain_tag || 0x00 || canonical_bytes), taken
        # over the canonical sorted resources array alone.
        contentHash=$(
          {
            printf 'd2b:v3:resource-bundle\000'
            cat "$resourcesJsonPath"
          } | sha256sum | cut -d' ' -f1
        )

        mkdir -p "$out"
        {
          printf '%s' '{"bundleVersion":1,"contentHash":"sha256:'
          printf '%s' "$contentHash"
          printf '%s' '","generatedAt":"1970-01-01T00:00:00.000Z"'
          printf '%s' ',"providerSchemaDigests":'
          cat "$providerSchemaDigestsJsonPath"
          printf '%s' ',"resources":'
          cat "$resourcesJsonPath"
          printf '%s' ',"schemaVersion":3,"zone":'
          printf '%s' "$zoneJson"
          printf '%s' '}'
        } > "$out/bundle.json"
      '';

  zoneNames = lib.sort lib.lessThan (lib.attrNames cfg.zones);

  zoneBundleArtifacts = lib.listToAttrs (map
    (zoneName: {
      name = zoneName;
      value = {
        path = "${bundleDerivation zoneName cfg.zones.${zoneName}}/bundle.json";
        installFileName = "zones/${zoneName}/resource-bundle.json";
        classification = "contractPrivateNonSecret";
        sensitivity = "nonSecret";
      };
    })
    zoneNames);

  # Compiler-only topology. `parentZone` never enters a resource bundle or
  # Zone.spec; it is canonicalized here as sorted child-to-parent rows for the
  # local-root allocator bootstrap sealer, which is why this artifact declares
  # no install path.
  topologyRows = map
    (zoneName: {
      childZone = zoneName;
      parentZone = cfg.zones.${zoneName}.parentZone;
    })
    (lib.filter (zoneName: cfg.zones.${zoneName}.parentZone != null) zoneNames);
in
{
  config = lib.mkIf (cfg.zones != { }) {
    d2b._bundle.zoneResourceBundles = zoneBundleArtifacts;

    d2b._bundle.extraArtifacts.zoneTopology = {
      data = {
        schemaVersion = 3;
        localRootZone = localRootZoneName;
        edges = topologyRows;
      };
      installFileName = null;
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };
  };
}
