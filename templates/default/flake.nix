{
  description = "TODO: short description of this host";

  inputs = {
    # Pin to the same nixpkgs channel nixling tracks. nixling itself
    # follows `nixos-unstable`; if you need a stable channel here,
    # remember to also override `nixling.inputs.nixpkgs.follows`.
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # The framework. Pin to a tagged release once one exists; pinning
    # to `main` (or any unstable ref) means every `nix flake update`
    # can move the API under you.
    nixling.url = "github:vicondoa/nixling";
    nixling.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, nixling, ... }:
    {
      # TODO: rename `desktop` to your host's NixOS configuration name.
      # You'll rebuild with:
      #
      #   sudo nixos-rebuild switch --flake .#<this-attr-name>
      #
      # (The attr name and the value of `networking.hostName` do not
      # have to match, but conventionally they do.)
      nixosConfigurations.desktop = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          # The framework. Brings in `nixling.site.*`, `nixling.envs.*`,
          # `nixling.vms.*`, host-side sidecar services, the per-VM
          # `/nix/store` hardlink farm, the `nixling` CLI, etc.
          nixling.nixosModules.default

          # Your host config — the file you edit next.
          ./configuration.nix
        ];
      };
    };
}
