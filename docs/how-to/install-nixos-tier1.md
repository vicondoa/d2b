# Install d2b on NixOS

This is the current module-first path for a host whose consumer configuration
lives under `/etc/nixos`.

## 1. Add the flake input

Import `d2b.nixosModules.default` from the host flake and keep its `nixpkgs`
input aligned:

```nix
inputs.d2b.inputs.nixpkgs.follows = "nixpkgs";
```

For a new consumer, start with:

```bash
nix flake init -t github:vicondoa/d2b
```

Declare Zone-owned Guest resources and their immutable artifacts. Do not add
the retired v1/v2 hierarchy.

## 2. Build and switch

```bash
sudo nixos-rebuild build --flake /etc/nixos#desktop
sudo nixos-rebuild dry-activate --flake /etc/nixos#desktop
sudo nixos-rebuild switch --flake /etc/nixos#desktop
```

The host switch installs the three-unit d2b control plane:

```text
d2bd.service
d2b-broker.socket
d2b-broker.service
```

## 3. Start a Guest

```bash
d2b host check --json
d2b host doctor --read-only
d2b guest list --zone local-root
d2b guest start <name> --zone <zone> --dry-run
d2b guest start <name> --zone <zone> --apply
```

The Guest controller creates and reconciles child Resources. Cloud Hypervisor
boot, restart adoption, deletion, and real-host cleanup are host-lane
acceptance work; U19 does not claim that evidence.

## Clean-break cleanup

The current line does not promise v1/v2 state migration or data retention.
After the new host generation is healthy, remove obsolete declarations and old
host-path state according to the operator's cleanup decision. Do not treat
historical migration pages as a supported preservation or rollback procedure.

See [host preparation](./host-prepare.md),
[Zone Nix authoring](../reference/zone-control-nix.md), and
[the compatibility policy](../reference/compatibility.md).
