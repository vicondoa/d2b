# How to run the hardware smoke walkthroughs

This guide covers the two operator-facing manual validations that sit above the
usual static gate today:

- local NixOS dev host with GPU + optional YubiKey
- Ubuntu 24.04 Tier-1 manual scaffold

## Local NixOS GPU + YubiKey smoke

### Preconditions

Run this from the repo root on the intended NixOS validation host.

Minimum expectations:

- `/dev/dri/renderD128` exists;
- `/dev/bus/usb` exists;
- `nix` is on `PATH`;
- if you want the USBIP leg, a YubiKey is physically present;
- you are ready for the manual live phase to disrupt the active Wayland session.

### Run the automated phases

```bash
bash tests/host-integration/hardware/hardware-smoke-gpu-yubikey.sh
```

Use `NIXLING_HARDWARE_SMOKE_STRICT=1` to fail closed on cargo/minijail/
example-eval regressions instead of logging an explicit skip reason.

### What the script proves

| Phase | What it checks |
| --- | --- |
| preflight | GPU render node, USB bus, and Nix availability |
| yubikey-optional | whether a YubiKey is plugged in |
| cargo build | workspace + broker buildability |
| minijail invariants | `BundleResolver::validate_minijail_profiles()` |
| bundle drift | `cargo xtask gen-schemas && cargo xtask gen-daemon-api` leaves `docs/reference/` clean |
| example eval | `examples/graphics-workstation` and `examples/with-entra-id` still eval/build |
| live smoke documentation | prints the manual live steps instead of running them automatically |

### Run the manual live phase

When the host is idle, follow the script's printed sequence:

1. start `nixlingd` with the explicit broker binary overrides;
2. run `packages/target/debug/nixling host install --apply`;
3. run `packages/target/debug/nixling vm start work-vm --apply`;
4. attach the YubiKey via `packages/target/debug/nixling usb attach work-vm <busid> --apply` if you are
   validating the USBIP leg (the legacy `nixling usb work-vm` bash orchestrator was retired in v1.0 per ADR 0015);
5. confirm `ExportBrokerAudit` contains the expected `ApplyNftables`,
   `SpawnRunner`, `OpenPidfd`, and `UsbipBind` rows.

The script intentionally leaves this as a manual step because it would disrupt
the operator's own graphics session.

### Record validation evidence

Once the live phase is green, write the readiness evidence files:

```bash
NIXLING_HARDWARE_SMOKE_RECORD_EVIDENCE_ONLY=1 \
NIXLING_HARDWARE_SMOKE_LIVE_GREEN=1 \
NIXLING_HARDWARE_SMOKE_OPERATOR_SIGNATURE='alice@example' \
bash tests/host-integration/hardware/hardware-smoke-gpu-yubikey.sh
```

That writes:

- `/var/lib/nixling/validated/w5Fu.json`
- `/var/lib/nixling/validated/w6Fu.json`

After that, set the matching readiness bits in host config:

```nix
nixling.defaultSwitchReadiness.w5Fu.validated = true;
nixling.defaultSwitchReadiness.w6Fu.validated = true;
```

## Ubuntu 24.04 Tier-1 manual scaffold

### Preconditions

Run this on an Ubuntu 24.04 x86_64 host with KVM and root access:

```bash
sudo NIXLING_REPO=/path/to/nixling \
  tests/integration/distro-matrix/ubuntu-2404-tier1.sh
```

On non-Ubuntu hosts the harness automatically flips into
`NIXLING_UBUNTU_SCAFFOLD_ONLY=1`; you can also set that variable yourself when
you only want the documented scaffold shape.

### What the Ubuntu script covers

| Phase | What it checks |
| --- | --- |
| preflight | Ubuntu + root + KVM + Nix prerequisites |
| install | release build + `host install --apply` scaffold path |
| host prepare | host-prepare apply leg under the manual Tier-1 harness (returns `daemon-down` (exit 1) until daemon-side dispatch ships) |
| vm start | `vm start minimal-vm --apply` |
| probe | guest reachability over SSH |
| vm stop | stop path + pidfd-table drain expectation |
| host destroy | host-reconcile rollback leg (returns `daemon-down` (exit 1) until daemon-side dispatch ships) |
| audit replay | required audit rows + installer artifacts |

Use `NIXLING_UBUNTU_TIER1_STRICT=1` if missing audit rows or installer
artifacts should fail closed.

### Inspect the expected outputs

The canonical fixtures are:

- `tests/integration/distro-matrix/fixtures/ubuntu-2404/expected-audit-ops.txt`
- `tests/integration/distro-matrix/fixtures/ubuntu-2404/expected-installer-artifacts.txt`
- `tests/integration/distro-matrix/fixtures/ubuntu-2404/README.md`

After a live run, compare the observed audit log and written artifacts against
those fixtures before signing off the wave.

### Teardown expectations

The Ubuntu harness already tries to stop the VM and unwind the host changes in
its later phases. If you abort early, run the matching `vm stop` / `host destroy`
steps by hand before reusing the host.

## See also

- [`install-ubuntu-2404.md`](./install-ubuntu-2404.md)
- [`../reference/support-matrix.md`](../reference/support-matrix.md)
- [`../explanation/default-switch-and-deprecation.md`](../explanation/default-switch-and-deprecation.md)
