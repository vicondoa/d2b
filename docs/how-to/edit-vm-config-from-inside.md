# Edit a workload guest configuration

Guest configuration is part of the host-evaluated realm workload. Keep the
module in your host flake and reference it from `workloads.<name>.config`:

```nix
d2b.realms.work.workloads.dev = {
  provider = "runtime";
  config = import ./workloads/dev.nix;
};
```

For example, `workloads/dev.nix` can install packages and configure the guest:

```nix
{ pkgs, ... }:
{
  environment.systemPackages = [ pkgs.git pkgs.ripgrep ];
  services.openssh.enable = true;
  users.users.alice = {
    isNormalUser = true;
    uid = 1000;
  };
}
```

Edit that source from a trusted checkout, rebuild the host configuration, and
apply the workload:

```console
$ sudo nixos-rebuild switch --flake .#host
$ d2b switch dev.work.local-root.d2b --apply
```

The guest cannot replace its own evaluated module. This preserves the bundle
hash, closure-only store view, and realm-controller ownership of the process
DAG. Put mutable application data on declared workload volumes instead.
