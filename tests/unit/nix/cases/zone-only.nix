{ mkModuleEval, lib, flakeRoot, ... }:

let
  legacyConfig = name: {
    d2b = {
      ${name} = {
        sentinel = true;
      };
    };
  };

  unknownOption = name:
    builtins.tryEval (builtins.deepSeq
      (mkModuleEval [ (legacyConfig name) ]).config
      true);

  removedFiles = [
    "nixos-modules/options-envs.nix"
    "nixos-modules/options-realms.nix"
    "nixos-modules/options-realms-network.nix"
    "nixos-modules/options-realms-workloads.nix"
    "nixos-modules/options-vms.nix"
    "nixos-modules/options-gateway.nix"
    "nixos-modules/gateway-vm.nix"
    "nixos-modules/processes-json.nix"
    "nixos-modules/realm-controller-config-json.nix"
    "nixos-modules/realm-identity-config-json.nix"
  ];

  optionsSource = builtins.readFile
    (flakeRoot + "/nixos-modules/options.nix");
in
{
  "zone-only/removed-options-use-ordinary-unknown-option" = {
    expr = {
      envs = !(unknownOption "envs").success;
      realms = !(unknownOption "realms").success;
      vms = !(unknownOption "vms").success;
      gateways = !(unknownOption "gateways").success;
    };
    expected = {
      envs = true;
      realms = true;
      vms = true;
      gateways = true;
    };
  };

  "zone-only/legacy-modules-and-process-emitter-are-absent" = {
    expr = {
      files = lib.filter
        (path: builtins.pathExists (flakeRoot + "/${path}"))
        removedFiles;
      imports = lib.any
        (needle: lib.hasInfix needle optionsSource)
        [
          "options-envs.nix"
          "options-realms.nix"
          "options-vms.nix"
          "options-gateway.nix"
        ];
    };
    expected = {
      files = [ ];
      imports = false;
    };
  };
}
