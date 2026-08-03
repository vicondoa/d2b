# Canonical monolithic per-Zone resource bundle compiler.
{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  specCanonical = import ./generated/zone-spec-canonical.nix;
  artifactRenderer = import ./zone-resources-json.nix { inherit pkgs; };
  apiVersion = "resources.d2bus.org/v3";

  providerCatalogEntries = map
    (entry: {
      inherit (entry) id type;
      inherit (entry) storePath;
      packageDigest = entry.entry.packageDigest or null;
      closureMetadata = {
        executableDigest = entry.entry.executableDigest or null;
        manifestDigest = entry.entry.manifestDigest or null;
        componentDigest = entry.entry.componentDigest or null;
        descriptorDigest = entry.entry.descriptorDigest or null;
        configDigest = entry.entry.configDigest or null;
        systems = entry.entry.systems or [ ];
        platform = entry.entry.platform or null;
      };
    })
    (cfg._providerCatalog.entries or [ ]);
  artifactCatalogPreimage = {
    schemaVersion = 3;
    entries = providerCatalogEntries;
  };
  artifactCatalogPreimageJson = builtins.toJSON artifactCatalogPreimage;
  canonicalArtifactCatalog =
    if cfg ? _artifactCatalogV3 then cfg._artifactCatalogV3 else { };
  canonicalArtifactCatalogPreimageJson =
    canonicalArtifactCatalog.preimageJson or artifactCatalogPreimageJson;
  artifactCatalogPath =
    canonicalArtifactCatalog.path or (artifactRenderer.mkArtifactCatalog {
      entriesJson = builtins.toJSON providerCatalogEntries;
      preimageJson = canonicalArtifactCatalogPreimageJson;
    });
  schemaValidation = cfg._resourceCompiler.schemaValidation or { };
  schemaValidationPath = schemaValidation.buildValidation or null;

  emittedResources = resources:
    lib.filterAttrs (_: resource: resource.type != "Zone") resources;

  zoneResources = zoneName: zone:
    zone.resources
    // (cfg._resourceCompiler.volumeGenerated.byZone.${zoneName} or { })
    // (cfg._resourceCompiler.volumeShorthand.${zoneName} or { });

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
    let
      metadata = resource.metadata or { };
      ownerRef = metadata.ownerRef or null;
      labels = metadata.labels or { };
      annotations = metadata.annotations or { };
    in
    lib.optionalAttrs (ownerRef != null) { inherit ownerRef; }
    // lib.optionalAttrs (labels != { }) { inherit labels; }
    // lib.optionalAttrs (annotations != { }) { inherit annotations; };

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
      (emittedResources (zoneResources zoneName zone)));

  catalogEntry = artifactId:
    lib.findFirst (entry: entry.id == artifactId) null providerCatalogEntries;

  providerSchemaDigests = zoneName: zone:
    lib.listToAttrs (lib.filter (entry: entry != null) (lib.mapAttrsToList
      (resourceName: resource:
        if resource.type != "Provider" || !(resource.spec ? artifactId) then null
        else
          let
            catalog = catalogEntry resource.spec.artifactId;
            digest =
              if catalog != null
                && (catalog.closureMetadata.configDigest or null) != null
              then catalog.closureMetadata.configDigest
              else "sha256:${builtins.hashString "sha256"
                "d2b:v3:schema/${resource.spec.artifactId or resourceName}"}";
          in lib.nameValuePair "Provider/${resourceName}" digest)
      (emittedResources (zoneResources zoneName zone))));

  bundleData = zoneName: zone:
    let
      resources = zoneResourceList zoneName zone;
    in {
      schemaVersion = 3;
      bundleVersion = 1;
      zone = zoneName;
      inherit resources;
      generatedAt = "1970-01-01T00:00:00.000Z";
      providerSchemaDigests = providerSchemaDigests zoneName zone;
    };

  bundlePath = zoneName: data:
    artifactRenderer.mkZoneResourceBundle {
      zoneName = zoneName;
      artifactCatalogPreimageJson = canonicalArtifactCatalogPreimageJson;
      resourcesJson = builtins.toJSON data.resources;
      providerSchemaDigestsJson = builtins.toJSON data.providerSchemaDigests;
      zoneJson = builtins.toJSON zoneName;
      artifactCatalogPath = artifactCatalogPath;
      schemaValidationPath = schemaValidationPath;
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
    # Keep only the old eval data for compatibility. Its legacy path and
    # install metadata are deliberately not exposed to the bundle aggregator.
    d2b._bundle.zoneResourceBundlesCompatibility = lib.mapAttrs
      (_: bundle: { data = bundle.data; })
      zoneBundles;
  };
}
