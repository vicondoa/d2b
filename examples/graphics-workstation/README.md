# Zone Guest on a Wayland host

This example keeps the current Zone and Guest lifecycle while enabling the
host-side Wayland admission setting. Graphics, audio, and security-key
capabilities are added as Provider resources in the same Zone when their
contracts are available; they are not legacy Guest options.

The `Guest/corp-desktop` controller owns its child Resource graph and waits for
the selected Provider and Guest evaluator generations before reporting Ready.

## Evaluate

```bash
nix flake check --no-build --no-write-lock-file
```

## Operate

```bash
d2b guest status corp-desktop --zone local-root
d2b guest start corp-desktop --zone local-root --apply
d2b display list --zone local-root
```

The checked example uses placeholder artifacts and does not realize a desktop
image. Replace them with the signed Cloud Hypervisor and Guest artifacts used
by the host.
