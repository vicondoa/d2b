# How to migrate a NixOS host from systemd-owned to daemon-owned VM lifecycle

> **v1.0 status:** This guide documents the pre-P6 → P6 transition.
> In v1.0 (per [ADR 0015](../adr/0015-daemon-only-clean-break.md))
> the pre-P6 `nixling@<vm>.service` wrapper was retired entirely;
> there is no longer a coexistence period. New hosts deploy
> straight to v1.0 daemon-owned lifecycle. Existing v0.x hosts
> upgrading to v1.0 should follow
> [`migrate-nixling-v0-to-v1.md`](./migrate-nixling-v0-to-v1.md)
> instead of this guide; this file is preserved as historical
> record of how the per-VM migration worked during the
> incremental v0.x → v1.0 development sequence.

This guide moves a NixOS host from the legacy per-VM `nixling@<vm>.service`
ownership model to `nixlingd`-owned per-VM lifecycle.

The safest pattern is **one VM at a time**.

## 1. Confirm the host is ready

Before switching any VM, make sure all of these are true:

- the host already boots the VM successfully under the legacy `systemd`
  supervisor path;
- you know which users belong in `nixling.site.launcherUsers` and
  `nixling.site.adminUsers`;
- you have a rollback path (`nixos-rebuild --rollback` or a known-good boot
  generation).

## 2. Enable the daemon control plane

Add the daemon gate and user lists first:

```nix
{
  nixling = {
    daemonExperimental.enable = true;
    site.launcherUsers = [ "alice" ];
    site.adminUsers = [ "alice" ];
  };
}
```

`supervisor = "nixlingd"` is rejected unless `daemonExperimental.enable = true`.

## 3. Move one VM to the daemon-owned supervisor

For the VM you want to migrate first:

```nix
{
  nixling.vms.work = {
    enable = true;
    supervisor = "nixlingd";
  };
}
```

The default is `supervisor = "systemd"`, so this is the per-VM switch that
changes ownership.

## 4. Rebuild the host

```bash
sudo nixos-rebuild switch
```

Then verify the daemon surface itself:

```bash
nixling auth status --json
systemctl status nixlingd.service --no-pager
```

## 5. Stop the old instance if it is still running

After the rebuild, the old `nixling@<vm>.service` instance is no longer the
owner for that VM, but a previously started instance may still be alive. Stop it
before the first daemon-owned start:

```bash
sudo systemctl stop nixling@work.service microvm@work.service
```

## 6. Dry-run, then start through the daemon

```bash
nixling vm start work --dry-run --json
sudo nixling vm start work --apply
```

For the first migration boot, prefer explicit `nixling vm start` / `restart`
commands over relying on the old `nixling@<vm>.service` autostart path.

## 7. Validate the migrated VM

Use per-VM checks rather than assuming the still-stubbed inventory surfaces are
complete:

```bash
nixling status work
nixling trust work
```

If a verb is daemon-deferred or the daemon is unreachable, you will
see a typed envelope (`not-yet-implemented` exit 78 / `daemon-down`
exit 1) rather than any silent bash invocation; the historical
`NIXLING_NATIVE_ONLY=1` knob is now a no-op (always-on default).

## 8. Roll back if needed

To hand the VM back to the legacy owner:

```nix
{
  nixling.vms.work.supervisor = "systemd";
}
```

Then rebuild again:

```bash
sudo nixos-rebuild switch
```

If you want to unwind the daemon rollout completely, also set
`nixling.daemonExperimental.enable = false` once every migrated VM is back on
`systemd`.

## See also

- [`uninstall-nixling.md`](./uninstall-nixling.md)
- [`headless-alpha-walkthrough.md`](./headless-alpha-walkthrough.md)
- [`../explanation/default-switch-and-deprecation.md`](../explanation/default-switch-and-deprecation.md)
