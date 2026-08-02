# Canonical monolithic per-Zone v3 resource bundle emitter.
#
# The bundle is immutable Nix output. Runtime generation ordinals,
# configuration ownership, and cleanup are assigned by the controller after
# this document has passed its integrity checks.
{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  apiVersion = "resources.d2bus.org/v3";
  providerCatalogEntries = cfg._providerCatalog.entries or [ ];
  schemaValidation = cfg._resourceCompiler.schemaValidation or { };
  schemaValidationPath = schemaValidation.buildValidation or null;
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
  catalogDigest =
    if cfg ? _artifactCatalogV3 && cfg._artifactCatalogV3 ? catalogDigest
    then cfg._artifactCatalogV3.catalogDigest
    else "sha256:${builtins.hashString "sha256" "d2b:v3:artifact-catalog\000{}"}";
  catalogPath =
    if cfg ? _artifactCatalogV3 && cfg._artifactCatalogV3 ? path
    then cfg._artifactCatalogV3.path
    else null;
  catalogPathArg = if catalogPath == null then "" else "${catalogPath}";

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
      (lib.filterAttrs (_: resource: resource.type != "Zone") zone.resources));

  providerSchemaDigests = zone:
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
                else "sha256:${builtins.hashString "sha256"
                  "d2b:v3:schema/${if artifactId == null then resourceName else artifactId}"}";
            in
            lib.nameValuePair "Provider/${resourceName}" digest)
        (lib.filterAttrs (_: resource: resource.type != "Zone") zone.resources)));

  bundleData = zoneName: zone:
    let
      resources = resourceList zoneName zone;
      resourcesJson = builtins.toJSON resources;
      contentHash =
        "sha256:${builtins.hashString "sha256"
          ("d2b:v3:resource-bundle\000" + resourcesJson)}";
    in {
      schemaVersion = 3;
      bundleVersion = 1;
      zone = zoneName;
      inherit contentHash;
      artifactCatalogDigest = catalogDigest;
      generatedAt = "1970-01-01T00:00:00.000Z";
      inherit resources;
      providerSchemaDigests = providerSchemaDigests zone;
    };

  bundlePath = zoneName: data:
    let
      resourcesJson = builtins.toJSON data.resources;
      providerDigestsJson = builtins.toJSON data.providerSchemaDigests;
      zoneJson = builtins.toJSON zoneName;
    in pkgs.runCommand "d2b-zone-${zoneName}-resource-bundle.json"
      {
        inherit resourcesJson providerDigestsJson zoneJson catalogDigest catalogPathArg;
        schemaValidationPathArg =
          if schemaValidationPath == null then "" else "${schemaValidationPath}";
        passAsFile = [ "resourcesJson" "providerDigestsJson" ];
        nativeBuildInputs = [ pkgs.python3 ];
      } ''
        set -euo pipefail
        if [ -n "$schemaValidationPathArg" ]; then
          test -e "$schemaValidationPathArg"
        fi
        python3 - "$resourcesJsonPath" "$providerDigestsJsonPath" "$zoneJson" \
          "$catalogDigest" "$catalogPathArg" "$out" <<'PY'
        import hashlib
        import json
        import pathlib
        import sys

        resources_path, digests_path, zone_json, catalog, catalog_path, output = sys.argv[1:]
        resources = json.loads(pathlib.Path(resources_path).read_text())
        provider_digests = json.loads(pathlib.Path(digests_path).read_text())
        if catalog_path:
            catalog = json.loads(pathlib.Path(catalog_path).read_text())["catalogDigest"]
        resources_bytes = pathlib.Path(resources_path).read_bytes()
        content = hashlib.sha256(
            b"d2b:v3:resource-bundle\0" + resources_bytes
        ).hexdigest()
        document = {
            "artifactCatalogDigest": catalog,
            "bundleVersion": 1,
            "contentHash": "sha256:" + content,
            "generatedAt": "1970-01-01T00:00:00.000Z",
            "providerSchemaDigests": provider_digests,
            "resources": resources,
            "schemaVersion": 3,
            "zone": json.loads(zone_json),
        }
        pathlib.Path(output).write_text(
            json.dumps(document, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
        PY
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
    d2b._bundle.zoneResourceBundlesV3 = bundles;
    # The old emitter remains the compatibility default until the integrator
    # switches the aggregator. A direct v3 import still exposes the canonical
    # destination through zoneResourceBundlesV3.
    d2b._bundle.zoneResourceBundles = lib.mkDefault bundles;
  };
}
