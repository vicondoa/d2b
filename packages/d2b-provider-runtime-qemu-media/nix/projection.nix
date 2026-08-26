# Zone resource projection for Provider/runtime-qemu-media.
#
# QEMU media Guests receive a signed Guest settings envelope and one
# Guest-owned Process intent. Media locators remain Volume references; binary,
# QMP, and fd details are private to the Provider runtime.
{ config, lib, ... }:

let
  cfg = config.d2b;
  providerRef = "Provider/runtime-qemu-media";
  processExecutorRef = "Provider/system-minijail";
  zones = cfg.zones or { };
  resourcesFor = zoneName: zones.${zoneName}.resources or { };

  providerFor = zoneName:
    if builtins.hasAttr "runtime-qemu-media" (resourcesFor zoneName)
      && (resourcesFor zoneName).runtime-qemu-media.type == "Provider"
    then (resourcesFor zoneName).runtime-qemu-media
    else null;

  providerAssertions = zoneName:
    let
      provider = providerFor zoneName;
      resources = resourcesFor zoneName;
      config = if provider == null then { } else provider.spec.config or { };
      keys = [
        "controllerExecutionRef"
        "qemuBinaryArtifactId"
        "networkProviderRef"
        "volumeProviderRef"
        "displayProviderRef"
        "pausedAtBootDefault"
        "qmpReadyTimeoutSeconds"
        "qmpOperationTimeoutSeconds"
        "runtimeTmpfsQuotaBytes"
        "runtimeTmpfsQuotaInodes"
      ];
      resolvesHost = value:
        builtins.isString value
        && lib.hasPrefix "Host/" value
        && lib.length (lib.splitString "/" value) == 2
        && builtins.hasAttr (builtins.elemAt (lib.splitString "/" value) 1) resources
        && (resources.${builtins.elemAt (lib.splitString "/" value) 1}).type == "Host";
      path = "d2b.zones.${zoneName}.resources.runtime-qemu-media.spec.config";
    in
    if provider == null
    then [ ]
    else [
      {
        assertion = lib.all (key: builtins.elem key keys) (lib.attrNames config);
        message = "${path} contains an unsupported runtime-qemu-media Provider field.";
      }
      {
        assertion = (config.controllerExecutionRef or null) == null
          || resolvesHost config.controllerExecutionRef;
        message = "${path}.controllerExecutionRef must resolve to a same-Zone Host.";
      }
      {
        assertion = !(builtins.hasAttr "qmpReadyTimeoutSeconds" config)
          || (builtins.isInt config.qmpReadyTimeoutSeconds
            && config.qmpReadyTimeoutSeconds >= 5
            && config.qmpReadyTimeoutSeconds <= 300);
        message = "${path}.qmpReadyTimeoutSeconds is out of bounds.";
      }
      {
        assertion = !(builtins.hasAttr "qmpOperationTimeoutSeconds" config)
          || (builtins.isInt config.qmpOperationTimeoutSeconds
            && config.qmpOperationTimeoutSeconds >= 5
            && config.qmpOperationTimeoutSeconds <= 300);
        message = "${path}.qmpOperationTimeoutSeconds is out of bounds.";
      }
    ];

  executionRefFor = zoneName:
    let provider = providerFor zoneName;
    in if provider == null
      then null
      else (provider.spec.config or { }).controllerExecutionRef or null;

  networkUsage = spec:
    let attachments = spec.networkAttachments or [ ];
    in if attachments == [ ]
      then null
      else {
        networkRef = (builtins.head attachments).networkRef;
        ports = [ ];
        allowEgress = true;
      };

  deviceUsage = spec:
    let attachments = spec.deviceAttachments or [ ];
    in if attachments == [ ]
      then [ ]
      else [{
        deviceRef = (builtins.head attachments).deviceRef;
        access = if (builtins.head attachments).exclusive or false
          then "exclusive"
          else "shared";
        purpose = "kvm-acceleration";
      }];

  guestRowsFor = zoneName:
    if providerFor zoneName == null
    then [ ]
    else lib.mapAttrsToList
      (guestName: guest: {
          inherit zoneName guestName guest;
          spec = guest.spec or { };
        })
        (lib.filterAttrs
          (_: resource:
            resource.type == "Guest"
            && (resource.spec.providerRef or null) == providerRef)
          (resourcesFor zoneName));

  processFor = row:
    let executionRef = executionRefFor row.zoneName;
    in lib.optionalAttrs (executionRef != null) {
      type = "Process";
      metadata = {
        name = "qemu-media-${row.guestName}";
        zone = row.zoneName;
        ownerRef = "Guest/${row.guestName}";
      };
      spec = {
        providerRef = processExecutorRef;
        inherit executionRef;
        domain = "system";
        processClass = "worker";
        template = "qemu-media-runner";
        desiredLifecycle = "running";
        networkUsage = networkUsage row.spec;
        deviceUsage = deviceUsage row.spec;
      };
    };

  runnerRowsForZone = zoneName:
    lib.filter
      (row: processFor row != { })
      (guestRowsFor zoneName);

  processRowsForZone = zoneName:
    map processFor (runnerRowsForZone zoneName);

  processForZone = zoneName:
    lib.listToAttrs (map
      (process: lib.nameValuePair process.metadata.name process)
      (processRowsForZone zoneName));

  resourcesForZone = zoneName:
    lib.listToAttrs (map
      (row:
        let process = processFor row;
        in lib.nameValuePair "qemu-${row.guestName}-qmp" {
          type = "Endpoint";
          metadata = {
            name = "qemu-${row.guestName}-qmp";
            zone = zoneName;
            ownerRef = "Guest/${row.guestName}";
          };
          spec = {
            producerRef = "${process.type}/${process.metadata.name}";
            providerRef = providerRef;
            endpointClass = "control";
            transport = "opaque-carriage";
            purpose = "runtime-qemu-media.d2bus.org/qmp";
            serviceFingerprint = null;
            locality = "host-local";
            visibility = "owner";
            attachmentPolicy = {
              supported = true;
              maxAttachments = 1;
            };
            consumerPolicy = {
              allowedSubjects = [ "Guest/${row.guestName}" ];
              allowedProviderComponents = [ "runtime-qemu-media" ];
              allowedOperations = [ "resolve" "attach" ];
            };
            lifecyclePolicy = "recycle-with-producer";
          };
        })
      (runnerRowsForZone zoneName));

  guestPatchForZone = zoneName:
    lib.listToAttrs (map
      (row:
        lib.nameValuePair row.guestName {
          provider = {
            schemaId = "runtime-qemu-media.d2bus.org/Guest/spec";
            schemaVersion = "1.0";
            settings = {
              pauseAtBoot = true;
              displayWindow = false;
              serialConsole = true;
              tablet = true;
              rtcBase = "utc";
            };
          };
        })
      (runnerRowsForZone zoneName));
in
{
  config = {
    assertions = lib.concatMap providerAssertions (lib.attrNames zones);
    d2b._resourceCompiler.providerProjectionRuntimeQemuMedia = {
      enabled = lib.any
        (zoneName: processRowsForZone zoneName != [ ])
        (lib.attrNames zones);
      processesByZone = lib.genAttrs (lib.attrNames zones) processForZone;
      resourcesByZone = lib.genAttrs (lib.attrNames zones) resourcesForZone;
      guestPatchesByZone = lib.genAttrs (lib.attrNames zones) guestPatchForZone;
      privateArtifact = {
        schemaVersion = 1;
        providerRef = providerRef;
        processRefs = lib.concatMap
          (zoneName: map
            (process: "${process.type}/${process.metadata.name}")
            (processRowsForZone zoneName))
          (lib.attrNames zones);
        endpointRefs = lib.concatMap
          (zoneName: map
            (row: "Endpoint/qemu-${row.guestName}-qmp")
            (runnerRowsForZone zoneName))
          (lib.attrNames zones);
      };
    };
  };
}
