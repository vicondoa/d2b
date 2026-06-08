{
  description = "nixling + nixos-entra-id composition example (Entra-joined work VM)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # Local path during the in-flight refactor. Once both flakes
    # are tagged, downstream consumers should pin GitHub refs:
    #
    #   nixling.url        = "github:vicondoa/nixling/v0.1.0";
    #   nixos-entra-id.url = "github:vicondoa/nixos-entra-id/v0.1.0";
    #
    # The relative path here exists so `nix flake check` in this
    # subdirectory exercises the in-tree nixling sources without
    # needing a network fetch.
    nixling.url = "path:../..";
    nixling.inputs.nixpkgs.follows = "nixpkgs";

    nixos-entra-id.url = "github:vicondoa/nixos-entra-id";
    nixos-entra-id.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, nixling, nixos-entra-id, ... }@inputs: {
    nixosConfigurations.demo = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        nixling.nixosModules.default
        ./configuration.nix

        # Per-VM glue: register `work-entra` with nixling, hand its
        # NixOS config (including the nixos-entra-id module) to
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
                nixos-entra-id.nixosModules.default
                ./work-entra.nix
              ];
            };
          };
        }
      ];
    };
  };
}
