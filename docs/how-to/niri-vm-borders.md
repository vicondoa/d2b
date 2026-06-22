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
nixling.site.ui.compositors.niri.enable = true;
```

After `nixos-rebuild switch`, nixling installs a KDL file at
`/etc/nixling/niri-vm-borders.kdl` and the shared UI color artifacts at
`/etc/nixling/ui-colors.json` and `/etc/nixling/ui-colors.css`.

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
        inactive-color "#7fc8ff"
        urgent-color "#7fc8ff"
    }
}
```

The border color is derived deterministically from the VM name when no
override is set, so the same VM always gets the same color across
rebuilds. Inactive and urgent colors default to the active identity
color; set them in nixling if you prefer a neutral inactive color.

### qemu-media host-window rules

Each enabled qemu-media VM routes the host QEMU window through the
nixling Wayland filter proxy. The generated `window-rule` block matches
the proxy-rewritten app-id prefix `nixling.<vm>.`, just like graphics VM
windows:

```kdl
window-rule {
    match app-id=r#"^nixling\.media\."#
    border {
        on
        active-color "#800080"
        inactive-color "#800080"
        urgent-color "#800080"
    }
}
```

## Customizing border colors

To choose a specific border color for a VM, set the compositor-agnostic
UI color option:

```nix
nixling.vms.work.ui.border.activeColor = "#ff8c00";
```

For a qemu-media host window, use the same VM-level option:

```nix
nixling.vms.media.ui.border.activeColor = "#800080";
```

The value must be a six-digit hex color (e.g. `#rrggbb`).

To use a different inactive or urgent color, set:

```nix
nixling.vms.work.ui.border.inactiveColor = "#505050";
nixling.vms.work.ui.border.urgentColor = "#ff8c00";
```

Do not add supplemental niri rules just to keep inactive VM borders in
the VM identity color; nixling renders that state from the same source
model.

## Changing the output path

The default install path is `/etc/nixling/niri-vm-borders.kdl`.  To
use a different location under `/etc/`:

```nix
nixling.site.ui.compositors.niri.outputPath = "/etc/nixling/custom-borders.kdl";
```

Then update the `include` line in `config.kdl` accordingly.

## Verifying the setup

1. After `nixos-rebuild switch`, confirm the file exists and contains
   your VM names:

   ```bash
   cat /etc/nixling/niri-vm-borders.kdl
   ```

   The shared JSON/CSS artifacts are available at:

   ```bash
   cat /etc/nixling/ui-colors.json
   cat /etc/nixling/ui-colors.css
   ```

2. Check that niri loaded the config without errors:

   ```bash
   niri msg action reload-config
   ```

3. Open a window in a graphics VM, or a qemu-media host window, and inspect its app-id from the
   niri IPC:

   ```bash
   niri msg windows
   ```

   The `app_id` field should start with `nixling.<vm>.`.  For graphics
   VMs, if it shows the original app-id without the prefix, the VM's
   `crossDomainTrusted` may be false or the Wayland filter proxy may
   not be running. For qemu-media VMs, qemu-media itself should start
   only after the per-VM Wayland proxy is ready.

4. Confirm the border rule is active by switching focus to a VM
   window — the active border should appear in the configured color.

## Why `crossDomainTrusted` is required for app-id matching

App-id rewriting is performed by the host-side Wayland filter proxy,
which runs only when `graphics.crossDomainTrusted = true`.  With the
proxy absent, guest windows retain their original app-ids and the
`nixling.<vm>.` prefix is never written, so the generated niri rules
cannot match.

If you enable the niri backend for a VM whose `crossDomainTrusted` is
false, the border rule is generated but will not match any window.
Set `crossDomainTrusted = true` to activate app-id rewriting for
that VM.

## Legacy options

The legacy `nixling.site.niriVmBorders.*`,
`nixling.vms.<vm>.graphics.niriBorderColor`, and
`nixling.vms.<vm>.qemuMedia.window.niriBorderColor` options remain as
compatibility inputs for one release. New configurations should use
`nixling.site.ui.compositors.niri.*` and
`nixling.vms.<vm>.ui.border.*`.

## Minimum niri version

The `include` directive was introduced in niri 0.1.9.  On older niri
versions the `include` line is silently ignored; the generated file
exists but has no effect.  Check your niri version with:

```bash
niri --version
```
