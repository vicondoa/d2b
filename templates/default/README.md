# d2b host template

Initialize a consumer flake with:

```bash
nix flake init -t github:vicondoa/d2b
```

Before evaluating, replace the `TODO-*` host, user, key, filesystem, and
bootloader values in `configuration.nix`. Choose non-overlapping LAN and uplink
CIDRs for the example realm.

The template declares:

- one host-local `work` realm;
- a realm-owned declared network;
- one Cloud Hypervisor runtime provider; and
- one `corp-vm` workload whose `config` is evaluated as guest NixOS.

Add workloads under `d2b.realms.<realm>.workloads`, and bind each workload to
an enabled provider in the same realm. Add independent trust boundaries as
separate realms rather than sharing a network or controller.

Validate before switching:

```bash
nix flake check
sudo nixos-rebuild test --flake .#host
sudo nixos-rebuild switch --flake .#host
```

PID1 owns only the fixed local-root controller and broker endpoint set. Child
realm controllers and brokers are parent-spawned and pidfd-supervised; workload
lifecycle does not create per-workload systemd units.

For desktop workloads, provider capabilities, and sibling identity modules,
see the examples and reference documentation in the d2b repository.
