# d2b.realms.<realm>.providers.<provider> — typed provider instances.
{ lib, ... }:

let
  labelType = lib.types.strMatching "^[a-z][a-z0-9-]{0,127}$";
  implementationType =
    lib.types.strMatching "^[a-z][a-z0-9-]{0,63}$";
  primaryAuthorities = [
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
  placementKinds = [
    "host-local"
    "gateway-vm"
    "cloud-full-host"
    "provider-controller"
    "provider-agent"
    "provider-specific"
  ];

  providerType = lib.types.submodule ({ name, ... }: {
    freeformType = null;
    options = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Whether this provider instance is enabled.";
      };

      id = lib.mkOption {
        type = labelType;
        default = name;
        description = "Stable provider instance identifier within the realm.";
      };

      type = lib.mkOption {
        type = lib.types.enum primaryAuthorities;
        description = "The provider's single closed primary authority.";
      };

      implementationId = lib.mkOption {
        type = implementationType;
        description = "Canonical provider implementation identifier.";
      };

      placement = lib.mkOption {
        type = lib.types.nullOr (lib.types.enum placementKinds);
        default = null;
        description = "Optional provider placement override.";
      };

      capabilities = lib.mkOption {
        type = lib.types.listOf implementationType;
        default = [ ];
        description = "Positive provider capability claims.";
      };

      configRef = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Opaque reference to non-secret provider configuration.";
      };
    };
  });
in
{
  options.providers = lib.mkOption {
    type = lib.types.attrsOf providerType;
    default = { };
    description = ''
      Provider instances owned by this realm. Every instance has exactly one
      primary authority and one canonical implementation identifier; free-form
      provider kinds and placeholder providers are not accepted.
    '';
  };
}
