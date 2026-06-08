# Tier modules + devices

Reference fragment listing kernel-module + device-node requirements
per support tier. The integrator assembles this into
[`docs/reference/support-matrix.md`](../support-matrix.md).

## Tier 0 — NixOS legacy

| Module     | Min kernel | Disposition |
| ---------- | ---------- | ----------- |
| `kvm_intel` / `kvm_amd` | 6.6 | NixOS module declares; loaded at activation. |
| `tun`      | 6.6        | NixOS module declares; loaded at activation. |
| `vhost_net`| 6.6        | NixOS module declares; loaded at activation. |
| `fuse`     | 6.6        | NixOS module declares; loaded at activation. |

`nixling host prepare --apply` is refused on Tier 0 with
`tier-0-legacy-uses-nixos-module` (exit 78). The NixOS module owns the
module + device-node activation contract.

## Tier 1 alpha — Ubuntu 24.04 LTS

| Module     | Min kernel | Disposition |
| ---------- | ---------- | ----------- |
| `kvm_intel` / `kvm_amd` | 6.6 (6.8.0-45-generic ships) | Loadable; `ModprobeIfAllowed` may run. |
| `tun`      | 6.6        | Loadable. |
| `vhost_net`| 6.6        | Loadable. |
| `fuse`     | 6.6        | Loadable. |

Glibc 2.39, cgroup v2 unified, NetworkManager 1.46, nftables 1.0.9,
Cloud Hypervisor v40+, Nix-built minijail v17 (see
`tests/minijail-version-check.sh`).

| Device class    | Required path        | Required mode | Required group |
| --------------- | -------------------- | ------------- | -------------- |
| `kvm`           | `/dev/kvm`           | `0660`        | `kvm`          |
| `net-tun`       | `/dev/net/tun`       | `0660`        | `kvm`          |
| `vhost-net`     | `/dev/vhost-net`     | `0660`        | `kvm`          |
| `fuse`          | `/dev/fuse`          | `0660`        | `fuse`         |

## Tier 1 later — Fedora Server 40+

| Module     | Min kernel | Disposition |
| ---------- | ---------- | ----------- |
| `kvm_intel` / `kvm_amd` | 6.8.5 | Loadable; may need explicit `vhost_net` modprobe on first boot. |
| `tun`      | 6.8.5      | Loadable. |
| `vhost_net`| 6.8.5      | Loadable. |
| `fuse`     | 6.8.5      | Loadable. |

nftables 1.0.9, NetworkManager 1.46.

Device classes match Tier 1 alpha; group names follow Fedora defaults
(`kvm`, `fuse`).

## Tier 2 — Arch Linux

| Module     | Min kernel | Disposition |
| ---------- | ---------- | ----------- |
| `kvm_intel` / `kvm_amd` | 6.10 | Loadable. |
| `tun`      | 6.10       | Loadable. |
| `vhost_net`| 6.10       | Loadable. |
| `fuse`     | 6.10       | Loadable. |

nftables 1.1.0, NetworkManager 1.48.

Best-effort tier: the framework runs but the matrix is community-
maintained.

## `kernel.modules_disabled=1` posture

Every tier refuses to start required-module VMs when
`/proc/sys/kernel/modules_disabled` reads `1` unless every required
module is detected as built-in or already loaded. The probe surfaces
`host-modules-locked` with a per-tier remediation hint pointing at
`/etc/sysctl.d/`.

## Optional accelerator + USBIP + TPM + VFIO

These rows are optional on every tier. Absence surfaces as
`optional-absent`, not `host-modules-locked`.
