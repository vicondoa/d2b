# Set up niri window borders for nixling VMs

This guide covers enabling nixling's opt-in niri KDL window-rule
generator so each graphics VM and qemu-media host window gets a
distinct border color and the crosvm GPU sidecar's scanout window is
hidden on the host compositor.

## Prerequisites

- niri ≥ 0.1.9 (for `include` directive support)
- At least one VM with `nixling.vms.<vm>.graphics.enable = true` or
  `nixling.vms.<vm>.runtime.kind = "qemu-media"`
- `nixling.vms.<vm>.graphics.crossDomainTrusted = true` on VMs that
  use the Wayland filter proxy (required for app-id rewriting)

## Enabling the generated include

Add the following to your NixOS host configuration:

```nix
nixling.site.niriVmBorders.enable = true;
```

After `nixos-rebuild switch`, nixling installs a KDL file at
`/etc/nixling/niri-vm-borders.kdl`.

## Sourcing the file from niri

Add the `include` line to your `config.kdl` (typically
`~/.config/niri/config.kdl`):

```kdl
include "/etc/nixling/niri-vm-borders.kdl"
```

The include line can go anywhere in `config.kdl`; placing it near
your other `window-rule` blocks is recommended for readability.
Reload niri (`niri msg action reload-config`) or log out and back in
for the change to take effect.

## What the generated file contains

The generated KDL contains three kinds of rules:

### Crosvm scanout-window hide rule

The crosvm GPU sidecar creates a small host-side window on the host
compositor when a graphics VM starts.  This window is normally
invisible or blank but appears in the niri window overview.  The
generated file includes a rule to remove its border and prevent it
from drawing a background:

```kdl
window-rule {
    match app-id=r#"^crosvm$"#
    draw-border-with-background false
    border {
        off
    }
}
```

### Per-VM border rules

Each enabled graphics VM gets a `window-rule` block that matches its
app-id prefix.  The host-side Wayland filter proxy rewrites guest
app-ids to `nixling.<vm>.<original-app-id>`, so the regex
`^nixling\.<vm>\.` reliably selects only windows from that VM:

```kdl
window-rule {
    match app-id=r#"^nixling\.work\."#
    border {
        on
        active-color "#7fc8ff"
        inactive-color "#505050"
    }
}
```

The active border color is derived deterministically from the VM name
when no override is set, so the same VM always gets the same color
across rebuilds.

### qemu-media host-window rules

Each enabled qemu-media VM gets a `window-rule` block that matches the
stable host QEMU window title `nixling-<vm>-qemu-media`. This is a host
window rule, not a guest Wayland app-id rule:

```kdl
window-rule {
    match title=r#"^nixling-media-qemu-media$"#
    border {
        on
        active-color "#800080"
        inactive-color "#505050"
    }
}
```

## Customizing border colors

To choose a specific active border color for a VM, set:

```nix
nixling.vms.work.graphics.niriBorderColor = "#ff8c00";
```

For a qemu-media host window, set:

```nix
nixling.vms.media.qemuMedia.window.niriBorderColor = "#800080";
```

The value must be a six-digit hex color (e.g. `#rrggbb`).

The inactive border color (`#505050`) is the same for all VMs.  To
use a different inactive color, add a supplementary rule in your own
`config.kdl` that overrides `inactive-color` for the affected VM.

## Changing the output path

The default install path is `/etc/nixling/niri-vm-borders.kdl`.  To
use a different location under `/etc/`:

```nix
nixling.site.niriVmBorders.outputPath = "/etc/nixling/custom-borders.kdl";
```

Then update the `include` line in `config.kdl` accordingly.

## Verifying the setup

1. After `nixos-rebuild switch`, confirm the file exists and contains
   your VM names:

   ```bash
   cat /etc/nixling/niri-vm-borders.kdl
   ```

2. Check that niri loaded the config without errors:

   ```bash
   niri msg action reload-config
   ```

3. Open a window in a graphics VM and inspect its app-id from the
   niri IPC:

   ```bash
   niri msg windows
   ```

   The `app_id` field should start with `nixling.<vm>.`.  If it shows
   the original app-id without the prefix, the VM's
   `crossDomainTrusted` may be false or the Wayland filter proxy may
   not be running.

4. Confirm the border rule is active by switching focus to a VM
   window — the active border should appear in the configured color.

## Why `crossDomainTrusted` is required for app-id matching

App-id rewriting is performed by the host-side Wayland filter proxy,
which runs only when `graphics.crossDomainTrusted = true`.  With the
proxy absent, guest windows retain their original app-ids and the
`nixling.<vm>.` prefix is never written, so the generated niri rules
cannot match.

If you enable `niriVmBorders` for a VM whose `crossDomainTrusted` is
false, the border rule is generated but will not match any window.
Set `crossDomainTrusted = true` to activate app-id rewriting for
that VM.

## Minimum niri version

The `include` directive was introduced in niri 0.1.9.  On older niri
versions the `include` line is silently ignored; the generated file
exists but has no effect.  Check your niri version with:

```bash
niri --version
```
