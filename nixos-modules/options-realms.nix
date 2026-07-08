# d2b.realms.<realm>.* — realm-native control-plane option foundation.
#
# This file declares the public realm-native Nix schema.  Extended
# sub-option groups live in focused companion files:
#
#   options-realms-network.nix    — d2b.realms.<realm>.network.*
#                                   Full env-replacement shape:
#                                   bridge/subnet/uplink/externalNetwork/
#                                   mDNS/port-forward.
#   options-realms-workloads.nix  — d2b.realms.<realm>.workloads.*
#                                   Per-workload declarations with kind
#                                   support for local-vm and qemu-media,
#                                   plus desktop-launcher metadata.
#
# Host-local realms materialise control-plane users, groups, sockets, and
# unit definitions.  Existing `d2b.envs` and `d2b.vms.<vm>.env` remain the
# active runtime substrate during the metadata-first migration; see
# docs/how-to/migrate-d2b-v1-2-to-v2.md for the transition guide.
{ lib, config, ... }:

let
  outerConfig = config;

  label = "^[a-z][a-z0-9-]*$";
  realmPath = "^[a-z][a-z0-9-]*(\\.[a-z][a-z0-9-]*)*$";
  providerKind = "^[a-z][a-z0-9-]*$";
  absolutePath = "^/.*$";
  fingerprint = "^sha256:[0-9a-f]{64}$";

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
      Realm-native control-plane declarations.  A realm is the unit of
      daemon, broker, state, audit, provider, relay, policy, and workload
      namespace ownership in the v2 model.

      Each realm may declare:
        - `network.*`   — env-replacement network shape (bridges, subnets,
                          external network, mDNS, port-forwards).
        - `workloads.*` — per-workload declarations (kind = local-vm or
                          qemu-media) with desktop-launcher metadata.

      Host-local realms materialise deterministic control-plane units and
      access principals.  The v2 migration is metadata-first: `d2b.envs` and
      `d2b.vms` remain the active runtime substrate until an operator
      completes the transition.  See
      docs/how-to/migrate-d2b-v1-2-to-v2.md for the step-by-step guide.
    '';
    default = { };
    type = lib.types.attrsOf (lib.types.submodule ({ name, config, ... }:
      let
        realmConfig = config;
        realmStateDir = "${toString outerConfig.d2b.site.stateDir}/realms/${realmConfig.id}";
        realmAuditDir = "/var/lib/d2b/audit/realms/${realmConfig.id}";
        realmRunDir = "/run/d2b/realms/${realmConfig.id}";
      in
      {
        imports = [
          ./options-realms-network.nix
          ./options-realms-workloads.nix
        ];
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
              local public socket. Host-local realms map these users into a
              deterministic realm socket-access group; users must still be
              declared elsewhere in the host configuration.
            '';
          };

          allowedGroups = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
            example = [ "realm-work" ];
            description = ''
              Host groups intended to receive direct access to this realm's
              local public socket. It is preserved as direct-access metadata
              for controllers that can apply ACLs without making users members
              of the daemon's own group.
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
                Realm network model.

                `none`         — no bridges, net VM, or host network
                                 resources are claimed.  Safe default for
                                 metadata-only realm declarations.
                `inherit-env`  — delegates network to an existing
                                 `d2b.envs.<env>` entry in `network.envs`.
                                 Bridge lifecycle remains controlled by the
                                 env.
                `declared`     — the realm owns the network declaration.
                                 `network.lanSubnet` and
                                 `network.uplinkSubnet` must be set.
                                 d2b materialises bridges + net VM under a
                                 realm-derived name.
                `external`     — externally managed network; no d2b
                                 bridges are created.
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
            realmIdentityRef = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = ''
                Opaque reference to this realm's identity key metadata. This is
                only a locator; private or public key material does not belong
                in Nix.
              '';
            };

            realmIdentityFingerprint = lib.mkOption {
              type = lib.types.nullOr (lib.types.strMatching fingerprint);
              default = null;
              description = "SHA-256 fingerprint for the realm identity key metadata, never key material.";
            };

            controllerKeyRef = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = ''
                Opaque reference to this realm controller's signing/encryption
                credential metadata. This is a locator only; signing or
                encryption key material does not belong in Nix.
              '';
            };

            controllerCredentialFingerprint = lib.mkOption {
              type = lib.types.nullOr (lib.types.strMatching fingerprint);
              default = null;
              description = "SHA-256 fingerprint for the controller-generation credential metadata, never credential material.";
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
                Derived realm state directory. Host-local realm controller
                units create and own it through tmpfiles.
              '';
            };

            auditDir = lib.mkOption {
              type = lib.types.strMatching absolutePath;
              default = realmAuditDir;
              defaultText = lib.literalExpression "\"/var/lib/d2b/audit/realms/<realm>\"";
              description = ''
                Derived per-realm audit directory. The default is deliberately
                outside `paths.stateDir` so daemon-owned mutable state cannot
                replace or spoof the broker-owned audit stream.
              '';
            };

            runDir = lib.mkOption {
              type = lib.types.strMatching absolutePath;
              default = realmRunDir;
              defaultText = lib.literalExpression "\"/run/d2b/realms/<realm>\"";
              description = "Derived per-realm runtime directory.";
            };

            publicSocket = lib.mkOption {
              type = lib.types.strMatching absolutePath;
              default = "${realmRunDir}/public.sock";
              defaultText = lib.literalExpression "paths.runDir + \"/public.sock\"";
              description = "Realm public socket path for host-local controller units.";
            };

            brokerSocket = lib.mkOption {
              type = lib.types.strMatching absolutePath;
              default = "${realmRunDir}/broker.sock";
              defaultText = lib.literalExpression "paths.runDir + \"/broker.sock\"";
              description = "Realm broker socket path when host mutation is enabled.";
            };
          };

          broker = {
            enable = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = ''
                Opt-in for a realm-local privileged broker. A host-local realm
                materialises broker units only when both this and
                `hostMutation` are true.
              '';
            };

            hostMutation = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = ''
                Whether this realm is intended to request host-mutation leases
                from the local-root allocator. Host-local broker units are
                materialised only when this is true and broker.enable is true.
              '';
            };
          };
        };
      }));
  };
}
