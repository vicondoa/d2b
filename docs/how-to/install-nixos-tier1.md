# Install nixling on NixOS (Tier 1)

This is the module-first Tier-1 path for hosts that already run
NixOS. Prefer it over the generic host-install scaffold whenever you
control the host configuration directly.

## 1. Import the module

Add the nixling flake input, follow its pinned `nixpkgs`, and import
`nixling.nixosModules.default` in your host's `nixosSystem`.

The fastest scaffold is still:

```bash
nix flake init -t github:vicondoa/nixling
```

If you already own the host flake, follow the manual-integration
block in [`../../README.md`](../../README.md#manual-integration-without-the-template).

## 2. Build and switch

```bash
sudo nixos-rebuild build --flake .#desktop
sudo nixos-rebuild switch --flake .#desktop
```

On NixOS, this is the canonical install step: the framework's units,
sidecars, bundles, and CLI all land through the host generation.

## 3. Validate host-side prerequisites

Run at least:

```bash
nixling auth status --json
nixling host check --strict
nixling host doctor --read-only --json
```

If you are onboarding a non-trivial host or importing pre-existing
bridges / firewall state, work through
[`host-prepare.md`](./host-prepare.md) before turning on daemon-owned
lifecycle for more VMs.

## 4. Start the first VM with the Rust CLI

```bash
sudo nixling vm start personal-dev --apply
```

Drop `NIXLING_NATIVE_ONLY=1` if you still want the default
v1.0 daemon-only behavior (per ADR 0015; NIXLING_NATIVE_ONLY is a no-op for lifecycle verbs).

For the Entra showcase, the matching command is:

```bash
sudo nixling vm start work-entra --apply
```

## 5. Migrating an existing bash-era host

The on-disk VM state, store generations, managed keys, and
`known_hosts` data carry forward. Start with dry runs, then move the
host onto daemon-owned lifecycle with
[`migrate-nixos-to-daemon.md`](./migrate-nixos-to-daemon.md).

Roll back by rebuilding to the prior host generation (the v1.0
daemon-only contract per ADR 0015 has no env-var escape hatch;
`NIXLING_LEGACY_BASH_OPT_IN=1` was retired in P6 along with the
bash CLI).

## See also

- [`host-prepare.md`](./host-prepare.md) — generic Linux Tier-1
  onboarding and prerequisite reconciliation.
- [`migrate-nixos-to-daemon.md`](./migrate-nixos-to-daemon.md) —
  move an existing NixOS host from legacy systemd-owned VMs to
  `nixlingd`.
- [`install-ubuntu-2404.md`](./install-ubuntu-2404.md)
- [`install-fedora.md`](./install-fedora.md)
