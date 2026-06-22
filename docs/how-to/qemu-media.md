# How to run a qemu-media VM

This runbook uses the neutral VM name `dark-live` and does not depend on
any specific live image. Use it for either a raw image file or a
physical USB block device.

Running sensitive external media inside QEMU is convenient, but it is not
equivalent to bare-metal boot. The host OS, compositor, and QEMU process
can observe the session. `lockMemory` addresses host swap for guest RAM
when the host can satisfy QEMU's mem-lock request; `dump=off` addresses
QEMU/process core dumps. Host kernel crash dumps require separate host-level
policy.

## 1. Declare the VM

For a direct raw image file:

```nix
nixling.vms.dark-live = {
  enable = true;
  runtime.kind = "qemu-media";
  env = "dark";
  index = 10;
  autostart = false;

  qemuMedia = {
    resources = {
      memoryMiB = 4096;
      vcpu = 2;
    };

    security = {
      lockMemory = true;
    };

    source = {
      kind = "image-file";
      path = "/var/lib/nixling/images/dark-live.raw";
      format = "raw";
      readOnly = true;
    };

  };

  ui.border.activeColor = "#301934";
};
```

For physical USB media, keep only opaque refs in Nix:

```nix
nixling.vms.dark-live.qemuMedia = {
  resources = {
    memoryMiB = 4096;
    vcpu = 2;
  };

  security.lockMemory = true;

  source = {
    kind = "physical-usb";
    ref = "boot";
    usbSelector.byIdName = "usb-Example_Dark_Live_0001-0:0";
    format = "raw";
    readOnly = true;
  };

  removableSlots.backup.source = {
    kind = "physical-usb";
    ref = "backup";
    format = "raw";
    readOnly = true;
  };
};
```

Rebuild the host and restart `nixlingd` so it reloads the updated
bundle.

The `usbSelector.byIdName` value is the basename of a stable
`/dev/disk/by-id/*` symlink for the physical USB block device, not a
path and not a transient busid. For example, if the host has
`/dev/disk/by-id/usb-Example_Dark_Live_0001-0:0`, configure
`usbSelector.byIdName = "usb-Example_Dark_Live_0001-0:0";`. Do not
commit real serial numbers or host-specific device identifiers to shared
examples or issue text.

## 2. Probe physical USB media

Skip this section for `image-file` sources.

```bash
nixling usb probe
```

Probe output is redacted: it shows the transient busid selector without
by-id names, serials, block paths, or registry paths. If the boot-drive
slot shows `enrollable` rather than `enrolled`, verify that the
qemu-media source has the intended `usbSelector.byIdName` and re-run
`nixling usb probe`; `nixling status <vm>` shows the registry state when
you need to distinguish `missing`, `present`, and `stale`. There is no
public enrollment verb.

## 3. Start and inspect

```bash
nixling vm start dark-live --dry-run
nixling vm start dark-live --apply
nixling list
nixling status dark-live
```

The dry-run should show `host-reconcile → qemu-media`. After start,
status should show the qemu-media runner, QMP readiness, source refs,
source kind/format/read-only policy, and registry state. The host QEMU
window is routed through the nixling Wayland filter proxy, so the niri
border rule matches the proxy-rewritten app-id prefix
`nixling.dark-live.`.

## 4. Hotplug configured media

For physical USB removable slots:

```bash
nixling usb attach dark-live 1-2.3 --dry-run
nixling usb attach dark-live 1-2.3 --apply
nixling usb detach dark-live 1-2.3 --apply
```

For qemu-media VMs these commands do not start USBIP runners and do not
SSH into a guest. They dispatch broker-owned QMP attach/detach plans and
redact the runtime selector from success output.

## 5. Capture validation evidence

Record:

- `nixling vm start dark-live --dry-run`
- `nixling vm start dark-live --apply`
- `nixling status dark-live`
- `nixling usb probe`
- any `usb attach` and `usb detach` dry-run/apply output
- broker audit rows for `QemuMediaBoot`, `QemuMediaAttach`, and
  `QemuMediaDetach`

Do not copy raw physical identifiers into issue comments or PR text.
Use the redacted CLI summaries and audit fields.

## 6. Stop

```bash
nixling vm stop dark-live --apply
```

See [the qemu-media reference](../reference/qemu-media.md) for the full
runtime and security contract.
