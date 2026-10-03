# Zone resource projection for Provider/display-wayland.
#
# WaylandSession is the authored policy boundary. Its host proxy, Guest
# frontend, their private Endpoints, and the exact host compositor Endpoint the
# proxy is admitted to reach are typed child intents; the socket's locator and
# the display credentials remain private to the Provider runtime. A worker
# reaches an endpoint only through its admitted relationship, so the projected
# consumer policy names that one worker and nothing else.
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
      purpose = "display-wayland-cross-domain";
      serviceFingerprint = null;
      locality = "cross-domain";
      visibility = "owner";
      attachmentPolicy = {
        supported = true;
        maxAttachments = 1;
      };
      consumerPolicy = {
        # The one worker admitted to this exact socket.
        allowedSubjects =
          [ "Process/wayland-frontend-${row.sessionName}" ];
        allowedProviderComponents = [ "runtime-cloud-hypervisor" ];
        allowedOperations = [ "resolve" "attach" ];
      };
      lifecyclePolicy = "recycle-with-producer";
    };
  };

  # The exact host compositor socket the proxy may reach. It is declared as an
  # Endpoint of the session so the relationship that admits it names the socket
  # itself rather than a runtime directory or an inherited display name.
  compositorEndpointFor = row: {
    type = "Endpoint";
    metadata = {
      name = "wayland-compositor-${row.sessionName}";
      ownerRef =
        "display-wayland.d2bus.org.WaylandSession/${row.sessionName}";
    };
    spec = {
      producerRef = row.spec.hostRef;
      providerRef = providerRef;
      endpointClass = "transport";
      transport = "unix";
      purpose = row.spec.compositorDisplay or "display-compositor";
      serviceFingerprint = "display-wayland-compositor-r${
        row.spec.reconnectGeneration or 1
      }";
      locality = "cross-domain";
      visibility = "owner";
      attachmentPolicy = {
        supported = false;
        maxAttachments = 0;
      };
      consumerPolicy = {
        allowedSubjects = [ "Process/wayland-proxy-${row.sessionName}" ];
        allowedProviderComponents = [ ];
        allowedOperations = [ "resolve" ];
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
    lib.listToAttrs (lib.concatMap
      (row: [
        (lib.nameValuePair "wayland-${row.sessionName}" (endpointFor row))
        (lib.nameValuePair
          "wayland-compositor-${row.sessionName}"
          (compositorEndpointFor row))
      ])
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
