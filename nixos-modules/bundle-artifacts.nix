{ config, lib, pkgs, ... }:

let
  topConfig = config;
  types = lib.types;

  artifactModule = types.submodule ({ name, config, ... }: {
    freeformType = types.attrsOf types.unspecified;

    options = {
      data = lib.mkOption {
        type = types.unspecified;
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
        type = types.nullOr types.unspecified;
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
          then "nixling-${name}.json"
          else "nixling-${baseNameOf config.installFileName}";
        internal = true;
        visible = false;
        description = "Internal derivation name used when path is generated from jsonText.";
      };

      installFileName = lib.mkOption {
        type = types.nullOr types.str;
        default = null;
        internal = true;
        visible = false;
        description = "Internal path below /etc/nixling for central bundle artifact installation.";
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
        description = "Internal switch for central /etc/nixling artifact installation.";
      };
    };
  });

  privateGroup =
    if topConfig.nixling.daemonExperimental.enable
    then "nixlingd"
    else "root";

  artifactRowType = types.addCheck artifactModule
    (value: value ? data || value ? jsonText || value ? path || value ? installFileName);

  nestedTableType = types.attrsOf types.unspecified;

  bundleValueType = types.oneOf [ artifactRowType nestedTableType ];

  shouldInstall = artifact:
    (artifact ? enableEtc) && artifact.enableEtc && artifact.installFileName != null && artifact.path != null;

  centrallyInstalledArtifacts =
    lib.filterAttrs
      (_: artifact: shouldInstall artifact)
      topConfig.nixling._bundle;

  installedArtifactAssertions = lib.mapAttrsToList
    (name: artifact: {
      assertion =
        !shouldInstall artifact
        || (artifact.classification != null && artifact.sensitivity == "nonSecret");
      message =
        "nixling internal bundle artifact `${name}` installs into /etc/nixling "
        + "without non-secret classification metadata.";
    })
    topConfig.nixling._bundle;
in
{
  options.nixling._bundle = lib.mkOption {
    type = types.attrsOf bundleValueType;
    default = { };
    internal = true;
    visible = false;
    description = "Internal typed bundle artifact metadata.";
  };

  config = {
    assertions = installedArtifactAssertions;

    environment.etc = lib.mapAttrs'
      (_: artifact: lib.nameValuePair "nixling/${artifact.installFileName}" {
        source = artifact.path;
        inherit (artifact) mode user group;
      })
      centrallyInstalledArtifacts;
  };
}
