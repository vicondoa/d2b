{ mkEval, lib, ... }:

let
  base = { lib, ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };
    nixling.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
    };
    nixling.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
  };

  defaultedArtifact = {
    nixling._bundle.extraArtifacts.defaultedJson = {
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
    nixling.daemonExperimental.enable = true;
  }) ]).config;

  cfgCompat = (mkEval [ base defaultedArtifact ({ lib, ... }: {
    nixling.daemonExperimental.enable = lib.mkForce false;
  }) ]).config;

  storePathString = path:
    builtins.unsafeDiscardStringContext (toString path);
in
{
  "bundle-artifacts/storage-json-central-etc" = {
    expr = {
      storage = {
        mode = cfgDaemon.environment.etc."nixling/storage.json".mode;
        user = cfgDaemon.environment.etc."nixling/storage.json".user;
        group = cfgDaemon.environment.etc."nixling/storage.json".group;
      };
      sync = {
        mode = cfgDaemon.environment.etc."nixling/sync.json".mode;
        user = cfgDaemon.environment.etc."nixling/sync.json".user;
        group = cfgDaemon.environment.etc."nixling/sync.json".group;
      };
    };
    expected = {
      storage = {
        mode = "0640";
        user = "root";
        group = "nixlingd";
      };
      sync = {
        mode = "0640";
        user = "root";
        group = "nixlingd";
      };
    };
  };

  "bundle-artifacts/default-json-text" = {
    expr = cfgDaemon.nixling._bundle.extraArtifacts.defaultedJson.jsonText;
    expected = builtins.toJSON cfgDaemon.nixling._bundle.extraArtifacts.defaultedJson.data;
  };

  "bundle-artifacts/default-derivation-name" = {
    expr = lib.hasSuffix "-nixling-defaulted.json"
      (storePathString cfgDaemon.nixling._bundle.extraArtifacts.defaultedJson.path);
    expected = true;
  };

  "bundle-artifacts/defaulted-central-etc" = {
    expr = {
      sourceHasDefaultName = lib.hasSuffix "-nixling-defaulted.json"
        (storePathString cfgDaemon.environment.etc."nixling/defaulted.json".source);
      group = cfgDaemon.environment.etc."nixling/defaulted.json".group;
    };
    expected = {
      sourceHasDefaultName = true;
      group = "nixlingd";
    };
  };

  "bundle-artifacts/root-group-compat" = {
    expr = cfgCompat.environment.etc."nixling/defaulted.json".group;
    expected = "root";
  };

  "bundle-artifacts/nested-tables-are-not-artifact-rows" = {
    expr =
      !(builtins.elem "data" (builtins.attrNames cfgDaemon.nixling._bundle.closures))
      && !(builtins.elem "installFileName" (builtins.attrNames cfgDaemon.nixling._bundle.minijailProfiles));
    expected = true;
  };

  "bundle-artifacts/nested-table-field-name-collisions-are-not-rows" = {
    expr =
      let
        cfg = (mkEval [ base defaultedArtifact ({ ... }: {
          nixling._bundle.closures = {
            data = { vm = "data"; path = "/nix/store/example"; };
            path = { vm = "path"; path = "/nix/store/example"; };
            installFileName = { vm = "installFileName"; path = "/nix/store/example"; };
            enableEtc = { vm = "enableEtc"; path = "/nix/store/example"; };
          };
        }) ]).config;
      in {
        closureKeys = lib.sort lib.lessThan (builtins.attrNames cfg.nixling._bundle.closures);
        defaultedInstalled = cfg.environment.etc ? "nixling/defaulted.json";
        collisionInstalled =
          (cfg.environment.etc ? "nixling/data")
          || (cfg.environment.etc ? "nixling/path")
          || (cfg.environment.etc ? "nixling/installFileName")
          || (cfg.environment.etc ? "nixling/enableEtc");
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
          nixling._bundle.extraArtifacts.bundle = {
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
          nixling._bundle.extraArtifacts.alsoDefaulted = {
            data = { value = "bad"; };
            installFileName = "defaulted.json";
          };
        }) ]).config;
      in cfg.environment.etc."nixling/defaulted.json";
    expectedError = { };
  };
}
