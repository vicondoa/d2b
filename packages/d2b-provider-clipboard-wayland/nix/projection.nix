# Zone resource projection for Provider/clipboard-wayland.
#
# Clipboard payloads stay on ComponentSession streams. Nix emits only the
# host/Guest process intents and the private bridge Endpoint identity.
{ config, lib, ... }:

let
  cfg = config.d2b;
  providerRef = "Provider/clipboard-wayland";
  zones = cfg.zones or { };
  resourcesFor = zoneName: zones.${zoneName}.resources or { };

  providerFor = zoneName:
    if builtins.hasAttr "clipboard-wayland" (resourcesFor zoneName)
      && (resourcesFor zoneName).clipboard-wayland.type == "Provider"
    then (resourcesFor zoneName).clipboard-wayland
    else null;

  providerAssertions = zoneName:
    let
      provider = providerFor zoneName;
      providerConfig =
        if provider == null then { } else provider.spec.config or { };
      allowed = [
        "hostExecutionRef"
        "hostUserRef"
        "displayWaylandRef"
        "pickerArtifactId"
        "policy"
        "caps"
        "ttl"
        "guestSources"
      ];
    in lib.optionals (provider != null) [{
      assertion = lib.all (key: builtins.elem key allowed)
        (lib.attrNames providerConfig);
      message = "d2b.zones.${zoneName}.resources.clipboard-wayland.spec.config contains an unsupported Provider field.";
    }];

  processProviderRef = "Provider/system-systemd";

  sourceRows = zoneName:
    let provider = providerFor zoneName;
    in if provider == null
      then [ ]
      else map
        (source: {
          inherit zoneName source;
          spec = provider.spec.config or { };
        })
        ((provider.spec.config or { }).guestSources or [ ]);

  hostProcessFor = zoneName:
    let
      provider = providerFor zoneName;
      providerConfig = if provider == null then { } else provider.spec.config or { };
      executionRef = providerConfig.hostExecutionRef or null;
      parts = if builtins.isString executionRef
        then lib.splitString "/" executionRef
        else [ ];
      host =
        if lib.length parts == 2
        && builtins.hasAttr (builtins.elemAt parts 1) (resourcesFor zoneName)
        then (resourcesFor zoneName).${builtins.elemAt parts 1}
        else { };
      userRef = providerConfig.hostUserRef or null;
      domain = if userRef != null
        && builtins.elem "user" ((host.spec or {}).allowedDomains or [ ])
        then "user"
        else "system";
    in lib.optionalAttrs (executionRef != null) {
      type = "Process";
      metadata = {
        name = "clipboard-host";
        ownerRef = providerRef;
      };
      spec = {
        providerRef = processProviderRef;
        inherit executionRef;
        inherit domain;
        userRef = if domain == "user" then userRef else null;
        processClass = "service";
        template = "clipd-host";
        desiredLifecycle = "running";
        deviceUsage = [ ];
        networkUsage = null;
      };
    };

  guestProcessFor = row:
    let
      source = if builtins.isAttrs row.source then row.source else { };
      guestRef = source.guestRef or null;
    in lib.optionalAttrs (guestRef != null) {
      type = "Process";
      metadata = {
        name = "clipboard-guest-${builtins.elemAt (lib.splitString "/" guestRef) 1}";
        ownerRef = providerRef;
      };
      spec = {
        providerRef = processProviderRef;
        executionRef = guestRef;
        domain = "system";
        processClass = "service";
        template = "clipboard-guest-source";
        desiredLifecycle = "running";
        deviceUsage = [ ];
        networkUsage = null;
      };
    };

  processesForZone = zoneName:
    let
      host = hostProcessFor zoneName;
      guests = lib.filter (resource: resource != { })
        (map guestProcessFor (sourceRows zoneName));
    in
    (lib.optionalAttrs (host != { }) { clipboard-host = host; })
    // lib.listToAttrs (map
      (resource: lib.nameValuePair resource.metadata.name resource)
      guests);

  resourcesForZone = zoneName:
    let host = hostProcessFor zoneName;
    in lib.optionalAttrs (host != { }) {
      "clipboard-bridge" = {
        type = "Endpoint";
        metadata = {
          ownerRef = providerRef;
        };
        spec = {
          producerRef = "Process/clipboard-host";
          providerRef = providerRef;
          endpointClass = "service";
          transport = "opaque-carriage";
          purpose = "clipboard-wayland.d2bus.org/bridge";
          serviceFingerprint = null;
          locality = "host-local";
          visibility = "provider";
          attachmentPolicy = {
            supported = true;
            maxAttachments = 16;
          };
          consumerPolicy = {
            allowedSubjects = [ ];
            allowedProviderComponents = [ "display-wayland" ];
            allowedOperations = [ "resolve" "attach" ];
          };
          lifecyclePolicy = "recycle-with-producer";
        };
      };
    };
in
{
  config = {
    assertions = lib.concatLists
      (map providerAssertions (lib.attrNames zones));
    d2b._resourceCompiler.providerProjectionClipboardWayland = {
      enabled = lib.any
        (zoneName: processesForZone zoneName != { })
        (lib.attrNames zones);
      processesByZone = lib.genAttrs (lib.attrNames zones) processesForZone;
      resourcesByZone = lib.genAttrs (lib.attrNames zones) resourcesForZone;
      guestPatchesByZone = { };
      privateArtifact = {
        schemaVersion = 1;
        providerRef = providerRef;
        endpointRef = "Endpoint/clipboard-bridge";
      };
    };
  };
}
