{ lib }:

let
  inherit (lib) mkOption types;

  standardResourceTypes = [
    "Zone"
    "ZoneLink"
    "Provider"
    "Role"
    "RoleBinding"
    "Quota"
    "EmergencyPolicy"
    "Host"
    "Guest"
    "Process"
    "EphemeralProcess"
    "Volume"
    "Network"
    "Device"
    "User"
    "Credential"
    "Endpoint"
    "ResourceExport"
    "ResourceImport"
  ];

  qualifiedResourceTypePattern =
    "^[a-z][a-z0-9-]{0,62}\\.d2bus\\.org\\.[A-Z][A-Za-z0-9]{0,62}$";
  resourceRefPattern =
    "^([A-Z][A-Za-z0-9]{0,62}|[a-z][a-z0-9-]{0,62}\\.d2bus\\.org\\.[A-Z][A-Za-z0-9]{0,62})/[a-z][a-z0-9-]{0,62}$";

  validResourceType = value:
    builtins.elem value standardResourceTypes
    || builtins.match qualifiedResourceTypePattern value != null;

  validResourceRef = value:
    builtins.match resourceRefPattern value != null
    && validResourceType (builtins.head (lib.splitString "/" value));

  resourceTypeNameType = types.addCheck types.str validResourceType;
  resourceRefType = types.addCheck types.str validResourceRef;

  cpuQuantityType = types.addCheck types.str
    (value:
      let match = builtins.match "^([0-9]+)m$" value;
      in match != null && lib.toInt (builtins.head match) <= 1024000);

  memoryQuantityType = types.addCheck types.str
    (value:
      let
        match = builtins.match "^([0-9]+)(B|KB|MB|GB|TB|KiB|MiB|GiB|TiB)$" value;
        multipliers = {
          B = 1;
          KB = 1000;
          MB = 1000000;
          GB = 1000000000;
          TB = 1000000000000;
          KiB = 1024;
          MiB = 1048576;
          GiB = 1073741824;
          TiB = 1099511627776;
        };
      in
      match != null
      && lib.toInt (builtins.elemAt match 0)
        * multipliers.${builtins.elemAt match 1} <= 4398046511104);

  budgetType = types.submodule {
    freeformType = null;
    options = {
      cpu = mkOption {
        type = types.submodule {
          freeformType = null;
          options = {
            request = mkOption {
              type = types.nullOr cpuQuantityType;
              default = null;
            };
            limit = mkOption {
              type = types.nullOr cpuQuantityType;
              default = null;
            };
          };
        };
        default = { };
      };
      memory = mkOption {
        type = types.submodule {
          freeformType = null;
          options = {
            request = mkOption {
              type = types.nullOr memoryQuantityType;
              default = null;
            };
            limit = mkOption {
              type = types.nullOr memoryQuantityType;
              default = null;
            };
          };
        };
        default = { };
      };
      pids.limit = mkOption {
        type = types.nullOr (types.ints.between 1 65535);
        default = null;
      };
      fds.limit = mkOption {
        type = types.nullOr (types.ints.between 1 1048576);
        default = null;
      };
      ioWeight = mkOption {
        type = types.nullOr (types.ints.between 1 10000);
        default = null;
      };
      networkEgressBps = mkOption {
        type = types.nullOr (types.ints.between 0 1000000000000);
        default = null;
      };
      threadLimit = mkOption {
        type = types.nullOr types.ints.positive;
        default = null;
      };
    };
  };

  networkAttachmentType = types.submodule {
    freeformType = null;
    options = {
      networkRef = mkOption {
        type = resourceRefType;
      };
      default = mkOption {
        type = types.bool;
        default = false;
      };
    };
  };

  deviceAttachmentType = types.submodule {
    freeformType = null;
    options = {
      deviceRef = mkOption {
        type = resourceRefType;
      };
      exclusive = mkOption {
        type = types.bool;
        default = false;
      };
    };
  };

  executionPolicyType = types.submodule {
    freeformType = types.attrsOf types.unspecified;
    options = {
      providerRef = mkOption {
        type = types.nullOr resourceRefType;
        default = null;
      };
      defaultDomain = mkOption {
        type = types.enum [ "system" "user" ];
        default = "system";
      };
      allowedDomains = mkOption {
        type = types.listOf (types.enum [ "system" "user" ]);
        default = [ "system" ];
      };
      defaultUserRef = mkOption {
        type = types.nullOr resourceRefType;
        default = null;
      };
      budget = mkOption {
        type = budgetType;
        default = { };
      };
      networkAttachments = mkOption {
        type = types.listOf networkAttachmentType;
        default = [ ];
      };
      deviceAttachments = mkOption {
        type = types.listOf deviceAttachmentType;
        default = [ ];
      };
      volumeAttachmentDefaults = mkOption {
        type = types.listOf (types.attrsOf types.unspecified);
        default = [ ];
      };
    };
  };

  resourceModule = { ... }: {
    freeformType = null;
    options = {
      type = mkOption {
        type = resourceTypeNameType;
      };
      metadata = mkOption {
        type = types.submodule {
          freeformType = null;
          options.ownerRef = mkOption {
            type = types.nullOr resourceRefType;
            default = null;
          };
        };
        default = { };
      };
      spec = mkOption {
        type = executionPolicyType;
        default = { };
      };
    };
  };

  # The telemetry/audit resource fields are kept in a small schema-shaped
  # helper so bundle emitters and Nix-unit cases use the same bounds without
  # duplicating them in a provider-specific module.
  telemetryEmitterType = types.submodule {
    # Keep ResourceType-specific fields available while still type-checking
    # the fields owned by the telemetry contract.
    freeformType = types.attrsOf types.unspecified;
    options.ringCapacityBytes = mkOption {
      type = types.ints.between (64 * 1024) (64 * 1024 * 1024);
      default = 2 * 1024 * 1024;
    };
  };

  auditResourceType = types.submodule {
    freeformType = types.attrsOf types.unspecified;
    options = {
      retentionDays = mkOption {
        type = types.ints.between 1 3650;
        default = 30;
      };
      maxSegmentBytes = mkOption {
        type = types.ints.between (1024 * 1024) (1024 * 1024 * 1024);
        default = 64 * 1024 * 1024;
      };
    };
  };

  telemetryResourceSpecType = types.submodule {
    # This is deliberately a type provider, not a NixOS module.  Unknown
    # ResourceType fields remain available to generated schemas, but known
    # telemetry/audit fields cannot bypass their bounds through freeform data.
    freeformType = types.attrsOf types.unspecified;
    options = {
      telemetry = mkOption {
        type = types.submodule {
          freeformType = types.attrsOf types.unspecified;
          options.emitter = mkOption {
            type = telemetryEmitterType;
            default = { };
          };
        };
        default = { };
      };
      audit = mkOption {
        type = auditResourceType;
        default = { };
      };
    };
  };

  schemaAwareResourceModule = { ... }: {
    freeformType = null;
    options = {
      type = mkOption {
        type = resourceTypeNameType;
      };
      metadata = mkOption {
        type = types.submodule {
          freeformType = null;
          options = {
            ownerRef = mkOption {
              type = types.nullOr resourceRefType;
              default = null;
            };
            labels = mkOption {
              type = types.attrsOf types.str;
              default = { };
            };
            annotations = mkOption {
              type = types.attrsOf types.str;
              default = { };
            };
          };
        };
        default = { };
      };
      spec = mkOption {
        type = telemetryResourceSpecType;
        default = { };
      };
    };
  };
in
{
  inherit
    resourceModule
    schemaAwareResourceModule
    resourceRefPattern
    resourceRefType
    resourceTypeNameType
    standardResourceTypes
    telemetryResourceSpecType
    validResourceRef
    validResourceType
    ;
}
