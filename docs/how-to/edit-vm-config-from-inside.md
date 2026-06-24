# Edit a VM's config from inside the VM

nixling normally treats each VM's config as host-owned: you declare it
in your host config (`nixling.vms.<vm>.config`), build it on the host,
and the guest boots a read-only closure. That keeps the trusted host in
control of the runner substrate (mounts, devices, hypervisor args, …).

But sometimes you want to iterate on what's *installed inside* a VM
from inside the VM, then persist that change on the host with review.
That's what `guestConfigFile` + `nixling config` are for.

## The split: host-owned vs guest-editable

| Concern | Where it lives | Editable in VM? |
| --- | --- | --- |
| Mounts / `microvm.*` runner substrate, `nixling.*` framework, env, components | host-owned `nixling.vms.<vm>.config` | no |
| Installed software: `environment.systemPackages`, `services.*`, in-guest `users.users.*`, `programs.*`, files, desktop | **`nixling.vms.<vm>.guestConfigFile`** | **yes** |

Both merge into the single per-VM closure the guest boots, so the
guest-editable layer genuinely runs in the VM. The guest-editable file
is **contained**: if it tries to set any host-owned `microvm.*` /
`nixling.*` option, the host rebuild fails with a clear assertion. The
guest can change its own OS, never the host's control of it.

## One-time setup

Point a VM at a dedicated guest file and move the in-VM software layer
into it:

```nix
# host config
nixling.vms.work.guestConfigFile = ./vms/work.guest.nix;
```

```nix
# ./vms/work.guest.nix — only guest OS options
{ ... }:
{
  environment.systemPackages = [ ];   # add your packages
  services.openssh.enable = true;
  # microvm.* / nixling.* here would FAIL the build (contained).
}
```

Rebuild the host once (`nixling switch work --apply`). The guest now carries:

- `/etc/nixling/guest-config.nix` — a **read-only** copy of the current
  approved guest config (always reflects what's live).
- `/var/lib/nixling-guest/guest-config.nix` — a **writable** working
  copy, seeded once from the baseline. It is owned by the VM's
  `ssh.user` when one is declared, and by `root` otherwise (the
  guest-control exec path can edit it either way).

### Prerequisite: the guest-control channel

`config sync` reads the edited file over the authenticated
**guest-control** vsock — the daemon's `ReadGuestConfig` →
guestd `ReadGuestFile` path — not over SSH. It is wired exactly when
the VM both enables guest-control and declares a `guestConfigFile`:

```nix
nixling.vms.work.guest.control.enable = true;   # the guest-control credential + guestd service
nixling.vms.work.guestConfigFile = ./vms/work.guest.nix;
```

With those set, guestd advertises the `ReadGuestFile` capability and
serves a bounded read of exactly the working-copy path. Without them
the capability stays absent and `config sync` **fails closed** with a
typed error — it never falls back to SSH. `ssh.user` is **not**
required for sync; it only chooses a non-root owner for the writable
working copy (and remains the in-VM account you edit as when you reach
the VM over SSH/console):

```nix
nixling.vms.work.ssh.user = "alice";   # optional: the in-VM account that owns the writable copy
```

When `ssh.user` is unset the working copy is owned by `root`.

## The edit → sync → review → approve loop

1. **Edit inside the VM.** SSH/console into the VM and edit the writable
   working copy:

   ```bash
   $EDITOR /var/lib/nixling-guest/guest-config.nix
   ```

2. **Sync it back to the host (on-demand).** From the host:

   ```bash
   nixling config sync work
   ```

   This pulls the edited file over the authenticated guest-control
   channel into a host-side staging copy
   (`~/.local/state/nixling/config-staging/work.guest.nix`). The host
   treats it as untrusted data — nothing is evaluated yet.

3. **Review the change.**

   ```bash
   nixling config diff work --against ./vms/work.guest.nix
   ```

4. **Approve (or reject).** Approve writes the staged copy onto your
   guest file:

   ```bash
   nixling config approve work --to ./vms/work.guest.nix
   # or, to discard:
   nixling config reject work
   ```

   `approve` is atomic and only validates the bytes; the **real**
   containment + eval gate is the next step.

5. **Build + activate.**

   ```bash
   nixling switch work --apply
   ```

   The `guestConfigFile` containment assertion runs during this eval —
   a change that reached for a host-owned option is rejected here,
   before anything is built or activated.

## You can also build on the host

Nothing forces the in-VM loop. Editing `./vms/work.guest.nix` directly
on the host and running `nixling switch work --apply` works exactly the same —
the same file, the same containment. The in-VM loop is just an
ergonomic way to iterate from inside the workspace.

## Status

`nixling config status --all` lists VMs with a pending (un-approved)
staged config. `nixling status` and `nixling vm start` also print a
note when a VM has a pending edit (human output only), so an in-progress
edit isn't silently forgotten before you approve it.

## Notes

- The CLI never auto-writes your config tree: `approve` only writes the
  `--to` path you name. It never touches anything you don't point it at.
- `config sync` is host-initiated (the host reads the guest's working
  copy over the authenticated guest-control vsock). The guest never
  initiates a connection to the host control plane, and there is no new
  socket or virtiofs share.
- If `/var` is not persistent in your VM, the writable working copy is
  re-seeded from the read-only baseline on each boot.
