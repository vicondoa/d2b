{
  description = "d2b + entrablau composition example (Entra-joined work VM)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # Local path during the in-flight refactor. Once both flakes
    # are tagged, downstream consumers should pin GitHub refs:
    #
    #   d2b.url   = "github:vicondoa/d2b";
    #   entrablau.url = "github:vicondoa/entrablau.nix/v1.0.0";
    #
    # The relative path here exists so `nix flake check` in this
    # subdirectory exercises the in-tree d2b sources without
    # needing a network fetch.
    d2b.url = "path:../..";
    d2b.inputs.nixpkgs.follows = "nixpkgs";

    entrablau.url = "github:vicondoa/entrablau.nix/v1.0.0";
    entrablau.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, d2b, entrablau, ... }@inputs: {
    nixosConfigurations.demo = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        d2b.nixosModules.default
        ./configuration.nix

        # Realm workload glue: register `work-entra` with d2b, hand its
        # NixOS config (including the entrablau module) to
        # the framework via `config.imports`. The two flakes know
        # nothing about each other; this attrset is where they meet.
        {
          d2b.realms.work.workloads.work-entra = {
            providerRefs = {
              runtime = "runtime";
              device = "devices";
              network = "network";
              storage = "storage";
            };
            tpm.enable = true;
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
