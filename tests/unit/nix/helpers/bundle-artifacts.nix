{ mkEval, lib, pkgs, system, flakeRoot, ... }:

let
  digestHelpers = import ../../../../nixos-modules/resources-bundle.nix { inherit lib; };
  compilerCommand = "d2b-resource-compiler";
  compilerStub = pkgs.writeShellScriptBin
    compilerCommand
    "exit 0";
  mkEvalStub = modules: mkEval (modules ++ [
    ({ lib, ... }: {
      d2b._resourceCompiler.phase2.compiler = lib.mkForce compilerStub;
    })
  ]);

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

  cfgDaemon = (mkEvalStub [ base defaultedArtifact ({ ... }: {
    d2b.daemonExperimental.enable = true;
  }) ]).config;

  cfgCompat = (mkEvalStub [ base defaultedArtifact ({ lib, ... }: {
    d2b.daemonExperimental.enable = lib.mkForce false;
  }) ]).config;

  zoneStorageCfg = (mkEvalStub [ base ({ ... }: {
    d2b.daemonExperimental.enable = true;
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

  compilerSource = builtins.readFile (flakeRoot + "/nixos-modules/bundle-zones.nix");
  compilerMainSource =
    builtins.readFile (flakeRoot + "/packages/d2b-resource-compiler/src/main.rs");
  compilerSelected =
    let
      selected = cfgDaemon.d2b._resourceCompiler.phase2.compiler;
      selectedPath =
        builtins.unsafeDiscardStringContext (toString selected);
      stubPath = builtins.unsafeDiscardStringContext (toString compilerStub);
    in
    selectedPath == stubPath;
  providerSecretCfg = (mkEvalStub [ base ({ ... }: {
    d2b.artifacts.provider = {
      package = pkgs.writeText "provider-secret-artifact" "provider";
      type = "provider";
    };
    d2b.zones.local-root.resources.provider = {
      type = "Provider";
      spec = {
        artifactId = "provider";
        config.token = "inline-token";
      };
    };
  }) ]).config;
  helperWiringCfg = (mkEvalStub [ base ({ ... }: {
    d2b.zones.local-root.resources.telemetry = {
      type = "Provider";
      spec.telemetry.emitter.ringCapacityBytes = 0;
    };
  }) ]).config;
  nullProviderCfg = (mkEvalStub [ base ({ ... }: {
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
  inherit
    compilerMainSource
    compilerCommand
    compilerSource
    compilerStub
    compilerSelected
    base
    cfgCompat
    cfgDaemon
    digestHelpers
    defaultedArtifact
    expectedZoneStorageData
    helperWiringCfg
    installedZoneStorage
    mkEvalStub
    nullProviderCfg
    providerSecretCfg
    storePathString
    zoneStorageArtifact
    zoneStorageCfg
    ;
}
