{
  description = "nixling example: one workload VM plus the auto-declared observability stack (Grafana/Prometheus/Loki/Tempo/Alloy)";

  inputs = {
    # Pin nixling to a published release tag for real-world use:
    #
    #   nixling.url = "github:vicondoa/nixling/v0.2.0";
    #
    # The relative `path:../..` reference here is what makes this
    # example evaluate against the in-tree framework so
    # `nix flake check` runs without a network or a published tag.
    nixling.url = "path:../..";

    # Share nixling's pinned nixpkgs so option types line up between
    # the framework and your top-level NixOS config.
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
