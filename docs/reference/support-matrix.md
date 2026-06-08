# Reference: nixling support matrix

> Diataxis: reference. Per-platform-tier support matrix. Assembled by
> the W3 integrator from fragments under
> `docs/reference/support-matrix.d/*.md`; the scope-owned fragments
> are the source of truth for their respective tier columns.

## Tier model

nixling targets a small, panel-approved set of host platforms. Tiers
are graded by where in the wave plan they entered the supported set
and what level of pre-merge verification each one carries:

| Tier | Meaning |
| --- | --- |
| **Tier 0** | NixOS x86_64 with the upstream nixling NixOS module — the legacy compatibility tier for consumer flakes pinning pre-v1.0 VM declarations with `supervisor = "systemd"` (the framework default until consumer flakes opt in to the v1.0 daemon-only `"nixlingd"` supervisor per ADR 0015). `nixling host prepare --apply` is refused (`tier-0-legacy-uses-nixos-module`, exit 78) because there is nothing for the broker to reconcile. v1.0 daemon-only consumers should declare every workload VM as `supervisor = "nixlingd"` and enable `nixling.daemonExperimental.enable = true`. KVM-backed L2 sign-off required. |
| **Tier 1** | Ubuntu 24.04 LTS x86_64, kernel ≥ 6.6 (`6.8.0-45-generic` shipped). `host check`, `host prepare --dry-run`, and `host prepare --apply` are wired live in v1.0 (per ADR 0015) through the broker reconcile ops (ApplyNftables / ApplyRoute / ApplySysctl / UpdateHostsFile / ApplyNmUnmanaged); failures surface a typed `broker-error` envelope (exit 78). KVM-backed L2 sign-off required against the pinned cloud image in `tests/golden/l3-matrix/w3-ubuntu.txt`. |
| **Tier 1-later** | Fedora Server 40+. Best-effort pin exists (`tests/golden/l3-matrix/w3-fedora.txt`) and the L3 sign-off matrix gates against it, but the v1.0 SLA only applies to Tier 0/1. `--apply` carries the same live-broker disposition as Tier 1. |
| **Tier 2** | Arch Linux current, and any other Linux distro on x86_64 with cgroup v2 unified hierarchy. Manifest evaluation works; `host prepare --dry-run` reports `host-check-warning` whenever the broker cannot positively confirm a host-prepare prerequisite. `--apply` routes through the same live broker reconcile ops as Tier 1; failures surface as typed `broker-error` envelopes. Arch carries a best-effort pin (`tests/golden/l3-matrix/w3-arch.txt`); other distros are community-maintained. Operators are expected to read the audit log and the per-distro troubleshooting anchor in `docs/how-to/host-prepare.md`. |

Anything not in the Tier 0/1/1-later/2 set is **rejected**; see ADR
0008 ("Supported platforms and rejected targets") for the explicit
rejected list and the rationale.

## Canonical platform support table (W3 v0.4.0 baseline)

Per-row kernel / cgroup / nftables / NetworkManager / Cloud Hypervisor
/ minijail / glibc minima. The "Status" column states the v1.0 SLA
posture; "Tier" matches the tier model above.

| Platform | Tier | Status | Kernel | cgroup | nftables | NetworkManager | Cloud Hypervisor | minijail | glibc |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| NixOS unstable x86_64 (legacy compatibility tier for pre-v1.0 supervisor="systemd" pins) | 0 | must-not-regress | >= 6.6 | v2 unified | >= 1.0.0 | n/a (systemd-networkd) | >= 40 | nix-built v17+ | nix-shipped |
| Ubuntu 24.04 LTS x86_64 | 1 alpha | first non-NixOS target | >= 6.6 (6.8 shipped) | v2 unified | >= 1.0.9 | >= 1.46 | >= 40 | nix-built v17+ | >= 2.39 |
| Fedora 40+ x86_64 | 1 later | after Ubuntu green | >= 6.8.5 | v2 unified | >= 1.0.9 | >= 1.46 | >= 40 | nix-built v17+ | >= 2.39 |
| Arch rolling x86_64 | 2 | best-effort | >= 6.10 | v2 unified | >= 1.1.0 | >= 1.48 | >= 40 | nix-built v17+ | rolling |

The `nix-built v17+` minijail row applies on every tier because
`packages/nixling-host` packages minijail from source as part of the
trusted bundle; the host's distro-shipped minijail is never used.

## Cross-references

- [ADR 0008 — Supported platforms and rejected targets](../adr/0008-supported-platforms-and-rejected-targets.md)
- [ADR 0011 — cgroup v2 delegation and pidfd handoff](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md)
- [ADR 0012 — W3 IPv6-off sysctl set, hash-derived IfName, bridge-port defaults](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md)
- [ADR 0013 — W3 firewall coexistence policy matrix + `inet nixling` chain layout](../adr/0013-w3-firewall-coexistence-policy.md)
- [ADR 0014 — W3 `kernel.modules_disabled=1` behavior, module probe order, CH net handoff selection, and runner-shape preflight](../adr/0014-w3-modules-devices-runner-shape.md)
- [`docs/reference/compatibility.md`](compatibility.md) — full Tier
  0/1/1-later/2 status table including per-tier behavior of the new
  W3 host verbs.

---

## Section: kernel modules + devices (W3 s4)

# Tier modules + devices (W3 s4 fragment)

Reference fragment listing kernel-module + device-node requirements
per support tier. The integrator assembles this into
[`docs/reference/support-matrix.md`](./support-matrix.md).

## Tier 0 — legacy compatibility (pre-v1.0 supervisor="systemd" pins)

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
