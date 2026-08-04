{ lib, ... }@ctx:

let
  h = import ../helpers/bundle-artifacts.nix ctx;
  inherit (h)
    cfgCompat
    cfgDaemon
    defaultedArtifact
    expectedZoneStorageData
    installedZoneStorage
    installedZoneStorageEnv
    zoneStoragePrincipal
    zoneStorageArtifact
    zoneStorageEnvArtifact
    ;
in
{
  "bundle-artifacts/storage-json-central-etc" = {
    expr = {
      storage = {
        mode = cfgDaemon.environment.etc."d2b/storage.json".mode;
        user = cfgDaemon.environment.etc."d2b/storage.json".user;
        group = cfgDaemon.environment.etc."d2b/storage.json".group;
      };
      sync = {
        mode = cfgDaemon.environment.etc."d2b/sync.json".mode;
        user = cfgDaemon.environment.etc."d2b/sync.json".user;
        group = cfgDaemon.environment.etc."d2b/sync.json".group;
      };
    };
    expected = {
      storage = {
        mode = "0640";
        user = "root";
        group = "d2bd";
      };
      sync = {
        mode = "0640";
        user = "root";
        group = "d2bd";
      };
    };
  };

  "bundle-artifacts/default-json-text" = {
    expr = cfgDaemon.d2b._bundle.extraArtifacts.defaultedJson.jsonText;
    expected = builtins.toJSON cfgDaemon.d2b._bundle.extraArtifacts.defaultedJson.data;
  };

  "bundle-artifacts/default-derivation-name" = {
    expr = lib.hasSuffix "-d2b-defaulted.json"
      (h.storePathString cfgDaemon.d2b._bundle.extraArtifacts.defaultedJson.path);
    expected = true;
  };

  "bundle-artifacts/defaulted-central-etc" = {
    expr = {
      sourceHasDefaultName = lib.hasSuffix "-d2b-defaulted.json"
        (h.storePathString cfgDaemon.environment.etc."d2b/defaulted.json".source);
      group = cfgDaemon.environment.etc."d2b/defaulted.json".group;
    };
    expected = {
      sourceHasDefaultName = true;
      group = "d2bd";
    };
  };

  "zone-storage-contract/rendered-artifact-and-etc-wiring" = {
    expr = {
      data = zoneStorageArtifact.data;
      renderedData = builtins.fromJSON zoneStorageArtifact.jsonText;
      installFileName = zoneStorageArtifact.installFileName;
      classification = zoneStorageArtifact.classification;
      etc = {
        sourceMatches = installedZoneStorage.source == zoneStorageArtifact.path;
        inherit (installedZoneStorage) mode user group;
      };
    };
    expected = {
      data = expectedZoneStorageData;
      renderedData = expectedZoneStorageData;
      installFileName = "zones/local-root/storage.json";
      classification = "contractPrivateNonSecret";
      etc = {
        sourceMatches = true;
        mode = "0640";
        user = "root";
        group = "d2bd";
      };
    };
  };

  "zone-storage-contract/env-storage-row-and-etc-wiring" = {
    expr = {
      data = zoneStorageEnvArtifact.data;
      installFileName = zoneStorageEnvArtifact.installFileName;
      sourceMatches = installedZoneStorageEnv.source == zoneStorageEnvArtifact.path;
      inherit (installedZoneStorageEnv) mode user group;
    };
    expected = {
      data = {
        zoneStoreId = "zone-store-work";
        storageOwnerPrincipal = "d2b-zonert";
        parentDirectoryId = "zone-store-parent-work";
        ownership = {
          owner = "d2b-zonert";
          group = "d2b-zonert";
          mode = "0640";
          linkCount = 1;
        };
        filesystem = "regular-file-anchored-fd-relative-no-follow";
        locking = "ofd-close-on-exec";
        marker.identityMarkerId = "zone-store-marker-work";
        replacementDetection = "fail-closed-on-missing-replaced-or-identity-mismatch";
        fsync = "database-and-parent-directory";
        publication = {
          descriptor = "owned-descriptor-close-on-exec-verified-before-concurrency";
          replacement = "atomic-rename-retain-prior-quarantine-ambiguity";
        };
      };
      installFileName = "zones/work/storage.json";
      sourceMatches = true;
      mode = "0640";
      user = "root";
      group = "d2bd";
    };
  };

  "zone-storage-contract/principal-is-installed" = {
    expr = zoneStoragePrincipal;
    expected = {
      userExists = true;
      groupExists = true;
      userGroup = "d2b-zonert";
    };
  };

  "bundle-artifacts/root-group-compat" = {
    expr = cfgCompat.environment.etc."d2b/defaulted.json".group;
    expected = "root";
  };

  "bundle-artifacts/nested-tables-are-not-artifact-rows" = {
    expr =
      !(builtins.elem "data" (builtins.attrNames cfgDaemon.d2b._bundle.closures))
      && !(builtins.elem "installFileName"
        (builtins.attrNames cfgDaemon.d2b._bundle.minijailProfiles));
    expected = true;
  };

  "bundle-artifacts/nested-table-field-name-collisions-are-not-rows" = {
    expr =
      let
        cfg = (h.mkEvalStub [
          h.base
          h.defaultedArtifact
          ({ ... }: {
            d2b._bundle.closures = {
              data = { vm = "data"; path = "/nix/store/example"; };
              path = { vm = "path"; path = "/nix/store/example"; };
              installFileName = { vm = "installFileName"; path = "/nix/store/example"; };
              enableEtc = { vm = "enableEtc"; path = "/nix/store/example"; };
            };
          })
        ]).config;
      in {
        closureKeys = lib.sort lib.lessThan (builtins.attrNames cfg.d2b._bundle.closures);
        defaultedInstalled = cfg.environment.etc ? "d2b/defaulted.json";
        collisionInstalled =
          (cfg.environment.etc ? "d2b/data")
          || (cfg.environment.etc ? "d2b/path")
          || (cfg.environment.etc ? "d2b/installFileName")
          || (cfg.environment.etc ? "d2b/enableEtc");
      };
    expected = {
      closureKeys = [ "data" "enableEtc" "installFileName" "path" "sys-work-net" ];
      defaultedInstalled = true;
      collisionInstalled = false;
    };
  };

  "bundle-artifacts/extra-artifact-reserved-name-collision-asserts" = {
    expr =
      let
        cfg = (h.mkEvalStub [
          h.base
          h.defaultedArtifact
          ({ ... }: {
            d2b._bundle.extraArtifacts.bundle = {
              data = { value = "bad"; };
              installFileName = "extra-bundle.json";
            };
          })
        ]).config;
      in lib.any
        (a:
          !a.assertion
          && lib.hasInfix "extraArtifacts collide with reserved artifact names"
            a.message
          && lib.hasInfix "bundle" a.message)
        cfg.assertions;
    expected = true;
  };

  "bundle-artifacts/extra-artifact-install-path-collision-conflicts" = {
    expr =
      let
        cfg = (h.mkEvalStub [
          h.base
          h.defaultedArtifact
          ({ ... }: {
            d2b._bundle.extraArtifacts.alsoDefaulted = {
              data = { value = "bad"; };
              installFileName = "defaulted.json";
            };
          })
        ]).config;
      in cfg.environment.etc."d2b/defaulted.json";
    expectedError = { };
  };
}
