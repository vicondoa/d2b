{ config, lib, ... }:

let
  cfg = config.d2b;

  sortNames = names: lib.sort lib.lessThan names;
  sortedAttrs = attrs:
    lib.listToAttrs (map
      (name: lib.nameValuePair name attrs.${name})
      (sortNames (lib.attrNames attrs)));
  declaredZones = sortedAttrs (cfg.zones or { });

  resourceIdentityLess = left: right:
    if left.type == right.type
    then lib.lessThan left.name right.name
    else lib.lessThan left.type right.type;

  zoneResourceIdentities = zoneName: zone:
    lib.sort resourceIdentityLess (lib.mapAttrsToList
      (resourceName: resource: {
        inherit zoneName;
        type = resource.type;
        name = resourceName;
        ref = "${resource.type}/${resourceName}";
        ownerRef = resource.metadata.ownerRef or null;
      })
      (zone.resources or { }));

  zoneRows = lib.mapAttrsToList
    (zoneName: zone: {
      name = zoneName;
      resources = zoneResourceIdentities zoneName zone;
    })
    declaredZones;

  zoneRowsByName = lib.listToAttrs (map
    (zone: lib.nameValuePair zone.name zone)
    zoneRows);

  zoneResourceRows = zoneName: zone:
    lib.mapAttrsToList
      (resourceName: resource: {
        inherit zoneName resourceName resource;
        ref = "${resource.type}/${resourceName}";
      })
      (zone.resources or { });

  resources = lib.concatMap
    (zoneName: zoneResourceRows zoneName declaredZones.${zoneName})
    (sortNames (lib.attrNames declaredZones));

  rowsOfType = resourceType:
    lib.filter (row: row.resource.type == resourceType) resources;

  namesOfTypeInZone = resourceType: zoneName:
    map (row: row.resourceName)
      (lib.sort
        (left: right: left.resourceName < right.resourceName)
        (lib.filter
          (row: row.zoneName == zoneName && row.resource.type == resourceType)
          resources));

  executionRefRows = lib.filter
    (row: row.resource.type == "Host" || row.resource.type == "Guest")
    resources;

  processRefsFor = executionRow:
    map (row: row.resourceName)
      (lib.sort
        (left: right: left.resourceName < right.resourceName)
        (lib.filter
          (row:
            row.zoneName == executionRow.zoneName
            && builtins.elem row.resource.type [ "Process" "EphemeralProcess" ]
            && (row.resource.spec.executionRef or null) == executionRow.ref)
          resources));

  executionIndex = lib.listToAttrs (map
    (row: lib.nameValuePair row.ref {
      zone = row.zoneName;
      providerRef = row.resource.spec.providerRef or null;
      processes = processRefsFor row;
    })
    executionRefRows);

  networkIndex = lib.listToAttrs (map
    (row:
      let
        attachedGuests = map (guest: guest.resourceName)
          (lib.sort
            (left: right: left.resourceName < right.resourceName)
            (lib.filter
              (guest:
                guest.zoneName == row.zoneName
                && guest.resource.type == "Guest"
                && lib.any
                  (attachment:
                    (attachment.networkRef or null) == row.ref)
                  (guest.resource.spec.networkAttachments or [ ]))
              resources));
      in
      lib.nameValuePair row.ref {
        zone = row.zoneName;
        lanSubnet = row.resource.spec.lanCidr or null;
        attachedGuests = attachedGuests;
      })
    (rowsOfType "Network"));

  closureIndex = lib.listToAttrs (map
    (row:
      let
        artifactName = "guestClosure-${row.zoneName}-${row.resourceName}";
        artifact = (cfg._guestClosureArtifacts or { }).${artifactName} or null;
      in
      lib.nameValuePair "Guest/${row.zoneName}/${row.resourceName}" {
        zone = row.zoneName;
        guest = row.resourceName;
        closureArtifact = row.resource.spec.systemArtifactId or null;
        closurePath =
          if artifact != null && artifact.installFileName != null
          then "/etc/d2b/${artifact.installFileName}"
          else "/etc/d2b/closures/zones/${row.zoneName}/${row.resourceName}.json";
        toplevel = if artifact == null then null else artifact.data.toplevel or null;
        storeView = if artifact == null then null else artifact.data.storeView or null;
      })
    (lib.filter
      (row: row.resource.type == "Guest"
        && (row.resource.spec.systemArtifactId or null) != null)
      resources));

  zoneSummary = zoneName: {
    hosts = namesOfTypeInZone "Host" zoneName;
    guests = namesOfTypeInZone "Guest" zoneName;
    networks = namesOfTypeInZone "Network" zoneName;
    providers = namesOfTypeInZone "Provider" zoneName;
  };

  indexData = {
    schemaVersion = "v1";
    zones = lib.listToAttrs (map
      (zoneName: lib.nameValuePair zoneName (zoneSummary zoneName))
      (sortNames (lib.attrNames declaredZones)));
    topology = cfg._resourceCompiler.zoneControl.allocatorTopology or null;
    inherit executionIndex networkIndex closureIndex;
  };

  index = {
    zones = {
      names = sortNames (lib.attrNames declaredZones);
      list = zoneRows;
      byName = zoneRowsByName;
      resourceIdentities = lib.concatMap (zone: zone.resources) zoneRows;
      topology = cfg._zoneCompiler.topology or { };
      control = cfg._resourceCompiler.zoneControl or { };
    };
  };
in
{
  imports = [
    ./options-artifacts.nix
    ./artifact-catalog.nix
    ./options-zones.nix
    ./options-zones-resources.nix
    ./generated/options-zones-Zone.nix
    ./generated/options-zones-ZoneLink.nix
    ./zone-resources.nix
    ./bundle-zones.nix
    ./resources-zones-processes.nix
    ../packages/d2b-provider-volume-local/nix/resources-zones-volumes.nix
    ./resources-device.nix
    ../packages/d2b-provider-volume-local/nix/resources-volume.nix
    ../packages/d2b-provider-network-local/nix/resources-network.nix
    ./options-resources.nix
    ./activation-nixos-cleanup.nix
  ];

  options.d2b._index = lib.mkOption {
    type = lib.types.attrs;
    default = { };
    internal = true;
    visible = false;
    description = "Internal deterministic index of Zone resources.";
  };

  config.d2b._index = index;

  config.d2b._bundle.extraArtifacts.index = {
    data = indexData;
    installFileName = "index.json";
    classification = "contractPrivateNonSecret";
    sensitivity = "nonSecret";
  };
}
