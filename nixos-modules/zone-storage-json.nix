{ config, lib, ... }:

let
  cfg = config.d2b;

  storageRow = zoneName: {
    zoneStoreId = "zone-store-${zoneName}";
    storageOwnerPrincipal = "d2b-zonert";
    parentDirectoryId = "zone-store-parent-${zoneName}";
    ownership = {
      owner = "d2b-zonert";
      group = "d2b-zonert";
      mode = "0640";
      linkCount = 1;
    };
    filesystem = "regular-file-anchored-fd-relative-no-follow";
    locking = "ofd-close-on-exec";
    marker.identityMarkerId = "zone-store-marker-${zoneName}";
    replacementDetection = "fail-closed-on-missing-replaced-or-identity-mismatch";
    fsync = "database-and-parent-directory";
    publication = {
      descriptor = "owned-descriptor-close-on-exec-verified-before-concurrency";
      replacement = "atomic-rename-retain-prior-quarantine-ambiguity";
    };
  };

  zoneStorageArtifacts = lib.mapAttrs'
    (zoneName: _: lib.nameValuePair "zoneStorage-${zoneName}" {
        data = storageRow zoneName;
        installFileName = "zones/${zoneName}/storage.json";
        classification = "contractPrivateNonSecret";
        sensitivity = "nonSecret";
      })
    cfg.zones;
in
{
  config = lib.mkIf (cfg.zones != { }) {
    d2b._bundle.extraArtifacts = zoneStorageArtifacts;
  };
}
