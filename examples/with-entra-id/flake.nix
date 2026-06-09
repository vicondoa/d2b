{
  description = "nixling + entrablau composition example (Entra-joined work VM)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # Local path during the in-flight refactor. Once both flakes
    # are tagged, downstream consumers should pin GitHub refs:
    #
    #   nixling.url   = "github:vicondoa/nixling";
    #   entrablau.url = "github:vicondoa/entrablau.nix/v1.0.0";
    #
    # The relative path here exists so `nix flake check` in this
    # subdirectory exercises the in-tree nixling sources without
    # needing a network fetch.
    nixling.url = "path:../..";
    nixling.inputs.nixpkgs.follows = "nixpkgs";

    entrablau.url = "github:vicondoa/entrablau.nix/v1.0.0";
    entrablau.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, nixling, entrablau, ... }@inputs: {
    nixosConfigurations.demo = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        nixling.nixosModules.default
        ./configuration.nix

        # Per-VM glue: register `work-entra` with nixling, hand its
        # NixOS config (including the entrablau module) to
        # the framework via `config.imports`. The two flakes know
        # nothing about each other; this attrset is where they meet.
        {
          nixling.vms.work-entra = {
            enable = true;
            tpm.enable = true;
            env = "work";
            index = 10;
            ssh.user = "alice";

            config = {
              imports = [
                entrablau.nixosModules.default
                ./work-entra.nix
              ];
            };
          };
        }
      ];
    };
  };
}
