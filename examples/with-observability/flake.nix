{
  description = "d2b example: one workload VM plus the auto-declared native SigNoz observability stack";

  inputs = {
    # Pin d2b to a published release tag for real-world use:
    #
    #   d2b.url = "github:vicondoa/d2b/v0.2.0";
    #
    # The relative `path:../..` reference here is what makes this
    # example evaluate against the in-tree framework so
    # `nix flake check` runs without a network or a published tag.
    d2b.url = "path:../..";

    # Share d2b's pinned nixpkgs so option types line up between
    # the framework and your top-level NixOS config.
    nixpkgs.follows = "d2b/nixpkgs";
  };

  outputs = { self, nixpkgs, d2b, ... }: {
    nixosConfigurations.demo = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        d2b.nixosModules.default
        ./configuration.nix
      ];
    };
  };
}
