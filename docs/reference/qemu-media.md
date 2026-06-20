# qemu-media runtime contract

**Diataxis category:** reference.

`runtime.kind = "qemu-media"` declares a manually started local QEMU VM
for external media workflows. It uses nixling's daemon/broker control
plane, but not the per-VM NixOS evaluator, Cloud Hypervisor, store
sync, guest-control, SSH, or in-guest observability paths.

## Runtime shape

| Surface | Contract |
| --- | --- |
| Provider | `local-qemu-media` with QEMU as the runner. |
| Autostart | Manual-only. Start with `nixling vm start <vm> --apply`; daemon startup skips it. |
| Process DAG | `host-reconcile` → `qemu-media`. The runner starts paused with a QMP socket under `/run/nixling/vms/<vm>/qmp.sock`. |
| Boot media | After the runner is alive, `nixlingd` asks the broker to run `QemuMediaBoot`; the broker opens the declared boot source, sends the fd to QEMU over QMP, attaches USB storage, waits for QMP success responses, then continues QEMU. |
| Hotplug | `nixling usb attach` / `detach` route to `QemuMediaAttach` / `QemuMediaDetach`, not USBIP. |
| Unsupported capabilities | guest-control, exec, config-sync, SSH, store-sync, keys, and in-guest observability. |

## Options

Set the runtime kind on the VM:

```nix
nixling.vms.dark-live = {
  enable = true;
  runtime.kind = "qemu-media";
  env = "dark";
  index = 10;
  autostart = false;
};
```

### Resources

`qemu-media` passes explicit RAM and vCPU sizing to QEMU. The defaults are
4 GiB and 2 vCPUs, avoiding QEMU's small built-in memory default.

```nix
nixling.vms.dark-live.qemuMedia.resources = {
  memoryMiB = 4096;
  vcpu = 2;
};
```

The runner presents boot media as a removable USB storage device on an
EHCI controller, matching the USB-disk/removable shape recommended by
Linux VM frontends for external live media.

### Direct image file

Direct image files are configured in Nix. They do not use enrollment.
The path is operator-authored configuration, and the broker still
validates ownership, mode, symlink safety, regular-file type,
non-mounted/non-loop use, locks, and raw format before opening it.

```nix
nixling.vms.dark-live.qemuMedia.source = {
  kind = "image-file";
  path = "/var/lib/nixling/images/dark-live.raw";
  format = "raw";
  readOnly = true;
};
```

### Physical USB

Physical USB sources use opaque refs in Nix and are enrolled at runtime:

```nix
nixling.vms.dark-live.qemuMedia = {
  source = {
    kind = "physical-usb";
    ref = "boot";
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

Use `nixling usb probe` to find the current selector, then enroll:

```bash
nixling usb enroll dark-live boot --busid 1-2.3 --apply
```

The busid is a transient selector only. It is not stored in Nix-backed
artifacts and is not echoed by successful enroll/attach/detach output.

## CLI behavior

| Command | qemu-media behavior |
| --- | --- |
| `nixling vm start <vm> --dry-run` | Reports the 2-node qemu-media DAG. |
| `nixling vm start <vm> --apply` | Spawns the QEMU runner, waits for QMP readiness, runs `QemuMediaBoot`, and continues QEMU after boot media is attached. |
| `nixling vm stop <vm> --apply` | Stops the daemon-supervised qemu-media runner through the same pidfd/broker path as other runners. |
| `nixling list` / `nixling vm list` | Marks qemu-media rows as `manual-only` and includes QMP readiness when available. JSON may include `runtimeKind`, `autostart`, `unsupportedCapabilities`, and `qemuMedia`. |
| `nixling status <vm>` | Shows qemu-media runner state, QMP readiness, source refs, source kind, format, read-only policy, and registry state. |
| `nixling usb enroll <vm> <ref> --busid <busid> --apply` | Physical USB only. Writes the root-only registry record and udev ignore rule through the broker. |
| `nixling usb attach <vm> <busid> --apply` | Resolves the current USB identity against enrolled refs, preflights that the block device is unused, opens the fd in the broker, sends it to QEMU over QMP, and returns only after QMP accepts the fd/block/device commands. |
| `nixling usb detach <vm> <busid> --apply` | Resolves the enrolled source and removes the QMP device/block/fd nodes. |
| `nixling usb probe` | Shows qemu-media slots as `unbound`, `enrollable`, `enrolled`, `stale`, or `direct-config`; direct image-file rows have no enrollment command. |

Dry-run JSON for enroll and hotplug includes `busIdProvided: true`, but
not the busid value. Successful broker audit records include VM/ref,
slot, read-only policy, QMP plan labels, and udev flags only; they omit
busid, by-id names, serials, block paths, image paths, and registry
paths.

## Security contract

- Physical USB identity lives in the root-only qemu-media registry and
  runtime udev rule file, not in the Nix store.
- The qemu-media runner has an empty capability set, private PID/mount
  namespaces, a read-only root, no broad media path bind mounts, no
  `/dev/bus/usb`, and `/dev/kvm` as its focused device class.
- Media fds stay broker-local until QMP fd passing. The daemon and CLI
  name only VM/ref/busid selectors.
- Direct image-file paths are trusted bundle configuration. Public CLI status
  reports source kind/format/read-only policy without echoing those paths; the
  broker fail-closes on unsafe paths and non-raw formats.
- Host window presentation for niri matches the stable title
  `nixling-<vm>-qemu-media`; set
  `nixling.vms.<vm>.qemuMedia.window.niriBorderColor` for a fixed color.

## See also

- [qemu-media how-to](../how-to/qemu-media.md)
- [CLI contract](./cli-contract.md)
- [Privileges](./privileges.md)
- [niri VM borders](../how-to/niri-vm-borders.md)
