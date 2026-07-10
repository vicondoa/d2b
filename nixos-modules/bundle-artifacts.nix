{ config, lib, pkgs, ... }:

let
  topConfig = config;
  types = lib.types;

  artifactModule = types.submodule ({ name, config, ... }: {
    options = {
      data = lib.mkOption {
        type = types.attrsOf types.anything;
        default = { };
        internal = true;
        visible = false;
        description = "Internal non-secret bundle artifact data.";
      };

      jsonText = lib.mkOption {
        type = types.str;
        default = builtins.toJSON config.data;
        internal = true;
        visible = false;
        description = "Internal JSON rendering for this bundle artifact.";
      };

      path = lib.mkOption {
        type = types.nullOr (types.oneOf [ types.path types.str types.package ]);
        default =
          if config.installFileName == null
          then null
          else pkgs.writeText config.derivationName config.jsonText;
        internal = true;
        visible = false;
        description = "Internal realised path for this bundle artifact.";
      };

      derivationName = lib.mkOption {
        type = types.str;
        default =
          if config.installFileName == null
          then "d2b-${name}.json"
          else "d2b-${baseNameOf config.installFileName}";
        internal = true;
        visible = false;
        description = "Internal derivation name used when path is generated from jsonText.";
      };

      installFileName = lib.mkOption {
        type = types.nullOr types.str;
        default = null;
        internal = true;
        visible = false;
        description = "Internal path below /etc/d2b for central bundle artifact installation.";
      };

      mode = lib.mkOption {
        type = types.str;
        default = "0640";
        internal = true;
        visible = false;
        description = "Internal /etc mode for this bundle artifact.";
      };

      user = lib.mkOption {
        type = types.str;
        default = "root";
        internal = true;
        visible = false;
        description = "Internal /etc owner for this bundle artifact.";
      };

      group = lib.mkOption {
        type = types.str;
        default = privateGroup;
        internal = true;
        visible = false;
        description = "Internal /etc group for this bundle artifact.";
      };

      classification = lib.mkOption {
        type = types.enum [ "contractPublic" "contractPrivateNonSecret" ];
        default = "contractPrivateNonSecret";
        internal = true;
        visible = false;
        description = "Internal non-secret bundle artifact classification.";
      };

      sensitivity = lib.mkOption {
        type = types.enum [ "nonSecret" ];
        default = "nonSecret";
        internal = true;
        visible = false;
        description = "Internal sensitivity marker for store-materialised bundle artifacts.";
      };

      enableEtc = lib.mkOption {
        type = types.bool;
        default = config.installFileName != null;
        internal = true;
        visible = false;
        description = "Internal switch for central /etc/d2b artifact installation.";
      };
    };
  });

  nestedArtifactModule = types.submodule {
    freeformType = types.attrsOf types.unspecified;

    options = {
      classification = lib.mkOption {
        type = types.enum [ "contractPublic" "contractPrivateNonSecret" ];
        default = "contractPrivateNonSecret";
        internal = true;
        visible = false;
        description = "Internal non-secret nested bundle artifact classification.";
      };

      sensitivity = lib.mkOption {
        type = types.enum [ "nonSecret" ];
        default = "nonSecret";
        internal = true;
        visible = false;
        description = "Internal sensitivity marker for nested store-materialised bundle artifacts.";
      };
    };
  };

  privateGroup =
    if topConfig.d2b.daemonExperimental.enable
    then "d2bd"
    else "root";

  singletonArtifactNames = [
    "bundle"
    "hostJson"
    "processesJson"
    "privilegesJson"
    "storageJson"
    "syncJson"
    "allocatorJson"
    "realmControllersJson"
    "realmIdentityJson"
    "realmWorkloadsLauncherJson"
    "realmWorkloadsLauncherV2Json"
    "unsafeLocalWorkloadsJson"
  ];

  shouldInstall = artifact:
    artifact.enableEtc && artifact.installFileName != null;

  singletonArtifacts = lib.genAttrs singletonArtifactNames
    (name: topConfig.d2b._bundle.${name});

  extraArtifacts = topConfig.d2b._bundle.extraArtifacts;

  collidingExtraArtifactNames =
    lib.attrNames (builtins.intersectAttrs singletonArtifacts extraArtifacts);

  centrallyInstalledArtifacts =
    lib.filterAttrs
      (_: artifact: shouldInstall artifact)
      (singletonArtifacts // extraArtifacts);

in
{
  options.d2b._bundle = {
    bundle = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed bundle.json artifact metadata.";
    };

    hostJson = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed host.json artifact metadata.";
    };

    processesJson = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed processes.json artifact metadata.";
    };

    privilegesJson = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed privileges.json artifact metadata.";
    };

    storageJson = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed storage.json artifact metadata.";
    };

    syncJson = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed sync.json artifact metadata.";
    };

    allocatorJson = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed allocator.json artifact metadata.";
    };

    realmControllersJson = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed realm-controllers.json artifact metadata.";
    };

    realmIdentityJson = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed realm-identity.json artifact metadata.";
    };

    realmWorkloadsLauncherJson = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed realm-workloads-launcher.json artifact metadata for desktop launcher consumers.";
    };

    realmWorkloadsLauncherV2Json = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed realm-workloads-launcher-v2.json public metadata artifact.";
    };

    unsafeLocalWorkloadsJson = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed unsafe-local-workloads.json private configured-item artifact.";
    };

    extraArtifacts = lib.mkOption {
      type = types.attrsOf artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed extension point for future singleton bundle artifacts.";
    };

    closures = lib.mkOption {
      type = types.attrsOf nestedArtifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed closures/<vm>.json artifact metadata table.";
    };

    minijailProfiles = lib.mkOption {
      type = types.attrsOf nestedArtifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed minijail profile artifact metadata table.";
    };
  };

  config = {
    assertions = [
      {
        assertion = collidingExtraArtifactNames == [ ];
        message =
          "d2b internal bundle extraArtifacts collide with reserved artifact names: "
          + lib.concatStringsSep ", " collidingExtraArtifactNames;
      }
    ];

    environment.etc = lib.mkMerge (lib.mapAttrsToList
      (_: artifact: {
        "d2b/${artifact.installFileName}" = {
          source = artifact.path;
          inherit (artifact) mode user group;
        };
      })
      centrallyInstalledArtifacts);
  };
}
