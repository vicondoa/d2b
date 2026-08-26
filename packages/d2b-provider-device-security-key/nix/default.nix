# Zone resource projection for Provider/device-security-key.
#
# The Device and semantic Service/Binding remain provider-neutral. This
# projection describes the Binding-owned Guest frontend Process without
# exposing hidraw paths, vsock addresses, or inherited file descriptors.
{ config, lib, ... }:

let
  cfg = config.d2b;
  providerRef = "Provider/device-security-key";
  zones = cfg.zones or { };

  resourcesFor = zoneName: zones.${zoneName}.resources or { };

  providerPresent = zoneName:
    builtins.hasAttr "device-security-key" (resourcesFor zoneName)
    && (resourcesFor zoneName).device-security-key.type == "Provider";

  processProviderRef = "Provider/system-systemd";

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
            resource.type
            == "security-key.d2bus.org.SecurityKeyBinding"
            && (resource.spec.providerRef or null) == providerRef)
          (resourcesFor zoneName));

  processFor = row:
    let
      target = row.spec.target or { };
      guestRef = target.guestRef or row.spec.guestRef or null;
      userRef = target.userRef or row.spec.userRef or null;
      processProvider = processProviderRef;
    in
    lib.optionalAttrs (guestRef != null) {
      type = "Process";
      metadata = {
        name = "security-key-${row.bindingName}";
        zone = row.zoneName;
        ownerRef =
          "security-key.d2bus.org.SecurityKeyBinding/${row.bindingName}";
      };
      spec = {
        providerRef = processProvider;
        executionRef = guestRef;
        domain = if userRef != null then "user" else "system";
        inherit userRef;
        processClass = "service";
        template = "security-key-frontend";
        desiredLifecycle = "running";
        deviceUsage = [ ];
        networkUsage = null;
      };
    };

  emittedBindingRowsForZone = zoneName:
    lib.filter
      (row: processFor row != {})
      (bindingRows zoneName);

  rows = lib.concatMap
    (zoneName:
      map processFor (emittedBindingRowsForZone zoneName))
    (lib.sort lib.lessThan (lib.attrNames zones));

  resourcesForZone = zoneName:
    lib.listToAttrs (map
      (row:
        let
          process = processFor row;
          guestRef = (row.spec.target or { }).guestRef or row.spec.guestRef or null;
        in lib.nameValuePair "security-key-${row.bindingName}" {
          type = "Endpoint";
          metadata = {
            name = "security-key-${row.bindingName}";
            zone = zoneName;
            ownerRef =
              "security-key.d2bus.org.SecurityKeyBinding/${row.bindingName}";
          };
          spec = {
            producerRef = "${process.type}/${process.metadata.name}";
            providerRef = providerRef;
            endpointClass = "device";
            transport = "opaque-carriage";
            purpose = "security-key.d2bus.org/ctaphid";
            serviceFingerprint = null;
            locality = "guest-local";
            visibility = "owner";
            attachmentPolicy = {
              supported = true;
              maxAttachments = 1;
            };
            consumerPolicy = {
              allowedSubjects = [ guestRef ];
              allowedProviderComponents = [ "device-security-key" ];
              allowedOperations = [ "resolve" "attach" ];
            };
            lifecyclePolicy = "recycle-with-producer";
          };
        })
      (emittedBindingRowsForZone zoneName));

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
  config.d2b._resourceCompiler.providerProjectionDeviceSecurityKey = {
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
    };
  };
}
