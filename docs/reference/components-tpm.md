# `nixling.vms.<vm>.tpm.*`

> Reference for the `tpm` component module.
> Source: [`nixos-modules/components/tpm.nix`](../../nixos-modules/components/tpm.nix)
> Host-side wiring: [`nixos-modules/host-sidecars.nix`](../../nixos-modules/host-sidecars.nix), [`nixos-modules/host-users.nix`](../../nixos-modules/host-users.nix)

## What this component does

Attaches a software-emulated TPM 2.0 device to the guest. Per-VM
`swtpm socket` runs on the host as a dedicated `nixling-<vm>-swtpm`
system user; cloud-hypervisor connects to it via
`--tpm socket=/run/nixling/vms/<vm>/tpm.sock`. The guest kernel sees a normal
TPM CRB device, exposes `/dev/tpm0` + `/dev/tpmrm0`, and an in-guest
oneshot provisions the TPM2 Storage Root Key at the standard
persistent handle `0x81000001` (ECC P-256 preferred, RSA-2048
fallback) so downstream services (Himmelblau, sbctl, systemd-tpm2-setup
consumers) can bind keys without bootstrapping themselves.

TPM state — including the SRK and any keys bound to it by services
running inside the VM — is **persisted on the host** at
`/var/lib/nixling/vms/<vm>/swtpm/`.

## Options (host-side)

| Option | Type | Default | Description |
|---|---|---|---|
| `nixling.vms.<vm>.tpm.enable` | bool | `false` | Attach an swtpm 2.0 device to this VM as TPM CRB. Implies `hypervisor = cloud-hypervisor` (the only one microvm.nix can wire swtpm to). |

## Options (guest-side propagation)

None. The component module is layered directly into the guest config
by `host.nix` (`++ lib.optional vm'.tpm.enable ./components/tpm.nix`);
all guest-side wiring is unconditional within the module.

## Host-side resources created

- **`nixling-<vm>-swtpm` system user + group**
  ([`host-users.nix`](../../nixos-modules/host-users.nix)). Static
  per-VM user — `DynamicUser = false` so state under
  `/var/lib/nixling/vms/<vm>/swtpm/` has a stable owner.
- **`nixling-<vm>-swtpm.service`**
  ([`host-sidecars.nix`](../../nixos-modules/host-sidecars.nix)).
  - Runs as `nixling-<vm>-swtpm:nixling-<vm>-swtpm`.
  - `StateDirectory = "nixling/vms/<vm>/swtpm"`, `StateDirectoryMode = 0700`.
  - `RuntimeDirectory = "swtpm/<vm>"`, `RuntimeDirectoryMode = 0711`.
  - `ExecStart`:
    `swtpm socket --tpmstate dir=/var/lib/nixling/vms/<vm>/swtpm
    --ctrl type=unixio,path=/run/nixling/vms/<vm>/tpm.sock,mode=0660 --tpm2
    --flags startup-clear`.
  - `ExecStartPost`: `setfacl -m u:nixling-<vm>-gpu:rw /run/nixling/vms/<vm>/tpm.sock`
    so cloud-hypervisor (running as `nixling-<vm>-gpu` when graphics
    is also enabled) can connect. Failures are tolerated for non-
    graphics VMs.
  - `partOf = [ "microvms.target" ]` so a system-wide microvm
    restart cycles it; `Restart = "on-failure"`, `RestartSec = 2`.
  - `restartIfChanged = false` (v0.1.5+, top-level NixOS option; emitted under `[Service]`) — a
    `nixos-rebuild switch` updates the unit file but does NOT
    cycle the running swtpm. Killing swtpm under a live VM means
    the guest loses its TPM socket and Entra/Intune device-bound
    credentials become unreachable; the framework refuses to do
    this silently. Use `nixling vm restart <vm>` to apply pending
    changes. (Pre-v0.1.7 this was the broken
    `unitConfig.X-RestartIfChanged = false` form; see v0.1.7
    CHANGELOG.)
