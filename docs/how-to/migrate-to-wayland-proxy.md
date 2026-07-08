# Migrate to the host-side Wayland proxy

This guide covers the changes you need to make when a graphics VM
switches from the legacy in-guest-only proxy path to the host-side
Wayland proxy.

## What changes

### Guest proxy is replaced

The old in-guest proxy service is replaced by `wl-cross-domain-proxy`.
The new proxy only bridges the
virtio-gpu cross-domain transport to the guest's Wayland socket; it
does not perform security filtering or app-id rewriting.  Those
responsibilities move to the host-side Wayland proxy, which runs as a
separate broker-spawned process outside the guest.

### App-ids are rewritten on the host

All guest window surfaces acquire the prefix `d2b.<vm>.` on
their `xdg_toplevel` `app_id`.  For example, `org.mozilla.firefox`
becomes `d2b.work.org.mozilla.firefox` when viewed from the host
compositor.

This affects any niri (or other compositor) rules you have written
that match by app-id.  Rules that matched `org.mozilla.firefox` must
be updated to match `d2b.work.org.mozilla.firefox` or the regex
pattern `^d2b\.work\.`.

The proxy process also receives a d2b-asserted canonical realm target
(`--realm-target <vm>.local.d2b` during the host-local transition). This
metadata is separate from guest-provided titles and app ids: downstream d2b
tools should prefer the d2b-provided realm target when they need trusted VM
identity, and treat rewritten app ids as presentation/routing hints only.

### Window title prefix is retained

The window title prefix `[<vm>] ` behavior is preserved for
compositors that rely on title-based VM disambiguation.

### Disable proxy-drawn borders when needed

When the host-side Wayland proxy is enabled, d2b wraps proxied Wayland
toplevels in a proxy-owned host-visible toplevel and draws a colored VM identity
rail on the left side by default, including qemu-media host windows routed
through the proxy. The color comes from the same `d2b.vms.<vm>.ui.border` model
used by generated compositor artifacts, and the default label is the
authenticated VM name. Guest buffers remain attached to the guest surface as an
embedded subsurface; the proxy-drawn rail uses only proxy-owned SHM buffers.

Disable the proxy-drawn border for a VM with:

```nix
d2b.vms.work.graphics.waylandProxy.border.enable = false;
```

Disable only the label while keeping the colored border with:

```nix
d2b.vms.work.graphics.waylandProxy.border.label.enable = false;
```

### Xwayland must be disabled before the migration

`graphics.xwayland.enable = true` is not supported during the
Wayland-only migration phase.  Setting it now fails eval with a clear
message.  Set `graphics.xwayland.enable = false` (or remove the option —
false is the default) before switching.

Future work will add a validated Xwayland path.

### `crossDomainTrusted` is required for the proxy

The host-side Wayland proxy activates only when
`graphics.crossDomainTrusted = true`.  The default is still false.
The proxy path requires explicit opt-in because the cross-domain
virtio-gpu channel carries all guest Wayland messages and must be
trusted before the host proxy can forward them.

## Step-by-step migration

### 1. Disable Xwayland if enabled

```nix
# before
d2b.vms.work.graphics.xwayland.enable = true;

# after (or remove the option entirely)
d2b.vms.work.graphics.xwayland.enable = false;
```

### 2. Enable cross-domain forwarding

```nix
d2b.vms.work.graphics.crossDomainTrusted = true;
```

> **Note:** Do not set `crossDomainTrusted = true` for VMs that run
> Docker with privileged containers.  A privileged-container escape
> inside such a VM could leverage the cross-domain channel to reach
> the host compositor.

### 3. Confirm the Wayland proxy is enabled

The Wayland proxy is enabled by default for graphics VMs with
`crossDomainTrusted = true`.  You can set it explicitly for clarity:

```nix
d2b.vms.work.graphics.waylandProxy.enable = true;
```

