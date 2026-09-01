# Zone resource projection for Provider/device-gpu.
#
# Device remains the provider-neutral allocation resource. The Provider adds
# only its signed Process children; host paths, device nodes, and argv are
# resolved privately by the controller and broker.
{ config, lib, ... }:

let
  cfg = config.d2b;
  providerRef = "Provider/device-gpu";
  zones = cfg.zones or { };

  resourcesFor = zoneName: zones.${zoneName}.resources or { };

  providerExecutionRef = zoneName:
    let resources = resourcesFor zoneName;
    in if builtins.hasAttr "device-gpu" resources
      && resources.device-gpu.type == "Provider"
      then (resources.device-gpu.spec.config or { }).controllerExecutionRef or null
      else null;

  processProviderRef = "Provider/system-minijail";

  ownerGuest = device:
    let owner = (device.metadata or { }).ownerRef or null;
    in if builtins.isString owner && lib.hasPrefix "Guest/" owner then owner else null;

  processFor = zoneName: deviceName: device:
    let
      settings = ((device.spec or { }).provider or { }).settings or { };
      executionRef = providerExecutionRef zoneName;
      ownerRef = ownerGuest device;
      renderNodeOnly = settings.renderNodeOnly or false;
    in lib.optionalAttrs (executionRef != null && ownerRef != null) {
      type = "Process";
      metadata = {
        name = "gpu-${deviceName}";
        zone = zoneName;
        ownerRef = "Device/${deviceName}";
      };
      spec = {
        providerRef = processProviderRef;
        inherit executionRef;
        domain = "system";
        processClass = "worker";
        template = if renderNodeOnly then "gpu-render-node" else "gpu-worker";
        desiredLifecycle = "running";
        deviceUsage = [{
          deviceRef = "Device/${deviceName}";
          access = if ((device.spec or { }).arbitration or "exclusive") == "shared"
            then "shared"
            else "exclusive";
          purpose = "gpu-virtio";
        }];
        networkUsage = null;
      };
    };

  videoFor = zoneName: deviceName: device:
    let
      settings = ((device.spec or { }).provider or { }).settings or { };
      executionRef = providerExecutionRef zoneName;
      ownerRef = ownerGuest device;
    in lib.optionalAttrs (
      executionRef != null
      && ownerRef != null
      && (settings.videoSidecar or false)
    ) {
      type = "Process";
      metadata = {
        name = "video-${deviceName}";
        zone = zoneName;
        ownerRef = "Device/${deviceName}";
      };
      spec = {
        providerRef = processProviderRef;
        inherit executionRef;
        domain = "system";
        processClass = "worker";
        template = "video-worker";
        desiredLifecycle = "running";
        deviceUsage = [{
          deviceRef = "Device/${deviceName}";
          access = "shared";
          purpose = "video-decode";
        }];
        networkUsage = null;
      };
    };

  rows = lib.concatMap
    (zoneName:
      lib.concatMap
        (row:
          let
            gpu = processFor zoneName row.deviceName row.device;
            video = videoFor zoneName row.deviceName row.device;
          in lib.filter (resource: resource != { }) [ gpu video ])
        (lib.mapAttrsToList
          (deviceName: device: { inherit deviceName device; })
          (lib.filterAttrs
          (_: resource:
            resource.type == "Device"
            && (resource.spec.providerRef or null) == providerRef)
          (resourcesFor zoneName))))
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
    rows;
in
{
  config.d2b._resourceCompiler.providerProjectionDeviceGpu = {
    enabled = rows != [ ];
    inherit processesByZone;
    resourcesByZone = { };
    guestPatchesByZone = { };
    privateArtifact = {
      schemaVersion = 1;
      providerRef = providerRef;
      processRefs = map (resource: "Process/${resource.metadata.name}") rows;
    };
  };
}
