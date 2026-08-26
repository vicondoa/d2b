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
  projected = lib.evalModules {
    modules = [
      {
        options.d2b.zones = lib.mkOption {
          type = lib.types.attrs;
          default = { };
        };
        options.d2b._resourceCompiler = lib.mkOption {
          type = lib.types.attrs;
          default = { };
          internal = true;
          visible = false;
        };
      }
      (import ../default.nix)
      {
        config.d2b.zones.dev.resources = {
          device-security-key = { type = "Provider"; spec = { }; };
          guest = { type = "Guest"; spec = { }; };
          user = { type = "User"; spec = { }; };
          binding = {
            type = "security-key.d2bus.org.SecurityKeyBinding";
            spec = {
              providerRef = "Provider/device-security-key";
              target = {
                guestRef = "Guest/guest";
                userRef = "User/user";
              };
            };
          };
        };
      }
    ];
  };
  missingGuest = lib.evalModules {
    modules = [
      {
        options.d2b.zones = lib.mkOption {
          type = lib.types.attrs;
          default = { };
        };
        options.d2b._resourceCompiler = lib.mkOption {
          type = lib.types.attrs;
          default = { };
          internal = true;
          visible = false;
        };
      }
      (import ../default.nix)
      {
        config.d2b.zones.dev.resources = {
          device-security-key = { type = "Provider"; spec = { }; };
          user = { type = "User"; spec = { }; };
          binding = {
            type = "security-key.d2bus.org.SecurityKeyBinding";
            spec = {
              providerRef = "Provider/device-security-key";
              target.userRef = "User/user";
            };
          };
        };
      }
    ];
  };
  mixedBindings = lib.evalModules {
    modules = [
      {
        options.d2b.zones = lib.mkOption {
          type = lib.types.attrs;
          default = { };
        };
        options.d2b._resourceCompiler = lib.mkOption {
          type = lib.types.attrs;
          default = { };
          internal = true;
          visible = false;
        };
      }
      (import ../default.nix)
      {
        config.d2b.zones.dev.resources = {
          device-security-key = { type = "Provider"; spec = { }; };
          good-guest = { type = "Guest"; spec = { }; };
          user = { type = "User"; spec = { }; };
          binding-good = {
            type = "security-key.d2bus.org.SecurityKeyBinding";
            spec = {
              providerRef = "Provider/device-security-key";
              target.guestRef = "Guest/good-guest";
            };
          };
          binding-missing = {
            type = "security-key.d2bus.org.SecurityKeyBinding";
            spec = {
              providerRef = "Provider/device-security-key";
              target.userRef = "User/user";
            };
          };
        };
      }
    ];
  };
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
    "provider-device-security-key/projects-guest-frontend" = {
      expr = {
        enabled = projected.config.d2b._resourceCompiler
          .providerProjectionDeviceSecurityKey.enabled;
        process = projected.config.d2b._resourceCompiler
          .providerProjectionDeviceSecurityKey.processesByZone.dev
          ."security-key-binding".spec.template;
        endpoint = builtins.hasAttr "security-key-binding"
          projected.config.d2b._resourceCompiler
          .providerProjectionDeviceSecurityKey.resourcesByZone.dev;
        processRefs = projected.config.d2b._resourceCompiler
          .providerProjectionDeviceSecurityKey.privateArtifact.processRefs;
      };
      expected = {
        enabled = true;
        process = "security-key-frontend";
        endpoint = true;
        processRefs = [ "Process/security-key-binding" ];
      };
    };

    "provider-device-security-key/missing-guest-emits-no-children" = {
      expr = let
        projection = missingGuest.config.d2b._resourceCompiler
          .providerProjectionDeviceSecurityKey;
      in {
        enabled = projection.enabled;
        processes = projection.processesByZone or { };
        resources = projection.resourcesByZone.dev;
        processRefs = projection.privateArtifact.processRefs;
      };
      expected = {
        enabled = false;
        processes = { };
        resources = { };
        processRefs = [ ];
      };
    };

    "provider-device-security-key/mixed-bindings-have-no-dangling-endpoint" = {
      expr = let
        projection = mixedBindings.config.d2b._resourceCompiler
          .providerProjectionDeviceSecurityKey;
      in {
        processes = lib.attrNames (projection.processesByZone.dev);
        endpoints = lib.attrNames (projection.resourcesByZone.dev);
        processRefs = projection.privateArtifact.processRefs;
      };
      expected = {
        processes = [ "security-key-binding-good" ];
        endpoints = [ "security-key-binding-good" ];
        processRefs = [ "Process/security-key-binding-good" ];
      };
    };
  };
}
