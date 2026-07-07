# d2b.realms.<realm>.* — realm-native control-plane option foundation.
#
# This file declares the public Nix schema selected by ADR 0043 without
# materialising per-realm daemons, brokers, networks, allocators, or VM/env
# migrations. Existing `d2b.envs` and `d2b.vms.<vm>.env` behaviour remains the
# runtime source of truth; the realm index and assertions only normalize and
# validate inert planning metadata.
{ lib, config, ... }:

let
  outerConfig = config;

  label = "^[a-z][a-z0-9-]*$";
  realmPath = "^[a-z][a-z0-9-]*(\\.[a-z][a-z0-9-]*)*$";
  providerKind = "^[a-z][a-z0-9-]*$";
  absolutePath = "^/.*$";

  placementKinds = [
    "host-local"
    "gateway-vm"
    "cloud-full-host"
    "provider-controller"
    "provider-agent"
    "provider-specific"
  ];

  providerType = lib.types.submodule ({ name, ... }: {
    freeformType = lib.types.attrsOf lib.types.unspecified;
    options = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Whether this provider declaration is active for the realm.";
      };

      id = lib.mkOption {
        type = lib.types.str;
        default = name;
        description = "Stable provider identifier within the realm.";
      };

      kind = lib.mkOption {
        type = lib.types.nullOr (lib.types.strMatching providerKind);
        default = null;
        example = "aca";
        description = ''
          Provider family or adapter name. This is descriptive schema
          foundation only; no provider adapter is instantiated from this
          option in the current scope.
        '';
      };

      placement = lib.mkOption {
        type = lib.types.nullOr (lib.types.enum placementKinds);
        default = null;
        description = ''
          Optional provider-specific placement override. When null, the
          provider inherits `d2b.realms.<realm>.placement`.
        '';
      };

      capabilityRefs = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = "Opaque references to provider capability bundles or advertisements.";
      };

      configRef = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = ''
          Opaque reference to non-secret provider configuration material. Do not
          put credentials directly in Nix; use enrollment/key refs below.
        '';
      };
    };
  });
