# Zone resource projection for Provider/volume-virtiofs.
#
# The Provider owns the virtiofsd worker contract. This module emits only a
# Guest-owned Process intent; executable paths and socket locators stay in the
# signed Provider package and private launch ticket.
{ config, lib, ... }:

let
  cfg = config.d2b;
  providerRef = "Provider/volume-virtiofs";
  processProviderRef = "Provider/system-minijail";
  zones = cfg.zones or { };

  resourcesFor = zoneName: zones.${zoneName}.resources or { };

  providerExecutionRef = zoneName:
    let resources = resourcesFor zoneName;
    in if builtins.hasAttr "volume-virtiofs" resources
      && resources.volume-virtiofs.type == "Provider"
      then (resources.volume-virtiofs.spec.config or { }).controllerExecutionRef or null
      else null;

  virtiofsGuest = zoneName: guestName:
    lib.any
      (volume:
        lib.any
          (attachment:
            (attachment.executionRef or null) == "Guest/${guestName}"
            && (attachment.transport or null) == "virtiofs")
          ((volume.spec or { }).attachments or [ ]))
      (lib.attrValues (lib.filterAttrs
        (_: resource: resource.type == "Volume")
        (resourcesFor zoneName)));

  processFor = zoneName: guestName:
    let executionRef = providerExecutionRef zoneName;
    in lib.optionalAttrs (executionRef != null && virtiofsGuest zoneName guestName) {
      type = "Process";
      metadata = {
        name = "virtiofsd-${guestName}";
        zone = zoneName;
        ownerRef = "Guest/${guestName}";
      };
      spec = {
        providerRef = processProviderRef;
        inherit executionRef;
        domain = "system";
        processClass = "worker";
        template = "virtiofsd";
        desiredLifecycle = "running";
        deviceUsage = [ ];
        networkUsage = null;
      };
    };

  preflightFor = zoneName: guestName:
    let executionRef = providerExecutionRef zoneName;
    in lib.optionalAttrs (executionRef != null && virtiofsGuest zoneName guestName) {
      type = "EphemeralProcess";
      metadata = {
        name = "store-preflight-${guestName}";
        zone = zoneName;
        ownerRef = "Guest/${guestName}";
      };
      spec = {
        providerRef = processProviderRef;
        inherit executionRef;
        domain = "system";
        processClass = "worker";
        template = "store-virtiofs-preflight";
        deviceUsage = [ ];
        networkUsage = null;
      };
    };

  rows = lib.concatMap
    (zoneName:
      lib.concatMap
        (guestName:
          lib.filter (resource: resource != { }) [
            (preflightFor zoneName guestName)
            (processFor zoneName guestName)
          ])
        (lib.attrNames (lib.filterAttrs
          (_: resource: resource.type == "Guest")
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
  config.d2b._resourceCompiler.providerProjectionVolumeVirtiofs = {
    enabled = rows != [ ];
    inherit processesByZone;
    resourcesByZone = { };
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
