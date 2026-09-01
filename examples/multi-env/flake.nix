{
  description = "d2b example: two isolated Zones with same-name Guest isolation";

  # Consume d2b as a path input so this example works without
  # pinning a tag. In a real consumer flake you'd write:
  #   d2b.url = "github:vicondoa/d2b/v0.1.0";
  # Nixpkgs comes through d2b's own inputs so the consumer doesn't
  # have to pin it separately.
  inputs.d2b.url = "path:../..";

  outputs = { self, d2b }: {
    nixosConfigurations.demo = d2b.inputs.nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        d2b.nixosModules.default
        ./configuration.nix
      ];
    };
  };
}
