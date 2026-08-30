{
  description = "d2b example: Zone Guest reserved for external identity integration";

  inputs.d2b.url = "path:../..";

  outputs = { d2b, ... }: {
    nixosConfigurations.demo = d2b.inputs.nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        d2b.nixosModules.default
        ./configuration.nix
      ];
    };
  };
}
