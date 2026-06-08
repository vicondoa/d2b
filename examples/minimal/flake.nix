{
  description = "Minimal nixling example — one headless workload VM in one env";

  inputs = {
    # Pin nixling to a published release tag for real-world use:
    #
    #   nixling.url = "github:vicondoa/nixling/v0.1.0";
    #
    # The relative `path:../..` reference here is what makes this
    # example evaluate against the in-tree framework so
    # `nix flake check` runs without a network or a published tag.
    # Substitute the github:… URL above when you copy this layout
    # for your own host.
    nixling.url = "path:../..";

    # Share nixling's pinned nixpkgs so option types line up
    # between the framework and your top-level NixOS config. New
    # consumers should follow this pattern; pulling in an
    # unrelated nixpkgs is a common source of subtle eval errors.
    nixpkgs.follows = "nixling/nixpkgs";
  };

  outputs = { self, nixpkgs, nixling, ... }: {
    nixosConfigurations.demo = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        nixling.nixosModules.default
        ./configuration.nix
      ];
    };
  };
}
