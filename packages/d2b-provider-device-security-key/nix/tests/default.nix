{ lib, modules, pkgs, system, ... }:

let
  module = builtins.head modules;
  evaluated = lib.evalModules {
    specialArgs = {
      inherit pkgs;
      name = "provider-test";
      d2bInputs = {
        self.packages.${system}.d2b-sk-frontend-static =
          pkgs.writeTextDir "bin/d2b-sk-frontend" "";
      };
    };
    modules = [
      ({ lib, ... }: {
        options = {
          boot.kernelModules = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
          };
          users.groups = lib.mkOption {
            type = lib.types.attrsOf lib.types.anything;
            default = { };
          };
          services.udev.extraRules = lib.mkOption {
            type = lib.types.lines;
            default = "";
          };
          systemd.services = lib.mkOption {
            type = lib.types.attrsOf lib.types.anything;
            default = { };
          };
        };
      })
      module
    ];
  };
  config = evaluated.config;
  frontend = config.systemd.services.d2b-sk-frontend;
in
{
  cases = {
    "provider-device-security-key/modules-evaluate" = {
      expr = builtins.deepSeq config.boot.kernelModules true;
      expected = true;
      propagateError = true;
    };

    "provider-device-security-key/guest-frontend-contract" = {
      expr = {
        uhid = builtins.elem "uhid" config.boot.kernelModules;
        plugdev = builtins.hasAttr "plugdev" config.users.groups;
        restart = frontend.serviceConfig.Restart;
        protected = frontend.serviceConfig.NoNewPrivileges;
      };
      expected = {
        uhid = true;
        plugdev = true;
        restart = "always";
        protected = true;
      };
    };
  };
}
