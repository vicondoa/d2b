# Process and EphemeralProcess resource compiler.
#
# The legacy processes.json emitter remains active while the Provider
# migration is in progress. This module is the v3 projection: executable
# paths, argv, environment maps, and numeric identities never cross this
# boundary.
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
      volumeRef = mount.volumeRef or null;
      view = mount.view or null;
      access = mount.access or null;
      mountPath = mount.mountPath or null;
    in [
      {
        assertion = exactKeys [ "volumeRef" "view" "mountPath" "access" "required" ] mount;
        message = "${path} contains an unsupported field.";
      }
      {
        assertion = resolvesAs row.zone.resources [ "Volume" ] volumeRef;
        message = "${path}.volumeRef must resolve to a Volume in the same Zone.";
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

  roleProviderMap = {
    StoreVirtiofsPreflight = "Provider/volume-virtiofs";
    SwtpmPreStartFlush = "Provider/device-tpm";
    Swtpm = "Provider/device-tpm";
    Virtiofsd = "Provider/volume-virtiofs";
    Video = "Provider/device-gpu";
    Gpu = "Provider/device-gpu";
    GpuRenderNode = "Provider/device-gpu";
    Audio = "Provider/audio-pipewire";
    CloudHypervisorRunner = "Provider/runtime-cloud-hypervisor";
    QemuMediaRunner = "Provider/runtime-qemu-media";
    ActivationNixosRunner = "Provider/activation-nixos";
    VsockRelay = "Provider/transport-vsock";
    OtelHostBridge = "Provider/observability-otel";
    Usbip = "Provider/device-usbip";
    SecurityKeyFrontend = "Provider/device-security-key";
    WaylandProxy = "Provider/display-wayland";
  };

  # Provider packages publish their Process intents through one fixed,
  # owner-keyed compiler table. This is an internal merge seam, not a public
  # extension registry or a second resource vocabulary.
  providerProjectionOwners = [
    "volume-local"
    "volume-virtiofs"
    "device-gpu"
    "device-usbip"
    "device-security-key"
    "device-tpm"
    "display-wayland"
    "audio-pipewire"
    "clipboard-wayland"
    "notification-desktop"
    "activation-nixos"
    "observability-otel"
    "shell-terminal"
    "runtime-cloud-hypervisor"
    "runtime-qemu-media"
    "runtime-azure-container-apps"
    "runtime-azure-virtual-machine"
  ];

  providerProjectionKeys = {
    "volume-local" = "providerProjectionVolumeLocal";
    "volume-virtiofs" = "providerProjectionVolumeVirtiofs";
    "device-gpu" = "providerProjectionDeviceGpu";
    "device-usbip" = "providerProjectionDeviceUsbip";
    "device-security-key" = "providerProjectionDeviceSecurityKey";
    "device-tpm" = "providerProjectionDeviceTpm";
    "display-wayland" = "providerProjectionDisplayWayland";
    "audio-pipewire" = "providerProjectionAudioPipewire";
    "clipboard-wayland" = "providerProjectionClipboardWayland";
    "notification-desktop" = "providerProjectionNotificationDesktop";
    "activation-nixos" = "providerProjectionActivationNixos";
    "observability-otel" = "providerProjectionObservabilityOtel";
    "shell-terminal" = "providerProjectionShellTerminal";
    "runtime-cloud-hypervisor" = "providerProjectionRuntimeCloudHypervisor";
    "runtime-qemu-media" = "providerProjectionRuntimeQemuMedia";
    "runtime-azure-container-apps" = "providerProjectionRuntimeAzureContainerApps";
    "runtime-azure-virtual-machine" = "providerProjectionRuntimeAzureVirtualMachine";
  };

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
