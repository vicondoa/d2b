# Zone resource projection for Provider/device-tpm.
#
# TPM state and socket details are resolved by the controller. The Nix
# projection carries only the Device-owned Process intents and typed Device
# reference; no Volume or filesystem locator is authored here.
{ config, lib, ... }:

let
  cfg = config.d2b;
  providerRef = "Provider/device-tpm";
  zones = cfg.zones or { };

  resourcesFor = zoneName: zones.${zoneName}.resources or { };

  providerFor = zoneName:
    let resources = resourcesFor zoneName;
    in if builtins.hasAttr "device-tpm" resources
      && resources.device-tpm.type == "Provider"
      then resources.device-tpm
      else null;

  providerExecutionRef = zoneName:
    let
      provider = providerFor zoneName;
      resources = resourcesFor zoneName;
      providerConfig =
        if provider == null then { } else provider.spec.config or { };
      executionRef = providerConfig.controllerExecutionRef or null;
      parts = if builtins.isString executionRef
        then lib.splitString "/" executionRef
        else [ ];
      hostName = if lib.length parts == 2
        && builtins.elemAt parts 0 == "Host"
        then builtins.elemAt parts 1
        else null;
    in if hostName != null
      && builtins.hasAttr hostName resources
      && (resources.${hostName}).type == "Host"
      then executionRef
      else null;

  processProviderRef = "Provider/system-minijail";

  deviceRows = zoneName:
    let executionRef = providerExecutionRef zoneName;
    in if executionRef == null
      then [ ]
      else lib.mapAttrsToList
        (deviceName: device: {
          inherit zoneName deviceName device executionRef;
        })
        (lib.filterAttrs
          (_: resource:
            resource.type == "Device"
            && (resource.spec.providerRef or null) == providerRef
            && lib.hasPrefix "Guest/" ((resource.metadata or { }).ownerRef or ""))
          (resourcesFor zoneName));

  processFor = row:
    let executionRef = row.executionRef;
    in {
      type = "Process";
      metadata = {
        name = "swtpm-${row.deviceName}";
        zone = row.zoneName;
        ownerRef = "Device/${row.deviceName}";
      };
      spec = {
        providerRef = processProviderRef;
        inherit executionRef;
        domain = "system";
        processClass = "worker";
        template = "swtpm-socket";
        desiredLifecycle = "running";
        deviceUsage = [ ];
        networkUsage = null;
      };
    };

  flushFor = row:
    let executionRef = row.executionRef;
    in {
      type = "EphemeralProcess";
      metadata = {
        name = "swtpm-flush-${row.deviceName}";
        zone = row.zoneName;
        ownerRef = "Device/${row.deviceName}";
      };
      spec = {
        providerRef = processProviderRef;
        inherit executionRef;
        domain = "system";
        processClass = "worker";
        template = "swtpm-init-flush";
        deviceUsage = [ ];
        networkUsage = null;
      };
    };

  rows = lib.concatMap
    (zoneName:
      lib.concatMap
        (row:
          lib.filter (resource: resource != { }) [
            (processFor row)
            (flushFor row)
          ])
        (deviceRows zoneName))
    (lib.sort lib.lessThan (lib.attrNames zones));

  endpointResourcesForZone = zoneName:
    lib.concatMap
      (row:
        let process = processFor row;
        in if process == { }
          then [ ]
          else [
        (lib.nameValuePair "tpm-${row.deviceName}" {
          type = "Endpoint";
          metadata = {
            name = "tpm-${row.deviceName}";
            zone = zoneName;
            ownerRef = "Device/${row.deviceName}";
          };
          spec = {
            producerRef = "${process.type}/${process.metadata.name}";
            providerRef = providerRef;
            endpointClass = "device";
            transport = "opaque-carriage";
            purpose = "device-tpm.d2bus.org/tpm";
            serviceFingerprint = null;
            locality = "host-local";
            visibility = "owner";
            attachmentPolicy = {
              supported = true;
              maxAttachments = 1;
            };
            consumerPolicy = {
              allowedSubjects = [ ];
              allowedProviderComponents = [ "runtime-cloud-hypervisor" ];
              allowedOperations = [ "resolve" "attach" ];
            };
            lifecyclePolicy = "recycle-with-producer";
          };
        })
        (lib.nameValuePair "tpm-ctrl-${row.deviceName}" {
          type = "Endpoint";
          metadata = {
            name = "tpm-ctrl-${row.deviceName}";
            zone = zoneName;
            ownerRef = "Device/${row.deviceName}";
          };
          spec = {
            producerRef = "${process.type}/${process.metadata.name}";
            providerRef = providerRef;
            endpointClass = "control";
            transport = "opaque-carriage";
            purpose = "device-tpm.d2bus.org/control";
            serviceFingerprint = null;
            locality = "host-local";
            visibility = "owner";
            attachmentPolicy = {
              supported = true;
              maxAttachments = 1;
            };
            consumerPolicy = {
              allowedSubjects = [ ];
              allowedProviderComponents = [ "device-tpm" ];
              allowedOperations = [ "resolve" "attach" ];
            };
            lifecyclePolicy = "recycle-with-producer";
          };
        })
      ])
      (deviceRows zoneName);

  resourcesForZone = zoneName:
    lib.listToAttrs (endpointResourcesForZone zoneName);

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
    rows;
in
{
  config.d2b._resourceCompiler.providerProjectionDeviceTpm = {
    enabled = rows != [ ];
    inherit processesByZone;
    resourcesByZone = lib.genAttrs (lib.attrNames zones) resourcesForZone;
    guestPatchesByZone = { };
    privateArtifact = {
      schemaVersion = 1;
      providerRef = providerRef;
      processRefs = map
        (resource: "${resource.type}/${resource.metadata.name}")
        rows;
      endpointRefs = lib.concatMap
        (zoneName: map
          (resource: "Endpoint/${resource.metadata.name}")
          (endpointResourcesForZone zoneName))
        (lib.attrNames zones);
    };
  };
}
