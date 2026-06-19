# Realm gateway declarations.
{ lib, ... }:

let
  label = "^[a-z][a-z0-9-]*$";
  realmPath = "^[a-z][a-z0-9-]*(\\.[a-z][a-z0-9-]*)*$";
in
{
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
            `nixling.site.stateDir`; assertions reject `/nix/store` or
            secret-looking inline values.
          '';
        };

        credentialPath = lib.mkOption {
          type = lib.types.str;
          default = "/var/lib/nixling/gateways/${name}/credential.json";
          description = ''
            Host path for the sealed gateway credential envelope. This is a
            runtime state path, not plaintext Nix data. Assertions require it to
            live under `nixling.site.stateDir` and reject secret-looking values.
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
            description = "ACA data-plane endpoint (non-secret).";
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
        };
      };
    }));
  };
}
