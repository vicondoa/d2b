# Canonical monolithic per-Zone resource bundle compiler.
{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  specCanonical = import ./generated/zone-spec-canonical.nix;
  artifactRenderer = import ./zone-resources-json.nix { inherit pkgs; };
  apiVersion = "resources.d2bus.org/v3";

  artifactCatalogEntries = map
    (entry: {
      inherit (entry) id type;
      inherit (entry) storePath;
      packageDigest = entry.entry.packageDigest;
      closureMetadata = {
        executableDigest = entry.entry.executableDigest;
        manifestDigest = entry.entry.manifestDigest;
        componentDigest = entry.entry.componentDigest;
        descriptorDigest = entry.entry.descriptorDigest;
        configDigest = entry.entry.configDigest;
        systems = entry.entry.systems;
        platform = entry.entry.platform;
      };
    })
    cfg._providerCatalog.entries;
  artifactCatalogPreimage = {
    schemaVersion = 3;
    entries = artifactCatalogEntries;
  };
  artifactCatalogPreimageJson = builtins.toJSON artifactCatalogPreimage;
  artifactCatalogPath = artifactRenderer.mkArtifactCatalog {
    entriesJson = builtins.toJSON artifactCatalogEntries;
    preimageJson = artifactCatalogPreimageJson;
  };

  emittedResources = resources:
    lib.filterAttrs (_: resource: resource.type != "Zone") resources;

  executionPolicyDefaults = {
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

  stripExecutionDefaults = spec:
    builtins.removeAttrs spec (lib.filter
      (field:
        builtins.hasAttr field spec
        && spec.${field} == executionPolicyDefaults.${field})
      (lib.attrNames executionPolicyDefaults));

  projectOntoDefaults = defaults: authored:
    if builtins.isAttrs defaults && defaults != { } then
      lib.mapAttrs
        (fieldName: fieldDefault:
          if builtins.isAttrs authored && builtins.hasAttr fieldName authored
          then projectOntoDefaults fieldDefault authored.${fieldName}
          else fieldDefault)
        defaults
    else authored;

  credentialSpec = authored:
    let
      spec = stripExecutionDefaults authored;
      scope = spec.scope or { };
      rotation = spec.rotation or { };
      expiry = spec.expiry or { };
      revocation = spec.revocation or { };
      base = {
        providerRef = spec.providerRef;
        scope = {
          executionRef = scope.executionRef or null;
          domainFilter = scope.domainFilter or null;
          userRef = scope.userRef or null;
        };
        audience = spec.audience;
        consumerRef = spec.consumerRef or null;
        allowedOperations = lib.sort lib.lessThan (lib.unique spec.allowedOperations);
        rotation = {
          policy = rotation.policy or "on-expiry";
          proactiveWindowMs = rotation.proactiveWindowMs or null;
          maxLeaseLifetimeMs = rotation.maxLeaseLifetimeMs or 0;
        };
        expiry.hardDeadlineMs = expiry.hardDeadlineMs or 0;
        revocation = {
          onOwnerDelete = revocation.onOwnerDelete or "immediate";
          onProviderGeneration = revocation.onProviderGeneration or "immediate";
        };
        identityGuestRef = spec.identityGuestRef or null;
        loginEndpointRef = spec.loginEndpointRef or null;
      };
    in base
      // lib.optionalAttrs (spec ? updatePolicy) { inherit (spec) updatePolicy; }
      // lib.optionalAttrs (spec ? provider) { inherit (spec) provider; };

  canonicalSpec = resource:
    if resource.type == "Credential" then credentialSpec resource.spec
    else if builtins.hasAttr resource.type specCanonical then
      projectOntoDefaults specCanonical.${resource.type}.defaults resource.spec
    else if builtins.elem resource.type [ "Host" "Guest" ] then resource.spec
    else stripExecutionDefaults resource.spec;

  optionalMetadata = resource:
    lib.optionalAttrs (resource.metadata.ownerRef != null) {
      inherit (resource.metadata) ownerRef;
    }
    // lib.optionalAttrs (resource.metadata.labels != { }) {
      inherit (resource.metadata) labels;
    }
    // lib.optionalAttrs (resource.metadata.annotations != { }) {
      inherit (resource.metadata) annotations;
    };

  canonicalResource = zoneName: resourceName: resource: {
    inherit apiVersion;
    inherit (resource) type;
    metadata = {
      name = resourceName;
      zone = zoneName;
    } // optionalMetadata resource;
    spec = canonicalSpec resource;
  };

  sortResources = resources:
    lib.sort
      (left: right:
        if left.type != right.type then left.type < right.type
        else if left.metadata.zone != right.metadata.zone
        then left.metadata.zone < right.metadata.zone
        else left.metadata.name < right.metadata.name)
      resources;

  zoneResourceList = zoneName: zone:
    sortResources (lib.mapAttrsToList
      (resourceName: resource: canonicalResource zoneName resourceName resource)
      (emittedResources zone.resources));

  catalogEntry = artifactId:
    lib.findFirst (entry: entry.id == artifactId) null artifactCatalogEntries;

  providerSchemaDigests = zone:
    lib.listToAttrs (lib.filter (entry: entry != null) (lib.mapAttrsToList
      (resourceName: resource:
        if resource.type != "Provider" || !(resource.spec ? artifactId) then null
        else
          let catalog = catalogEntry resource.spec.artifactId;
          in if catalog == null || !(catalog.closureMetadata ? configDigest) then null
          else lib.nameValuePair "Provider/${resourceName}" catalog.closureMetadata.configDigest)
      (emittedResources zone.resources)));

  bundleData = zoneName: zone:
    let
      resources = zoneResourceList zoneName zone;
    in {
      schemaVersion = 3;
      bundleVersion = 1;
      zone = zoneName;
      inherit resources;
      generatedAt = "1970-01-01T00:00:00.000Z";
      providerSchemaDigests = providerSchemaDigests zone;
    };

  bundlePath = zoneName: data:
    artifactRenderer.mkZoneResourceBundle {
      inherit zoneName artifactCatalogPreimageJson;
      resourcesJson = builtins.toJSON data.resources;
      providerSchemaDigestsJson = builtins.toJSON data.providerSchemaDigests;
      zoneJson = builtins.toJSON zoneName;
    };

  zoneBundles = lib.mapAttrs
    (zoneName: zone:
      let data = bundleData zoneName zone;
      in {
        inherit data;
        path = bundlePath zoneName data;
        installFileName = "zones/${zoneName}/resource-bundle.json";
        classification = "contractPrivateNonSecret";
        sensitivity = "nonSecret";
      })
    cfg.zones;
in
{
  config = lib.mkIf (cfg.zones != { }) {
    d2b._bundle.zoneResourceBundles = zoneBundles;
    d2b._bundle.extraArtifacts.artifactCatalog = {
      data = artifactCatalogPreimage;
      path = artifactCatalogPath;
      installFileName = "artifact-catalog.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };
  };
}
