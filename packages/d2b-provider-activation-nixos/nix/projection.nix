# Zone resource projection for Provider/activation-nixos.
#
# A configured Guest with a system artifact receives one deterministic
# activation resource. The activation controller owns its ephemeral runner
# and generation state; no executable or store path enters this projection.
{ config, lib, ... }:

let
  cfg = config.d2b;
  providerRef = "Provider/activation-nixos";
  resourceType = "activation-nixos.d2bus.org.NixosGeneration";
  zones = cfg.zones or { };
  resourcesFor = zoneName: zones.${zoneName}.resources or { };

  providerPresent = zoneName:
    builtins.hasAttr "activation-nixos" (resourcesFor zoneName)
    && (resourcesFor zoneName).activation-nixos.type == "Provider";

  generationFor = zoneName: guestName: guest:
    let artifactId = (guest.spec or { }).systemArtifactId or null;
    in lib.optionalAttrs (providerPresent zoneName && artifactId != null) {
      type = resourceType;
      metadata = {
        name = "activation-${guestName}";
        ownerRef = "Guest/${guestName}";
      };
      spec = {
        providerRef = providerRef;
        executionRef = "Guest/${guestName}";
        systemArtifactId = artifactId;
        activationMode = "switch";
      };
    };

  resourcesForZone = zoneName:
    lib.foldl'
      (result: entry:
        let resource = generationFor zoneName entry.name entry.resource;
        in if resource == { }
          then result
          else result // { ${resource.metadata.name} = resource; })
      { }
      (lib.mapAttrsToList
        (name: resource: { inherit name resource; })
        (lib.filterAttrs (_: resource: resource.type == "Guest")
          (resourcesFor zoneName)));

  resourcesByZone = lib.genAttrs (lib.attrNames zones) resourcesForZone;
  generations = lib.concatMap lib.attrValues (lib.attrValues resourcesByZone);

  processesForZone = zoneName:
    lib.mapAttrs'
      (name: resource:
        lib.nameValuePair "activation-runner-${name}" {
          type = "EphemeralProcess";
          metadata = {
            name = "activation-runner-${name}";
            zone = zoneName;
            ownerRef = "${resourceType}/${name}";
          };
          spec = {
            providerRef = "Provider/system-systemd";
            executionRef = resource.spec.executionRef;
            domain = "system";
            processClass = "worker";
            template = "activation-nixos-runner";
            deviceUsage = [ ];
            networkUsage = null;
          };
        })
      (resourcesForZone zoneName);
in
{
  config.d2b._resourceCompiler.providerProjectionActivationNixos = {
    enabled = generations != [ ];
    processesByZone = lib.genAttrs (lib.attrNames zones) processesForZone;
    inherit resourcesByZone;
    guestPatchesByZone = { };
    privateArtifact = {
      schemaVersion = 1;
      providerRef = providerRef;
      generationRefs = map
        (resource: "${resourceType}/${resource.metadata.name}")
        generations;
      processRefs = lib.concatMap
        (zoneName: map
          (resource: "EphemeralProcess/${resource.metadata.name}")
          (lib.attrValues (processesForZone zoneName)))
        (lib.attrNames zones);
    };
  };
}