- **State directory** `/var/lib/nixling/vms/<vm>/swtpm/`, mode 0700
  owned by `nixling-<vm>-swtpm`. Contents are swtpm NVRAM + state
  blobs — not human-readable, not portable across VMs. In the
  daemon/broker model the privileged broker **provisions this
  directory on first VM start** (fd-safe create, owner
  `nixling-<vm>-swtpm`, mode 0700, inherited ACLs cleared); an
  existing directory with the correct owner is reconciled in place
  (never wiped). If a previously-provisioned directory is missing or
  replaced, the broker **fails the start closed**
  (`previously-provisioned-swtpm-state-missing`) rather than
  re-creating an empty TPM — see
  [components-tpm recovery](#) and the v1.2→v1.3 migration guide.
- **Parent-dir posture.** The VM's state root at
  `/var/lib/nixling/vms/<vm>/` is `nixlingd:users 3770` — `setgid`
  so role users inherit the group, and **sticky (`+t`)** so a
  per-VM role UID (which holds rwx via POSIX ACL) cannot rename or
  unlink the principal-owned `swtpm/` directory it does not own. The
  `nixling-<vm>-swtpm` principal additionally gets a `--x` traverse
  ACL on the parent (gated on `tpm.enable`) so swtpm can reach its
  state directory. Without the traverse grant swtpm starts but
  EACCES'es on `tpm2-00.permall` → libtpms enters failure mode →
  guest boots with a fresh TPM → Entra/Intune treats the device
  as tampered. **No manual `chown` or `setfacl` required for new
  installs or per-VM additions; the framework handles it.**

## Lifecycle (v0.1.5+)

`nixling-<vm>-swtpm.service` carries `restartIfChanged = false`
(matches the [graphics sidecar lifecycle policy](./components-graphics.md#lifecycle-v015)).
A `nixos-rebuild switch` updates the unit file but does NOT cycle
the running swtpm — killing swtpm under a live VM tears down the
CH TPM socket, the guest's libtpms enters failure mode, and
Entra/Intune device-bound creds become unreachable. After a
rebuild, `nixling list` flags the VM with `[pending restart]` if
its `current` closure has drifted from `booted`; apply with
`nixling vm restart <vm>` (clean down+up cycles swtpm and CH
together so the TPM socket survives the round-trip). See
[`docs/reference/cli-contract.md` — Pending-restart signal](./cli-contract.md#pending-restart-signal-v015).

## Guest-side resources created

- `microvm.hypervisor = "cloud-hypervisor"` (via `mkDefault`).
- `microvm.cloud-hypervisor.extraArgs =
  [ "--tpm" "socket=/run/nixling/vms/<hostname>/tpm.sock" ]`.
- `security.tpm2.enable = true`.
- `boot.kernelModules = [ "tpm" "tpm_crb" ]` — belt-and-suspenders;
  the kernel normally auto-probes when it sees the CH TPM CRB at
  `fed40000-fed40fff`.
- `environment.systemPackages = [ pkgs.tpm2-tools ]` for in-guest
  diagnostics (`tpm2_getcap properties-fixed`, `tpm2_getrandom 16`).
- `systemd.services.tpm2-flush-sessions` — early oneshot, wanted by
  `sysinit.target`, that flushes only loaded/saved TPM sessions via
  `/dev/tpmrm0`. This prevents stale swtpm saved sessions from filling
  the TPM active-session table after guest reboots while preserving NV
  indices and persistent handles.
- `systemd.services.tpm2-srk-provision` — oneshot, `RemainAfterExit`,
  `wantedBy = [ "multi-user.target" ]`. Idempotently provisions the
  SRK at `0x81000001`. Pins `TPM2TOOLS_TCTI = "device:/dev/tpmrm0"`
  to skip the tabrmd D-Bus probe and orders after
  `tpm2-flush-sessions.service`. Services that need the SRK in place
  should add `after = [ "tpm2-srk-provision.service" ]`.

## Runtime invariants

- Each TPM-enabled VM has exactly one `nixling-<vm>-swtpm.service`
  on the host. The socket at `/run/nixling/vms/<vm>/tpm.sock` is mode 0660,
  owned by `nixling-<vm>-swtpm`. ACLs grant `nixling-<vm>-gpu` rw;
  no other user (including the kvm group) can reach the control
  protocol out-of-band.
- swtpm NVRAM persists across `nixling vm start`/`nixling vm stop` cycles
  and across host reboots — by design. Anything the guest binds to
  the TPM (LUKS keys, Himmelblau device key, sbctl PCR policies)
  survives a VM restart.
- The SRK at `0x81000001` exists exactly once. The provisioning
  oneshot short-circuits via
  `tpm2_getcap handles-persistent | grep -q $SRK_HANDLE`.

## Hardening notes

`nixling-<vm>-swtpm.service`:

- Dedicated static system user per VM. Earlier revisions ran swtpm
  under `DynamicUser` in the `kvm` group; this was tightened so a
  kvm-group process on the host cannot speak the swtpm control
  protocol.
- `NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome`,
  `PrivateDevices`, `PrivateTmp`, `ProtectKernelModules`,
  `ProtectKernelTunables`, `ProtectKernelLogs`,
  `ProtectControlGroups`, `LockPersonality`,
  `MemoryDenyWriteExecute = true`.
- `RestrictAddressFamilies = [ "AF_UNIX" ]` — swtpm needs no network.
- `UMask = "0007"` (v1.1.2-final, was `"0177"`): the broker's child
  closure honours the `umask` field declared in the swtpm role
  profile, which sets `umask = 0o007` so the bound control socket is
  mode 0660.
- The control socket is mode `0660` (v1.1.2-final, was `0600`); only
  per-VM ephemeral UIDs that have a default-ACL named-user grant on
  `/run/nixling/vms/<vm>/` (cloud-hypervisor's UID + the swtpm UID
  itself) can open it. Cross-VM UIDs do NOT have access because the
  per-VM runtime dir's default ACL only enumerates that VM's roles.

## Common gotchas / failure modes

- **DO NOT WIPE `/var/lib/nixling/vms/<vm>/swtpm/`.** Removing or
  replacing this directory regenerates a fresh, empty TPM with a
  new endorsement key. To remote IdPs (Entra ID via Himmelblau,
  any TPM-bound enrolment) this looks like device tampering and
  forces re-enrolment — and, depending on the IdP, may require
  out-of-band admin action to unblock. Treat the directory as part
  of the VM's stable identity.
- **Backups: encrypted, access-controlled media only.** swtpm state
  contains key material; a backup that leaks the state files leaks
  every key the VM bound to its TPM. If you back up
  `/var/lib/nixling/vms/<vm>/swtpm/`, do it to a LUKS volume (or
  equivalent) and limit who can mount it.
- **No swtpm without cloud-hypervisor.** `tpm.enable = true` pins
  `microvm.hypervisor` to `cloud-hypervisor` via `mkDefault`. The
  graphics and audio components also pin CH via `mkDefault`, so the
  three compose cleanly. A VM that hand-rolls `microvm.hypervisor =
  "qemu"` and sets `tpm.enable = true` will end up with an unwired
  TPM (CH `extraArgs` are emitted, but ignored by qemu).
- **`tpm2_*` complaining about TCTI.** The provisioning oneshot
  pins `TPM2TOOLS_TCTI = "device:/dev/tpmrm0"`. Ad-hoc invocations
  from a guest shell may default to dialing tabrmd over D-Bus and
  emit harmless warnings; pass `-T device:/dev/tpmrm0` or set
  the env var.

## See also

- [Design / threat model](../explanation/design.md) — TPM-bound
  credentials at rest is one of the in-scope threats.
- [Manifest schema](./manifest-schema.md) — `units.swtpm` field.
- [`examples/graphics-workstation`](../../examples/graphics-workstation/) —
  enables `tpm.enable` alongside graphics + audio.
- [`examples/with-entra-id`](../../examples/with-entra-id/) — uses
  the swtpm to bind the Entra device key.
