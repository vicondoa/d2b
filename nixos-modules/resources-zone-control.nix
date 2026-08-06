# Zone-control resource compiler binding.
#
# The existing Zone modules own the public topology and generic ResourceRef
# validation.  This module is the single compiler seam for the remaining
# Zone-control resource classes: it adds the type-specific closed constraints
# and exposes the already-canonical bundle resources together with the sealed
# allocator topology and generation identity.
{ config, lib, ... }:

let
  cfg = config.d2b;
  resourceModel = import ./resources.nix { inherit lib; };
  resourceBundle = import ./resources-bundle.nix { inherit lib; };
  coreSchemaFileName = resourceType:
    if builtins.elem resourceType resourceModel.standardResourceTypes
    then "core.d2bus.org_${resourceType}.schema.json"
    else "${resourceType}.schema.json";

  controlTypes = [
    "Zone"
    "ZoneLink"
    "Provider"
    "Role"
    "RoleBinding"
    "Quota"
    "EmergencyPolicy"
  ];
  resourceNamePattern = "^[a-z][a-z0-9-]{0,62}$";
  resourceRefPattern =
    "^([A-Z][A-Za-z0-9]{0,62}|[a-z][a-z0-9-]{0,62}\\.d2bus\\.org\\.[A-Z][A-Za-z0-9]{0,62})/[a-z][a-z0-9-]{0,62}$";
  digestPattern = "^sha256:[0-9a-f]{64}$";
  artifactIdPattern = "^[a-z][a-z0-9-]{0,62}$";
  transportProviderRefPattern = "^Provider/transport-[a-z][a-z0-9-]*$";

  zoneLinkSpecKeys = [
    "childZoneName"
    "disabled"
    "limits"
    "transportCredentials"
    "transportProviderRef"
    "transportSettings"
  ];
  providerSpecKeys = [ "artifactId" "config" ];
  roleSpecKeys = [ "providerRef" "rules" "updatePolicy" ];
  roleBindingSpecKeys = [
    "externalPrincipalSelector"
    "providerRef"
    "roleRef"
    "scopeNarrowing"
    "subjects"
    "updatePolicy"
  ];
  quotaSpecKeys = [
    "ceilings"
    "enforcementPolicy"
    "perTypeCeilings"
    "providerRef"
    "scope"
    "updatePolicy"
  ];
  emergencyPolicySpecKeys = [
    "drainDeadlineSeconds"
    "enabled"
    "providerRef"
    "reason"
    "scope"
    "updatePolicy"
  ];
  executionPolicyFields = [
    "defaultDomain"
    "allowedDomains"
    "defaultUserRef"
    "budget"
    "networkAttachments"
    "deviceAttachments"
    "volumeAttachmentDefaults"
  ];
  digestKey = key:
    key == "digest"
    || lib.hasSuffix "Digest" key
    || lib.hasSuffix "Fingerprint" key;

  attrOr = attrs: name: fallback:
    if builtins.isAttrs attrs && builtins.hasAttr name attrs
    then attrs.${name}
    else fallback;

  resourceTypeValid = value:
    builtins.isString value && resourceModel.validResourceType value;

  controlSpec = row:
    builtins.removeAttrs row.spec executionPolicyFields;

  exactKeys = allowed: value:
    builtins.isAttrs value
    && lib.all (key: builtins.elem key allowed) (builtins.attrNames value);

  parseRef = value:
    let
      parts = if builtins.isString value then lib.splitString "/" value else [ ];
    in
    if lib.length parts == 2 then {
      type = builtins.elemAt parts 0;
      name = builtins.elemAt parts 1;
    } else null;

  resolvesAs = resources: expectedTypes: value:
    let parsed = parseRef value;
    in parsed != null
      && builtins.elem parsed.type expectedTypes
      && builtins.hasAttr parsed.name resources
      && resources.${parsed.name}.type == parsed.type;

  resolvesAny = resources: value:
    let parsed = parseRef value;
    in parsed != null
      && builtins.hasAttr parsed.name resources
      && resources.${parsed.name}.type == parsed.type;

  artifactFor = artifactId:
    if builtins.isString artifactId
      && builtins.hasAttr artifactId (cfg.artifacts or { })
    then cfg.artifacts.${artifactId}
    else null;

  schemaFor = resourceType:
    let path = ../docs/reference/schemas/v3 + "/${coreSchemaFileName resourceType}";
    in if builtins.pathExists path
    then builtins.fromJSON (builtins.readFile path)
    else { };

  schemaSpecKeys = resourceType:
    let schema = schemaFor resourceType;
    in if schema ? properties && schema.properties ? spec
      then builtins.attrNames (schema.properties.spec.properties or { })
      else [ ];

  resourceRows = lib.concatMap
    (zoneName:
      let zone = cfg.zones.${zoneName};
      in lib.mapAttrsToList
        (resourceName: resource: {
          inherit zoneName resourceName resource zone;
          path = "d2b.zones.${zoneName}.resources.${resourceName}";
          spec = resource.spec or { };
        })
        zone.resources)
    (lib.sort lib.lessThan (lib.attrNames cfg.zones));

  controlRows = lib.filter
    (row: builtins.elem row.resource.type controlTypes)
    resourceRows;

  zoneLinkAssertions = row:
    let
      spec = controlSpec row;
      limits = attrOr spec "limits" { };
      credentials = attrOr spec "transportCredentials" [ ];
      transportProviderRef = attrOr spec "transportProviderRef" null;
      childZoneName = attrOr spec "childZoneName" null;
    in
    lib.optionals (row.resource.type == "ZoneLink") [
      {
        assertion = exactKeys zoneLinkSpecKeys spec;
        message = "${row.path}.spec must contain only the generated ZoneLink schema fields.";
      }
      {
        assertion = builtins.isString childZoneName
          && builtins.match resourceNamePattern childZoneName != null
          && childZoneName == row.zoneName;
        message = "${row.path}.spec.childZoneName must equal the enclosing Zone name.";
      }
      {
        assertion = builtins.isString transportProviderRef
          && builtins.match transportProviderRefPattern transportProviderRef != null
          && resolvesAs row.zone.resources [ "Provider" ] transportProviderRef;
        message = "${row.path}.spec.transportProviderRef must be a same-Zone transport Provider ref.";
      }
      {
        assertion = builtins.isAttrs (attrOr spec "transportSettings" { });
        message = "${row.path}.spec.transportSettings must be an attribute set.";
      }
      {
        assertion = builtins.isList credentials
          && lib.length credentials <= 8
          && lib.length (lib.unique credentials) == lib.length credentials
          && lib.all (ref: resolvesAs row.zone.resources [ "Credential" ] ref) credentials;
        message = "${row.path}.spec.transportCredentials must contain at most 8 unique same-Zone Credential refs.";
      }
      {
        assertion = builtins.isBool (attrOr spec "disabled" false);
        message = "${row.path}.spec.disabled must be boolean.";
      }
      {
        assertion = exactKeys [
          "maxActiveStreams"
          "maxPendingIntents"
          "reconnectMaxAttempts"
          "reconnectWindowSecs"
        ] limits;
        message = "${row.path}.spec.limits must contain only the generated ZoneLink limit fields.";
      }
      {
        assertion = builtins.isInt (attrOr limits "maxActiveStreams" 32)
          && (attrOr limits "maxActiveStreams" 32) >= 1
          && (attrOr limits "maxActiveStreams" 32) <= 128;
        message = "${row.path}.spec.limits.maxActiveStreams must be between 1 and 128.";
      }
      {
        assertion = builtins.isInt (attrOr limits "maxPendingIntents" 256)
          && (attrOr limits "maxPendingIntents" 256) >= 0
          && (attrOr limits "maxPendingIntents" 256) <= 1024;
        message = "${row.path}.spec.limits.maxPendingIntents must be between 0 and 1024.";
      }
      {
        assertion = builtins.isInt (attrOr limits "reconnectMaxAttempts" 10)
          && (attrOr limits "reconnectMaxAttempts" 10) >= 1;
        message = "${row.path}.spec.limits.reconnectMaxAttempts must be positive.";
      }
      {
        assertion = builtins.isInt (attrOr limits "reconnectWindowSecs" 300)
          && (attrOr limits "reconnectWindowSecs" 300) >= 1;
        message = "${row.path}.spec.limits.reconnectWindowSecs must be positive.";
      }
    ];

  providerAssertions = row:
    let
      spec = controlSpec row;
      artifactId = attrOr spec "artifactId" null;
      artifact = artifactFor artifactId;
      configValue = attrOr spec "config" { };
    in
    lib.optionals
      (row.resource.type == "Provider" && (cfg.artifacts or { }) != { }) [
      {
        assertion = exactKeys providerSpecKeys spec;
        message = "${row.path}.spec must contain only artifactId and config.";
      }
      {
        assertion = builtins.isString artifactId
          && builtins.match artifactIdPattern artifactId != null;
        message = "${row.path}.spec.artifactId must be a bounded plain artifact ID.";
      }
      {
        assertion = artifact != null && (artifact.type or null) == "provider";
        message = "${row.path}.spec.artifactId must resolve to a provider artifact.";
      }
      {
        assertion = builtins.isAttrs configValue;
        message = "${row.path}.spec.config must be an attribute set.";
      }
      {
        assertion = !(builtins.elem row.resourceName [ "system-core" "system-minijail" ]);
        message = "${row.path}: system-core and system-minijail are bootstrap-only providers and cannot be hand-authored.";
      }
    ];

  roleAssertions = row:
    let
      spec = controlSpec row;
      rules = attrOr spec "rules" [ ];
    in
    lib.optionals (row.resource.type == "Role") ([
      {
        assertion = exactKeys roleSpecKeys spec;
        message = "${row.path}.spec must contain only the generated Role fields.";
      }
      {
        assertion = builtins.isList rules && lib.length rules <= 32;
        message = "${row.path}.spec.rules must contain at most 32 rules.";
      }
      {
        assertion = builtins.isList rules
          && lib.all
            (rule:
              let
                resourceTypes = attrOr rule "resourceTypes" [ ];
                verbs = attrOr rule "verbs" [ ];
                session = attrOr rule "sessionVerbs" [ ];
              in builtins.isList resourceTypes
                && lib.length resourceTypes >= 1
                && (verbs != [ ] || session != [ ])
                && lib.length (attrOr rule "resourceNames" [ ]) <= 64
                && lib.length (attrOr rule "zones" [ ]) <= 8
                && lib.length (attrOr rule "executionRefs" [ ]) <= 32)
            rules;
        message = "${row.path}.spec.rules must keep bounded non-empty ResourceTypes and permission lists.";
      }
    ]);

  roleBindingAssertions = row:
    let
      spec = controlSpec row;
      subjects = attrOr spec "subjects" [ ];
      roleRef = attrOr spec "roleRef" null;
      external = attrOr spec "externalPrincipalSelector" null;
      hasExternal = external != null;
    in
    lib.optionals (row.resource.type == "RoleBinding") [
      {
        assertion = exactKeys roleBindingSpecKeys spec;
        message = "${row.path}.spec must contain no expiry and only generated RoleBinding fields.";
      }
      {
        assertion = resolvesAs row.zone.resources [ "Role" ] roleRef;
        message = "${row.path}.spec.roleRef must resolve to a same-Zone Role.";
      }
      {
        assertion = builtins.isList subjects
          && lib.length subjects <= 128
          && lib.length (lib.unique subjects) == lib.length subjects
          && lib.all (subject: builtins.isString subject
            && builtins.match resourceRefPattern subject != null) subjects;
        message = "${row.path}.spec.subjects must contain at most 128 unique same-Zone ResourceRefs.";
      }
      {
        assertion = builtins.isAttrs external || external == null;
        message = "${row.path}.spec.externalPrincipalSelector must be null or an attribute set.";
      }
      {
        assertion = subjects != [ ] || hasExternal;
        message = "${row.path}.spec must contain subjects unless an external principal selector is present.";
      }
      {
        assertion = !(builtins.hasAttr "expiry" spec)
          && !(builtins.hasAttr "expiresAt" spec)
          && !(builtins.hasAttr "ttl" spec);
        message = "${row.path}.spec must not contain expiry, expiresAt, or ttl.";
      }
    ];

  quotaAssertions = row:
    let
      spec = controlSpec row;
      ceilings = attrOr spec "ceilings" { };
      perType = attrOr spec "perTypeCeilings" { };
      positiveOrNull = value: value == null || (builtins.isInt value && value >= 1);
    in
    lib.optionals (row.resource.type == "Quota") [
      {
        assertion = exactKeys quotaSpecKeys spec;
        message = "${row.path}.spec contains an unsupported Quota field.";
      }
      {
        assertion = exactKeys [
          "maxResources"
          "maxResourcesPerType"
          "maxOwnerDepth"
          "maxCpu"
          "maxMemoryMib"
          "maxStorageGib"
        ] ceilings;
        message = "${row.path}.spec.ceilings contains an unsupported field.";
      }
      {
        assertion = builtins.isInt (attrOr ceilings "maxResources" 4096)
          && (attrOr ceilings "maxResources" 4096) >= 1
          && (attrOr ceilings "maxResources" 4096) <= 65536;
        message = "${row.path}.spec.ceilings.maxResources must be between 1 and 65536.";
      }
      {
        assertion = builtins.isInt (attrOr ceilings "maxResourcesPerType" 512)
          && (attrOr ceilings "maxResourcesPerType" 512) >= 1
          && (attrOr ceilings "maxResourcesPerType" 512) <= 65536;
        message = "${row.path}.spec.ceilings.maxResourcesPerType must be between 1 and 65536.";
      }
      {
        assertion = builtins.isInt (attrOr ceilings "maxOwnerDepth" 8)
          && (attrOr ceilings "maxOwnerDepth" 8) >= 1
          && (attrOr ceilings "maxOwnerDepth" 8) <= 32;
        message = "${row.path}.spec.ceilings.maxOwnerDepth must be between 1 and 32.";
      }
      {
        assertion = positiveOrNull (attrOr ceilings "maxCpu" null)
          && positiveOrNull (attrOr ceilings "maxMemoryMib" null)
          && positiveOrNull (attrOr ceilings "maxStorageGib" null);
        message = "${row.path}.spec.ceilings optional limits must be null or positive integers.";
      }
      {
        assertion = builtins.isAttrs perType
          && lib.length (builtins.attrNames perType) <= 64
          && lib.all resourceTypeValid (builtins.attrNames perType);
        message = "${row.path}.spec.perTypeCeilings must contain at most 64 known ResourceType keys.";
      }
      {
        assertion = builtins.elem (attrOr spec "scope" null) [ "zone" ];
        message = "${row.path}.spec.scope must be zone.";
      }
      {
        assertion = builtins.elem (attrOr spec "enforcementPolicy" null) [ "hard" "soft" ];
        message = "${row.path}.spec.enforcementPolicy must be hard or soft.";
      }
    ];

  emergencyAssertions = row:
    let
      spec = controlSpec row;
      scope = attrOr spec "scope" { };
    in
    lib.optionals (row.resource.type == "EmergencyPolicy") [
      {
        assertion = exactKeys emergencyPolicySpecKeys spec;
        message = "${row.path}.spec contains an unsupported EmergencyPolicy field.";
      }
      {
        assertion = builtins.isBool (attrOr spec "enabled" false);
        message = "${row.path}.spec.enabled must be boolean.";
      }
      {
        assertion = exactKeys [
          "stopNewAdmissions"
          "disconnectZoneLinks"
          "stopProviderProcesses"
          "drainOngoingOperations"
        ] scope
          && lib.all (field: builtins.isBool (attrOr scope field null))
            [
              "stopNewAdmissions"
              "disconnectZoneLinks"
              "stopProviderProcesses"
              "drainOngoingOperations"
            ];
        message = "${row.path}.spec.scope must contain exactly the four boolean emergency controls.";
      }
      {
        assertion = builtins.isInt (attrOr spec "drainDeadlineSeconds" 30)
          && (attrOr spec "drainDeadlineSeconds" 30) >= 1
          && (attrOr spec "drainDeadlineSeconds" 30) <= 300;
        message = "${row.path}.spec.drainDeadlineSeconds must be between 1 and 300.";
      }
      {
        assertion = builtins.isString (attrOr spec "reason" "")
          && builtins.stringLength (attrOr spec "reason" "") <= 256;
        message = "${row.path}.spec.reason must be a string of at most 256 bytes.";
      }
    ];

  digestAssertions = row:
    let
      rows = value: path:
        if builtins.isAttrs value
        then lib.concatMap
          (key:
            let next = path ++ [ key ];
            in (lib.optional (digestKey key) {
              assertion = builtins.isString value.${key}
                && builtins.match digestPattern value.${key} != null;
              message = "${row.path}.${lib.concatStringsSep "." next} must be a lowercase sha256 digest.";
            }) ++ rows value.${key} next)
          (builtins.attrNames value)
        else if builtins.isList value
        then lib.concatLists (lib.imap0
          (index: item: rows item (path ++ [ toString index ]))
          value)
        else [ ];
    in rows row.spec [ "spec" ];

  genericControlAssertions = row:
    let
      schemaKeys = schemaSpecKeys row.resource.type;
      spec = controlSpec row;
    in
    lib.optionals (builtins.elem row.resource.type controlTypes) [
      {
        assertion = builtins.match resourceNamePattern row.resourceName != null;
        message = "${row.path}: resource name must match ${resourceNamePattern}.";
      }
      {
        assertion = row.resource.type == "Zone" -> spec == { };
        message = "${row.path}.spec must be empty for the runtime-created Zone self-resource.";
      }
      {
        assertion = (row.resource.type == "Provider"
          && (cfg.artifacts or { }) == { })
          || row.resource.type == "Zone"
          || schemaKeys == [ ]
          || lib.all (key: builtins.elem key schemaKeys) (builtins.attrNames spec);
        message = "${row.path}.spec contains a field outside the committed ${row.resource.type} schema.";
      }
      {
        assertion = builtins.isAttrs spec;
        message = "${row.path}.spec must be an attribute set.";
      }
    ];

  controlAssertions = lib.concatMap
    (row:
      genericControlAssertions row
      ++ zoneLinkAssertions row
      ++ providerAssertions row
      ++ roleAssertions row
      ++ roleBindingAssertions row
      ++ quotaAssertions row
      ++ emergencyAssertions row
      ++ digestAssertions row)
    controlRows;

  bundleFor = zoneName:
    if builtins.hasAttr zoneName (cfg._bundle.zoneResourceBundlesV3 or { })
    then cfg._bundle.zoneResourceBundlesV3.${zoneName}.data
    else {
      resources = [ ];
      contentHash = "sha256:${resourceBundle.framedDigest
        "d2b:v3:resource-bundle" "[]"}";
    };

  canonicalResources = zoneName:
    let data = bundleFor zoneName;
    in data.resources or [ ];

  controlResourcesByZone = lib.mapAttrs
    (zoneName: _zone:
      lib.filter
        (resource: builtins.elem resource.type controlTypes)
        (canonicalResources zoneName))
    cfg.zones;

  parentMap = lib.mapAttrs
    (_: topology: topology.parentZone)
    (cfg._zoneCompiler.topology or { });
  parentMapDigest = "sha256:${resourceBundle.framedDigest
    "d2b:v3:parent-topology" (builtins.toJSON parentMap)}";
  generationByZone = lib.mapAttrs
    (zoneName: _zone: (bundleFor zoneName).contentHash)
    cfg.zones;

  # This projection is deliberately sealed: consumers receive only the
  # parent map, its digest, and the bundle generation identity. The compiler
  # never puts parentZone into the Zone self-resource or a reciprocal row.
  allocatorTopology = {
    sealed = true;
    parentMap = parentMap;
    parentMapDigest = parentMapDigest;
    generationByZone = generationByZone;
  };
in
{
  config = {
    assertions = controlAssertions;
    d2b._resourceCompiler.zoneControl = {
      types = controlTypes;
      byZone = controlResourcesByZone;
      generations = generationByZone;
      allocatorTopology = allocatorTopology;
      parentMap = parentMap;
      parentMapDigest = parentMapDigest;
    };
  };
}
