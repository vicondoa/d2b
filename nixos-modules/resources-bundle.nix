{ lib }:

let
  apiVersion = "resources.d2bus.org/v3";
  resourceNamePattern = "^[a-z][a-z0-9-]{0,62}$";
  qualifiedTypePattern =
    "^[a-z][a-z0-9-]{0,62}\\.d2bus\\.org\\.[A-Z][A-Za-z0-9-]{0,62}$";
  credentialRefPattern = "^Credential/[a-z][a-z0-9-]{0,62}$";
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
    "timestamp"
    "createdAt"
    "updatedAt"
    "status"
  ];
  resourceFields = [ "type" "metadata" "spec" ];
  metadataFields = [ "ownerRef" "labels" "annotations" ];
  providerConfigFields = [ "selfMetrics" ];
  telemetryFields = [ "emitter" ];
  emitterFields = [ "ringCapacityBytes" ];
  auditFields = [ "retentionDays" "maxSegmentBytes" ];
  forbiddenKeyNames = [
    "secret"
    "password"
    "token"
    "privateKey"
    "argv"
    "commandLine"
    "socket"
    "path"
    "pid"
    "uid"
    "env"
    "exe"
    "realm"
    "workload_id"
  ];
  forbiddenKeyPattern =
    ".*(Path|Socket|Argv|CommandLine|Pid|Uid|Env|Exe|WorkloadId)$";

  attrOr = attrs: name: fallback:
    if builtins.isAttrs attrs && builtins.hasAttr name attrs
    then builtins.getAttr name attrs
    else fallback;

  validName = value:
    builtins.isString value
    && builtins.match resourceNamePattern value != null;

  validType = value:
    builtins.isString value
    && (builtins.elem value registeredTypes
      || builtins.match qualifiedTypePattern value != null);

  validKeys = value: allowed:
    builtins.isAttrs value
    && lib.all (key: builtins.elem key allowed) (builtins.attrNames value);

  isBoundedInt = minimum: maximum: value:
    builtins.isInt value && value >= minimum && value <= maximum;

  secretShapedValue = value:
    builtins.isString value
    && (
      builtins.match ".*-----BEGIN [A-Z0-9 ]+-----.*" value != null
      || builtins.match
        "eyJ[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+"
        value
        != null
    );

  forbiddenKey = key:
    builtins.elem key forbiddenKeyNames
    || builtins.match forbiddenKeyPattern key != null;

  forbiddenRows = value:
    if builtins.isAttrs value
    then
      lib.concatMap
        (key:
          (lib.optional (forbiddenKey key) key)
          ++ forbiddenRows (builtins.getAttr key value))
        (builtins.attrNames value)
    else if builtins.isList value
    then lib.concatMap forbiddenRows value
    else lib.optional (secretShapedValue value) "<secret-shaped-value>";

  runtimeRows = value:
    if builtins.isAttrs value
    then
      lib.concatMap
        (key:
          (lib.optional (builtins.elem key runtimeFields) key)
          ++ runtimeRows (builtins.getAttr key value))
        (builtins.attrNames value)
    else if builtins.isList value
    then lib.concatMap runtimeRows value
    else [ ];

  credentialRows = value: path:
    if builtins.isAttrs value
    then
      lib.concatMap
        (key:
          let
            childPath = path ++ [ key ];
            child = builtins.getAttr key value;
            direct =
              lib.optional
                (lib.hasSuffix "credentialRef" key || key == "credentialRef")
                {
                  inherit key childPath;
                  value = child;
                };
          in
          direct ++ credentialRows child childPath)
        (builtins.attrNames value)
    else if builtins.isList value
    then lib.concatLists (lib.imap0
      (index: child: credentialRows child (path ++ [ toString index ]))
      value)
    else [ ];

  check = assertion: message: {
    inherit assertion message;
  };

  validateResource = zoneName: resourceName: resource:
    let
      path = "d2b.zones.${toString zoneName}.resources.${toString resourceName}";
      resourceIsAttrs = builtins.isAttrs resource;
      resourceType = attrOr resource "type" null;
      metadataPresent = resourceIsAttrs && builtins.hasAttr "metadata" resource;
      metadata = attrOr resource "metadata" { };
      specPresent = resourceIsAttrs && builtins.hasAttr "spec" resource;
      spec = attrOr resource "spec" { };
      providerConfigPresent = builtins.isAttrs spec && builtins.hasAttr "config" spec;
      providerConfig = attrOr spec "config" { };
      selfMetricsPresent =
        builtins.isAttrs providerConfig && builtins.hasAttr "selfMetrics" providerConfig;
      selfMetrics = attrOr providerConfig "selfMetrics" { };
      telemetryPresent = builtins.isAttrs spec && builtins.hasAttr "telemetry" spec;
      telemetry = attrOr spec "telemetry" { };
      emitterPresent = builtins.isAttrs telemetry && builtins.hasAttr "emitter" telemetry;
      emitter = attrOr telemetry "emitter" { };
      ringPresent = builtins.isAttrs emitter && builtins.hasAttr "ringCapacityBytes" emitter;
      ringCapacityBytes = attrOr emitter "ringCapacityBytes" (2 * 1024 * 1024);
      auditPresent = builtins.isAttrs spec && builtins.hasAttr "audit" spec;
      audit = attrOr spec "audit" { };
      retentionDays = attrOr audit "retentionDays" 30;
      maxSegmentBytes = attrOr audit "maxSegmentBytes" (64 * 1024 * 1024);
      credentialChecks = map
        (row:
          check
            (builtins.isString row.value
              && builtins.match credentialRefPattern row.value != null)
            "${path}.spec.${lib.concatStringsSep "." row.childPath} must use Credential/<name>.")
        (credentialRows spec [ ]);
      forbidden = forbiddenRows spec;
      runtime = runtimeRows resource;
    in
    [
      (check resourceIsAttrs
        "${path} must be an attribute set.")
      (check (validName resourceName)
        "${path}: resource name is invalid.")
      (check (validType resourceType)
        "${path}.type is not a registered ResourceType.")
      (check (validKeys resource resourceFields)
        "${path} contains an unknown top-level field.")
      (check (!metadataPresent || builtins.isAttrs metadata)
        "${path}.metadata must be an attribute set.")
      (check (!metadataPresent || validKeys metadata metadataFields)
        "${path}.metadata contains an unknown field.")
      (check
        (!metadataPresent
          || !(builtins.hasAttr "name" metadata)
          && !(builtins.hasAttr "zone" metadata))
        "${path}.metadata.name and metadata.zone are derived by the bundle compiler.")
      (check (!specPresent || builtins.isAttrs spec)
        "${path}.spec must be an attribute set.")
      (check
        (lib.all (field: !(builtins.hasAttr field resource)) runtimeFields)
        "${path}: runtime-managed fields must not be authored in the bundle.")
      (check (runtime == [ ])
        "${path} contains a nested runtime-managed field.")
      (check (forbidden == [ ])
        "${path}.spec contains a secret, path, argv, PID, or UID-shaped value.")
      (check
        (resourceType != "Provider"
          || !providerConfigPresent
          || builtins.isAttrs providerConfig)
        "${path}.spec.config must be an attribute set for Provider resources.")
      (check
        (resourceType != "Provider"
          || !providerConfigPresent
          || validKeys providerConfig providerConfigFields)
        "${path}.spec.config contains an unknown Provider field.")
      (check
        (!selfMetricsPresent
          || builtins.isAttrs selfMetrics
          && validKeys selfMetrics [ "enable" ]
          && builtins.isBool (attrOr selfMetrics "enable" null))
        "${path}.spec.config.selfMetrics.enable must be boolean.")
      (check
        (!telemetryPresent
          || builtins.isAttrs telemetry)
        "${path}.spec.telemetry must be an attribute set.")
      (check
        (!telemetryPresent
          || validKeys telemetry telemetryFields)
        "${path}.spec.telemetry contains an unknown field.")
      (check
        (!emitterPresent
          || builtins.isAttrs emitter)
        "${path}.spec.telemetry.emitter must be an attribute set.")
      (check
        (!emitterPresent
          || validKeys emitter emitterFields)
        "${path}.spec.telemetry.emitter contains an unknown field.")
      (check
        (!ringPresent
          || isBoundedInt (64 * 1024) (64 * 1024 * 1024) ringCapacityBytes)
        "${path}.spec.telemetry.emitter.ringCapacityBytes is out of bounds.")
      (check
        (!auditPresent
          || builtins.isAttrs audit)
        "${path}.spec.audit must be an attribute set.")
      (check
        (!auditPresent
          || validKeys audit auditFields)
        "${path}.spec.audit contains an unknown field.")
      (check
        (!auditPresent
          || isBoundedInt 1 3650 retentionDays
          && isBoundedInt (1024 * 1024) (1024 * 1024 * 1024) maxSegmentBytes)
        "${path}.spec.audit bounds are invalid.")
    ] ++ credentialChecks;

  canonical = value:
    if builtins.isAttrs value
    then lib.mapAttrs (_: canonical) value
    else if builtins.isList value
    then map canonical value
    else value;

  canonicalMetadata = zoneName: resource:
    let
      metadata = attrOr resource "metadata" { };
      labels = attrOr metadata "labels" { };
      annotations = attrOr metadata "annotations" { };
    in
    {
      name = attrOr resource "name" null;
      zone = zoneName;
    }
    // lib.optionalAttrs ((attrOr metadata "ownerRef" null) != null) {
      ownerRef = metadata.ownerRef;
    }
    // lib.optionalAttrs (labels != { }) { inherit labels; }
    // lib.optionalAttrs (annotations != { }) { inherit annotations; };

  canonicalResource = zoneName: resourceName: resource:
    let
      spec = attrOr resource "spec" { };
    in
    {
      apiVersion = apiVersion;
      type = resource.type;
      metadata = (canonicalMetadata zoneName resource) // {
        name = resourceName;
      };
      spec = canonical spec;
    };

  sortResources = resources:
    lib.sort
      (left: right:
        if left.type != right.type then left.type < right.type
        else left.metadata.name < right.metadata.name)
      resources;

  validationFor = zoneName: resources:
    let
      shapeChecks = [
        (check (builtins.isString zoneName
          && builtins.match resourceNamePattern zoneName != null)
          "d2b resource bundle Zone name is invalid.")
        (check (builtins.isAttrs resources)
          "d2b resource bundle resources must be an attribute set.")
      ];
      resourceChecks =
        if builtins.isAttrs resources
        then lib.flatten (lib.mapAttrsToList
          (resourceName: resource: validateResource zoneName resourceName resource)
          resources)
        else [ ];
      assertions = shapeChecks ++ resourceChecks;
      failures = lib.filter (entry: !entry.assertion) assertions;
    in
    {
      inherit assertions failures;
      valid = failures == [ ];
      errors = map (entry: entry.message) failures;
    };

  validateBundle = zoneName: resources:
    validationFor zoneName resources;

  compileBundle = { zoneName, resources }:
    let
      validation = validationFor zoneName resources;
    in
    if !validation.valid
    then throw
      "d2b resource bundle for Zone '${toString zoneName}' rejected: ${
        lib.concatStringsSep "; " validation.errors
      }"
    else
      let
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
        digest = builtins.hashString "sha256" json;
      in
      {
        assertions = validation.assertions;
        inherit data digest json;
        contentHash = "sha256:${digest}";
      };

  bundleForZone = zoneName: resources:
    compileBundle { inherit zoneName resources; };
in
{
  inherit
    apiVersion
    bundleForZone
    canonical
    canonicalResource
    compileBundle
    sortResources
    validateBundle
    validateResource
    ;
}
