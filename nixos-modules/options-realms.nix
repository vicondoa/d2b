# d2b.realms.<realm> — realm-owned control-plane declarations.
{ config, lib, ... }:

let
  labelType = lib.types.strMatching "^[a-z][a-z0-9-]{0,127}$";
  realmPathType =
    lib.types.strMatching "^[a-z][a-z0-9-]{0,127}(\\.[a-z][a-z0-9-]{0,127})*$";
  absolutePathType = lib.types.strMatching "^/.*$";
  fingerprintType = lib.types.strMatching "^sha256:[0-9a-f]{64}$";

  placementKinds = [
    "host-local"
    "gateway-vm"
    "cloud-full-host"
    "provider-controller"
    "provider-agent"
    "provider-specific"
  ];
  providerBackedPlacements = [
    "provider-controller"
    "provider-agent"
    "provider-specific"
  ];
  providerAuthorities = [
    "runtime"
    "infrastructure"
    "transport"
    "substrate"
    "credential"
    "display"
    "network"
    "storage"
    "device"
    "audio"
    "observability"
  ];

  enabledRealms = lib.filterAttrs (_: realm: realm.enable) config.d2b.realms;
  enabledRealmList = builtins.attrValues enabledRealms;
  enabledRealmPaths = map (realm: realm.path) enabledRealmList;
  unique = values: builtins.length values == builtins.length (lib.unique values);
  validLabel = value:
    builtins.isString value
    && builtins.stringLength value <= 128
    && builtins.match "^[a-z][a-z0-9-]*$" value != null;

  # `launcher` is reserved for the polkit-launcher group (`d2b`) singleton;
  # no framework module ever auto-declares a workload with this exact name,
  # so it is unconditionally rejected.
  reservedWorkloadExactName = name: name == "launcher";

  # `sys-` is reserved for d2b's own auto-declared stack workloads (e.g. the
  # observability sink named by `d2b.observability.vmName`, default
  # `sys-obs`). `network` is reserved for the auto-declared net VM workload
  # created by options-realms-workloads.nix when `network.mode == "declared"`.
  # A workload only clears these reservations when the owning framework
  # module attests to it via the internal `_frameworkReservedName` marker
  # (see options-realms-workloads.nix and components/observability/default.nix);
  # an operator-declared workload can never set that marker itself, so any
  # collision on these names fails closed instead of silently merging.
  reservedWorkloadPrefixOrName = name:
    lib.hasPrefix "sys-" name || name == "network";

  realmByPath = lib.listToAttrs
    (map (realm: {
      name = realm.path;
      value = realm;
    }) enabledRealmList);
  hasParentCycle = path: seen:
    if builtins.elem path seen then true
    else
      let
        realm = realmByPath.${path} or null;
      in
      realm != null
      && realm.parent != null
      && builtins.hasAttr realm.parent realmByPath
      && hasParentCycle realm.parent (seen ++ [ path ]);

  realmType = lib.types.submodule ({ name, config, ... }: {
    imports = [
      ./options-realms-network.nix
      ./options-realms-providers.nix
      ./options-realms-workloads.nix
    ];
    freeformType = null;

    options = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Whether this realm is part of the evaluated d2b configuration.";
      };

      id = lib.mkOption {
        type = labelType;
        default = name;
        description = "Stable realm identifier; defaults to the attribute name.";
      };

      name = lib.mkOption {
        type = lib.types.str;
        default = name;
        description = "Human-readable realm name.";
      };

      parent = lib.mkOption {
        type = lib.types.nullOr realmPathType;
        default = null;
        description = "Canonical path of the enabled parent realm, or null for a root realm.";
      };

      path = lib.mkOption {
        type = realmPathType;
        default =
          if config.parent == null
          then config.id
          else "${config.id}.${config.parent}";
        defaultText =
          lib.literalExpression ''if parent == null then id else "''${id}.''${parent}"'';
        description = "Canonical most-specific-first realm path.";
      };

      placement = lib.mkOption {
        type = lib.types.enum placementKinds;
        default = "host-local";
        description = "Placement of this realm's controller.";
      };

      placementProvider = lib.mkOption {
        type = lib.types.nullOr labelType;
        default = null;
        description = ''
          Provider instance that owns a provider-backed controller placement.
          It must name an enabled entry in this realm's providers set.
        '';
      };

      providerSpecificPlacement = lib.mkOption {
        type = lib.types.nullOr labelType;
        default = null;
        description = "Provider-defined placement, valid only for provider-specific placement.";
      };

      allowedUsers = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = "Host users granted direct access to this realm's local public endpoint.";
      };

      allowedGroups = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = "Host groups granted direct access to this realm's local public endpoint.";
      };

      defaultWorkloadNamespace = lib.mkOption {
        type = realmPathType;
        default = config.path;
        defaultText = lib.literalExpression "path";
        description = "Realm path used to qualify workload declarations by default.";
      };

      network = {
        mode = lib.mkOption {
          type = lib.types.enum [ "none" "declared" "external" ];
          default = "none";
          description = ''
            Realm network ownership. none claims no network resources, declared
            creates realm-owned resources, and external records an externally
            managed network boundary.
          '';
        };

        cidrRefs = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
          description = "Opaque references to realm-owned address allocations.";
        };
      };

      discovery = {
        enable = lib.mkEnableOption "dynamic realm discovery";

        domain = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Optional discovery domain or namespace.";
        };

        configRef = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Opaque reference to non-secret discovery configuration.";
        };
      };

      policy = {
        allowUnsafeLocal = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Permit a systemd-user runtime provider in this realm.";
        };

        bundleRef = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Opaque reference to the realm policy bundle.";
        };

        bundlePath = lib.mkOption {
          type = lib.types.nullOr absolutePathType;
          default = null;
          description = "Absolute runtime path to a realm policy bundle artifact.";
        };

        defaultDeny = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = "Start realm policy from default-deny.";
        };
      };

      keys = {
        realmIdentityRef = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Opaque realm identity metadata reference; never key material.";
        };

        realmIdentityFingerprint = lib.mkOption {
          type = lib.types.nullOr fingerprintType;
          default = null;
          description = "SHA-256 fingerprint of the realm identity metadata.";
        };

        controllerKeyRef = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Opaque controller credential reference; never key material.";
        };

        controllerCredentialFingerprint = lib.mkOption {
          type = lib.types.nullOr fingerprintType;
          default = null;
          description = "SHA-256 fingerprint of the controller credential metadata.";
        };

        trustBundleRef = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Opaque trust-bundle reference.";
        };

        enrollmentRef = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Opaque realm-enrollment reference.";
        };

        rotationPolicyRef = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Opaque key-rotation policy reference.";
        };
      };

      paths = {
        stateDir = lib.mkOption {
          type = absolutePathType;
          default = "/var/lib/d2b/realms/${config.id}";
          description = "Declarative realm state root; runtime emission replaces names with short IDs.";
        };

        auditDir = lib.mkOption {
          type = absolutePathType;
          default = "/var/lib/d2b/audit/realms/${config.id}";
          description = "Declarative realm audit root; runtime emission replaces names with short IDs.";
        };

        runDir = lib.mkOption {
          type = absolutePathType;
          default = "/run/d2b/realms/${config.id}";
          description = "Declarative realm runtime root; runtime emission replaces names with short IDs.";
        };

        publicSocket = lib.mkOption {
          type = absolutePathType;
          default = "${config.paths.runDir}/public.sock";
          description = "Realm public endpoint path.";
        };

        brokerSocket = lib.mkOption {
          type = absolutePathType;
          default = "${config.paths.runDir}/broker.sock";
          description = "Realm broker endpoint path.";
        };
      };

      broker = {
        enable = lib.mkEnableOption "this realm's confined privileged broker";

        hostMutation = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Allow allocator-approved host mutation leases for this realm broker.";
        };
      };
    };
  });
