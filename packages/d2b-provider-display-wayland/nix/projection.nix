# Zone resource projection for Provider/display-wayland.
#
# WaylandSession is the authored policy boundary. Its host proxy, Guest
# frontend, and private Endpoint are typed child intents; compositor sockets
# and display credentials remain private to the Provider runtime.
{ config, lib, ... }:

let
  cfg = config.d2b;
  providerRef = "Provider/display-wayland";
  zones = cfg.zones or { };

  resourcesFor = zoneName: zones.${zoneName}.resources or { };
  providerPresent = zoneName:
    builtins.hasAttr "display-wayland" (resourcesFor zoneName)
    && (resourcesFor zoneName).display-wayland.type == "Provider";

  providerAssertions = zoneName:
    let
      provider = if providerPresent zoneName
        then (resourcesFor zoneName).display-wayland
        else null;
      providerConfig =
        if provider == null then { } else provider.spec.config or { };
      runtimePolicy = providerConfig.runtimeVolumePolicyId or null;
    in lib.optionals (provider != null) [
      {
        assertion = lib.all
          (key: builtins.elem key [ "principalPoolSize" "runtimeVolumePolicyId" ])
          (lib.attrNames providerConfig);
        message = "d2b.zones.${zoneName}.resources.display-wayland.spec.config contains an unsupported Provider field.";
      }
      {
        assertion = !(builtins.hasAttr "principalPoolSize" providerConfig)
          || (builtins.isInt providerConfig.principalPoolSize
            && providerConfig.principalPoolSize >= 1
            && providerConfig.principalPoolSize <= 32);
        message = "d2b.zones.${zoneName}.resources.display-wayland.spec.config.principalPoolSize is out of bounds.";
      }
      {
        assertion = runtimePolicy == null
          || (builtins.isString runtimePolicy
            && builtins.stringLength runtimePolicy >= 1
            && builtins.stringLength runtimePolicy <= 63);
        message = "d2b.zones.${zoneName}.resources.display-wayland.spec.config.runtimeVolumePolicyId must be a bounded policy ID.";
      }
    ];

  processProviderRef = placement:
    if placement == "host"
    then "Provider/system-minijail"
    else "Provider/system-systemd";

  sessionRows = zoneName:
    if !(providerPresent zoneName)
    then [ ]
    else lib.mapAttrsToList
      (sessionName: session: {
        inherit zoneName sessionName session;
        spec = session.spec or { };
      })
      (lib.filterAttrs
        (sessionName: session:
          session.type == "display-wayland.d2bus.org.WaylandSession"
          && sessionName != "")
        (resourcesFor zoneName));

  hostProcessFor = row: {
    type = "Process";
    metadata = {
      name = "wayland-proxy-${row.sessionName}";
      ownerRef =
        "display-wayland.d2bus.org.WaylandSession/${row.sessionName}";
    };
    spec = {
      providerRef = processProviderRef "host";
      executionRef = row.spec.hostRef;
      domain = "system";
      processClass = "service";
      template = "wayland-proxy-worker";
      desiredLifecycle = "running";
      deviceUsage = [ ];
      networkUsage = null;
    };
  };

  guestProcessFor = row: {
    type = "Process";
    metadata = {
      name = "wayland-frontend-${row.sessionName}";
      ownerRef =
        "display-wayland.d2bus.org.WaylandSession/${row.sessionName}";
    };
    spec = {
      providerRef = processProviderRef "guest";
      executionRef = row.spec.guestRef;
      domain = "system";
      processClass = "service";
      template = "wayland-frontend-worker";
      desiredLifecycle = "running";
      deviceUsage = [ ];
      networkUsage = null;
    };
  };

  endpointFor = row: {
    type = "Endpoint";
    metadata = {
      name = "wayland-${row.sessionName}";
      ownerRef =
        "display-wayland.d2bus.org.WaylandSession/${row.sessionName}";
    };
    spec = {
      producerRef = "Process/wayland-proxy-${row.sessionName}";
      providerRef = providerRef;
      endpointClass = "transport";
      transport = "opaque-carriage";
      purpose = "display-wayland.d2bus.org/cross-domain";
      serviceFingerprint = null;
      locality = "cross-domain";
      visibility = "owner";
      attachmentPolicy = {
        supported = true;
        maxAttachments = 1;
      };
      consumerPolicy = {
        allowedSubjects = [ row.spec.guestRef ];
        allowedProviderComponents = [ "runtime-cloud-hypervisor" ];
        allowedOperations = [ "resolve" "attach" ];
      };
      lifecyclePolicy = "recycle-with-producer";
    };
  };

  processesForZone = zoneName:
    lib.foldl'
      (result: row:
        let
          host = hostProcessFor row;
          guest = guestProcessFor row;
        in result // {
          "wayland-proxy-${row.sessionName}" = host;
          "wayland-frontend-${row.sessionName}" = guest;
        })
      { }
      (sessionRows zoneName);

  resourcesForZone = zoneName:
    lib.listToAttrs (map
      (row:
        lib.nameValuePair "wayland-${row.sessionName}" (endpointFor row))
      (sessionRows zoneName));

  processesByZone = lib.genAttrs
    (lib.attrNames zones)
    processesForZone;

  resourcesByZone = lib.genAttrs
    (lib.attrNames zones)
    resourcesForZone;

  rows = lib.concatMap
    (zoneName: lib.attrValues (processesForZone zoneName))
    (lib.attrNames zones);

  resources = lib.concatMap
    (zoneName: lib.attrValues (resourcesForZone zoneName))
    (lib.attrNames zones);
in
{
  config = {
    assertions = lib.concatLists
      (map providerAssertions (lib.attrNames zones));
    d2b._resourceCompiler.providerProjectionDisplayWayland = {
      enabled = lib.any (zoneName: sessionRows zoneName != [ ])
        (lib.attrNames zones);
      inherit processesByZone resourcesByZone;
      guestPatchesByZone = { };
      privateArtifact = {
        schemaVersion = 1;
        providerRef = providerRef;
        processRefs = map (resource: "Process/${resource.metadata.name}") rows;
        endpointRefs = map (resource: "Endpoint/${resource.metadata.name}") resources;
      };
    };
  };
}
