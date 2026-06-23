# Migrate nixling v1.2 to v1.3

## TL;DR — required steps for operators

For most v1.2 hosts the upgrade is two commands:

```bash
sudo nixos-rebuild switch
sudo systemctl restart nixlingd.service
```

After these two steps, all VMs continue working with no further
operator action. v1.3 is a hardening + robustness release; there is
no consumer-facing option rename and no required configuration change.

## What changed

### TPM (`tpm.enable`) first-run is now self-provisioning

In v1.2, enabling `nixling.vms.<vm>.tpm.enable = true` on a VM that had
**no pre-existing** `/var/lib/nixling/vms/<vm>/swtpm` state directory
would wedge that VM's start: swtpm died with a fatal NVRAM `ENOENT`
because nothing materialized the per-VM swtpm state directory in the
daemon/broker model, and the start hung instead of failing cleanly.

v1.3 provisions and hardens the per-VM swtpm state directory
automatically, at VM start, through the privileged broker:

- The directory is created (owned by the per-VM `nixling-<vm>-swtpm`
  principal, mode `0700`) on first start. **The manual
  `install -d -o nixling-<vm>-swtpm … /var/lib/nixling/vms/<vm>/swtpm`
  workaround is no longer needed** — remove it from any operator
  runbooks.
- The per-VM state root `/var/lib/nixling/vms/<vm>/` is now `3770`
  (setgid **+ sticky**). The sticky bit prevents a non-owner per-VM
  role UID from renaming or replacing the principal-owned `swtpm`
  directory. `setgid` (group inheritance) is unchanged; the activation
  re-stamps the mode on the next `nixos-rebuild switch`, so existing
  `2770` roots converge automatically.
- The swtpm control socket readiness now waits for an active
  *listener* rather than the mere presence of the socket inode, so a
  failed swtpm surfaces as a fast, typed start error instead of a long
  hang.

### TPM state-loss is now fail-closed (anti-tamper)

If a VM was previously provisioned with TPM state and that state
directory later goes **missing or is replaced**, the broker now
**fails the VM start closed** with
`previously-provisioned-swtpm-state-missing` rather than silently
recreating an empty TPM (which would force IdP re-enrollment and look
like device tampering).

If you hit this and the loss was **unintentional**: restore the
original `/var/lib/nixling/vms/<vm>/swtpm` NVRAM contents from backup,
then start the VM. Creating an empty directory, a recursive `chown`,
or a bare `nixos-rebuild` is **not** recovery — the TPM NVRAM + EK
seed are irreplaceable and their absence is treated as tampering.

If the TPM reset was **intentional** (you are deliberately wiping the
device's TPM): follow your IdP's explicit TPM reset / re-enrollment
procedure for that VM, which is also what clears the fail-closed
state.

### `bundleVersion` 4 → 5

The trusted manifest-bundle schema version bumps from `4` to `5` for a
new audited broker operation (`PrepareSwtpmDir`). This is internal to
the host substrate; consumers that only use the public
`nixlingModules` surface need no change. If you vendor or pin bundle
artifacts directly, regenerate them after the upgrade.

### Daemon robustness

The daemon now handles client connections concurrently (bounded), so a
slow or failing VM start no longer stalls unrelated clients (for
example a host status feed). A required per-VM runner that dies during
start now fails the start fast with an actionable, typed error instead
of hanging.

### VM stop is graceful by default

Normal `nixling vm stop <vm> --apply`, `down`, and `restart` now ask
supported local guests to shut down before host-side VMM termination.
Cloud Hypervisor VMs use the CH shutdown API; qemu-media VMs use
broker-mediated QMP `system_powerdown`. The default wait is 90 seconds,
then nixling falls back to the previous SIGTERM/SIGKILL cleanup path.

No configuration change is required. If a guest intentionally cannot
respond to graceful shutdown, opt out globally:

```nix
nixling.daemon.lifecycle.gracefulShutdown.enable = false;
```

or for one VM:

```nix
nixling.vms.<vm>.lifecycle.gracefulShutdown.enable = false;
```

Use `timeoutSeconds = 1..600` globally or per VM to tune slow shutdowns.
For emergency operations, `nixling vm stop <vm> --force --apply` skips
only the graceful wait; it is not an immediate SIGKILL shortcut.

## What consumers must change

Nothing required. Remove any manual swtpm-directory provisioning
workaround from operator runbooks (see above); it is now handled by
the framework.
