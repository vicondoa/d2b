# Zone resource projection for Provider/runtime-cloud-hypervisor.
#
# The runtime controller owns binary and socket details. Nix contributes only
# the typed Guest-owned Process intent and its same-Zone attachment refs.
{ config, lib, ... }:

let
  cfg = config.d2b;
  providerRef = "Provider/runtime-cloud-hypervisor";
  processProviderRef = "Provider/system-minijail";
  zones = cfg.zones or { };

  resourcesFor = zoneName: zones.${zoneName}.resources or { };

  guestRowsFor = zoneName:
    lib.mapAttrsToList
      (guestName: guest: {
        inherit zoneName guestName guest;
        spec = guest.spec or { };
        resources = resourcesFor zoneName;
      })
      (lib.filterAttrs
        (_: resource:
          resource.type == "Guest"
          && (resource.spec.providerRef or null) == providerRef)
        (resourcesFor zoneName));

  providerFor = zoneName:
    if builtins.hasAttr "runtime-cloud-hypervisor" (resourcesFor zoneName)
      && (resourcesFor zoneName)."runtime-cloud-hypervisor".type == "Provider"
    then (resourcesFor zoneName)."runtime-cloud-hypervisor"
    else null;

  providerAssertions = zoneName:
    let
      provider = providerFor zoneName;
      resources = resourcesFor zoneName;
      c = if provider == null then { } else provider.spec.config or { };
      keys = [
        "controllerExecutionRef"
        "defaultVcpus"
        "defaultMemoryMb"
        "defaultMachineType"
        "watchdog"
        "adoptionWindow"
        "healthCheckInterval"
        "healthCheckTimeout"
        "healthCheckFailureThreshold"
        "startupDeadline"
      ];
      controller = c.controllerExecutionRef or null;
      parts = if builtins.isString controller
        then lib.splitString "/" controller
        else [ ];
      resolvesHost = builtins.isString controller
        && lib.length parts == 2
        && builtins.elemAt parts 0 == "Host"
        && builtins.hasAttr (builtins.elemAt parts 1) resources
        && (resources.${builtins.elemAt parts 1}).type == "Host";
      path = "d2b.zones.${zoneName}.resources.runtime-cloud-hypervisor.spec.config";
    in
    if provider == null
    then [ ]
    else [
      {
        assertion = lib.all (key: builtins.elem key keys) (lib.attrNames c);
        message = "${path} contains an unsupported runtime-cloud-hypervisor Provider field.";
      }
      {
        assertion = resolvesHost;
        message = "${path}.controllerExecutionRef must resolve to a same-Zone Host.";
      }
      {
        assertion = !(builtins.hasAttr "defaultVcpus" c)
          || (builtins.isInt c.defaultVcpus
            && c.defaultVcpus >= 1
            && c.defaultVcpus <= 1024);
        message = "${path}.defaultVcpus is out of bounds.";
      }
      {
        assertion = !(builtins.hasAttr "defaultMemoryMb" c)
          || (builtins.isInt c.defaultMemoryMb
            && c.defaultMemoryMb >= 128
            && c.defaultMemoryMb <= 524288);
        message = "${path}.defaultMemoryMb is out of bounds.";
      }
    ];

  controllerExecutionRef = zoneName:
    let provider = providerFor zoneName;
    in if provider == null
      then null
      else (provider.spec.config or { }).controllerExecutionRef or null;

  firstNetworkUsage = spec:
    let attachments = spec.networkAttachments or [ ];
    in if attachments == [ ]
      then null
      else {
        networkRef = (builtins.head attachments).networkRef;
        ports = [ ];
        allowEgress = true;
      };

  deviceUsage = spec:
    map
      (attachment: {
        deviceRef = attachment.deviceRef;
        access = if attachment.exclusive or false then "exclusive" else "shared";
        purpose = "runtime";
      })
      (spec.deviceAttachments or [ ]);

  processFor = row:
    let executionRef = controllerExecutionRef row.zoneName;
    in lib.optionalAttrs (executionRef != null) {
      type = "Process";
      metadata = {
        name = "cloud-hypervisor-${row.guestName}";
        zone = row.zoneName;
        ownerRef = "Guest/${row.guestName}";
      };
      spec = {
        providerRef = processProviderRef;
        inherit executionRef;
        domain = "system";
        processClass = "worker";
        template = "cloud-hypervisor-runner";
        desiredLifecycle = "running";
        networkUsage = firstNetworkUsage row.spec;
        deviceUsage = deviceUsage row.spec;
      } // lib.optionalAttrs (firstNetworkUsage row.spec == null) {
        networkUsage = null;
      };
    };

  processRows = lib.concatMap
    (zoneName:
      lib.filter (resource: resource != { })
        (map processFor (guestRowsFor zoneName)))
    (lib.sort lib.lessThan (lib.attrNames zones));

  processesByZone = lib.foldl'
    (result: resource:
      let
        zoneName = resource.metadata.zone or null;
        name = resource.metadata.name or null;
      in
      if zoneName == null || name == null
      then result
      else result // {
        ${zoneName} = (result.${zoneName} or { }) // {
          ${name} = resource;
        };
      })
    { }
    processRows;

  privateArtifact = {
    schemaVersion = 1;
    providerRef = providerRef;
    processRefs = map
      (resource: "Process/${resource.metadata.name}")
      processRows;
  };
  enabled = processRows != [ ];
in
{
  config = {
    assertions = lib.concatMap providerAssertions (lib.attrNames zones);
    d2b._resourceCompiler.providerProjectionRuntimeCloudHypervisor = {
      inherit enabled;
      inherit processesByZone;
      resourcesByZone = { };
      guestPatchesByZone = { };
      privateArtifact = privateArtifact;
    };
  };
}
