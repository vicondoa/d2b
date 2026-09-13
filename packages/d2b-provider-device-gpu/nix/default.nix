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

  # Declared worker sandbox posture. The private template binding pins the
  # crosvm executable and the seccomp profile; this block is the public half
  # the broker fences the launch plan against before it clones the worker,
  # and the resource compiler refuses a row whose sandbox disagrees with the
  # template's closed posture.
  #
  # umask 0007: every GPU worker binds a shared Unix socket its peer connects
  # to as a different uid, so the created socket must keep its group bits
  # (see RoleProfile::umask). The render-node worker declares no device bind:
  # the broker pre-opens the render node and passes it as an fd.
  gpuWorkerSandbox = {
    namespaceClasses = [ "mount" "pid" "ipc" "uts" "user" ];
    capabilityClasses = [ ];
    seccompClass = "strict";
    noNewPrivileges = true;
    startRoot = false;
    readOnlyRoot = true;
    environmentClass = "minimal";
    umask = "0007";
    oomScoreAdj = 0;
    userNamespace = { mappingClass = "process-principal-root"; };
  };

  # The video sidecar decodes into the GPU worker's DRI. It fences a pid
  # namespace and a DRI device bind, and never a user namespace: it is the
  # one Device worker whose grant is a bind rather than an fd. The same
  # sandbox serves both closed video templates - `video-worker` and the
  # NVIDIA-decode `video-worker-nvidia` - because the template alone decides
  # the posture's device-bind set.
  videoSandbox = {
    namespaceClasses = [ "mount" "pid" "ipc" "uts" ];
    capabilityClasses = [ ];
    seccompClass = "strict";
    noNewPrivileges = true;
    startRoot = false;
    readOnlyRoot = true;
    environmentClass = "minimal";
    umask = "0007";
    oomScoreAdj = 0;
    userNamespace = null;
  };

  # Declared worker restart ceiling. The launch of a Device worker names the
  # host-device grants the Provider resolved and the Wayland session facts the
  # site projected; the broker refuses a grant the host does not provide
  # (`device-bind-missing: <path>`). That refusal is not transient, so the
  # canonical unbounded `on-failure` policy would retry it forever. The
  # ceiling makes a persistent launch refusal terminal in the closed
  # vocabulary (`process-start-budget-exhausted`, refused, at
  # `reconcile/launch`) instead of an endless requeue. It is a
  # per-daemon-lifetime ceiling, not a window: the driver's `RestartBudget`
  # counts up and nothing consumes `resetAfter`
  # (`packages/d2bd/src/process_driver.rs`), so two restarts - however far
  # apart - exhaust it. The numeric defaults mirror
  # `nixos-modules/resources-zones-processes.nix` processDefaults; only
  # `maxRestarts` differs (canonical default: null, unbounded).
  workerRestartPolicy = {
    class = "on-failure";
    backoffBase = "1s";
    backoffMax = "60s";
    backoffMultiplierMilli = 2000;
    maxRestarts = 2;
    resetAfter = "300s";
  };

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
        sandbox = gpuWorkerSandbox;
        restartPolicy = workerRestartPolicy;
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
        template = if settings.videoNvidiaDecode or false
          then "video-worker-nvidia"
          else "video-worker";
        sandbox = videoSandbox;
        restartPolicy = workerRestartPolicy;
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