in
{
  options.d2b.realms = lib.mkOption {
    description = ''
      Realm-native control-plane declarations. A realm is the future unit of
      daemon, broker, state, audit, provider, relay, policy, and workload
      namespace ownership selected by ADR 0043.

      This release exposes the option schema only. Declaring a realm does not
      spawn per-realm daemons or brokers, allocate network resources, or alter
      current `d2b.envs` / `d2b.vms.<vm>.env` behaviour.
    '';
    default = { };
    type = lib.types.attrsOf (lib.types.submodule ({ name, config, ... }:
      let
        realmConfig = config;
        realmStateDir = "${toString outerConfig.d2b.site.stateDir}/realms/${realmConfig.id}";
        realmRunDir = "/run/d2b/realms/${realmConfig.id}";
      in
      {
        freeformType = null;
        options = {
          enable = lib.mkOption {
            type = lib.types.bool;
            default = true;
            description = "Whether this realm declaration participates in future realm-native planning.";
          };

          id = lib.mkOption {
            type = lib.types.strMatching label;
            default = name;
            description = ''
              Stable realm id. Defaults to the attribute name and is used to
              derive local paths until explicitly overridden.
            '';
          };

          name = lib.mkOption {
            type = lib.types.str;
            default = name;
            description = "Human-readable realm name; defaults to the attribute name.";
          };

          parent = lib.mkOption {
            type = lib.types.nullOr (lib.types.strMatching realmPath);
            default = null;
            example = "work";
            description = ''
              Optional parent realm path. Enabled child realms must point at an
              enabled parent path, and the parent graph must remain acyclic.
            '';
          };

          path = lib.mkOption {
            type = lib.types.strMatching realmPath;
            default =
              if realmConfig.parent == null
              then realmConfig.id
              else "${realmConfig.id}.${realmConfig.parent}";
            defaultText = lib.literalExpression "if parent == null then id else id + \".\" + parent";
            example = "payments.work";
            description = ''
              Canonical realm path, written most-specific first for target
              addresses such as `<workload>.<realm>[.<ancestor>].d2b`.
            '';
          };

          placement = lib.mkOption {
            type = lib.types.enum placementKinds;
            default = "host-local";
            description = ''
              Controller placement for this realm. `provider-specific` is an
              escape hatch for adapters that need a named placement before the
              shared enum grows.
            '';
          };

          placementProvider = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "aca";
            description = ''
              Provider identifier that owns provider-backed controller
              placements. Required when `placement` is `provider-controller`,
              `provider-agent`, or `provider-specific`; must be null for local
              placements.
            '';
          };

          providerSpecificPlacement = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "aca-managed-sandbox";
            description = ''
              Provider-defined placement name used only when `placement =
              "provider-specific"`. It is inert schema metadata in this scope.
            '';
          };

          allowedUsers = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
            example = [ "alice" ];
            description = ''
              Host users intended to receive direct access to this realm's
              future local public socket. This option does not create users,
              groups, ACLs, or socket units in the current scope.
            '';
          };

          defaultWorkloadNamespace = lib.mkOption {
            type = lib.types.strMatching realmPath;
            default = realmConfig.id;
            defaultText = lib.literalExpression "id";
            description = ''
              Default workload namespace for unqualified workload declarations
              inside this realm. Later scopes will use it for realm-qualified
              target resolution; current `d2b.vms` names are unchanged.
            '';
          };

          env = lib.mkOption {
            type = lib.types.nullOr (lib.types.strMatching label);
            default = null;
            example = "work";
            description = ''
              Transitional bridge to an existing `d2b.envs.<env>` network. It
              records intended membership only; it does not create, rename, or
              migrate envs, net VMs, or workload VM `env` assignments.
            '';
          };

          network = {
            envs = lib.mkOption {
              type = lib.types.listOf (lib.types.strMatching label);
              default = [ ];
              example = [ "work" ];
              description = ''
                Additional existing `d2b.envs` names associated with this realm
                during the transition from env groups to realms. Runtime network
                behaviour remains driven by `d2b.envs` until later scopes.
              '';
            };

            mode = lib.mkOption {
              type = lib.types.enum [ "none" "inherit-env" "declared" "external" ];
              default = "none";
              description = ''
                Placeholder for the future realm network model. `none` is the
                behaviour-safe default: no network resources are claimed by
                declaring a realm.
              '';
            };

            cidrRefs = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [ ];
              description = "Opaque references to realm-owned CIDR/address allocation records.";
            };
          };

          providers = lib.mkOption {
            type = lib.types.attrsOf providerType;
            default = { };
            description = ''
              Provider declarations owned by this realm. Entries are inert
              configuration records in this scope; provider daemons/adapters are
              not started from them yet.
            '';
          };

          relay = {
            enable = lib.mkEnableOption "realm relay reachability metadata";

            mode = lib.mkOption {
              type = lib.types.enum [ "disabled" "static" "discovery" ];
              default = "disabled";
              description = ''
                Relay configuration mode. The default is non-claiming and does
                not open listeners, connect relays, or alter routing.
              '';
            };

            endpoints = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [ ];
              description = "Non-secret relay endpoint names or refs for static relay mode.";
            };

            credentialRef = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = ''
                Opaque reference to relay credential material held outside the
                host Nix store. Plaintext credentials do not belong in this option.
              '';
            };
          };

          discovery = {
            enable = lib.mkEnableOption "dynamic realm discovery metadata";

            domain = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              example = "work.example.invalid";
              description = "Optional discovery domain or namespace for this realm.";
            };

            configRef = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = "Opaque reference to discovery configuration material.";
            };
          };

          policy = {
            bundleRef = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = "Opaque reference to the realm policy bundle.";
            };

            bundlePath = lib.mkOption {
              type = lib.types.nullOr (lib.types.strMatching absolutePath);
              default = null;
              description = "Optional absolute runtime path to a realm policy bundle artifact.";
            };

            defaultDeny = lib.mkOption {
              type = lib.types.bool;
              default = true;
              description = "Future realm policy starts from default-deny unless a later policy bundle says otherwise.";
            };
          };

          keys = {
            controllerKeyRef = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = "Opaque reference to this realm controller's signing/encryption key material.";
            };

            trustBundleRef = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = "Opaque reference to trusted parent/peer realm key material.";
            };

            enrollmentRef = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = "Opaque reference to realm enrollment material stored outside the Nix store.";
            };

            rotationPolicyRef = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = "Opaque reference to future key-rotation policy metadata.";
            };
          };

          paths = {
            stateDir = lib.mkOption {
              type = lib.types.strMatching absolutePath;
              default = realmStateDir;
              defaultText = lib.literalExpression "config.d2b.site.stateDir + \"/realms/<realm>\"";
              description = ''
                Derived realm state directory. It is not created or managed by
                this schema-only scope.
              '';
            };

            auditDir = lib.mkOption {
              type = lib.types.strMatching absolutePath;
              default = "${realmStateDir}/audit";
              defaultText = lib.literalExpression "paths.stateDir + \"/audit\"";
              description = "Derived per-realm audit directory placeholder.";
            };

            runDir = lib.mkOption {
              type = lib.types.strMatching absolutePath;
              default = realmRunDir;
              defaultText = lib.literalExpression "\"/run/d2b/realms/<realm>\"";
              description = "Derived per-realm runtime directory placeholder.";
            };

            publicSocket = lib.mkOption {
              type = lib.types.strMatching absolutePath;
              default = "${realmRunDir}/public.sock";
              defaultText = lib.literalExpression "paths.runDir + \"/public.sock\"";
              description = "Future realm public socket path. No socket is bound in this scope.";
            };

            brokerSocket = lib.mkOption {
              type = lib.types.strMatching absolutePath;
              default = "${realmRunDir}/broker.sock";
              defaultText = lib.literalExpression "paths.runDir + \"/broker.sock\"";
              description = "Future realm broker socket path. No broker is started in this scope.";
            };
          };

          broker = {
            enable = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = ''
                Future opt-in for a realm-local privileged broker. It is
                deliberately false and unused in this schema-only scope.
              '';
            };

            hostMutation = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = ''
                Whether this realm is intended to request host-mutation leases
                from the future local-root allocator. No allocator exists in
                this scope.
              '';
            };
          };
        };
      }));
  };
}
