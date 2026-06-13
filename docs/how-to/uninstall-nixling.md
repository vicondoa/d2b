# How to uninstall or roll back nixling

Use the smallest rollback that solves the problem:

- **bad VM generation only** -> use `nixling rollback <vm>`;
- **daemon-owned lifecycle issue** -> stop the VM (`nixling vm stop <vm>`),
  or stop/disable `nixlingd`, or remove the VM from your host config;
- **full framework removal** -> remove nixling from host config / host-install
  artifacts and then delete state only after backup.

## Before deleting anything

Back up or intentionally discard these first:

- `/var/lib/nixling/audit/`
- `nixling.site.keysDir` (default `/var/lib/nixling/keys`)
- `/var/lib/nixling/known_hosts.nixling`
- `/var/lib/nixling/vms/<vm>/swtpm/`
- `/var/lib/nixling/vms/<vm>/store-meta/generations/`

Do **not** wipe `swtpm/` casually. That is a guest identity reset, not routine
cleanup.

## Roll back a single VM generation

If the framework itself is fine and only one VM's current generation is bad:

```bash
sudo nixling rollback <vm> --apply
nixling status <vm>
```

That keeps nixling installed and only moves the VM back to its prior retained
generation.

## Stop daemon-owned lifecycle on NixOS

In v1.1 every VM is daemon-supervised by `nixlingd` — there is no
`nixling.vms.<vm>.supervisor` option to switch back to a systemd
backend (the option was removed in v1.1 per ADR 0015; setting it fails
eval). To stop using `nixlingd` as the VM owner without fully
uninstalling:

1. stop the running VMs:

   ```bash
   sudo nixling vm stop work --apply
   ```

2. stop (and, if desired, mask) the daemon so it does not re-reconcile:

   ```bash
   sudo systemctl stop nixlingd.service
   ```

   There are no per-VM `nixling@<vm>.service` / `microvm@<vm>.service`
   units to stop — the daemon supervises every VM in-process.

3. to keep the VM declared but not auto-started, set
   `nixling.vms.<vm>.autostart = false` and rebuild:

   ```bash
   sudo nixos-rebuild switch
   ```

   To remove a VM entirely, delete its `nixling.vms.<vm>` block from
   your host config and rebuild.

## Fully uninstall from a NixOS host

1. remove the nixling module import and nixling-specific configuration from your
   NixOS host config;
2. rebuild to a known-good non-nixling generation:

   ```bash
   sudo nixos-rebuild switch
   ```

3. verify the framework-owned units are gone or inactive:

   ```bash
   systemctl list-units --type=service 'nixling*' --no-pager
   ```

4. once you are sure you do not need recovery data, delete state deliberately:

   ```bash
   sudo rm -rf /var/lib/nixling
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
   sudo systemctl stop nixlingd.service || true
   sudo systemctl disable nixlingd.service || true
   ```

2. remove the installed artifacts after backing up anything you still need:

   ```bash
   sudo rm -f /etc/systemd/system/nixlingd.service
   sudo rm -f /etc/nixling/daemon-config.json
   sudo rm -f /var/lib/nixling/runtime/host-runtime.json
   sudo systemctl daemon-reload
   ```

3. if you installed the CLI with a Nix profile, remove it too:

   ```bash
   nix profile remove github:vicondoa/nixling#nixling
   ```

4. only then delete `/var/lib/nixling/` if you really want to discard audit,
   key, and VM state.

## Aftercare checklist

After any uninstall or rollback, confirm:

```bash
nixling auth status --json || true
systemctl list-units --type=service 'nixling*' --no-pager
```

If the command still reaches a daemon you thought you removed, stop there and
inspect the remaining unit/config artifacts before deleting more state.

## See also

- [`migrate-nixos-to-daemon.md`](./migrate-nixos-to-daemon.md)
- [`../reference/store-lifecycle.md`](../reference/store-lifecycle.md)
- [`../reference/key-lifecycle.md`](../reference/key-lifecycle.md)
- [`../reference/security-runbook.md`](../reference/security-runbook.md)
