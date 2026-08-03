# Canonical monolithic per-Zone v3 resource bundle emitter.
#
# The bundle is immutable Nix output. Runtime generation ordinals,
# configuration ownership, and cleanup are assigned by the controller after
# this document has passed its integrity checks.
{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  apiVersion = "resources.d2bus.org/v3";
  nul = builtins.fromJSON "\"\\u0000\"";
  resourcesBundle = import ./resources-bundle.nix { inherit lib; };
  providerCatalogEntries = cfg._providerCatalog.entries or [ ];
  phase2 = cfg._resourceCompiler.phase2 or { };
  compilerPackage = phase2.compiler;
  schemaRoot = phase2.schemaRoot;
  strictSecrets = phase2.strictSecrets or true;
  emptyArtifactCatalogPreimageJson = builtins.toJSON {
    entries = [ ];
    schemaVersion = 3;
  };
  runtimeFields = [
    "uid"
    "generation"
    "revision"
    "status"
    "managedBy"
    "configurationGeneration"
    "timestamp"
    "createdAt"
    "updatedAt"
    "finalizers"
  ];
  executionDefaults = {
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
  helperAssertions = lib.flatten (lib.mapAttrsToList
    (zoneName: zone:
      let
        # Volume layout paths are anchored policy paths, not host paths. Their
        # dedicated compiler owns the Volume schema, so the generic bundle
        # secret/path lint must not reinterpret LayoutEntry.path.
        genericResources = lib.filterAttrs
          (_: resource: resource.type != "Volume")
          zone.resources;
        validation = resourcesBundle.validateBundle zoneName genericResources;
        # Keep ordinary validation failures visible as assertions, but never
        # downgrade secret-shaped material to a soft assertion.
        hasForbiddenMaterial = lib.any
          (resource:
            builtins.isAttrs resource
            && resourcesBundle.forbiddenRows (resource.spec or { }) != [ ])
          (lib.attrValues genericResources);
      in
      if hasForbiddenMaterial
      then (resourcesBundle.bundleForZone zoneName genericResources).assertions
      else validation.assertions)
    cfg.zones);
  catalogDigest =
    if cfg ? _artifactCatalogV3 && cfg._artifactCatalogV3 ? catalogDigest
    then cfg._artifactCatalogV3.catalogDigest
    else "sha256:${builtins.hashString "sha256"
      ("d2b:v3:artifact-catalog" + nul + emptyArtifactCatalogPreimageJson)}";
  catalogPath =
    if cfg ? _artifactCatalogV3 && cfg._artifactCatalogV3 ? path
    then cfg._artifactCatalogV3.path
    else null;

  stripRuntime = value:
    if builtins.isAttrs value
    then builtins.removeAttrs
      (lib.mapAttrs (_: stripRuntime) value)
      runtimeFields
    else if builtins.isList value
    then map stripRuntime value
    else value;

  stripCompilerDefaults = spec:
    builtins.removeAttrs spec (lib.filter
      (field:
        builtins.hasAttr field spec
        && spec.${field} == executionDefaults.${field})
      (lib.attrNames executionDefaults));

  optionalMetadata = resource:
    lib.optionalAttrs ((resource.metadata.ownerRef or null) != null) {
      ownerRef = resource.metadata.ownerRef;
    }
    // lib.optionalAttrs ((resource.metadata.labels or { }) != { }) {
      labels = resource.metadata.labels;
    }
    // lib.optionalAttrs ((resource.metadata.annotations or { }) != { }) {
      annotations = resource.metadata.annotations;
    };

  zoneResources = zoneName: zone:
    zone.resources
    // (cfg._resourceCompiler.volumeGenerated.byZone.${zoneName} or { })
    // (cfg._resourceCompiler.volumeShorthand.${zoneName} or { });

  canonicalResource = zoneName: resourceName: resource: {
    inherit apiVersion;
    type = resource.type;
    metadata = {
      name = resourceName;
      zone = zoneName;
    } // optionalMetadata resource;
    spec = stripRuntime (stripCompilerDefaults (resource.spec or { }));
  };

  sortResources = resources:
    lib.sort
      (left: right:
        if left.type != right.type
        then left.type < right.type
        else left.metadata.name < right.metadata.name)
      resources;

  resourceList = zoneName: zone:
    sortResources (lib.mapAttrsToList
      (resourceName: resource: canonicalResource zoneName resourceName resource)
      (lib.filterAttrs (_: resource: resource.type != "Zone")
        (zoneResources zoneName zone)));
  canonicalJson = value: builtins.toJSON (resourcesBundle.canonical value);

  providerSchemaDigests = zoneName: zone:
    lib.listToAttrs (lib.filter
      (entry: entry != null)
      (lib.mapAttrsToList
        (resourceName: resource:
          if resource.type != "Provider" then null
          else
            let
              artifactId = resource.spec.artifactId or null;
              catalog = lib.findFirst
                (entry: entry.id == artifactId)
                null
                providerCatalogEntries;
              digest =
                if catalog != null
                  && catalog ? entry
                  && catalog.entry ? configDigest
                then catalog.entry.configDigest
                else null;
            in
            if digest == null
            then null
            else lib.nameValuePair "Provider/${resourceName}" digest)
        (lib.filterAttrs (_: resource: resource.type != "Zone")
          (zoneResources zoneName zone))));

  providerCatalogEntry = artifactId:
    lib.findFirst (entry: entry.id == artifactId) null providerCatalogEntries;

  publisherFor = artifactId:
    let
      artifact = cfg.artifacts.${artifactId};
      catalog = artifact.catalog or { };
      providerEntry = providerCatalogEntry artifactId;
    in
    catalog.publisher
      or (if providerEntry == null then null else providerEntry.entry.publisher or null);

  signatureIdFor = artifactId:
    let
      catalog = cfg.artifacts.${artifactId}.catalog or { };
      signature = catalog.signature or { };
    in
    catalog.signatureId or signature.signatureId or signature.id or "default";

  signingKeyFor = zone: publisher: catalog:
    let
      trusted = zone.trustedPublishers.${publisher} or null;
    in
    if trusted == null then catalog.signingKey or "" else trusted.signingKey;

  providerInputs = zone:
    let
      providerArtifacts = lib.filterAttrs
        (_: artifact: artifact.type == "provider" && artifact.catalog != null)
        (cfg.artifacts or { });
    in
    lib.concatMap
      (artifactId:
        let
          artifact = providerArtifacts.${artifactId};
          catalog = artifact.catalog;
          publisher = publisherFor artifactId;
          signingKey =
            if publisher == null
            then ""
            else signingKeyFor zone publisher catalog;
          complete =
            publisher != null
            && catalog ? packageDigest
            && catalog ? executableDigest
            && catalog ? manifestDigest
            && catalog ? configDigest;
        in
        lib.optional complete {
          artifactId = artifactId;
          type = artifact.type;
          storePath = "${artifact.package}";
          inherit publisher signingKey;
          signatureId = signatureIdFor artifactId;
          packageDigest = catalog.packageDigest;
          executableDigest = catalog.executableDigest;
          manifestDigest = catalog.manifestDigest;
          configSchemaDigest = catalog.configDigest;
        })
      (lib.sort lib.lessThan (lib.attrNames providerArtifacts));

  bundleData = zoneName: zone:
    let
      resources = resourceList zoneName zone;
      resourcesJson = canonicalJson resources;
      contentHash =
        "sha256:${builtins.hashString "sha256"
          ("d2b:v3:resource-bundle" + nul + resourcesJson)}";
    in {
      schemaVersion = 3;
      bundleVersion = 1;
      zone = zoneName;
      inherit contentHash;
      artifactCatalogDigest = catalogDigest;
      generatedAt = "1970-01-01T00:00:00.000Z";
      inherit resources;
      providerSchemaDigests = providerSchemaDigests zoneName zone;
    };

  bundlePath = zoneName: data:
    let
      compilerInput = pkgs.writeText "d2b-resource-compiler-${zoneName}.json"
        (builtins.toJSON {
          zone = zoneName;
          resources = data.resources;
          providerSchemaDigests = data.providerSchemaDigests;
          providers = providerInputs cfg.zones.${zoneName};
          artifactCatalogPath =
            if catalogPath == null then null else "${catalogPath}";
          expectedArtifactCatalogDigest = catalogDigest;
          schemaRoot = "${schemaRoot}";
          expectedContentHash = data.contentHash;
          inherit strictSecrets;
        });
    in pkgs.runCommand "d2b-zone-${zoneName}-resource-bundle.json"
      {
        inherit compilerInput;
        nativeBuildInputs = [ compilerPackage ];
      } ''
        set -euo pipefail
        d2b-resource-compiler compile \
          --input "$compilerInput" \
          --output "$out"
      '';

  bundles = lib.mapAttrs
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
  activeBundles = lib.mapAttrs
      (zoneName: bundle:
        let compatibility = cfg._bundle.zoneResourceBundlesCompatibility.${zoneName} or { };
        in bundle // {
          # Keep the old eval projection for compatibility, but never its path:
          # the active artifact always comes from the coherent v3 emitter.
          data = compatibility.data or bundle.data;
        })
      bundles;
in
{
  options.d2b._bundle = {
    zoneResourceBundlesV3 = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
      internal = true;
      visible = false;
    };
  };

  config = {
    assertions = helperAssertions;
    d2b._bundle.zoneResourceBundlesV3 = bundles;
    # The v3 emitter owns every installed path. Only the eval-visible data
    # field retains the compatibility projection used by older consumers.
    d2b._bundle.zoneResourceBundles = lib.mkForce activeBundles;
  };
}
