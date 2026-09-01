# d2b Zone host template

This scaffold declares one `Guest/corp-vm` in `Zone/local-root` using the
current controller-owned lifecycle. Replace the TODO values and placeholder
artifacts before switching a real host.

## Required edits

1. Replace the hostname and host account.
2. Add at least one SSH public key to `d2b.site.userAuthorizedKeys`.
3. Replace `guest-system` with the Guest evaluator's
   `system.build.toplevel`.
4. Replace `cloud-hypervisor-provider` with the signed runtime Provider
   artifact and catalog entry.
5. Review `waylandUser`, launcher/admin membership, host LAN CIDRs, and
   hardware settings for the target host.

The Guest system evaluator is consumer-owned. The Guest controller creates and
reconciles its direct child Resources; specialized controllers own process,
storage, network, device, and session effects.

## Build and use

```bash
nixos-rebuild build --flake .#desktop
nixos-rebuild switch --flake .#desktop
d2b guest list --zone local-root
d2b guest start corp-vm --zone local-root --apply
```

Use `d2b guest status`, `d2b process list`, and `d2b host doctor` to inspect
readiness. Lifecycle commands require `--dry-run` or `--apply`.
