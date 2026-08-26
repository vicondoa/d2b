# Zone resource projection for Provider/audio-pipewire.
#
# AudioBinding remains the durable operator intent. The two target-local
# Process children are expressed here as typed intents; PipeWire handles and
# all executable details stay in the signed Provider implementation.
{ config, lib, ... }:

let
  cfg = config.d2b;
  providerRef = "Provider/audio-pipewire";
  zones = cfg.zones or { };

  resourcesFor = zoneName: zones.${zoneName}.resources or { };

  providerFor = zoneName:
    let resources = resourcesFor zoneName;
    in if builtins.hasAttr "audio-pipewire" resources
      && resources.audio-pipewire.type == "Provider"
      then resources.audio-pipewire
      else null;

  providerPresent = zoneName: providerFor zoneName != null;

  providerAssertions = zoneName:
    let
      provider = providerFor zoneName;
      settings = if provider == null then { } else provider.spec.config or { };
      captureAlias = settings.captureAlias or null;
      resources = resourcesFor zoneName;
      resolvesHost = value:
        let parts = if builtins.isString value
          then lib.splitString "/" value
          else [ ];
        in builtins.isString value
          && lib.length parts == 2
          && builtins.elemAt parts 0 == "Host"
          && builtins.hasAttr (builtins.elemAt parts 1) resources
          && (resources.${builtins.elemAt parts 1}).type == "Host";
      path = "d2b.zones.${zoneName}.resources.audio-pipewire.spec.config";
    in lib.optionals (provider != null) [
      {
        assertion = lib.all (key:
          builtins.elem key [
            "captureAlias"
            "hostExecutionRef"
            "controllerExecutionRef"
          ])
          (lib.attrNames settings);
        message = "${path} contains an unsupported audio-pipewire Provider field.";
      }
      {
        assertion = (settings.hostExecutionRef or null) == null
          || resolvesHost settings.hostExecutionRef;
        message = "${path}.hostExecutionRef must resolve to a same-Zone Host.";
      }
      {
        assertion = (settings.controllerExecutionRef or null) == null
          || resolvesHost settings.controllerExecutionRef;
        message = "${path}.controllerExecutionRef must resolve to a same-Zone Host.";
      }
      {
        assertion = captureAlias == null
          || (builtins.isString captureAlias
            && builtins.stringLength captureAlias <= 64
            && builtins.match "^[a-z][a-z0-9-]*$" captureAlias != null);
        message = "${path}.captureAlias must be null or a bounded named PipeWire alias.";
      }
    ];

  hostExecutionRef = zoneName:
    let
      provider = providerFor zoneName;
      resources = resourcesFor zoneName;
    in if provider == null
      then null
      else
        let
          c = provider.spec.config or { };
          executionRef = c.hostExecutionRef or null;
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

  processProviderRef = placement:
    if placement == "host"
    then "Provider/system-minijail"
    else "Provider/system-systemd";

  bindingRows = zoneName:
    if !(providerPresent zoneName)
    then [ ]
    else lib.mapAttrsToList
      (bindingName: binding: {
          inherit zoneName bindingName binding;
          spec = binding.spec or { };
        })
        (lib.filterAttrs
          (_: resource:
            resource.type == "audio.d2bus.org.AudioBinding"
            && (resource.spec.providerRef or null) == providerRef)
          (resourcesFor zoneName));

  hostProcessFor = row:
    let executionRef = hostExecutionRef row.zoneName;
    in lib.optionalAttrs (executionRef != null) {
      type = "Process";
      metadata = {
        name = "audio-host-${row.bindingName}";
        zone = row.zoneName;
        ownerRef = "audio.d2bus.org.AudioBinding/${row.bindingName}";
      };
      spec = {
        providerRef = processProviderRef "host";
        inherit executionRef;
        domain = "system";
        processClass = "worker";
        template = "vhost-user-sound-worker";
        desiredLifecycle = "running";
        deviceUsage = [ ];
        networkUsage = null;
      };
    };

  guestProcessFor = row:
    let
      targetRef = row.spec.targetRef or null;
      guestUsers = row.spec.guestUsers or [ ];
      userRef = if guestUsers == [ ] then null else builtins.head guestUsers;
    in lib.optionalAttrs (targetRef != null) {
      type = "Process";
      metadata = {
        name = "audio-guest-${row.bindingName}";
        zone = row.zoneName;
        ownerRef = "audio.d2bus.org.AudioBinding/${row.bindingName}";
      };
      spec = {
        providerRef = processProviderRef "guest";
        executionRef = targetRef;
        domain = if userRef == null then "system" else "user";
        inherit userRef;
        processClass = "service";
        template = "guest-audio-agent";
        desiredLifecycle = "running";
        deviceUsage = [ ];
        networkUsage = null;
      };
    };

  endpointFor = row: placement:
    let
      host = placement == "host";
      executionRef = if host
        then hostExecutionRef row.zoneName
        else row.spec.targetRef or null;
      processName = if host
        then "audio-host-${row.bindingName}"
        else "audio-guest-${row.bindingName}";
      endpointName = if host
        then "audio-host-${row.bindingName}"
        else "audio-guest-${row.bindingName}";
    in lib.optionalAttrs (executionRef != null) {
      type = "Endpoint";
      metadata = {
        name = endpointName;
        zone = row.zoneName;
        ownerRef = "audio.d2bus.org.AudioBinding/${row.bindingName}";
      };
      spec = {
        producerRef = "Process/${processName}";
        providerRef = providerRef;
        endpointClass = "service";
        transport = "opaque-carriage";
        purpose = if host
          then "audio.d2bus.org/host-worker"
          else "audio.d2bus.org/guest-agent";
        serviceFingerprint = null;
        locality = if host then "host-local" else "guest-local";
        visibility = "owner";
        attachmentPolicy = {
          supported = true;
          maxAttachments = 1;
        };
        consumerPolicy = {
          allowedSubjects = lib.optional (!host) row.spec.targetRef;
          allowedProviderComponents = [ "audio-pipewire" ];
          allowedOperations = [ "resolve" "attach" ];
        };
        lifecyclePolicy = "recycle-with-producer";
      };
    };

  rows = lib.concatMap
    (zoneName:
      lib.concatMap
        (row:
          lib.filter (resource: resource != { }) [
            (hostProcessFor row)
            (guestProcessFor row)
          ])
        (bindingRows zoneName))
    (lib.sort lib.lessThan (lib.attrNames zones));

  processesByZone = lib.foldl'
    (result: resource:
      let
        zoneName = resource.metadata.zone;
        name = resource.metadata.name;
      in result // {
        ${zoneName} = (result.${zoneName} or { }) // {
          ${name} = resource;
        };
      })
    { }
    rows;

  resourcesForZone = zoneName:
    let
      bindings = bindingRows zoneName;
      endpoints = lib.concatMap
        (row: [
          (endpointFor row "host")
          (endpointFor row "guest")
        ])
        bindings;
    in lib.listToAttrs (lib.filter
      (entry: entry.value != { })
      (map
        (resource: lib.nameValuePair resource.metadata.name resource)
        endpoints));
in
{
  config = {
    assertions = lib.concatLists
      (map providerAssertions (lib.attrNames zones));
    d2b._resourceCompiler.providerProjectionAudioPipewire = {
      enabled = rows != [ ];
      inherit processesByZone;
      resourcesByZone = lib.genAttrs (lib.attrNames zones) resourcesForZone;
      guestPatchesByZone = { };
      privateArtifact = {
        schemaVersion = 1;
        providerRef = providerRef;
        processRefs = map (resource: "Process/${resource.metadata.name}") rows;
      };
    };
  };
}
