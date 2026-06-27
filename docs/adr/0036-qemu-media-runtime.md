# ADR 0036: qemu-media runtime

- Status: Accepted
- Date: 2026-06-20
- Related: ADR 0015 (daemon-only clean break), ADR 0025 (host-jailed
  Wayland filter proxy role), ADR 0034 (storage lifecycle, restart adoption,
  and synchronization), ADR 0035 (efficiency and simplification roadmap)

## Context

`qemu-media` exists to run external media in a local QEMU VM under d2b's
normal daemon and broker authority. It is intentionally narrower than the
NixOS workload runtime: the guest is not evaluated from d2b modules, does
not receive the framework store view, and does not run the guest-control or
observability stack.

The current implementation is already useful and security-sensitive. It owns
host windows, physical USB or image-file boot media, QMP fd passing, broker
preflights, and redacted audit surfaces. Those details need a stable record
before the local-hypervisor seam is cleaned up and shared with the Cloud
Hypervisor/crosvm runtime.

## Decision

The current `qemu-media` implementation is canon for the existing runtime:

- VM manifests declare `runtime.kind = "qemu-media"`.
- The local provider identity is `local-qemu-media`.
- The daemon owns lifecycle orchestration; the privileged broker owns host
  media and QMP mutations.
- QEMU is the only runner for this runtime. Cloud Hypervisor, crosvm,
  virtiofs store sync, guest-control, SSH, in-guest observability, and NixOS
  guest evaluation are not part of the runtime.

### Process DAG

A qemu-media VM has a small process DAG:

1. `host-reconcile` prepares host runtime state and validates the declared
   media source posture.
2. `wayland-proxy` is added when the VM uses the host graphics window path.
3. `qemu-media` starts the QEMU process.

The QEMU runner starts paused and exposes a QMP socket under the VM runtime
directory. Starting paused is load-bearing: it lets the broker complete the
boot-media transaction before guest firmware can observe a missing or wrong
boot disk. After the runner is alive and QMP is ready, the daemon dispatches
`QemuMediaBoot`; only after successful QMP fd/block/device attachment does the
broker continue QEMU.

The Wayland proxy, when present, is a dependency of the QEMU runner rather
than an independent VM runtime. If a qemu-media start fails after the proxy is
spawned, restart reconciliation must clean up the leftover dependency sidecar
before the next start attempt.

### Broker operations

The qemu-media broker operation set is:

| Operation | Purpose |
| --- | --- |
| `QemuMediaEnroll` | Current registry operation for physical USB refs. It records a root-only mapping from an opaque VM/ref slot to a physical USB identity and writes the matching udev ignore rule. |
| `QemuMediaBoot` | Opens the declared boot media, passes the fd to QEMU over QMP, creates the boot block/device nodes, and resumes the paused VM after QMP success. |
| `QemuMediaAttach` | Resolves an enrolled removable slot at runtime, preflights that the host block device is safe to use, passes the fd over QMP, and adds the runtime block/device nodes. |
| `QemuMediaDetach` | Removes or reconciles runtime QMP device/block/fd nodes idempotently, including the current same-device fallback for a uniquely attached moved selector. |

`QemuMediaEnroll` is current behavior, not a general design ideal. It exists so
Nix can store stable opaque refs while transient bus IDs stay out of generated
artifacts. Successful enroll and hotplug flows do not echo raw bus IDs,
serials, by-id paths, block paths, image paths, or registry paths.

### Media sources

`image-file` sources are direct bundle configuration. They do not use
enrollment. The broker treats the path as trusted operator-authored input but
still validates ownership, mode, symlink safety, regular-file type,
non-mounted/non-loop use, locks, and raw format before opening the file for
QMP fd passing.

`physical-usb` sources are declared in Nix by opaque refs. In the current
implementation, the ref is bound to a physical USB identity through
`QemuMediaEnroll`. Attach and detach commands take a transient runtime
selector, resolve it against the registry, and keep the resolved device path in
the broker/QMP boundary rather than in public daemon or CLI output.

### Manual and autostart posture

qemu-media VMs are manual-only today. Operators start them explicitly with the
normal VM lifecycle command. Daemon startup does not autostart them, because
external media, a host graphics window, and physical USB boot devices require
operator presence and current-host context.

This manual posture is part of the current safety model. A qemu-media VM may
hold a sensitive host window, a physical boot disk, or both. Starting it during
host boot without an operator at the console would make the security posture
and failure modes surprising.

### Unsupported guest features

qemu-media guests do not support the NixOS workload features that require
d2b's in-guest stack or Cloud Hypervisor/crosvm runtime wiring:

- guest-control health, exec, detached exec, and config sync;
- SSH framework operations and d2b-managed SSH keys;
- per-VM store sync, virtiofs shares, and guest NixOS activation;
- in-guest observability collectors;
- USBIP guest-side import/export;
- TPM, audio sidecars, and Cloud Hypervisor/crosvm-specific device features.

Public status surfaces should report these as unsupported capabilities rather
than implying that the qemu-media VM is degraded for not having them.

### Redaction and security

The security boundary is the local host plus a QEMU process, not bare metal.
The host OS, compositor, QEMU, and host memory subsystem can observe or retain
sensitive guest activity unless separate host policy prevents it.

The current implementation reduces unnecessary exposure by:

- using broker-owned media fd opening and QMP fd passing instead of broad media
  path bind mounts;
- keeping physical USB identities in a root-only registry and udev rule file,
  not the Nix store;
- redacting physical selectors, serials, by-id names, device paths, image
  paths, and registry paths from CLI success output and audit records;
- running the QEMU runner with an empty capability set, private PID and mount
  namespaces, a read-only root, no `/dev/bus/usb`, and focused access to
  required host devices such as `/dev/kvm`;
- using a QEMU memory backend with guest RAM excluded from process core dumps
  and KSM by default;
- allowing operators to enable a bounded memlock posture that fails closed when
  the host cannot satisfy the declared guest RAM plus QEMU overhead.

Host window presentation through the Wayland filter proxy remains part of the
runtime's host-graphics security posture. The VM-specific app-id rewrite is a
presentation affordance and must not be treated as an authorization boundary.

## Consequences

This ADR freezes the current qemu-media behavior so cleanup can distinguish
between compatibility with existing users and the desired shared runtime seam.
Future work may replace the public enrollment UX and share more lifecycle code
with other local hypervisors, but it must preserve the core authority split:
`d2bd` orchestrates lifecycle, the broker performs privileged media/QMP
mutations, and public output stays redacted.
