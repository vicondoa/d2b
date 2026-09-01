{
  description = "d2b example: Wayland host with one Zone Guest.";

  inputs = {
    # Pin d2b to a published release tag for real-world use:
    #
    #   d2b.url = "github:vicondoa/d2b/v0.1.0";
    #
    # The relative `path:../..` reference here is what makes this
    # example evaluate against the in-tree framework so
    # `nix flake check` runs without a network or a published tag.
    # Substitute the github:… URL above when you copy this layout
    # for your own host.
    d2b.url = "path:../..";

    # Share d2b's pinned nixpkgs so option types line up
    # between the framework and your top-level NixOS config. New
    # consumers should follow this pattern; pulling in an
    # unrelated nixpkgs is a common source of subtle eval errors.
    nixpkgs.follows = "d2b/nixpkgs";
  };

  outputs = { self, nixpkgs, d2b, ... }: {
    # Single x86_64-linux desktop host. Provider packages and the
    # consumer Guest evaluator are selected by the Zone resource graph.
    nixosConfigurations.demo = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        d2b.nixosModules.default
        ./configuration.nix
      ];
    };
  };
}
