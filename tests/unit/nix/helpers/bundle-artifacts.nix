{ mkEval, lib, pkgs, system, flakeRoot, ... }:

let
  digestHelpers = import ../../../../nixos-modules/resources-bundle.nix { inherit lib; };

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

  digestCfg = (mkEval [ base ({ ... }: {
    d2b.artifacts = lib.optionalAttrs (system == "x86_64-linux") {
      sample = {
        package = pkgs.writeText "sample-artifact" "sample";
        type = "provider";
      };
    };
    d2b.zones.local-root.resources = lib.optionalAttrs
      (system == "x86_64-linux") {
      sample = {
        type = "User";
        spec = {
          displayName = "Sample";
          groups = [ ];
          osUsername = "sample";
        };
      };
    };
  }) ]).config;
  digestBundle = digestCfg.d2b._bundle.zoneResourceBundlesV3.local-root;
  activeDigestBundle = digestCfg.d2b._bundle.zoneResourceBundles.local-root;
  compatibilityDigestBundle =
    digestCfg.d2b._bundle.zoneResourceBundlesCompatibility.local-root;
  installedDigestBundle =
    digestCfg.environment.etc."d2b/zones/local-root/resource-bundle.json";
  activeCatalog = digestCfg.d2b._bundle.extraArtifacts.artifactCatalog;
  installedCatalog = digestCfg.environment.etc."d2b/artifact-catalog.json";
  realisedDigestBundle =
    builtins.fromJSON (builtins.readFile digestBundle.path);
  realisedCatalog =
    builtins.fromJSON
      (builtins.unsafeDiscardStringContext
        (builtins.readFile digestCfg.d2b._artifactCatalogV3.path));
  compilerPackage = digestCfg.d2b._resourceCompiler.phase2.compiler;
  compilerSource = builtins.readFile (flakeRoot + "/nixos-modules/bundle-zones.nix");
  compilerMainSource =
    builtins.readFile (flakeRoot + "/packages/d2b-resource-compiler/src/main.rs");
  hostileProviderOutput = pkgs.writeText "d2b-hostile-provider-output" "not-a-directory";
  hostileCompilerInput = pkgs.writeText "d2b-hostile-resource-compiler-input"
    (builtins.toJSON {
      zone = "local-root";
      resources = [ ];
      providerSchemaDigests = { };
      providers = [
        {
          artifactId = "sample";
          type = "provider";
          storePath = "${hostileProviderOutput}";
          publisher = "first-party";
          signatureId = "default";
          packageDigest = (builtins.elemAt realisedCatalog.entries 0).packageDigest;
          executableDigest = "sha256:0000000000000000000000000000000000000000000000000000000000000001";
          manifestDigest = "sha256:0000000000000000000000000000000000000000000000000000000000000001";
          configSchemaDigest = "sha256:0000000000000000000000000000000000000000000000000000000000000001";
          signingKey = "-----BEGIN PUBLIC KEY-----";
        }
      ];
      artifactCatalogPath = "${digestCfg.d2b._artifactCatalogV3.path}";
      expectedArtifactCatalogDigest = realisedCatalog.catalogDigest;
      schemaRoot = null;
      expectedContentHash = null;
      strictSecrets = true;
    });
  hostileCompilerBuild = pkgs.runCommand "d2b-resource-compiler-hostile-fixture"
    { nativeBuildInputs = [ compilerPackage ]; }
    ''
      set -euo pipefail
      if d2b-resource-compiler compile \
        --input ${hostileCompilerInput} \
        --output "$out" 2>"$TMPDIR/compiler-error"; then
        echo "hostile Provider output unexpectedly passed" >&2
        exit 1
      fi
      grep -F "provider-required-output-not-regular" "$TMPDIR/compiler-error"
      printf '%s\n' compiler-ran > "$out"
    '';
  shimProgram = pkgs.writeTextFile {
    name = "d2b-bundle-shim-program";
    destination = "/share/d2b/provider/shim-program.py";
    text = "print('shim')\n";
  };
  acceptedShim = import (flakeRoot + "/nix/provider-elf-shim.nix") {
    inherit pkgs;
    name = "d2b-bundle-shim";
    interpreterPkg = pkgs.coreutils;
    interpreterPath = "bin/cat";
    program = "${shimProgram}/share/d2b/provider/shim-program.py";
  };
  acceptedShimBuild = pkgs.runCommand "d2b-resource-compiler-accepted-shim"
    { nativeBuildInputs = [ pkgs.binutils ]; }
    ''
      set -euo pipefail
      test -x ${acceptedShim}/bin/d2b-bundle-shim
      ${pkgs.binutils}/bin/readelf -h ${acceptedShim}/bin/d2b-bundle-shim >/dev/null
      printf '%s\n' accepted-shim > "$out"
    '';
  providerSecretCfg = (mkEval [ base ({ ... }: {
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
  inherit
    acceptedShimBuild
    activeCatalog
    activeDigestBundle
    compatibilityDigestBundle
    compilerMainSource
    compilerPackage
    compilerSource
    base
    cfgCompat
    cfgDaemon
    digestBundle
    digestCfg
    digestHelpers
    defaultedArtifact
    expectedZoneStorageData
    helperWiringCfg
    hostileCompilerBuild
    installedCatalog
    installedDigestBundle
    installedZoneStorage
    nullProviderCfg
    providerSecretCfg
    realisedCatalog
    realisedDigestBundle
    storePathString
    zoneStorageArtifact
    zoneStorageCfg
    ;
}
