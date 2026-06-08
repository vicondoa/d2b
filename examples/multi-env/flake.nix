{
  description = "nixling example: two isolated envs (work + personal) demonstrating per-env network separation";

  # Consume nixling as a path input so this example works without
  # pinning a tag. In a real consumer flake you'd write:
  #   nixling.url = "github:vicondoa/nixling/v0.1.0";
  # Nixpkgs and microvm.nix come through nixling's own inputs so the
  # consumer doesn't have to pin them separately.
  inputs.nixling.url = "path:../..";

  outputs = { self, nixling }: {
    nixosConfigurations.demo = nixling.inputs.nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        nixling.nixosModules.default
        ./configuration.nix
      ];
    };
  };
}
