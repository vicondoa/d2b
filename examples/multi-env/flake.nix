{
  description = "d2b example: two isolated realm networks";

  # Consume d2b as a path input so this example works without
  # pinning a tag. In a real consumer flake you'd write:
  #   d2b.url = "github:vicondoa/d2b/v0.1.0";
  # Nixpkgs comes through d2b's own inputs so the consumer doesn't
  # have to pin it separately.
  inputs.d2b.url = "path:../..";

  outputs = { self, d2b }: {
    # Base variant: two isolated realms with controller-supervised workloads.
    nixosConfigurations.demo = d2b.inputs.nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        d2b.nixosModules.default
        ./configuration.nix
      ];
    };

    # Network-knob variant: exercises per-realm `mtu`, `mssClamp`, and
    # `lan.allowEastWest` knobs together with the site-level
    # `allowUnsafeEastWest` acknowledgement. VM supervision is still
    # daemon-only; see ./README.md for the operator UX.
    nixosConfigurations.multi-env-daemon-experimental =
      d2b.inputs.nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          d2b.nixosModules.default
          ./configuration.nix
          ({ lib, ... }: {
            # Site-level acknowledgement that this host accepts
            # the relaxed east-west isolation for realms that
            # opt in below.
            d2b.site.allowUnsafeEastWest = true;

            # Network knobs on the `work` realm:
            #   * MTU clamp to 1400 (tunneled uplink reference).
            #   * MSS clamp on the net VM's nft forward chain.
            #   * East-west between workload LAN ports — double
            #     opt-in with site.allowUnsafeEastWest above.
            d2b.realms.work.network.mtu = lib.mkForce 1400;
            d2b.realms.work.network.mssClamp = lib.mkForce true;
            d2b.realms.work.network.lan.allowEastWest = lib.mkForce true;

            # Every enabled VM is daemon-supervised by default.
          })
        ];
      };
  };
}
