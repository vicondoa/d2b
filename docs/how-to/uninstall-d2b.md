# How to uninstall or roll back d2b

Use the smallest rollback that solves the problem:

- **bad VM generation only** -> use `d2b rollback <vm>`;
- **daemon-owned lifecycle issue** -> stop the VM (`d2b vm stop <vm>`),
  or stop/disable `d2bd`, or remove the VM from your host config;
- **full framework removal** -> remove d2b from host config / host-install
  artifacts and then delete state only after backup.

## Before deleting anything

Back up or intentionally discard these first:

- `/var/lib/d2b/audit/`
- `d2b.site.keysDir` (default `/var/lib/d2b/keys`)
- `/var/lib/d2b/known_hosts.d2b`
- `/var/lib/d2b/vms/<vm>/swtpm/`
- `/var/lib/d2b/vms/<vm>/store-meta/generations/`

Do **not** wipe `swtpm/` casually. That is a guest identity reset, not routine
cleanup.

## Roll back a single VM generation

If the framework itself is fine and only one VM's current generation is bad:

```bash
sudo d2b rollback <vm> --apply
d2b status <vm>
```

That keeps d2b installed and only moves the VM back to its prior retained
generation.

## Stop daemon-owned lifecycle on NixOS

In v1.1 every VM is daemon-supervised by `d2bd` — there is no
`d2b.vms.<vm>.supervisor` option to switch back to a systemd
backend (the option was removed in v1.1 per ADR 0015; setting it fails
eval). To stop using `d2bd` as the VM owner without fully
uninstalling:

1. stop the running VMs:

   ```bash
   sudo d2b vm stop work --apply
   ```

2. stop (and, if desired, mask) the daemon so it does not re-reconcile:

   ```bash
   sudo systemctl stop d2bd.service
   ```

   There are no per-VM `d2b@<vm>.service` / `microvm@<vm>.service`
   units to stop — the daemon supervises every VM in-process.

3. to keep the VM declared but not auto-started, set
   `d2b.vms.<vm>.autostart = false` and rebuild:

   ```bash
   sudo nixos-rebuild switch
   ```

   To remove a VM entirely, delete its `d2b.vms.<vm>` block from
   your host config and rebuild.

## Fully uninstall from a NixOS host

1. remove the d2b module import and d2b-specific configuration from your
   NixOS host config;
2. rebuild to a known-good non-d2b generation:

   ```bash
   sudo nixos-rebuild switch
   ```

3. verify the framework-owned units are gone or inactive:

   ```bash
   systemctl list-units --type=service 'd2b*' --no-pager
   ```

4. once you are sure you do not need recovery data, delete state deliberately:

   ```bash
   sudo rm -rf /var/lib/d2b
   ```

5. optionally prune old host generations and unreferenced store paths:

   ```bash
   sudo nix-collect-garbage --delete-older-than 7d
   ```

## Uninstall the non-NixOS host-install scaffold

The public `host destroy --apply` path is still a separately staged command, so
manual cleanup is the current uninstall path on Ubuntu/Fedora-style installs.

1. stop and disable the service if it was enabled:

   ```bash
   sudo systemctl stop d2bd.service || true
   sudo systemctl disable d2bd.service || true
   ```

2. remove the installed artifacts after backing up anything you still need:

   ```bash
   sudo rm -f /etc/systemd/system/d2bd.service
   sudo rm -f /etc/d2b/daemon-config.json
   sudo rm -f /var/lib/d2b/runtime/host-runtime.json
   sudo systemctl daemon-reload
   ```

3. if you installed the CLI with a Nix profile, remove it too:

   ```bash
   nix profile remove github:vicondoa/d2b#d2b
   ```

4. only then delete `/var/lib/d2b/` if you really want to discard audit,
   key, and VM state.

## Aftercare checklist

After any uninstall or rollback, confirm:

```bash
d2b auth status --json || true
systemctl list-units --type=service 'd2b*' --no-pager
```

If the command still reaches a daemon you thought you removed, stop there and
inspect the remaining unit/config artifacts before deleting more state.

## See also

- [`migrate-nixos-to-daemon.md`](./migrate-nixos-to-daemon.md)
- [`../reference/store-lifecycle.md`](../reference/store-lifecycle.md)
- [`../reference/key-lifecycle.md`](../reference/key-lifecycle.md)
- [`../reference/security-runbook.md`](../reference/security-runbook.md)
