# Zone resource projection for Provider/device-usbip.
#
# USBIP attachment intent is carried by the provider-neutral Binding. The
# Provider contributes a Guest proxy Process and a private Endpoint identity;
# bus IDs, ports, addresses, and device paths stay out of the bundle.
{ config, lib, ... }:

let
  cfg = config.d2b;
  providerRef = "Provider/device-usbip";
  zones = cfg.zones or { };
  resourcesFor = zoneName: zones.${zoneName}.resources or { };

  providerPresent = zoneName:
    builtins.hasAttr "device-usbip" (resourcesFor zoneName)
    && (resourcesFor zoneName).device-usbip.type == "Provider";

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
            resource.type == "usb.d2bus.org.UsbBinding"
            && (resource.spec.providerRef or null) == providerRef)
          (resourcesFor zoneName));

  processFor = row:
    let guestRef = row.spec.guestRef or null;
    in lib.optionalAttrs (guestRef != null) {
      type = "Process";
      metadata = {
        name = "usbip-${row.bindingName}";
        zone = row.zoneName;
        ownerRef = "usb.d2bus.org.UsbBinding/${row.bindingName}";
      };
      spec = {
        providerRef = processProviderRef;
        executionRef = guestRef;
        domain = "system";
        processClass = "service";
        template = "usbip-guest-proxy";
        desiredLifecycle = "running";
        deviceUsage = [ ];
        networkUsage = null;
      };
    };

  processForZone = zoneName:
    lib.listToAttrs (lib.filter
      (entry: entry.value != { })
      (map
        (row: lib.nameValuePair "usbip-${row.bindingName}" (processFor row))
        (bindingRows zoneName)));

  resourcesForZone = zoneName:
    lib.listToAttrs (map
      (row:
        lib.nameValuePair "usbip-${row.bindingName}" {
          type = "Endpoint";
          metadata = {
            zone = zoneName;
            ownerRef = "usb.d2bus.org.UsbBinding/${row.bindingName}";
          };
          spec = {
            producerRef = "Process/usbip-${row.bindingName}";
            providerRef = providerRef;
            endpointClass = "transport";
            transport = "opaque-carriage";
            purpose = "usb.d2bus.org/guest-proxy";
            serviceFingerprint = null;
            locality = "guest-local";
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
        })
      (lib.filter
        (row: ((row.spec or { }).guestRef or null) != null)
        (bindingRows zoneName)));
in
{
  config.d2b._resourceCompiler.providerProjectionDeviceUsbip = {
    enabled = lib.any
      (zoneName: processForZone zoneName != { })
      (lib.attrNames zones);
    processesByZone = lib.genAttrs (lib.attrNames zones) processForZone;
    resourcesByZone = lib.genAttrs (lib.attrNames zones) resourcesForZone;
    guestPatchesByZone = { };
    privateArtifact = {
      schemaVersion = 1;
      providerRef = providerRef;
      endpointRefs = lib.concatMap
        (zoneName: map
          (row: "Endpoint/usbip-${row.bindingName}")
          (lib.filter (row: ((row.spec or { }).guestRef or null) != null)
            (bindingRows zoneName)))
        (lib.attrNames zones);
    };
  };
}
