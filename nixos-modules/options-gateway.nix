# Realm gateway declarations.
{ lib, ... }:

let
  label = "^[a-z][a-z0-9-]*$";
  realmPath = "^[a-z][a-z0-9-]*(\\.[a-z][a-z0-9-]*)*$";
in
{
  options.d2b._hostToolPackages = {
    d2b = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = null;
      internal = true;
      description = "Internal: resolved host d2b CLI package.";
    };

    d2bd = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = null;
      internal = true;
      description = "Internal: resolved host d2bd package.";
    };

    d2bGatewayRuntime = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = null;
      internal = true;
      description = "Internal: resolved gateway runtime helper package.";
    };
  };

  options.d2b.gateways = lib.mkOption {
    description = ''
      Realm gateway guests. Each enabled entry auto-declares a dedicated
      d2b VM that holds realm provider/relay credentials inside the guest
      boundary. The host declaration carries only non-secret coordinates and
      state-directory paths; plaintext credentials are never represented in the
      host Nix store.
    '';
    default = { };
    type = lib.types.attrsOf (lib.types.submodule ({ name, ... }: {
      options = {
        enable = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = "Whether to declare this gateway guest.";
        };

        realm = lib.mkOption {
          type = lib.types.strMatching realmPath;
          default = name;
          example = "work";
          description = ''
            Realm path served by this gateway, in target-label form
            (for example `work` or `prod.work`).
          '';
        };

        env = lib.mkOption {
          type = lib.types.str;
          example = "work";
          description = ''
            Existing d2b env the gateway VM joins. The gateway lives inside
            the same isolated env as the workloads it fronts.
          '';
        };

        index = lib.mkOption {
          type = lib.types.ints.between 10 250;
          default = 10;
          description = ''
            LAN address index for the gateway VM inside `env`. Must be unique
            in the env; the existing network assertion catches conflicts.
          '';
        };

        vmName = lib.mkOption {
          type = lib.types.strMatching label;
          default = "sys-${name}-gateway";
          description = ''
            Auto-declared VM name for the gateway guest. The default uses the
            framework-reserved `sys-*` prefix; assertions admit these names as
            framework-owned system VMs.
          '';
        };

        stateDir = lib.mkOption {
          type = lib.types.str;
          default = "/var/lib/d2b/gateways/${name}";
          description = ''
            Gateway guest runtime state directory. Must live under
            `d2b.site.stateDir`, outside the per-VM state root; assertions
            reject `/nix/store`, `..` path components, trailing slashes, or
            secret-looking inline values. The host does not manage gateway
            credential files or their sealing key; the gateway guest creates
            and owns that runtime state as `d2bd:d2bd`.
          '';
        };

        credentialPath = lib.mkOption {
          type = lib.types.str;
          default = "/var/lib/d2b/gateways/${name}/credential.sealed.json";
          description = ''
            Guest runtime path for the sealed gateway credential envelope. This
            is not plaintext Nix data. Assertions require it to
            live under this gateway's `stateDir` and reject `/nix/store`,
            path-traversal, trailing-slash, or secret-looking values.
          '';
        };

        sealKeyPath = lib.mkOption {
          type = lib.types.str;
          default = "/var/lib/d2b/gateways/${name}/seal.key";
          description = ''
            Guest-local sealing key path for the encrypted gateway credential
            envelope. The key is created by the in-guest enrollment flow with
            mode `0600`; it is a runtime state path and must live under this
            gateway's `stateDir`.
          '';
        };

        allowHostRelayCredentials = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Retired compatibility option. Host-side gateway credential reads
            and Relay Send bearer minting are rejected; enroll credentials
            inside the gateway guest instead.
          '';
        };

        relay = {
          namespace = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "relns-example.servicebus.windows.net";
            description = "Azure Relay namespace FQDN (non-secret).";
          };

          entity = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "hc-d2b-display";
            description = "Azure Relay hybrid-connection entity name (non-secret).";
          };
        };

        aca = {
          endpoint = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "https://example.eastus.azurecontainerapps.io";
            description = ''
              Legacy ACA data-plane endpoint coordinate (non-secret). New
              deployments should set `region`; the daemon derives the preview
              endpoint from it.
            '';
          };

          subscription = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "00000000-0000-0000-0000-000000000000";
            description = "Azure subscription id for ACA sandbox data-plane calls (non-secret).";
          };

          resourceGroup = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "rg-d2b-centralus";
            description = "Azure resource group containing the ACA sandbox group (non-secret).";
          };

          sandboxGroup = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "casbx-d2b-demo";
            description = "ACA sandbox group name (non-secret).";
          };

          region = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "centralus";
            description = "Azure region selecting the ACA preview data-plane endpoint.";
          };

          diskImageId = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "7d9de4d2-f953-4a4c-84ae-e90bf208f9cf";
            description = ''
              Existing ACA private disk image id to use for sandbox creation.
              If unset, `image` is registered as a disk image using the REST
              data-plane `PUT /diskimages` contract.
            '';
          };

          image = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "registry.example.azurecr.io/d2b-wayland:mi";
            description = "Container image reference registered as an ACA disk image (non-secret).";
          };

          diskName = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "d2b-wayland-mi";
            description = "Stable ACA disk image label/name used for idempotent disk reuse.";
          };

          managedIdentityResourceId = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "/subscriptions/.../resourceGroups/.../providers/Microsoft.ManagedIdentity/userAssignedIdentities/d2b";
            description = ''
              Optional user-assigned managed identity resource id used by ACA
              to pull the configured private image. This is an Azure resource
              id, not a credential.
            '';
          };

          managedIdentityClientId = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "00000000-0000-0000-0000-000000000000";
            description = ''
              Optional user-assigned managed identity client id passed to the
              in-sandbox MSI endpoint. ACA sandboxes can require this client id
              even when the identity endpoint is injected.
            '';
          };

          cpu = lib.mkOption {
            type = lib.types.str;
            default = "1000m";
            description = "ACA sandbox CPU request for provider-created sandboxes.";
          };

          memory = lib.mkOption {
            type = lib.types.str;
            default = "2048Mi";
            description = "ACA sandbox memory request for provider-created sandboxes.";
          };

          autoSuspendIntervalSecs = lib.mkOption {
            type = lib.types.ints.positive;
            default = 600;
            description = "ACA auto-suspend interval for provider-created sandboxes.";
          };
        };

        display = {
          vsockPort = lib.mkOption {
            type = lib.types.ints.between 1 4294967295;
            default = 14319;
            description = "Dedicated AF_VSOCK port used for display streams.";
          };

          waypipeCompression = lib.mkOption {
            type = lib.types.enum [ "zstd" "lz4" "none" ];
            default = "zstd";
            description = "Waypipe compression setting used by the gateway display bridge.";
          };

          waypipeSocket = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "/run/user/1000/wpc.sock";
            description = "Operator-side Waypipe client socket for the display relay bridge.";
          };
        };
      };
    }));
  };
}
