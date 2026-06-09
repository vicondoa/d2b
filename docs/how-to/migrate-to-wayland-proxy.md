# Migrate to the host-side Wayland filter proxy

This guide covers the changes you need to make when a graphics VM
switches from the legacy in-guest `wayland-proxy-virtwl` path to the
new host-side Wayland filter proxy.

## What changes

### Guest proxy is replaced

The in-guest `wayland-proxy-virtwl` systemd user service is replaced
by `wl-cross-domain-proxy`.  The new proxy only bridges the
virtio-gpu cross-domain transport to the guest's Wayland socket; it
does not perform security filtering or app-id rewriting.  Those
responsibilities move to the host-side filter proxy, which runs as a
separate broker-spawned process outside the guest.

### App-ids are rewritten on the host

All guest window surfaces acquire the prefix `nixling.<vm>.` on
their `xdg_toplevel` `app_id`.  For example, `org.mozilla.firefox`
becomes `nixling.work.org.mozilla.firefox` when viewed from the host
compositor.

This affects any niri (or other compositor) rules you have written
that match by app-id.  Rules that matched `org.mozilla.firefox` must
be updated to match `nixling.work.org.mozilla.firefox` or the regex
pattern `^nixling\.work\.`.

### Window title prefix is retained

The window title prefix `[<vm>] ` behavior is preserved for
compositors that rely on title-based VM disambiguation.

### Xwayland must be disabled before the migration

`graphics.xwayland.enable = true` is not supported during the
Wayland-only migration phase.  Set `graphics.xwayland.enable = false`
(or remove the option — false is the default) before switching.

The central proxy wiring will add a hard eval assertion that rejects
`graphics.xwayland.enable = true`.  Until then, treat the option as an
unsupported legacy path during this migration rather than relying on it to
work.

Future work will add a validated Xwayland path.

### `crossDomainTrusted` is required for the proxy

The host-side filter proxy activates only when
`graphics.crossDomainTrusted = true`.  The default is still false.
The proxy path requires explicit opt-in because the cross-domain
virtio-gpu channel carries all guest Wayland messages and must be
trusted before the host filter can forward them.

## Step-by-step migration

### 1. Disable Xwayland if enabled

```nix
# before
nixling.vms.work.graphics.xwayland.enable = true;

# after (or remove the option entirely)
nixling.vms.work.graphics.xwayland.enable = false;
```

### 2. Enable cross-domain forwarding

```nix
nixling.vms.work.graphics.crossDomainTrusted = true;
```

> **Note:** Do not set `crossDomainTrusted = true` for VMs that run
> Docker with privileged containers.  A privileged-container escape
> inside such a VM could leverage the cross-domain channel to reach
> the host compositor.

### 3. Enable the Wayland filter (when available)

When `graphics.waylandFilter.enable` is wired by the central module,
enable it:

```nix
nixling.vms.work.graphics.waylandFilter.enable = true;
```

The filter is enabled by default for graphics VMs with
`crossDomainTrusted = true` once the module wiring lands; this step
is included here for clarity.

### 4. Update niri window rules (niri users)

Remove or update any niri `window-rule` blocks that match by the
original app-id:

```kdl
// before
window-rule {
    match app-id="org.mozilla.firefox"
    // ...
}

// after (match the prefixed form)
window-rule {
    match app-id=r#"^nixling\.work\.org\.mozilla\.firefox$"#
    // ...
}
```

Or use the generated include file (see
[Set up niri window borders for nixling VMs](./niri-vm-borders.md)):

```nix
nixling.site.niriVmBorders.enable = true;
```

```kdl
// config.kdl
include "/etc/nixling/niri-vm-borders.kdl"
```

The generated rules use the prefix regex `^nixling\.<vm>\.` so they
match all windows from a given VM regardless of their original
app-ids.

### 5. Restart the VM

Apply the configuration and restart the affected VM:

```bash
sudo nixos-rebuild switch
nixling down work --apply
nixling up work --apply
```

After `nixling up`, confirm the guest starts successfully and windows
appear on the host compositor.

## Verifying the migration

### Check the app-id prefix

Inspect running windows from a niri IPC session:

```bash
niri msg windows
```

Every window from the VM should have `app_id` starting with
`nixling.<vm>.`.  If the original app-id appears without the prefix,
confirm `crossDomainTrusted = true` and that the filter proxy is
running (`nixling vm status <vm>`).

### Check the host compositor socket ownership

After migration, the GPU runner should no longer hold a direct file
descriptor to the host compositor socket.  The Wayland filter proxy
should be the only VM-specific process with compositor socket access:

```bash
# Confirm the GPU runner has no compositor socket fd (should be empty)
ls -la /proc/$(pgrep -f "crosvm.*work")/fd 2>/dev/null \
  | grep wayland

# Confirm the proxy holds the compositor socket
ls -la /proc/$(pgrep -f "nixling-wayland-filter.*work")/fd 2>/dev/null \
  | grep wayland
```

## Understanding the warning model

The filter proxy's policy engine emits NixOS `warnings` when an
operator override changes a rule nixling considers required or
high-risk.  These warnings are advisory: the configuration still
evaluates and builds.  They are surfaced in `nixos-rebuild switch`
output and through the `nixling down/up --apply` path.

For a full list of warning conditions, see
[`docs/reference/wayland-filter-warnings.md`](../reference/wayland-filter-warnings.md).

## Rollback

If you encounter a regression and need to roll back before completing
the migration:

1. Set `graphics.crossDomainTrusted = false`.
   When the central filter option is available, also set
   `graphics.waylandFilter.enable = false`.
2. Run `sudo nixos-rebuild switch`.
3. Restart the VM: `nixling down <vm> --apply && nixling up <vm> --apply`.

The guest will revert to the previous proxy path.

## Known limitations at migration time

- **Xwayland is not supported.** Set `graphics.xwayland.enable = false`.
  The central proxy wiring will add a hard eval assertion for this
  unsupported state.
- **Multi-output enumeration** works through the filter; verify with
  `wayland-info` inside the guest if you use a multi-monitor setup.
- **Clipboard and DnD** are forwarded for standard protocols
  (`wl_data_device_manager`) by default; privileged clipboard-control
  globals remain opt-in.
