# Process and EphemeralProcess resource compiler.
#
# This is the sole Nix process projection. Executable paths, argv, environment
# maps, and numeric identities never cross the Zone resource boundary.
{ config, lib, ... }:

let
  cfg = config.d2b;
  genericExecutionFields = [
    "defaultDomain"
    "allowedDomains"
    "defaultUserRef"
    "networkAttachments"
    "deviceAttachments"
    "volumeAttachmentDefaults"
  ];

  processFields = [
    "providerRef"
    "executionRef"
    "domain"
    "userRef"
    "processClass"
    "template"
    "configRef"
    "credentialRefs"
    "mounts"
    "sandbox"
    "budget"
    "networkUsage"
    "deviceUsage"
    "telemetry"
    "desiredLifecycle"
    "restartPolicy"
    "readiness"
    "healthCheck"
    "adoptionPolicy"
    "drainTimeout"
    "provider"
    "updatePolicy"
  ];

  ephemeralFields = [
    "providerRef"
    "executionRef"
    "domain"
    "userRef"
    "processClass"
    "template"
    "configRef"
    "credentialRefs"
    "mounts"
    "sandbox"
    "budget"
    "networkUsage"
    "deviceUsage"
    "telemetry"
    "activationInput"
    "startDeadline"
    "runtimeDeadline"
    "successfulTtl"
    "failedTtl"
    "incidentHold"
    "provider"
    "updatePolicy"
  ];

  forbiddenLegacyFields = [
    "packageRef"
    "network"
    "devices"
    "endpoints"
    "restart"
    "runDeadline"
    "binaryPath"
    "argv"
    "commandLine"
    "environment"
    "uid"
    "gid"
  ];

  tokenPattern = "^[a-z][a-z0-9-]{0,62}$";
  durationPattern = "^[0-9]+(ms|s|m|h)$";

  defaultBudget = {
    cpu = { request = null; limit = null; };
    memory = { request = null; limit = null; };
    pids = { limit = null; };
    fds = { limit = null; };
    ioWeight = null;
    networkEgressBps = null;
    threadLimit = null;
  };

  executionDefaults = {
    domain = null;
    userRef = null;
    configRef = null;
    credentialRefs = [ ];
    mounts = [ ];
    sandbox = {
      namespaceClasses = [ ];
      capabilityClasses = [ ];
      seccompClass = "strict";
      noNewPrivileges = true;
      startRoot = false;
      environmentClass = "minimal";
      readOnlyRoot = true;
      umask = "0022";
      oomScoreAdj = 0;
      userNamespace = null;
    };
    budget = { };
    networkUsage = null;
    deviceUsage = [ ];
    telemetry = {
      metricsEnabled = true;
      tracingEnabled = true;
      logLevel = "info";
      sensitiveLabels = false;
    };
  };

  processDefaults = executionDefaults // {
    desiredLifecycle = "running";
    restartPolicy = {
      class = "on-failure";
      backoffBase = "1s";
      backoffMax = "60s";
      backoffMultiplierMilli = 2000;
      maxRestarts = null;
      resetAfter = "300s";
    };
    readiness = {
      initialDelay = "0s";
      timeout = "30s";
      failureThreshold = 3;
      successThreshold = 1;
      class = "ready-condition";
    };
    healthCheck = {
      enabled = false;
      interval = "30s";
      timeout = "5s";
      failureThreshold = 3;
      class = "provider-defined";
    };
    adoptionPolicy = "adopt-on-restart";
    drainTimeout = "30s";
  };

  ephemeralDefaults = executionDefaults // {
    startDeadline = "60s";
    runtimeDeadline = "300s";
    successfulTtl = "1h";
    failedTtl = "24h";
    incidentHold = false;
  };

  attrOr = attrs: name: fallback:
    if builtins.isAttrs attrs && builtins.hasAttr name attrs
    then attrs.${name}
    else fallback;

  parseRef = value:
    let parts = if builtins.isString value then lib.splitString "/" value else [ ];
    in if lib.length parts == 2 then {
      type = builtins.elemAt parts 0;
      name = builtins.elemAt parts 1;
    } else null;

  resolvesAs = resources: types: value:
    let parsed = parseRef value;
    in parsed != null
      && builtins.elem parsed.type types
      && builtins.hasAttr parsed.name resources
      && resources.${parsed.name}.type == parsed.type;

  # A declared mount names its Volume in exactly one of two mutually exclusive
  # ways.
  #
  # `volumeRef` is a Volume row declared in the same Zone. That rule is
  # unchanged and complete on its own: it needs no owner, and a row that
  # declares none is still admitted.
  #
  # `ownVolumeSuffix` is the controller-composed child Volume of the row's own
  # declaring Device. That child is a real shape in the resource plane, not a
  # leak: a Device-owned Volume is built by the Device manager at runtime
  # (`build_tpm_state_volume_resource`, `managedBy = "controller"`), so it is
  # never a declared row. Its concrete name embeds the owning Device's durable
  # uid (`device-<uid>-<suffix>`), and a resource uid is a digest over
  # NUL-separated identity fields that a Nix expression cannot spell at all -
  # so the row declares WHICH child, the Device that owns it and the role
  # within that Device's private child set, and the framework materializes the
  # name. What this layer can check is the declaration, not a derived name:
  # the row must declare a Device owner that resolves to a Device of this
  # Zone, and the role must be a bounded token. A row with no Device owner
  # cannot name a Device's private child, and a projection cannot name a
  # sibling's child at all, because the owning Device is the row's own
  # `ownerRef` and never a field of the mount.
  #
  # The ownership rule over the concrete name is enforced where the name
  # exists: `d2b_core::bundle_resolver::device_owns_volume`, run on the
  # resolved bundle and again by the resource compiler on the declared row.
  declaredVolume = row: mount:
    resolvesAs row.zone.resources [ "Volume" ] (mount.volumeRef or null);

  ownChildVolume = row: mount:
    let
      ownerRef = (row.resource.metadata or { }).ownerRef or null;
      suffix = mount.ownVolumeSuffix or null;
    in builtins.isString suffix
      && builtins.match tokenPattern suffix != null
      && resolvesAs row.zone.resources [ "Device" ] ownerRef;

  namesOneVolume = mount:
    (builtins.hasAttr "volumeRef" mount)
    != (builtins.hasAttr "ownVolumeSuffix" mount);

  volumeAdmitted = row: mount:
    declaredVolume row mount || ownChildVolume row mount;

  exactKeys = allowed: value:
    builtins.isAttrs value
    && lib.all (key: builtins.elem key allowed) (lib.attrNames value);

  stripGeneric = spec: builtins.removeAttrs spec genericExecutionFields;

  rows = lib.concatMap
    (zoneName:
      let zone = cfg.zones.${zoneName};
      in lib.mapAttrsToList
        (resourceName: resource: {
          inherit zoneName resourceName resource zone;
          path = "d2b.zones.${zoneName}.resources.${resourceName}";
          spec = stripGeneric (resource.spec or { });
        })
        (lib.filterAttrs
          (_: resource:
            builtins.elem resource.type [ "Process" "EphemeralProcess" ])
          zone.resources))
    (lib.sort lib.lessThan (lib.attrNames cfg.zones));

  checkMount = row: index: mount:
    let
      path = "${row.path}.spec.mounts.${toString index}";
      view = mount.view or null;
      access = mount.access or null;
      mountPath = mount.mountPath or null;
    in [
      {
        assertion = exactKeys [ "volumeRef" "ownVolumeSuffix" "view" "mountPath" "access" "required" ] mount;
        message = "${path} contains an unsupported field.";
      }
      {
        assertion = namesOneVolume mount;
        message = "${path} must name exactly one of volumeRef or ownVolumeSuffix.";
      }
      {
        assertion = volumeAdmitted row mount;
        message = "${path} must name a Volume declared in the same Zone, or ownVolumeSuffix naming this row's owning Device's controller-created child Volume.";
      }
      {
        assertion = builtins.isString view && builtins.match tokenPattern view != null;
        message = "${path}.view must be a bounded view name.";
      }
      {
        assertion = builtins.isString mountPath
          && lib.hasPrefix "/" mountPath
          && !(builtins.elem ".." (lib.splitString "/" mountPath));
        message = "${path}.mountPath must be an absolute guest path without traversal.";
      }
      {
        assertion = builtins.elem access [ "read-only" "read-write" "shared-write" ];
        message = "${path}.access must be read-only, read-write, or shared-write.";
      }
      {
        assertion = builtins.isBool (mount.required or true);
        message = "${path}.required must be boolean.";
      }
    ];

  processAssertions = row:
    let
      spec = row.spec;
      fields = if row.resource.type == "Process" then processFields else ephemeralFields;
      unknown = lib.filter
        (field: !(builtins.elem field (fields ++ genericExecutionFields)))
        (lib.attrNames (row.resource.spec or { }));
      mounts = spec.mounts or [ ];
      credentialRefs = spec.credentialRefs or [ ];
      providerRef = spec.providerRef or null;
      executionRef = spec.executionRef or null;
      target =
        let parsed = parseRef executionRef;
        in if parsed != null
          && builtins.elem parsed.type [ "Host" "Guest" ]
          && builtins.hasAttr parsed.name row.zone.resources
          && row.zone.resources.${parsed.name}.type == parsed.type
          then row.zone.resources.${parsed.name}
          else null;
      allowedDomains = if target == null then [ ] else target.spec.allowedDomains or [ ];
      effectiveDomain =
        if (spec.domain or null) != null
        then spec.domain
        else if target != null
        then target.spec.defaultDomain or null
        else null;
      targetNoIsolation =
        target != null
        && target.type == "Host"
        && (target.spec.isolationPosture or null) == "none";
      providerResolved =
        resolvesAs row.zone.resources [ "Provider" ] providerRef
        || ((row.generated or false)
          && builtins.elem providerRef [
            "Provider/system-minijail"
            "Provider/system-systemd"
          ]);
      durationFields =
        if row.resource.type == "Process"
        then [ "drainTimeout" ]
        else [ "startDeadline" "runtimeDeadline" "successfulTtl" "failedTtl" ];
      durationChecks = map
        (field: {
          assertion = !builtins.hasAttr field spec
            || (builtins.isString spec.${field}
              && builtins.match durationPattern spec.${field} != null);
          message = "${row.path}.spec.${field} must be a bounded duration string.";
        })
        durationFields;
    in
    [
      {
        assertion = unknown == [ ];
        message = "${row.path}.spec contains unsupported Process fields: ${lib.concatStringsSep ", " unknown}.";
      }
      {
        assertion = lib.all (field: !(builtins.elem field forbiddenLegacyFields))
          (lib.attrNames (row.resource.spec or { }));
        message = "${row.path}.spec contains a retired executable or runtime field.";
      }
      {
        assertion = providerResolved;
        message = "${row.path}.spec.providerRef must resolve to a Provider in the same Zone.";
      }
      {
        assertion = resolvesAs row.zone.resources [ "Host" "Guest" ] executionRef;
        message = "${row.path}.spec.executionRef must resolve to a Host or Guest in the same Zone.";
      }
      {
        assertion = (spec.domain or null) == null
          || builtins.elem spec.domain [ "system" "user" ];
        message = "${row.path}.spec.domain must be system or user.";
      }
      {
        assertion = target == null
          || effectiveDomain == null
          || builtins.elem effectiveDomain allowedDomains;
        message = "${row.path}.spec.domain must be allowed by its execution target.";
      }
      {
        assertion = !targetNoIsolation || effectiveDomain == "user";
        message = "${row.path}.spec.domain must be user for a no-isolation Host target.";
      }
      {
        assertion =
          effectiveDomain != "user"
          || (spec.userRef or null) != null
          || (target != null && (target.spec.defaultUserRef or null) != null);
        message = "${row.path}.spec.userRef is required for user-domain execution when the target has no default user.";
      }
      {
        assertion = (spec.userRef or null) == null
          || resolvesAs row.zone.resources [ "User" ] spec.userRef;
        message = "${row.path}.spec.userRef must resolve to a User in the same Zone.";
      }
      {
        assertion = builtins.isString (spec.processClass or "")
          && builtins.match tokenPattern spec.processClass != null;
        message = "${row.path}.spec.processClass must be a bounded process class.";
      }
      {
        assertion = builtins.isString (spec.template or "")
          && builtins.match tokenPattern spec.template != null;
        message = "${row.path}.spec.template must be a bounded Provider template name.";
      }
      {
        assertion = builtins.isList credentialRefs
          && lib.length credentialRefs <= 16
          && lib.all (ref: resolvesAs row.zone.resources [ "Credential" ] ref) credentialRefs;
        message = "${row.path}.spec.credentialRefs must contain at most 16 same-Zone Credential resources.";
      }
      {
        assertion = builtins.isList mounts && lib.length mounts <= 64;
        message = "${row.path}.spec.mounts must contain at most 64 entries.";
      }
      {
        assertion = row.resource.type != "EphemeralProcess"
          || !(builtins.hasAttr "restartPolicy" spec);
        message = "${row.path}.spec.restartPolicy is not valid for EphemeralProcess.";
      }
      {
        assertion = row.resource.type != "Process"
          || !(builtins.hasAttr "runtimeDeadline" spec)
          && !(builtins.hasAttr "runDeadline" (row.resource.spec or { }));
        message = "${row.path}.spec.runtimeDeadline is only valid for EphemeralProcess.";
      }
    ]
    ++ durationChecks
    ++ lib.concatLists (lib.imap0 (checkMount row) mounts);

  # The ProcessRole -> owning Provider map is generated from the process
  # family's `resource-types.json` declaration (U4), so renaming an owning
  # provider reference needs no hand edit here.
  roleProviderMap = import ./generated/process-role-providers.nix;

  # Provider packages publish their Process intents through one fixed,
  # owner-keyed compiler table. This is an internal merge seam, not a public
  # extension registry or a second resource vocabulary.
  # The Provider projections this consumer folds, and the compiler option key
  # each one lands on. Generated from the closed Provider matrix, so the three
  # consumers and the projection producers cannot drift apart.
  providerProjections = import ./generated/provider-projections.nix;
  providerProjectionOwners = providerProjections.owners;
  providerProjectionKeys = providerProjections.keys;

  providerProjection = owner:
    let
      table = cfg._resourceCompiler or { };
      key = builtins.getAttr owner providerProjectionKeys;
    in if builtins.hasAttr key table
    then builtins.getAttr key table
    else { };

  providerProcessRows = lib.concatMap
    (owner:
      let projection = providerProjection owner;
      in if !(projection.enabled or false)
      then [ ]
      else lib.concatMap
        (zoneName:
          lib.mapAttrsToList
            (resourceName: resource: {
              inherit zoneName resourceName resource;
              path = "d2b.zones.${zoneName}.resources.${resourceName}";
              spec = resource.spec or { };
              zone = cfg.zones.${zoneName};
              generated = true;
            })
            ((projection.processesByZone or { }).${zoneName} or { }))
        (lib.sort lib.lessThan (lib.attrNames ((projection.processesByZone or { })))))
    providerProjectionOwners;

  canonical = row:
    let
      spec = row.spec;
      fields = if row.resource.type == "Process" then processFields else ephemeralFields;
      selectedRaw = lib.filterAttrs (key: _: builtins.elem key fields) spec;
      selected =
        if selectedRaw ? budget && selectedRaw.budget == defaultBudget
        then selectedRaw // { budget = { }; }
        else selectedRaw;
      defaults =
        if row.resource.type == "Process"
        then processDefaults
        else ephemeralDefaults;
    in lib.recursiveUpdate defaults selected;

  canonicalResource = row: {
    apiVersion = "resources.d2bus.org/v3";
    type = row.resource.type;
    metadata = {
      name = row.resourceName;
      zone = row.zoneName;
    }
    // lib.optionalAttrs ((row.resource.metadata.ownerRef or null) != null) {
      ownerRef = row.resource.metadata.ownerRef;
    }
    // lib.optionalAttrs ((row.resource.metadata.labels or { }) != { }) {
      labels = row.resource.metadata.labels;
    }
    // lib.optionalAttrs ((row.resource.metadata.annotations or { }) != { }) {
      annotations = row.resource.metadata.annotations;
    };
    spec = canonical row;
  };

  compiled = lib.foldl'
    (result: row:
      result // {
        ${row.zoneName} = (result.${row.zoneName} or { }) // {
          ${row.resourceName} = canonicalResource row;
        };
      })
    { }
    (rows ++ providerProcessRows);
in
{
  config = {
    assertions = lib.concatMap processAssertions (rows ++ providerProcessRows);
    d2b._resourceCompiler.processes = {
      byZone = compiled;
      roles = roleProviderMap;
      rows = rows ++ providerProcessRows;
    };
  };
}
