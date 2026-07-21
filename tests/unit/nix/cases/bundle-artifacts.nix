{ mkEval, lib, flakeRoot, ... }:

let
  identity = import (flakeRoot + "/nixos-modules/v2-identity.nix");
  observabilityRealmId = identity.deriveRealmId "local-root";
  observabilityWorkloadId =
    identity.deriveWorkloadId observabilityRealmId "sys-obs";

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
    d2b.acceptDestructiveV2Cutover = true;
    d2b.realms.work = {
      path = "work";
      placement = "host-local";
      broker = {
        enable = true;
        hostMutation = true;
      };
      network = {
        mode = "declared";
        lanSubnet = "10.20.0.0/24";
        uplinkSubnet = "192.0.2.0/30";
      };
      providers.runtime = {
        type = "runtime";
        implementationId = "cloud-hypervisor";
      };
      workloads.app = {
        providerRefs.runtime = "runtime";
        config = { };
      };
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

  "bundle-artifacts/realm-private-central-etc" = {
    expr = lib.genAttrs [
      "realm-controllers.json"
      "realm-identity.json"
    ] (name: {
      mode = cfgDaemon.environment.etc."d2b/${name}".mode;
      user = cfgDaemon.environment.etc."d2b/${name}".user;
      group = cfgDaemon.environment.etc."d2b/${name}".group;
    });
    expected = lib.genAttrs [
      "realm-controllers.json"
      "realm-identity.json"
    ] (_: {
      mode = "0640";
      user = "root";
      group = "d2bd";
    });
  };

  "bundle-artifacts/realm-private-classifications" = {
    expr = {
      bundleVersion = cfgDaemon.d2b._bundle.bundle.data.bundleVersion;
      schemaVersion = cfgDaemon.d2b._bundle.bundle.data.schemaVersion;
      bundle = {
        inherit (cfgDaemon.d2b._bundle.bundle) classification sensitivity;
      };
      controllers = {
        inherit (cfgDaemon.d2b._bundle.realmControllersJson) classification sensitivity;
      };
      identity = {
        inherit (cfgDaemon.d2b._bundle.realmIdentityJson) classification sensitivity;
      };
    };
    expected = {
      bundleVersion = 12;
      schemaVersion = "v2";
      bundle = {
        classification = "contractPrivateNonSecret";
        sensitivity = "nonSecret";
      };
      controllers = {
        classification = "contractPrivateNonSecret";
        sensitivity = "nonSecret";
      };
      identity = {
        classification = "contractPrivateNonSecret";
        sensitivity = "nonSecret";
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
        closureKeys = lib.filter
          (name: builtins.elem name [ "data" "enableEtc" "installFileName" "path" ])
          (lib.sort lib.lessThan (builtins.attrNames cfg.d2b._bundle.closures));
        defaultedInstalled = cfg.environment.etc ? "d2b/defaulted.json";
        collisionInstalled =
          (cfg.environment.etc ? "d2b/data")
          || (cfg.environment.etc ? "d2b/path")
          || (cfg.environment.etc ? "d2b/installFileName")
          || (cfg.environment.etc ? "d2b/enableEtc");
      };
    expected = {
      closureKeys = [ "data" "enableEtc" "installFileName" "path" ];
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

  "bundle-artifacts/observability-vsock-path-derived-when-disabled" = {
    expr = {
      enabled = cfgDaemon.d2b._manifestData._observability.enabled;
      obsVsockHostSocket =
        cfgDaemon.d2b._manifestData._observability.obsVsockHostSocket;
    };
    expected = {
      enabled = false;
      obsVsockHostSocket =
        "/var/lib/d2b/r/${observabilityRealmId}/w/${observabilityWorkloadId}/vsock.sock";
    };
  };

  "bundle-artifacts/observability-vsock-path-matches-canonical-row-when-enabled" = {
    expr =
      let
        cfgObservability = (mkEval [ base defaultedArtifact ({ ... }: {
          d2b.observability.enable = true;
        }) ]).config;
      in {
        enabled = cfgObservability.d2b._manifestData._observability.enabled;
        obsVsockHostSocket =
          cfgObservability.d2b._manifestData._observability.obsVsockHostSocket;
        canonicalRowPath =
          cfgObservability.d2b._realmObservability.endpoints.stackVsock.path;
      };
    expected = {
      enabled = true;
      obsVsockHostSocket =
        "/var/lib/d2b/r/${observabilityRealmId}/w/${observabilityWorkloadId}/vsock.sock";
      canonicalRowPath =
        "/var/lib/d2b/r/${observabilityRealmId}/w/${observabilityWorkloadId}/vsock.sock";
    };
  };
}
