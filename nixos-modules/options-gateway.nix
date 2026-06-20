# Realm gateway declarations.
{ lib, ... }:

let
  label = "^[a-z][a-z0-9-]*$";
  realmPath = "^[a-z][a-z0-9-]*(\\.[a-z][a-z0-9-]*)*$";
in
{
  options.nixling._hostToolPackages = {
    nixling = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = null;
      internal = true;
      description = "Internal: resolved host nixling CLI package.";
    };

    nixlingd = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = null;
      internal = true;
      description = "Internal: resolved host nixlingd package.";
    };
  };

  options.nixling.gateways = lib.mkOption {
    description = ''
      Realm gateway guests. Each enabled entry auto-declares a dedicated
      nixling VM that holds realm provider/relay credentials inside the guest
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
            Existing nixling env the gateway VM joins. The gateway lives inside
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
          default = "/var/lib/nixling/gateways/${name}";
          description = ''
            Gateway guest state directory on the host. Must live under
            `nixling.site.stateDir`, outside the per-VM state root; assertions
            reject `/nix/store`, `..` path components, trailing slashes, or
            secret-looking inline values. The host creates this directory as
            `root:nixlingd`, while the gateway guest creates its internal copy
            as `nixlingd:nixlingd`.
          '';
        };

        credentialPath = lib.mkOption {
          type = lib.types.str;
          default = "/var/lib/nixling/gateways/${name}/credential.json";
          description = ''
            Host path for the sealed gateway credential envelope. This is a
            runtime state path, not plaintext Nix data. Assertions require it to
            live under this gateway's `stateDir` and reject `/nix/store`,
            path-traversal, trailing-slash, or secret-looking values.
          '';
        };

        allowHostRelayCredentials = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Transitional P0 escape hatch that lets the host daemon read the
            gateway credential envelope and mint short-lived Relay Send bearers.
            This must stay disabled for production realm rollout; Wave 12
            removes or fail-closes the host-resident credential path.
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
            example = "hc-nixling-display";
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
            example = "rg-nixling-centralus";
            description = "Azure resource group containing the ACA sandbox group (non-secret).";
          };

          sandboxGroup = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "casbx-nixling-demo";
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
            example = "registry.example.azurecr.io/nixling-wayland:mi";
            description = "Container image reference registered as an ACA disk image (non-secret).";
          };

          diskName = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "nixling-wayland-mi";
            description = "Stable ACA disk image label/name used for idempotent disk reuse.";
          };

          managedIdentityResourceId = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "/subscriptions/.../resourceGroups/.../providers/Microsoft.ManagedIdentity/userAssignedIdentities/nixling";
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
