{ lib }:

let
  inherit (lib) concatMapStringsSep filter mapAttrsToList optionalString;

  apiVersion = "resources.d2bus.org/v3";
  resourceNamePattern = "^[a-z][a-z0-9-]{0,62}$";
  qualifiedTypePattern =
    "^[a-z][a-z0-9-]{0,62}\\.d2bus\\.org\\.[A-Z][A-Za-z0-9-]{0,62}$";
  registeredTypes = [
    "Zone"
    "ZoneLink"
    "Provider"
    "Role"
    "RoleBinding"
    "Quota"
    "EmergencyPolicy"
    "Host"
    "Guest"
    "Process"
    "EphemeralProcess"
    "Volume"
    "Network"
    "Device"
    "User"
    "Credential"
    "Endpoint"
    "ResourceExport"
    "ResourceImport"
  ];
  runtimeFields = [
    "uid"
    "revision"
    "generation"
    "finalizers"
    "managedBy"
    "configurationGeneration"
    "status"
  ];
  forbiddenLeafPattern =
    ".*(secret|password|token|privateKey|argv|commandLine|socket|path|pid|uid).*";

  canonical = value:
    if builtins.isAttrs value
    then lib.mapAttrs (_: canonical) value
    else if builtins.isList value
    then map canonical value
    else value;

  stringsIn = value:
    if builtins.isString value then [ value ]
    else if builtins.isList value then lib.concatMap stringsIn value
    else if builtins.isAttrs value then lib.concatMap stringsIn (builtins.attrValues value)
    else [ ];

  hasForbiddenLeaf = value:
    let
      leaves = stringsIn value;
      keyViolations =
        if builtins.isAttrs value
        then lib.concatMap
          (key:
            if builtins.match forbiddenLeafPattern key != null
            then [ key ]
            else hasForbiddenLeaf value.${key})
          (builtins.attrNames value)
        else [ ];
    in
      keyViolations
      ++ (lib.filter
        (value:
          builtins.match ".*(-----BEGIN|eyJ[A-Za-z0-9_-]{16,}|[0-9A-Fa-f]{32,}).*" value
          != null)
        leaves);

  validType = type:
    builtins.elem type registeredTypes
    || builtins.match qualifiedTypePattern type != null;

  validateResource = zoneName: resourceName: resource:
    let
      path = "d2b.zones.${zoneName}.resources.${resourceName}";
      metadata = resource.metadata or { };
      spec = resource.spec or { };
      providerConfig = spec.config or { };
      credentialRefs =
        lib.filter
          (value: builtins.isString value)
          (lib.concatMap
            (key:
              if lib.hasSuffix "credentialRef" key || key == "credentialRef"
              then [ spec.${key} or null ]
              else [ ])
            (builtins.attrNames spec));
      typeChecks = [
        {
          assertion = resource ? type && validType resource.type;
          message = "${path}.type is not a registered ResourceType.";
        }
        {
          assertion = builtins.match resourceNamePattern resourceName != null;
          message = "${path}: resource name is invalid.";
        }
        {
          assertion = lib.all (field: !(builtins.hasAttr field resource)) runtimeFields;
          message = "${path}: runtime-managed fields must not be authored in the bundle.";
        }
        {
          assertion = !(metadata ? name) && !(metadata ? zone);
          message = "${path}.metadata.name and metadata.zone are derived by the bundle compiler.";
        }
        {
          assertion = hasForbiddenLeaf spec == [ ];
          message = "${path}.spec contains a secret, path, argv, PID, or UID-shaped value.";
        }
        {
          assertion =
            resource.type != "Provider"
            || lib.all (key: key == "selfMetrics") (builtins.attrNames providerConfig);
          message = "${path}.spec.config contains an unknown Provider field.";
        }
        {
          assertion =
            !(providerConfig ? selfMetrics)
            || builtins.isAttrs providerConfig.selfMetrics
            && lib.attrNames providerConfig.selfMetrics == [ "enable" ]
            && builtins.isBool providerConfig.selfMetrics.enable;
          message = "${path}.spec.config.selfMetrics.enable must be boolean.";
        }
        {
          assertion =
            !(spec ? telemetry)
            || (spec.telemetry.emitter.ringCapacityBytes or (2 * 1024 * 1024))
              >= 64 * 1024
            && (spec.telemetry.emitter.ringCapacityBytes or (2 * 1024 * 1024))
              <= 64 * 1024 * 1024;
          message = "${path}.spec.telemetry.emitter.ringCapacityBytes is out of bounds.";
        }
        {
          assertion =
            !(spec ? audit)
            || (spec.audit.retentionDays or 30) >= 1
            && (spec.audit.retentionDays or 30) <= 3650
            && (spec.audit.maxSegmentBytes or (64 * 1024 * 1024)) >= 1024 * 1024
            && (spec.audit.maxSegmentBytes or (64 * 1024 * 1024)) <= 1024 * 1024 * 1024;
          message = "${path}.spec.audit bounds are invalid.";
        }
      ];
    in
      typeChecks
      ++ map
        (ref: {
          assertion = builtins.match "^Credential/[a-z][a-z0-9-]{0,62}$" ref != null;
          message = "${path}.spec credentialRef must use Credential/<name>.";
        })
        credentialRefs;

  canonicalResource = zoneName: resourceName: resource:
    {
      apiVersion = apiVersion;
      type = resource.type;
      metadata = {
        name = resourceName;
        zone = zoneName;
      }
      // lib.optionalAttrs
        (((resource.metadata or { }).ownerRef or null) != null)
        { ownerRef = resource.metadata.ownerRef; };
      spec = canonical (builtins.removeAttrs (resource.spec or { }) runtimeFields);
    };

  sortResources = resources:
    lib.sort
      (left: right:
        if left.type != right.type then left.type < right.type
        else left.metadata.name < right.metadata.name)
      resources;

  bundleForZone = zoneName: resources:
    let
      checks = lib.flatten (lib.mapAttrsToList
        (resourceName: resource: validateResource zoneName resourceName resource)
        resources);
      rendered = sortResources (lib.mapAttrsToList
        (resourceName: resource: canonicalResource zoneName resourceName resource)
        resources);
      data = {
        inherit apiVersion;
        schemaVersion = 3;
        bundleVersion = 1;
        zone = zoneName;
        resources = rendered;
      };
      json = builtins.toJSON (canonical data);
    in {
      assertions = checks;
      inherit data json;
      digest = builtins.hashString "sha256" json;
    };
in
{
  inherit
    apiVersion
    bundleForZone
    canonical
    canonicalResource
    registeredTypes
    validateResource
    ;
}
