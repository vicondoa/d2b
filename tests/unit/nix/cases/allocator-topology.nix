{ lib, ... }:

let
  allocatorJson = import ../../../../nixos-modules/allocator-json.nix;
  minimal = { lib, ... }: {
    options.assertions = lib.mkOption {
      type = lib.types.listOf lib.types.anything;
      default = [ ];
    };
    options.d2b._index = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = {
        realms.enabledList = [ ];
        envMeta = { };
      };
    };
    options.d2b.site.stateDir = lib.mkOption {
      type = lib.types.str;
      default = "/var/lib/d2b";
    };
    options.d2b._zoneCompiler = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
    };
    options.d2b._bundle.allocatorJson = lib.mkOption {
      type = lib.types.anything;
      default = { };
    };
  };
  eval = extra:
    (lib.evalModules {
      modules = [
        minimal
        allocatorJson
        extra
      ];
    }).config;
in
{
  "allocator-topology/requires-explicit-root" = {
    expr = builtins.hasAttr "data" ((eval {
      d2b._zoneCompiler = lib.mkForce {
        topology = { };
      };
    }).d2b._bundle.allocatorJson);
    expected = false;
  };

  "allocator-topology/requires-explicit-parent-map" = {
    expr = builtins.hasAttr "data" ((eval {
      d2b._zoneCompiler = lib.mkForce {
        localRoot = "root";
      };
    }).d2b._bundle.allocatorJson);
    expected = false;
  };

  "allocator-topology/emits-a-non-local-root" =
    let
      cfg = eval {
        d2b._zoneCompiler = {
          localRoot = "root";
          topology = {
            root = { parentZone = null; };
            child = { parentZone = "root"; };
          };
        };
      };
    in {
      expr = cfg.d2b._bundle.allocatorJson.data.zoneTopology;
      expected = {
        root = "root";
        parentMap = {
          root = null;
          child = "root";
        };
      };
    };
}
