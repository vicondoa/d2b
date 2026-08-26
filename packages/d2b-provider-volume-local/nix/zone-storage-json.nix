{ config, lib, ... }:

let
  cfg = config.d2b;
  identity = import ../../../nixos-modules/resources-bundle.nix { inherit lib; };
  enabledEnvNames = builtins.attrNames (lib.filterAttrs (_: env: env.enable) cfg.envs);
  declaredZoneNames = builtins.attrNames (cfg._zoneCompiler.topology or { });
  # The compiler topology is the authoritative declared-Zone index. Keep
  # enabled legacy environments as compatibility rows, but do not derive
  # storage membership from VM placement.
  zoneNames = lib.unique
    ([ cfg._zoneCompiler.localRoot ] ++ enabledEnvNames ++ declaredZoneNames);

  storageRow = zoneName: {
    identity = {
      zoneUid = identity.stableUid "d2b:v3:zone-uid" zoneName;
      storeUid = identity.stableUid "d2b:v3:store-uid" zoneName;
      storeEpoch = 1;
    };
    zoneStoreId = "zone-store-${zoneName}";
    storageOwnerPrincipal = "d2b-zonert";
    parentDirectoryId = "zone-store-parent-${zoneName}";
    ownership = {
      owner = "d2b-zonert";
      group = "d2b-zonert";
      mode = "0640";
      linkCount = 1;
    };
    auxiliaryDirectories = {
      audit = {
        directoryId = "zone-store-audit-${zoneName}";
        owner = "d2bd";
        group = "d2bd";
        mode = "0700";
        repairOwner = "privileged-broker";
      };
      telemetry = {
        directoryId = "zone-store-telemetry-${zoneName}";
        owner = "d2bd";
        group = "d2bd";
        mode = "0700";
        repairOwner = "privileged-broker";
      };
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

  zoneStorageArtifacts = lib.listToAttrs (map
    (zoneName: lib.nameValuePair "zoneStorage-${zoneName}" {
        data = storageRow zoneName;
        installFileName = "zones/${zoneName}/storage.json";
        classification = "contractPrivateNonSecret";
        sensitivity = "nonSecret";
      })
    zoneNames);
in
{
  config = lib.mkIf (cfg.daemonExperimental.enable || cfg.zones != { }) {
    d2b._bundle.extraArtifacts = zoneStorageArtifacts;
  };
}
