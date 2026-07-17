# Compose an Entra-enabled realm workload

This example composes d2b with the sibling
[`vicondoa/entrablau.nix`](https://github.com/vicondoa/entrablau.nix) flake.
d2b owns the host-local realm, provider binding, and workload lifecycle;
entrablau owns the guest identity configuration.

The composition seam is the workload's deferred NixOS module:

```nix
d2b.realms.work.workloads.work-entra = {
  provider = "runtime";
  config.imports = [
    entrablau.nixosModules.default
    ./work-entra.nix
  ];
};
```

`configuration.nix` declares the `work` realm, its network boundary, and its
Cloud Hypervisor runtime provider. `work-entra.nix` contains only guest NixOS
configuration and the `entrablau.*` options. d2b core does not import or depend
on the identity flake.

## Configure

Replace the placeholder Entra tenant/domain values in `work-entra.nix`, provide
the host's hardware configuration, and pin both flakes. Keep the shared nixpkgs
revision:

```nix
d2b.inputs.nixpkgs.follows = "nixpkgs";
entrablau.inputs.nixpkgs.follows = "nixpkgs";
```

Then evaluate the example:

```bash
nix flake check --no-build --all-systems --no-write-lock-file
```

See the entrablau documentation for tenant prerequisites and enrollment. Realm
access remains governed by d2b's local endpoint credentials and allowed-user
policy; identity-provider credentials stay inside the workload.
