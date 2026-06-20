# Kernel-module check

Operator reference for the daemon-startup self-check that verifies the
kernel-module matrix the running bundle requires is loaded into the
live kernel.

Source of truth: `packages/nixlingd/src/kernel_module_check.rs`.

## When it runs

`nixlingd serve` runs the check exactly once on startup, after the
state lock is acquired and after the pidfd table is restored, but
*before* the autostart pass dispatches any VM. Sequence:

1. Acquire daemon lock + bind public socket.
2. Drop privileges (if configured).
3. Restore the pidfd table from disk.
4. Adopt orphaned runners.
5. **kernel-module-check** ← this gate.
6. Autostart pass (consumes the check's degraded-VM list).
7. Accept loop.

## What it reads

* `/proc/modules` — currently-loaded modules. Parsed via
  `nixling_host::modules::LoadedModuleSet::parse_proc_modules`.
* The trusted bundle (already loaded by the resolver) — used to
  decide which conditional modules are in scope (virtiofs / graphics
  / usbip / tpm).

The check does **not** invoke `modprobe`, `modinfo`, or any
side-effecting helper. Mutating module load remains the broker's
responsibility (`nixling-priv-broker::ops::modprobe`).

If `/proc/modules` cannot be read, the check treats *every* module as
absent — required modules then read as missing and the daemon
refuses to start. This is intentional fail-closed posture: a host
without a readable `/proc` cannot safely host VMs.

## Required vs optional matrix

### REQUIRED — fatal when missing

Daemon refuses to start with typed error
`host-kernel-modules-missing` (exit code 64) and the missing-module
list in the public message. KVM alternatives render as
`kvm_intel|kvm_amd` to indicate either satisfies the requirement.

| Module | Gate |
| --- | --- |
| `kvm_intel` **or** `kvm_amd` | Always (at least one must be present). |
| `vhost_net` | Always. |
| `tun` | Always. |
| `virtio_net` | Always. |
| `virtio_blk` | Always. |
| `virtio_pci` | Always. |
| `virtio_console` | Always. |
| `virtiofs` | Any VM has a `Virtiofsd` process node. |
| `udmabuf` | Any VM has `graphics = true` or a `Gpu` process node. |

A module is "present" when either `/proc/modules` contains it OR the
daemon can prove the corresponding `CONFIG_*` option is built into the
running kernel. This matters for `udmabuf`, which many kernels expose
as `CONFIG_UDMABUF=y` with no loadable `.ko`.

### OPTIONAL — warn-only, may degrade VMs

Daemon continues startup. A `tracing::warn!` line is emitted for
each missing optional module. VMs that need that module are skipped
by the autostart pass with `Outcome::Degraded` and a stable
`"pre-degraded: kernel-module-check flagged '<vm>' …"` reason.

| Module | Gate | On miss |
| --- | --- | --- |
| `nvidia` | Any graphics VM declared. | Warn only — no VM degraded (software-render fallback works). |
| `nvidia_uvm` | Any graphics VM declared. | Warn only — no VM degraded. |
| `usbip_host` | Any VM has `usbip_yubikey = true` or a `Usbip` process node. | The affected VM(s) are marked degraded. |
| `tpm_vtpm_proxy` | Any VM has `tpm = true` or a `Swtpm`/`SwtpmPreStartFlush` process node. | The affected VM(s) are marked degraded. |

## Public error envelope

```
kind: host-kernel-modules-missing
exitCode: 64
message: "daemon refused to start: required kernel modules not loaded: kvm_intel|kvm_amd, vhost_net"
remediation: "load the listed kernel modules with `modprobe <name>` (or via `boot.kernelModules` in the NixOS host config) and restart nixlingd. ..."
```

## Remediation

For a missing REQUIRED module:

```bash
sudo modprobe kvm_intel     # or kvm_amd on AMD CPUs
sudo modprobe vhost_net tun virtio_net virtio_blk virtio_pci virtio_console
sudo systemctl restart nixlingd
```

For NixOS hosts, pin the modules via:

```nix
boot.kernelModules = [
  "kvm-intel"     # or "kvm-amd"
  "vhost_net" "tun"
  "virtio_net" "virtio_blk" "virtio_pci" "virtio_console"
];
```

For a degraded VM caused by a missing OPTIONAL module:

* USBIP-degraded VM → `sudo modprobe usbip_host`.
* TPM-degraded VM → `sudo modprobe tpm_vtpm_proxy`.
* Graphics warn (nvidia) — load `nvidia` + `nvidia_uvm` only if the
  host has nvidia hardware and you intend to use accelerated
  passthrough.

After loading, restart the daemon (or trigger an autostart re-run
on the next SIGHUP / reconnect) to pick up the change.

## Matrix gate

`tests/kernel-module-matrix-eval.sh` asserts that the
`REQUIRED_*` / `OPTIONAL_*` constants in
`packages/nixlingd/src/kernel_module_check.rs` stay in sync with
the table above. Run it after editing either side.

## Related

* `docs/reference/support-matrix.d/s4-tier-modules.md` — tier-by-
  tier module disposition (Tier 0 NixOS auto-load vs Tier 1+
  loadable).
* `docs/reference/host-prep-dag.md` — the broker-side module
  matrix (mutating side: `modprobe` allow/deny).
* `packages/nixling-host/src/modules.rs` — the four-step host probe
  (`/proc/modules` + builtin + `/boot/config-*`) the broker uses;
  the daemon check is a *consumer* of `LoadedModuleSet`.
