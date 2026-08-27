{ lib, modules, ... }:

let
  module = builtins.head modules;
  networkSource = builtins.readFile ../network.nix;
  netSource = builtins.readFile ../net.nix;
  evaluated = lib.evalModules {
    modules = [
      {
        _module.check = false;
      }
      ({ lib, ... }: {
        options.networking.networkmanager.unmanaged = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
        };
      })
      module
    ];
  };
in
{
  cases = {
    "provider-network-local/modules-evaluate" = {
      expr =
        builtins.deepSeq
          evaluated.config.networking.networkmanager.unmanaged
          true;
      expected = true;
      propagateError = true;
    };

    "network-local/module-is-a-module" = {
      expr = builtins.isFunction (import ../default.nix);
      expected = true;
    };

    "network-local/module-defines-networkmanager-unmanaged" = {
      expr =
        let value = (import ../default.nix { inherit lib; });
        in value.networking.networkmanager.unmanaged._type;
      expected = "order";
    };

    "network-local/host-module-has-no-retired-authority" = {
      expr =
        lib.all
          (needle: !(lib.hasInfix needle networkSource))
          [ "cfg.envs" "host.environments" "manifest" "route:env:" "netVmName" ];
      expected = true;
    };

    "network-local/net-guest-keeps-dhcp-neutralizer" = {
      expr = lib.hasInfix "\"10-eth-dhcp\" = lib.mkForce" netSource;
      expected = true;
    };

    "network-local/net-guest-has-no-network-desired-data" = {
      expr =
        lib.all
          (needle: !(lib.hasInfix needle netSource))
          [ "services.dnsmasq" "hostBlocklist" "attachments.json" "route:env:" ];
      expected = true;
    };
  };
}
