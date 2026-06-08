{
  description = "nixling example: desktop workstation VM with graphics, audio, and YubiKey USBIP.";

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
    # Single x86_64-linux desktop host. The example deliberately
    # pins `system` here rather than parameterising over
    # `forAllSystems`: graphics + audio components transitively
    # depend on x86_64-only packages (pkgs/spectrum-ch,
    # pkgs/crosvm-patched, pkgs/vhost-device-sound), and the
    # framework's `checkVmPlatform` gate in `nixos-modules/host.nix`
    # throws an eval-time error if a VM with `graphics.enable` or
    # `audio.enable` is evaluated against a non-x86_64-linux host.
    # See README.md → "Why this example is x86_64-linux-only".
    nixosConfigurations.demo = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        nixling.nixosModules.default
        ./configuration.nix
      ];
    };
  };
}
