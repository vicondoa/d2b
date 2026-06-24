{
  description = "nixling example: two isolated envs (work + personal) demonstrating per-env network separation";

  # Consume nixling as a path input so this example works without
  # pinning a tag. In a real consumer flake you'd write:
  #   nixling.url = "github:vicondoa/nixling/v0.1.0";
  # Nixpkgs comes through nixling's own inputs so the consumer doesn't
  # have to pin it separately.
  inputs.nixling.url = "path:../..";

  outputs = { self, nixling }: {
    # Base variant: two isolated envs with daemon-supervised VMs.
    nixosConfigurations.demo = nixling.inputs.nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        nixling.nixosModules.default
        ./configuration.nix
      ];
    };

    # Network-knob variant: exercises per-env `mtu`, `mssClamp`, and
    # `lan.allowEastWest` knobs together with the site-level
    # `allowUnsafeEastWest` acknowledgement. VM supervision is still
    # daemon-only; see ./README.md for the operator UX.
    nixosConfigurations.multi-env-daemon-experimental =
      nixling.inputs.nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          nixling.nixosModules.default
          ./configuration.nix
          ({ lib, ... }: {
            # Site-level acknowledgement that this host accepts
            # the relaxed east-west isolation for envs that
            # opt in below.
            nixling.site.allowUnsafeEastWest = true;

            # Per-env network knobs on the `work` env:
            #   * MTU clamp to 1400 (tunneled uplink reference).
            #   * MSS clamp on the net VM's nft forward chain.
            #   * East-west between workload LAN ports — double
            #     opt-in with site.allowUnsafeEastWest above.
            nixling.envs.work.mtu = lib.mkForce 1400;
            nixling.envs.work.mssClamp = lib.mkForce true;
            nixling.envs.work.lan.allowEastWest = lib.mkForce true;

            # Every enabled VM is daemon-supervised by default.
          })
        ];
      };
  };
}
