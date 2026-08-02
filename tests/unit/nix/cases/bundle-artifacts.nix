{ mkEval, lib, pkgs, system, ... }:

let
  base = { lib, ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
    };
    d2b.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
  };

  defaultedArtifact = {
    d2b._bundle.extraArtifacts.defaultedJson = {
      data = {
        schemaVersion = "test";
        value = 1;
      };
      installFileName = "defaulted.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };
  };

  cfgDaemon = (mkEval [ base defaultedArtifact ({ ... }: {
    d2b.daemonExperimental.enable = true;
  }) ]).config;

  cfgCompat = (mkEval [ base defaultedArtifact ({ lib, ... }: {
    d2b.daemonExperimental.enable = lib.mkForce false;
  }) ]).config;

  zoneStorageCfg = (mkEval [ base ({ ... }: {
    d2b.daemonExperimental.enable = true;
    d2b.zones.local-root = { };
  }) ]).config;
  zoneStorageArtifact =
    zoneStorageCfg.d2b._bundle.extraArtifacts."zoneStorage-local-root";
  installedZoneStorage =
    zoneStorageCfg.environment.etc."d2b/zones/local-root/storage.json";
  expectedZoneStorageData = {
    zoneStoreId = "zone-store-local-root";
    storageOwnerPrincipal = "d2b-zonert";
    parentDirectoryId = "zone-store-parent-local-root";
    ownership = {
      owner = "d2b-zonert";
      group = "d2b-zonert";
      mode = "0640";
      linkCount = 1;
    };
    filesystem = "regular-file-anchored-fd-relative-no-follow";
    locking = "ofd-close-on-exec";
    marker.identityMarkerId = "zone-store-marker-local-root";
    replacementDetection = "fail-closed-on-missing-replaced-or-identity-mismatch";
    fsync = "database-and-parent-directory";
    publication = {
      descriptor = "owned-descriptor-close-on-exec-verified-before-concurrency";
      replacement = "atomic-rename-retain-prior-quarantine-ambiguity";
    };
  };

  bundleNul = builtins.fromJSON "\"\\u0000\"";
  digestCfg = (mkEval [ base ({ ... }: {
    d2b.artifacts = lib.optionalAttrs (system == "x86_64-linux") {
      sample = {
        package = pkgs.writeText "sample-artifact" "sample";
        type = "config-bundle";
      };
    };
    d2b.zones.local-root = { };
  }) ]).config;
  digestBundle = digestCfg.d2b._bundle.zoneResourceBundlesV3.local-root;
  realisedDigestBundle =
    builtins.fromJSON (builtins.readFile digestBundle.path);
  realisedCatalog =
    builtins.fromJSON
      (builtins.unsafeDiscardStringContext
        (builtins.readFile digestCfg.d2b._artifactCatalogV3.path));
  helperWiringCfg = (mkEval [ base ({ ... }: {
    d2b.zones.local-root.resources.telemetry = {
      type = "Provider";
      spec.telemetry.emitter.ringCapacityBytes = 0;
    };
  }) ]).config;
  nullProviderCfg = (mkEval [ base ({ ... }: {
    d2b.artifacts.provider = {
      package = pkgs.writeText "provider-artifact" "provider";
      type = "provider";
    };
    d2b.zones.local-root.resources.provider = {
      type = "Provider";
      spec.artifactId = "provider";
    };
  }) ]).config;

  storePathString = path:
    builtins.unsafeDiscardStringContext (toString path);
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
      (storePathString cfgDaemon.d2b._bundle.extraArtifacts.defaultedJson.path);
    expected = true;
  };

  "bundle-artifacts/defaulted-central-etc" = {
    expr = {
      sourceHasDefaultName = lib.hasSuffix "-d2b-defaulted.json"
        (storePathString cfgDaemon.environment.etc."d2b/defaulted.json".source);
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

  "bundle-artifacts/v3-zone-data-matches-realised-json" = {
    expr = if system != "x86_64-linux"
      then true
      else digestBundle.data == realisedDigestBundle;
    expected = true;
  };

  "bundle-artifacts/v3-zone-content-hash-covers-shipped-resources" = {
    expr = if system != "x86_64-linux" then true else
      digestBundle.data.contentHash
        == "sha256:${builtins.hashString "sha256"
          ("d2b:v3:resource-bundle" + bundleNul
            + builtins.toJSON digestBundle.data.resources)}";
    expected = true;
  };

  "bundle-artifacts/v3-artifact-catalog-data-matches-realised-json" = {
    expr = if system != "x86_64-linux" then true else
      digestCfg.d2b._artifactCatalogV3.catalogData == realisedCatalog
      && digestBundle.data.artifactCatalogDigest == realisedCatalog.catalogDigest;
    expected = true;
  };

  "bundle-artifacts/v3-bundle-wires-shared-resource-validation" = {
    expr = lib.any
      (assertion:
        !assertion.assertion
        && lib.hasInfix "ringCapacityBytes is out of bounds"
          assertion.message)
      helperWiringCfg.assertions;
    expected = true;
  };

  "bundle-artifacts/v3-null-provider-digest-is-not-verified" = {
    expr = if system != "x86_64-linux" then true else
      nullProviderCfg.d2b._bundle.zoneResourceBundlesV3.local-root.data
        .providerSchemaDigests == { };
    expected = true;
  };

  "bundle-artifacts/root-group-compat" = {
    expr = cfgCompat.environment.etc."d2b/defaulted.json".group;
    expected = "root";
  };

  "bundle-artifacts/nested-tables-are-not-artifact-rows" = {
    expr =
      !(builtins.elem "data" (builtins.attrNames cfgDaemon.d2b._bundle.closures))
      && !(builtins.elem "installFileName" (builtins.attrNames cfgDaemon.d2b._bundle.minijailProfiles));
    expected = true;
  };

  "bundle-artifacts/nested-table-field-name-collisions-are-not-rows" = {
    expr =
      let
        cfg = (mkEval [ base defaultedArtifact ({ ... }: {
          d2b._bundle.closures = {
            data = { vm = "data"; path = "/nix/store/example"; };
            path = { vm = "path"; path = "/nix/store/example"; };
            installFileName = { vm = "installFileName"; path = "/nix/store/example"; };
            enableEtc = { vm = "enableEtc"; path = "/nix/store/example"; };
          };
        }) ]).config;
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
        cfg = (mkEval [ base defaultedArtifact ({ ... }: {
          d2b._bundle.extraArtifacts.bundle = {
            data = { value = "bad"; };
            installFileName = "extra-bundle.json";
          };
        }) ]).config;
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
        cfg = (mkEval [ base defaultedArtifact ({ ... }: {
          d2b._bundle.extraArtifacts.alsoDefaulted = {
            data = { value = "bad"; };
            installFileName = "defaulted.json";
          };
        }) ]).config;
      in cfg.environment.etc."d2b/defaulted.json";
    expectedError = { };
  };
}
