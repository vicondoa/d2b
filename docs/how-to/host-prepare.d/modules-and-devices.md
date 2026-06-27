# Modules and devices

Operator how-to fragment for the kernel-module and device-node
requirements introduced by host prepare. The integrator assembles this fragment
into [`docs/how-to/host-prepare.md`](../host-prepare.md).

## Kernel modules

Host prepare runs a four-step probe before any `ModprobeIfAllowed` broker call:

1. `/proc/sys/kernel/modules_disabled` — if the file reads `1`, every
   `required` module that is neither built-in nor loaded surfaces as
   `host-modules-locked`. There is no remediation other than rebooting
   with `modules_disabled=0` or shipping the module built-in.
2. `/proc/modules` plus `/sys/module/<name>/` — loaded-module
   detection. Modules listed here are accepted without any further
   action.
3. `/lib/modules/$(uname -r)/modules.builtin` (preferred) or
   `modules.builtin.bin` — built-in detection. Built-in modules
   satisfy the requirement without needing `modprobe`.
4. `/boot/config-$(uname -r)` or `/proc/config.gz` — `CONFIG_*` checks
   used only as **secondary evidence**. The probe never refuses solely
   on the basis of a missing `CONFIG_*` line.

The broker accepts a `ModprobeIfAllowed` request only when the module
name appears in the trusted bundle's `kernelModules` matrix with
`loadAllowed: true`. Every decision (allow + deny) is audited with the
`module_name`, `matrix_entry_id`, and the `modules_disabled` sysctl
value captured at decision time.

### `br_netfilter` posture

If step 2 detects `br_netfilter` as loaded, the probe recommends
pinning:

- `net.bridge.bridge-nf-call-iptables=0`
- `net.bridge.bridge-nf-call-ip6tables=0`

so iptables / ip6tables cannot route around the `inet d2b`
policy. An ADR opt-in is required to suppress this recommendation.

### Distro troubleshooting

- **Ubuntu 24.04 (Tier 1).** Required modules (`kvm_intel`/`kvm_amd`,
  `tun`, `vhost_net`, `fuse`) ship as loadable. `modprobe.d`
  blacklists for any of these surface as `host-modules-locked`.
- **Fedora 40+ (Tier 1 later).** Same module set; `vhost_net` may need
  an explicit `modprobe vhost_net` on first boot.
- **Arch (Tier 2).** Kernel built with `MODULES_DISABLED=y` requires a
  rebuild before VM startup is accepted.
- **NixOS (Tier 0 legacy).** The framework's NixOS module is the
  primary path; `d2b host prepare --apply` is refused with
  `tier-0-legacy-uses-nixos-module`.

## Device nodes

The matrix validated in read-only mode:

| Class           | Default path          | Required mode | Required group | Notes |
| --------------- | --------------------- | ------------- | -------------- | ----- |
| `kvm`           | `/dev/kvm`            | `0660`        | `kvm`          | KVM acceleration. |
| `net-tun`       | `/dev/net/tun`        | `0660`        | `kvm`          | TAP / TUN. |
| `vhost-net`     | `/dev/vhost-net`      | `0660`        | `kvm`          | Vhost-net offload. |
| `fuse`          | `/dev/fuse`           | `0660`        | `fuse`         | virtiofsd. |
| `dri`           | `/dev/dri`            | `0660`        | `video`        | Optional GPU passthrough. |
| `nvidia-*`      | `/dev/nvidia*`        | `0660`        | `video`        | Optional NVIDIA. |
| `pipewire`      | `/run/user/pipewire-0`| socket        | n/a            | Optional audio sidecar. |
| `usbip-host`    | `/dev/usbip-host`     | `0660`        | `usbip`        | Optional USBIP. |
| `tpm`           | `/dev/tpm0`           | `0660`        | `tss`          | Optional TPM passthrough. |
| `vfio`          | `/dev/vfio/vfio`      | `0660`        | `vfio`         | Optional VFIO. |

Stricter modes are accepted; **looser** modes (anything with extra
world bits) fail closed as `loose-mode`. Group ownership is checked by
name; mismatch surfaces as `wrong-group`. The host check **never
mutates** ACLs; remediation is via the trusted bundle / NixOS module.

### Preflight boundary

This check is read-only preflight only. The per-VM `/nix/store`
hardlink farm, the mount namespace, and the virtiofsd setup all
belong to runtime startup. Host prepare surfaces blocking findings
under `host doctor --read-only` and **refuses** to mutate store state.

## Runner-shape preflight

`d2b host check` consumes `host.json`, `processes.json`, and
`closures/<vm>.json` runner-parity snapshots, then validates them
without launching Cloud Hypervisor:

- packaged CH capabilities match `host.json`'s declared row;
- every enabled VM has a `declaredRunner` argv hash present;
- CH API socket paths declare `mode = 0660` and a non-empty owner;
- vsock transports are Unix-socket-backed (`transport = "unix"`);
- virtiofsd / swtpm sidecar `dagNodeId`s appear in the
  `processes.json` DAG.

The same module probes the CH binary for net-handoff support. The
preferred mode is `tap-fd` (broker opens TAP + `/dev/vhost-net` and
passes fds via `SCM_RIGHTS`; runner has **no** `CAP_NET_ADMIN`). The
fallback is `persistent-tap` (broker creates a persistent TAP with
`TUNSETOWNER`/`TUNSETGROUP`). If neither mode satisfies the declared
VM network resources without `CAP_NET_ADMIN`, the host check fails
closed with `ch-net-handoff-not-supported`.

## ioctl allowlist

The broker derives a per-role ioctl allowlist from typed
[`DeviceClass`](../../reference/manifest-bundle.md) entries; no
catch-all `ioctl: 1` exists. The 5-class negative-allowlist matrix
(`TAP/TUN`, cgroup chown, sysctl write, nft batch apply,
device-open) is exercised by `tests/ioctl-negative.sh` against fake
backends.