Leave it enabled unless you intentionally want the GPU sidecar to use the
direct compositor socket path.

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
    match app-id=r#"^d2b\.work\.org\.mozilla\.firefox$"#
    // ...
}
```

The proxy-drawn rail is the default VM identity border for proxied graphics and
qemu-media windows. Do not add compositor-native niri border rules for those
windows just to color their identity border; that creates a second outer border
around the proxy wrapper and no longer owns the VM identity color.

For intentionally non-proxied host windows only, use the generated include file
(see [Set up niri window borders for d2b VMs](./niri-vm-borders.md)):

```nix
d2b.site.ui.compositors.niri.enable = true;
```

```kdl
// config.kdl
include "/etc/d2b/niri-vm-borders.kdl"
```

The generated rules use the prefix regex `^d2b\.<vm>\.` for non-proxied VM
windows that need compositor-native borders. Proxied graphics and qemu-media VMs
already carry their identity border inside the proxy wrapper.

### 5. Restart the VM

Apply the configuration and restart the affected VM:

```bash
sudo nixos-rebuild switch
d2b vm stop work --apply
d2b vm start work --apply
```

After `d2b vm start`, confirm the guest starts successfully and windows
appear on the host compositor.

## Verifying the migration

### Check the app-id prefix

Inspect running windows from a niri IPC session:

```bash
niri msg windows
```

Every window from the VM should have `app_id` starting with
`d2b.<vm>.`.  If the original app-id appears without the prefix,
confirm `crossDomainTrusted = true` and that the Wayland proxy is
running (`d2b vm status <vm>`).

### Check the proxy-drawn border

Launch a Wayland client in the VM. The window should show a left-side rail using
the VM's resolved `ui.border.activeColor`/`inactiveColor`, and should show the VM
name label unless `graphics.waylandProxy.border.label.enable = false`.

The border is drawn from a proxy-owned decoration buffer. It does not require
the proxy to read or copy guest application buffers.

### Check the host compositor socket ownership

After migration, the GPU runner should no longer hold a direct file
descriptor to the host compositor socket.  The Wayland proxy
should be the only VM-specific process with compositor socket access:

```bash
# Confirm the GPU runner has no compositor socket fd (should be empty)
ls -la /proc/$(pgrep -f "crosvm.*work")/fd 2>/dev/null \
  | grep wayland

# Confirm the proxy holds the compositor socket
ls -la /proc/$(pgrep -f "d2b-wayland-proxy.*work")/fd 2>/dev/null \
  | grep wayland
```

## Understanding the warning model

The Wayland proxy's policy engine emits runtime advisory diagnostics when
an operator override changes a rule d2b considers required or
high-risk.  These warnings do not block evaluation or builds; they appear
in the `d2b-wayland-proxy` journal stream when the VM starts.

For a full list of warning conditions, see
[`docs/reference/wayland-proxy-warnings.md`](../reference/wayland-proxy-warnings.md).

## Rollback

If you encounter a regression and need to roll back before completing
the migration:

1. Set `graphics.crossDomainTrusted = false` and
   `graphics.waylandProxy.enable = false`.
2. Run `sudo nixos-rebuild switch`.
3. Restart the VM: `d2b vm stop <vm> --apply && d2b vm start <vm> --apply`.

The VM will stop using the proxied cross-domain Wayland path until
`crossDomainTrusted` is enabled again. Standard virtio-gpu display
continues to work, but host-side app-id rewriting and proxy policy no
longer apply.

## Known limitations at migration time

- **Xwayland is not supported.** Set `graphics.xwayland.enable = false`.
  Setting it to true fails eval during the Wayland-only migration phase.
- **Multi-output enumeration** works through the proxy; verify with
  `wayland-info` inside the guest if you use a multi-monitor setup.
- **Clipboard and DnD** are policy-owned by d2b. The standard clipboard
  (`wl_data_device_manager`) is synthetically advertised by
  `d2b-wayland-proxy` even when the host compositor omits that global. Primary
  selection, privileged clipboard-control globals, and DnD are explicitly
  denied by the proxy policy.
- **Wayland text-input v3** (`zwp_text_input_manager_v3`) is denied by default.
  Guest IME/text-input protocol features are disabled by default.
