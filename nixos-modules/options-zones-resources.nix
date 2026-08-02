# Shared validation for the v3 Zone resource authoring surface.
#
# This module intentionally does not import the public module aggregator.
# The integrator can place it beside the generated per-type option modules;
# keeping the compiler seam independent also lets nix-unit exercise the
# resource contract without evaluating host services.
{ config, lib, ... }:

let
  cfg = config.d2b;
  resourceModel = import ./resources.nix { inherit lib; };

  resourceNamePattern = "^[a-z][a-z0-9-]{0,62}$";
  artifactIdPattern = "^[a-z][a-z0-9-]*$";
  digestPattern = "^sha256:[0-9a-f]{64}$";

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

  resourceVerbSet = [
    "get"
    "list"
    "watch"
    "create"
    "update-spec"
    "update-status"
    "update-metadata"
    "update-finalizers"
    "delete"
    "use-credential"
    "admin-credential"
  ];
  sessionVerbSet = [
    "connect"
    "invoke"
    "open-stream"
    "relay"
    "attach"
    "cancel"
    "observe"
    "audit-export"
    "support-bundle"
  ];
  relayBoundTypes = [ "ZoneLink" ];
  subjectTypes = [ "Zone" "Provider" "Host" "Guest" "Process" "User" ];

  parseRef = value:
    let parts = if builtins.isString value then lib.splitString "/" value else [ ];
    in if lib.length parts == 2 then {
      type = builtins.elemAt parts 0;
      name = builtins.elemAt parts 1;
    } else null;

  attrOr = attrs: name: fallback:
    if builtins.isAttrs attrs && builtins.hasAttr name attrs
    then attrs.${name}
    else fallback;

  isRefField = field:
    (lib.hasSuffix "Ref" field || lib.hasSuffix "Refs" field
      || field == "subjects" || field == "resourceRef" || field == "target")
    && !(builtins.elem field [
      "artifactId"
      "systemArtifactId"
      "sourcePolicyId"
      "projectionSchemaFingerprint"
      "factoryFingerprint"
      "expectedProjectionSchemaFingerprint"
      "expectedFactoryFingerprint"
      "schemaFingerprint"
    ]);

  refValues = field: value:
    if !isRefField field then
      [ ]
    else if builtins.isString value then
      [ value ]
    else if builtins.isList value then
      lib.filter builtins.isString value
    else
      [ ];

  collectRefRows = zoneName: resources: value: path: field:
    let
      direct = map (ref: {
        inherit zoneName path field ref resources;
      }) (refValues field value);
    in
    if builtins.isAttrs value then
      direct ++ lib.concatMap
        (key: collectRefRows zoneName resources value.${key} (path ++ [ key ]) key)
        (lib.attrNames value)
    else if builtins.isList value then
      direct ++ lib.concatLists (lib.imap0
        (index: item:
          collectRefRows zoneName resources item (path ++ [ toString index ]) field)
        value)
    else
      direct;

  refResolves = resources: ref:
    let
      parsed = parseRef ref;
    in
    parsed != null
    && resourceModel.validResourceRef ref
    && builtins.hasAttr parsed.name resources
    && resources.${parsed.name}.type == parsed.type;

  stringsIn = value:
    if builtins.isString value then
      [ value ]
    else if builtins.isList value then
      lib.concatMap stringsIn value
    else if builtins.isAttrs value then
      lib.concatMap stringsIn (builtins.attrValues value)
    else
      [ ];

  keyRows = value: path:
    if !builtins.isAttrs value then
      [ ]
    else
      lib.concatMap
        (key:
          [ { inherit key path; } ]
          ++ keyRows value.${key} (path ++ [ key ]))
        (lib.attrNames value);

  hasRuntimeField = value:
    lib.any (row: builtins.elem row.key runtimeFields) (keyRows value [ ]);

  hasFloat = value:
    if builtins.isFloat value then
      true
    else if builtins.isList value then
      lib.any hasFloat value
    else if builtins.isAttrs value then
      lib.any hasFloat (builtins.attrValues value)
    else
      false;

  forbiddenKey = resourceType: key:
    builtins.elem key [
      "argv"
      "binaryPath"
      "commandLine"
      "environment"
      "envVars"
      "hostPath"
      "socketPath"
      "devicePath"
      "numericUid"
      "numericGid"
    ]
    || (resourceType == "Process" || resourceType == "EphemeralProcess")
      && builtins.elem key [ "env" "unit" "program" ];

  hasForbiddenKey = resourceType: value:
    lib.any
      (row: forbiddenKey resourceType row.key || hasForbiddenKey resourceType
        (if builtins.isAttrs value && builtins.hasAttr row.key value then value.${row.key} else { }))
      (keyRows value [ ]);

  hasRawSecret = value:
    lib.any
      (text:
        lib.hasInfix "/nix/store/" text
        || lib.hasInfix "-----BEGIN" text
        || lib.hasPrefix "eyJ" text
        || (
          builtins.match "^sha256:[0-9a-f]{64}$" text == null
          && builtins.match ".*[0-9A-Fa-f]{32,}.*" text != null
        ))
      (stringsIn value);

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

  providerRows = row:
    lib.filter
      (candidate: candidate.resource.type == "Provider")
      (lib.mapAttrsToList
        (name: resource: {
          inherit name resource;
        })
        row.zone.resources);

  providerFor = row: providerRef:
    let parsed = parseRef providerRef;
    in if parsed != null
      && parsed.type == "Provider"
      && builtins.hasAttr parsed.name row.zone.resources
      && row.zone.resources.${parsed.name}.type == "Provider"
    then row.zone.resources.${parsed.name}
    else null;

  artifactFor = artifactId:
    if builtins.isString artifactId && builtins.hasAttr artifactId (cfg.artifacts or { })
    then cfg.artifacts.${artifactId}
    else null;

  catalogMatches = artifactId:
    let catalog = cfg.providerCatalog or { };
    in lib.filter
      (entry:
        let value = entry.artifactId or (entry.entry.artifactId or null);
        in value == artifactId)
      (lib.attrValues catalog);

  providerArtifactAssertions = row:
    let
      spec = row.spec;
      artifactId = spec.artifactId or null;
      artifact = artifactFor artifactId;
      matches = catalogMatches artifactId;
    in
    lib.optionals
      (row.resource.type == "Provider" && (cfg.artifacts or { }) != { }) [
      {
        assertion = builtins.isString artifactId
          && builtins.match artifactIdPattern artifactId != null;
        message = "${row.path}.spec.artifactId must be a bounded plain artifact ID.";
      }
      {
        assertion = artifact != null && (artifact.type or null) == "provider";
        message = "${row.path}.spec.artifactId must resolve to an artifact of type provider.";
      }
      {
        assertion = ((cfg.providerCatalog or { }) == { }) || lib.length matches == 1;
        message = "${row.path}.spec.artifactId must resolve to exactly one provider catalog entry.";
      }
    ];

  systemArtifactAssertions = row:
    let
      spec = row.spec;
      source = spec.source or { };
      ids = [
        (spec.systemArtifactId or null)
        (source.systemArtifactId or null)
      ];
      selected = lib.filter (value: value != null) ids;
      valid = lib.all
        (artifactId:
          let artifact = artifactFor artifactId;
          in builtins.isString artifactId
            && builtins.match artifactIdPattern artifactId != null
            && artifact != null
            && (artifact.type or null) == "nixos-system")
        selected;
    in
    lib.optionals (selected != [ ]) [
      {
        assertion = valid;
        message = "${row.path}: systemArtifactId and source.systemArtifactId must resolve to nixos-system artifacts.";
      }
    ];

  executionAssertions = row:
    let
      spec = row.spec;
      executionRef = spec.executionRef or null;
      target = providerFor row (spec.providerRef or null);
      targetResource =
        let parsed = parseRef executionRef;
        in if parsed != null && builtins.hasAttr parsed.name row.zone.resources
          then row.zone.resources.${parsed.name}
          else null;
      domain = spec.domain or null;
      allowedDomains = if targetResource == null then [ ] else targetResource.spec.allowedDomains or [ ];
      userRef = spec.userRef or null;
      userResolved =
        let parsed = parseRef userRef;
        in parsed != null && parsed.type == "User"
          && builtins.hasAttr parsed.name row.zone.resources
          && row.zone.resources.${parsed.name}.type == "User";
    in
    lib.optionals (builtins.elem row.resource.type [ "Process" "EphemeralProcess" ]) [
      {
        assertion = refResolves row.zone.resources executionRef
          && (let parsed = parseRef executionRef;
              in builtins.elem parsed.type [ "Host" "Guest" ]);
        message = "${row.path}.spec.executionRef must resolve to a Host or Guest in the same Zone.";
      }
      {
        assertion = refResolves row.zone.resources (spec.providerRef or null);
        message = "${row.path}.spec.providerRef must resolve to a Provider in the same Zone.";
      }
      {
        assertion = builtins.elem domain [ "system" "user" ];
        message = "${row.path}.spec.domain must be system or user.";
      }
      {
        assertion = targetResource == null || builtins.elem domain allowedDomains;
        message = "${row.path}.spec.domain must be allowed by its execution target.";
      }
      {
        assertion = userRef == null || userResolved;
        message = "${row.path}.spec.userRef must resolve to a User in the same Zone.";
      }
    ];

  unsafeLocalAssertions = row:
    let
      spec = row.spec;
      providerRef = parseRef (spec.providerRef or null);
      userOnly =
        row.resource.type == "Host"
        && providerRef != null
        && providerRef.type == "Provider"
        && providerRef.name == "system-core"
        && (spec.defaultDomain or null) == "user"
        && (spec.allowedDomains or [ ]) == [ "user" ];
    in
    lib.optionals userOnly [
      {
        assertion = (spec.isolationPosture or null) == "none";
        message = "${row.path}.spec.isolationPosture must be none for a user-only system-core Host.";
      }
    ];

  roleAssertions = row:
    let
      spec = row.spec;
      rules = spec.rules or [ ];
      checkRule = rule:
        let
          verbs = rule.verbs or [ ];
          sessionVerbs = rule.sessionVerbs or [ ];
          relay = builtins.elem "relay" sessionVerbs;
          resourceRelay = builtins.elem "relay" verbs;
          zones = rule.zones or [ ];
          names = rule.resourceNames or [ ];
          relayAuthority = spec.relayAuthority or spec.relayProvenance or null;
          exactZoneBounds = lib.length zones == 1 && lib.length names >= 1
            && lib.all (zone: builtins.hasAttr zone cfg.zones) zones
            && lib.all (name: builtins.match resourceNamePattern name != null) names;
        in [
          {
            assertion = lib.all (verb: builtins.elem verb resourceVerbSet) verbs;
            message = "${row.path}.spec.rules contains an unknown resource verb.";
          }
          {
            assertion = lib.all (verb: builtins.elem verb sessionVerbSet) sessionVerbs
              && !resourceRelay;
            message = "${row.path}.spec.rules must keep session verbs separate from resource verbs.";
          }
          {
            assertion = !relay || exactZoneBounds
              && builtins.elem "ZoneLink" (rule.resourceTypes or [ ])
              && builtins.elem relayAuthority [ "core-generated" "durable-local-admin" ];
            message = "${row.path}.spec.rules relay grants require exact ZoneLink-scoped bounds.";
          }
        ];
    in
    lib.optionals (row.resource.type == "Role") (lib.concatMap checkRule rules);

  roleBindingAssertions = row:
    let
      spec = row.spec;
      subjects = spec.subjects or [ ];
      roleRef = spec.roleRef or null;
      subjectValid = subject:
        let parsed = parseRef subject;
        in parsed != null
          && builtins.elem parsed.type subjectTypes
          && refResolves row.zone.resources subject;
      roleValid =
        let parsed = parseRef roleRef;
        in parsed != null && parsed.type == "Role"
          && refResolves row.zone.resources roleRef;
    in
    lib.optionals (row.resource.type == "RoleBinding") [
      {
        assertion = roleValid;
        message = "${row.path}.spec.roleRef must resolve to a Role in the same Zone.";
      }
      {
        assertion = lib.all subjectValid subjects;
        message = "${row.path}.spec.subjects must contain only resolved same-Zone subjects.";
      }
    ];

  endpointAssertions = row:
    let
      spec = row.spec;
      visibility = spec.visibility or null;
      operations = (spec.consumerPolicy or { }).allowedOperations or [ ];
    in
    lib.optionals (row.resource.type == "Endpoint") [
      {
        assertion = builtins.elem visibility [ "owner" "provider" "zone" ];
        message = "${row.path}.spec.visibility must be owner, provider, or zone.";
      }
      {
        assertion = operations == [ ]
          || lib.all (operation: builtins.elem operation [ "resolve" "attach" "observe" ]) operations;
        message = "${row.path}.spec.consumerPolicy.allowedOperations must use canonical endpoint operations.";
      }
    ];

  credentialAssertions = row:
    let
      spec = row.spec;
      endpoint = row.zone.resources;
      identity = spec.identityGuestRef or null;
      login = spec.loginEndpointRef or null;
      consumer = spec.consumerRef or null;
      endpointResource =
        let parsed = parseRef login;
        in if parsed != null && builtins.hasAttr parsed.name endpoint
          then endpoint.${parsed.name}
          else null;
      policy = if endpointResource == null then { } else endpointResource.spec.consumerPolicy or { };
      subjects = policy.allowedSubjects or [ ];
      requiredSubjects = [
        "Provider/credential-entra"
        consumer
      ];
    in
    lib.optionals (row.resource.type == "Credential"
      && (let parsed = parseRef (spec.providerRef or null);
          in parsed != null && parsed.name == "credential-entra")) [
      {
        assertion = refResolves row.zone.resources identity
          && refResolves row.zone.resources login
          && refResolves row.zone.resources consumer;
        message = "${row.path}: credential-entra references must resolve within the Zone.";
      }
      {
        assertion = endpointResource != null
          && endpointResource.type == "Endpoint"
          && (endpointResource.spec.purpose or null)
            == "credential-entra.d2bus.org/entra-login-token"
          && (endpointResource.spec.visibility or null) == "provider"
          && lib.all (subject: builtins.elem subject subjects) requiredSubjects
          && builtins.sort builtins.lessThan (policy.allowedOperations or [ ])
            == [ "resolve" ];
        message = "${row.path}: credential-entra login Endpoint must use the provider-only resolve contract.";
      }
    ];

  resourceAssertions = row:
    let
      type = row.resource.type;
      spec = row.spec;
      parsedRefs = collectRefRows row.zoneName row.zone.resources spec [ "spec" ] "";
      runtimeSpec = lib.filter (field: builtins.hasAttr field spec) runtimeFields;
    in [
      {
        assertion = resourceModel.validResourceType type;
        message = "${row.path}.type is not a registered ResourceType.";
      }
      {
        assertion = runtimeSpec == [ ];
        message = "${row.path}.spec must not author runtime-managed fields: ${lib.concatStringsSep ", " runtimeSpec}.";
      }
      {
        assertion = !(builtins.hasAttr "name" (row.resource.metadata or { }))
          && !(builtins.hasAttr "zone" (row.resource.metadata or { }));
        message = "${row.path}.metadata.name and metadata.zone are compiler-derived.";
      }
      {
        assertion = !hasRuntimeField spec;
        message = "${row.path}.spec contains a runtime-managed field.";
      }
      {
        assertion = !hasFloat spec;
        message = "${row.path}.spec must contain JSON integers, not floating-point values.";
      }
      {
        assertion = !hasForbiddenKey type spec;
        message = "${row.path}.spec contains a free-form executable, socket, UID, GID, or environment field.";
      }
      {
        assertion = !hasRawSecret spec;
        message = "${row.path}.spec contains a raw store path or secret-shaped value.";
      }
      {
        assertion = type != "Zone" || spec == { };
        message = "${row.path}.spec must be the empty object for the runtime-created Zone self-resource.";
      }
      {
        assertion =
          builtins.elem row.resource.type [ "Host" "Guest" ]
          || lib.all
            (ref:
              resourceModel.validResourceRef ref.ref
              && refResolves row.zone.resources ref.ref)
            parsedRefs;
        message = "${row.path}: every ResourceRef must be canonical and resolve in the same Zone.";
      }
    ]
    ++ providerArtifactAssertions row
    ++ systemArtifactAssertions row
    ++ executionAssertions row
    ++ unsafeLocalAssertions row
    ++ roleAssertions row
    ++ roleBindingAssertions row
    ++ endpointAssertions row
    ++ credentialAssertions row;

  # Provider-neutral primitive checks.  Provider-owned extension fields stay
  # freeform here and are validated by the selected Provider schema.
  quantityValue = value:
    let
      match = if builtins.isString value
        then builtins.match "^([0-9]+)(m|B|KB|MB|GB|TB|KiB|MiB|GiB|TiB)$" value
        else null;
      multipliers = {
        m = 1;
        B = 1;
        KB = 1000;
        MB = 1000000;
        GB = 1000000000;
        TB = 1000000000000;
        KiB = 1024;
        MiB = 1048576;
        GiB = 1073741824;
        TiB = 1099511627776;
      };
    in
      if match == null then null
      else (lib.toInt (builtins.elemAt match 0)) * multipliers.${builtins.elemAt match 1};

  requestNotAboveLimit = value:
    let
      request = quantityValue (value.request or null);
      limit = quantityValue (value.limit or null);
    in request == null || limit == null || request <= limit;

  primitiveAssertions = row:
    let
      type = row.resource.type;
      spec = row.spec;
      posture = spec.isolationPosture or null;
      providerRef = parseRef (spec.providerRef or null);
      userOnly = type == "Host"
        && providerRef != null
        && providerRef.type == "Provider"
        && providerRef.name == "system-core"
        && (spec.defaultDomain or null) == "user"
        && (spec.allowedDomains or [ ]) == [ "user" ]
        && (spec.defaultUserRef or null) != null;
      budget = spec.budget or { };
      mounts = spec.mounts or [ ];
      mountPaths = map (mount: mount.mountPath or null) mounts;
    in
      lib.optionals (type == "User" && spec ? osUsername) [
        {
          assertion = builtins.isString spec.osUsername
            && builtins.stringLength spec.osUsername >= 1
            && builtins.stringLength spec.osUsername <= 255
            && !(lib.hasInfix "/" spec.osUsername)
            && !(lib.hasInfix "\\" spec.osUsername)
            && !(lib.hasInfix (builtins.fromJSON "\"\\u0000\"") spec.osUsername);
          message = "${row.path}.spec.osUsername must be a bounded OS username without NUL or path separators.";
        }
        {
          assertion = lib.all
            (group: builtins.isString group
              && builtins.match "^[a-z_][a-z0-9_-]{0,62}$" group != null)
            (spec.groups or [ ]);
          message = "${row.path}.spec.groups must contain lower-case OS group names.";
        }
      ]
      ++ lib.optionals (type == "Host") [
        {
          assertion = posture == null || posture == "none";
          message = "${row.path}.spec.isolationPosture must be null or none.";
        }
        {
          assertion = !userOnly || posture == "none";
          message = "${row.path}.spec.isolationPosture=none is required for a user-only Host.";
        }
        {
          assertion = posture != "none"
            || ((spec.defaultDomain or null) == "user"
              && (spec.allowedDomains or [ ]) == [ "user" ]
              && (spec.defaultUserRef or null) != null);
          message = "${row.path}.spec.isolationPosture=none requires a user-only Host.";
        }
      ]
      ++ lib.optionals (builtins.elem type
        [ "Host" "Guest" "Process" "EphemeralProcess" ]) [
        {
          assertion = requestNotAboveLimit (budget.cpu or { });
          message = "${row.path}.spec.budget.cpu.request must not exceed limit.";
        }
        {
          assertion = requestNotAboveLimit (budget.memory or { });
          message = "${row.path}.spec.budget.memory.request must not exceed limit.";
        }
      ]
      ++ lib.optionals (builtins.elem type [ "Process" "EphemeralProcess" ]) [
        {
          assertion = lib.length mountPaths == lib.length (lib.unique mountPaths);
          message = "${row.path}.spec.mounts must not repeat mountPath.";
        }
        {
          assertion = type != "EphemeralProcess"
            || (spec.processClass or null) == "worker";
          message = "${row.path}.spec.processClass must be worker for EphemeralProcess.";
        }
      ];

  allAssertions = lib.concatMap
    (row: resourceAssertions row ++ primitiveAssertions row)
    resourceRows;
in
{
  config.assertions = allAssertions;
}