in
{
  options.d2b.realms = lib.mkOption {
    type = lib.types.attrsOf realmType;
    default = { };
    description = "Realm-owned d2b 2.0 control-plane configuration.";
  };

  config.assertions =
    [
      {
        assertion =
          lib.all
            (realmName:
              validLabel realmName
              && !(builtins.elem realmName [ "all" "d2b" ]))
            (builtins.attrNames enabledRealms);
        message =
          "Enabled d2b.realms attribute names must be canonical labels and "
          + "must not use the reserved target labels all or d2b.";
      }
      {
        assertion = unique (map (realm: realm.id) enabledRealmList);
        message = "Enabled d2b.realms entries must have unique ids.";
      }
      {
        assertion = unique enabledRealmPaths;
        message = "Enabled d2b.realms entries must have unique canonical paths.";
      }
      {
        assertion =
          !(lib.any
            (path: hasParentCycle path [ ])
            enabledRealmPaths);
        message = "Enabled d2b.realms parent links must form an acyclic tree.";
      }
    ]
    ++ lib.concatMap
      (realmName:
        let
          realm = enabledRealms.${realmName};
          providerNames =
            builtins.attrNames (lib.filterAttrs (_: provider: provider.enable) realm.providers);
          providerIds =
            map (provider: provider.id)
              (builtins.attrValues
                (lib.filterAttrs (_: provider: provider.enable) realm.providers));
          placementIsProviderBacked =
            builtins.elem realm.placement providerBackedPlacements;
          placementProvider =
            if realm.placementProvider == null
            then null
            else realm.providers.${realm.placementProvider} or null;
        in
        [
          {
            assertion =
              lib.all validLabel (builtins.attrNames realm.providers);
            message =
              "Provider attribute names in d2b.realms.${realmName} must be canonical labels.";
          }
          {
            assertion =
              lib.all
                (workloadName:
                  validLabel workloadName
                  && !(builtins.elem workloadName [ "all" "d2b" ]))
                (builtins.attrNames realm.workloads);
            message =
              "Workload attribute names in d2b.realms.${realmName} must be "
              + "canonical labels and must not use all or d2b.";
          }
          {
            assertion =
              lib.all
                (workload:
                  lib.all
                    (providerType: builtins.elem providerType providerAuthorities)
                    (builtins.attrNames workload.providerRefs))
                (builtins.attrValues realm.workloads);
            message =
              "Workload providerRefs in d2b.realms.${realmName} must use a "
              + "closed provider authority name.";
          }
          {
            assertion =
              lib.all
                (workload:
                  !workload.enable
                  || builtins.hasAttr "runtime" workload.providerRefs)
                (builtins.attrValues realm.workloads);
            message =
              "Every enabled workload in d2b.realms.${realmName} must bind "
              + "providerRefs.runtime explicitly.";
          }
          {
            assertion =
              realm.parent == null || builtins.elem realm.parent enabledRealmPaths;
            message =
              "d2b.realms.${realmName}.parent must name an enabled realm path.";
          }
          {
            assertion = unique providerIds;
            message = "Enabled providers in d2b.realms.${realmName} must have unique ids.";
          }
          {
            assertion =
              unique
                (map (workload: workload.id)
                  (builtins.attrValues
                    (lib.filterAttrs (_: workload: workload.enable) realm.workloads)));
            message = "Enabled workloads in d2b.realms.${realmName} must have unique ids.";
          }
          {
            assertion =
              placementIsProviderBacked
              == (realm.placementProvider != null);
            message =
              "d2b.realms.${realmName}.placementProvider must be set exactly "
              + "for provider-backed realm placements.";
          }
          {
            assertion =
              realm.placementProvider == null
              || (builtins.elem realm.placementProvider providerNames
                && placementProvider != null
                && placementProvider.enable);
            message =
              "d2b.realms.${realmName}.placementProvider must name an enabled provider.";
          }
          {
            assertion =
              (realm.placement == "provider-specific")
              == (realm.providerSpecificPlacement != null);
            message =
              "d2b.realms.${realmName}.providerSpecificPlacement must be set "
              + "exactly for provider-specific placement.";
          }
        ]
        ++ lib.concatLists (lib.mapAttrsToList
          (workloadName: workload:
            let
              runtimeRef = workload.providerRefs.runtime or null;
              provider =
                if runtimeRef == null
                then null
                else realm.providers.${runtimeRef} or null;
              frameworkOwnsReservedName =
                workload._frameworkReservedName or false;
            in
            [
              {
                assertion =
                  !workload.enable
                  || (provider != null
                    && provider.enable
                    && provider.type == "runtime");
                message =
                  "d2b.realms.${realmName}.workloads.${workloadName}.providerRefs.runtime "
                  + "must name an enabled runtime provider in the same realm.";
              }
              {
                assertion = !(reservedWorkloadExactName workloadName);
                message =
                  "d2b.realms.${realmName}.workloads.${workloadName}: "
                  + "'launcher' is reserved for the polkit-launcher group "
                  + "(d2b); pick another workload name.";
              }
              {
                assertion =
                  !(reservedWorkloadPrefixOrName workloadName)
                  || frameworkOwnsReservedName;
                message =
                  "d2b.realms.${realmName}.workloads.${workloadName}: names "
                  + "starting with 'sys-' and the exact name 'network' are "
                  + "reserved for d2b's own auto-declared workloads (the "
                  + "net VM workload created when network.mode = "
                  + "\"declared\", and stack workloads such as "
                  + "d2b.observability.vmName's sys-obs). Rename this "
                  + "workload; it is not the framework's own auto-declared "
                  + "entry.";
              }
              {
                assertion =
                  !workload.enable
                  || provider == null
                  || provider.implementationId != "systemd-user"
                  || realm.policy.allowUnsafeLocal;
                message =
                  "d2b.realms.${realmName}.workloads.${workloadName} selects "
                  + "the systemd-user unsafe-local runtime implementation; "
                  + "set d2b.realms.${realmName}.policy.allowUnsafeLocal = "
                  + "true to opt in explicitly.";
              }
            ])
          realm.workloads))
      (builtins.attrNames enabledRealms);
}
