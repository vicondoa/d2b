{
  description = "nixling example: two isolated envs (work + personal) demonstrating per-env network separation";

  # Consume nixling as a path input so this example works without
  # pinning a tag. In a real consumer flake you'd write:
  #   nixling.url = "github:vicondoa/nixling/v0.1.0";
  # Nixpkgs and microvm.nix come through nixling's own inputs so the
  # consumer doesn't have to pin them separately.
  inputs.nixling.url = "path:../..";

  outputs = { self, nixling }: {
    # Legacy variant — the v0.1.x / v0.2.x / v0.3.x Tier 0 path with
    # systemd-supervised microVMs. This is the unchanged historical
    # output and stays the default consumer reference.
    nixosConfigurations.demo = nixling.inputs.nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        nixling.nixosModules.default
        ./configuration.nix
      ];
    };

    # W3 daemon-backed variant — exercises the v0.4.0 per-env
    # `mtu` / `mssClamp` / `lan.allowEastWest` knobs together with
    # the site-level `allowUnsafeEastWest` acknowledgement, and
    # opts one VM into the experimental nixlingd supervisor
    # (Tier 0 mixed mode per plan §"W3 daemon-vs-legacy migration
    # boundary"). See ./README.md for the operator UX.
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

            # Gate the experimental nixlingd daemon. Required for
            # any VM with supervisor = "nixlingd".
            nixling.daemonExperimental.enable = true;

            # Per-env v0.4.0 network knobs on the `work` env:
            #   * MTU clamp to 1400 (tunneled uplink reference).
            #   * MSS clamp on the net VM's nft forward chain.
            #   * East-west between workload LAN ports — double
            #     opt-in with site.allowUnsafeEastWest above.
            nixling.envs.work.mtu = lib.mkForce 1400;
            nixling.envs.work.mssClamp = lib.mkForce true;
            nixling.envs.work.lan.allowEastWest = lib.mkForce true;

            # Mixed Tier 0 mode: one VM keeps the legacy systemd
            # path, one VM opts into the experimental nixlingd
            # supervisor. The NixOS module skips emitting per-VM
            # systemd autostart wiring + processes.json unit
            # references for the daemon-owned VM so the
            # single-writer invariant holds.
            nixling.vms.work-app.supervisor = lib.mkForce "nixlingd";
            nixling.vms.personal-app.supervisor = lib.mkForce "systemd";
          })
        ];
      };
  };
}
