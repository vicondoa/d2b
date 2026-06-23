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
    nixling._bundle.defaultedJson = {
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
    expr = cfgDaemon.nixling._bundle.defaultedJson.jsonText;
    expected = builtins.toJSON cfgDaemon.nixling._bundle.defaultedJson.data;
  };

  "bundle-artifacts/default-derivation-name" = {
    expr = lib.hasSuffix "-nixling-defaulted.json"
      (storePathString cfgDaemon.nixling._bundle.defaultedJson.path);
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
}
