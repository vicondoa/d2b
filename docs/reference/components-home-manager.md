# Home Manager guest component

Home Manager runs as a NixOS module inside a realm-owned guest. Select the
capability on the workload and configure guest users in its module:

```nix
d2b.realms.work.workloads.dev = {
  provider = "runtime";
  launcher.capabilities = [ "home-manager" ];
  config = {
    d2b.homeManager.users.alice = {
      home.stateVersion = "25.11";
      programs.git.enable = true;
    };
  };
};
```

## Guest options

| Option | Type | Default | Meaning |
| --- | --- | --- | --- |
| `d2b.homeManager.users` | attribute set | `{ }` | Home Manager modules keyed by guest user. |

The framework imports `inputs.home-manager.nixosModules.home-manager` only
for workloads carrying the `home-manager` capability. It applies these
defaults:

```nix
home-manager.useGlobalPkgs = true;
home-manager.useUserPackages = true;
```

The component changes only guest composition. It does not create host users,
services, workload systemd units, or additional controller processes.
