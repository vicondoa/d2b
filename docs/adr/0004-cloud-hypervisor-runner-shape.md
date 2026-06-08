# 0004. Cloud Hypervisor runner shape

- Status: Accepted
- Date: 2026-05-25
- Wave: W0b
- Plan slice: "cloud-hypervisor + minijail spike + runner-shape audit"
- Companion ADRs: ADR 0001 (systemd-free orchestration), ADR 0003 (minijail), ADR 0006 (manifest bundle)

## Context

The portability plan keeps Nix as the producer of guest systems and
microvm configuration, but moves host orchestration into `nixlingd`.
`microvm.declaredRunner` is useful as a compatibility oracle, yet its
Cloud Hypervisor runner is a generated shell wrapper that can also fork
sidecars inline.

W0b audited the current runner by evaluating `examples/minimal`, forcing
Cloud Hypervisor for that headless VM, building the declared runner, and
inspecting `bin/microvm-run`, `bin/virtiofsd-run`, TAP helpers, and the
virtiofsd supervisord config. The audit also inspected the graphics
runner and confirmed that `crosvm device gpu` is forked inline before CH
execs.

## Decision

Nixling will generate its own Cloud Hypervisor argv from evaluated
microvm/nixling config and launch each role separately under daemon-owned
supervision.

The considered options are:

1. Reuse `declaredRunner` as a black-box shell wrapped in one union
   minijail profile — rejected because CH, GPU, virtiofsd, audio/video,
   swtpm, TAP, and relay roles need distinct uid/gid, capability,
   cgroup, mount, and seccomp/minijail profiles.
2. Generate nixling-owned CH argv from evaluated config — accepted. The
   daemon consumes `microvm.interfaces`, `microvm.shares`,
   `microvm.vsock`, `microvm.cloud-hypervisor.extraArgs`,
   `nixling.vms.<vm>.*`, and the W1 manifest bundle. For headless VMs,
   generated argv must match declaredRunner's snapshotted argv except for
   documented daemon divergences such as API socket ownership and vsock
   CID allocation.
3. Carry a small microvm.nix runner patch that skips inline GPU spawning
   — deferred as a fallback if argv generation proves unexpectedly
   complex in W4.
4. The W0b runner-shape contract is a documentation and audit pin only:
   it does not add future Rust crate stubs. The W2 crate names reserved
   by this ADR are `nixling-priv-broker`, `nixling-sandbox`,
   `nixling-supervisor`, `nixling-ch-api`, and `nixling-host`.

## Consequences

1. Positive: `nixlingd` can apply per-role minijail profiles instead of
   inheriting the broad privilege union required by a shell runner.
2. Positive: `declaredRunner` remains valuable as a parity oracle while
   the daemon runner is implemented.
3. Positive: W4 has a concrete CH argv fixture to compare against before
   replacing the current supervisor path.
4. Negative: Nixling must encode the CH argv model rather than delegating
   all backend details to microvm.nix's shell scripts.
5. Neutral: A tiny microvm.nix patch remains available if graphics inline
   GPU spawning blocks W4, but it is not the primary design.

## Alternatives considered

- Black-box `microvm-run` under a single minijail: rejected because it
  cannot enforce the portability plan's per-role sandbox model.
- Patch only the inline `crosvm device gpu` fork: deferred because it
  addresses one blocker but still leaves shell-runner supervision as the
  daemon ABI.
- Boot a live Cloud Hypervisor VM for W0b: not possible in this worktree
  because there is no KVM-capable/root host; the W0b artifact is instead
  an eval/build snapshot audit.

## Cloud Hypervisor capability matrix

| Capability / backend | W0b posture | Notes |
| --- | --- | --- |
| Headless Cloud Hypervisor VM | Supported runner-shape target | Nixling-owned argv must track declaredRunner parity for the audited headless shape. |
| Graphics Cloud Hypervisor VM | Supported shape, sidecar split required | Inline `crosvm device gpu` spawning remains a W4 implementation blocker to solve with per-role supervision or the deferred upstream patch. |
| Audio Cloud Hypervisor VM | Supported shape, sidecar split required | Audio/video sidecars must run as separate roles rather than inheriting one broad CH runner profile. |
| TPM Cloud Hypervisor VM | Supported shape, sidecar split required | `swtpm` remains a separate role with its own uid/gid, socket ownership, and sandbox. |
| Vsock Cloud Hypervisor VM | Supported shape | Vsock CID allocation and API socket ownership are allowed documented daemon divergences from declaredRunner snapshots. |
| Firecracker | Deferred non-goal | Firecracker feature parity is outside the first milestone and requires a later ADR plus panel review. |
| crosvm-as-full-VMM | Deferred non-goal | Crosvm may appear as a graphics helper, but using crosvm as the primary full VMM is outside the first milestone. |

## Field-by-field input contract

The nixling-owned Cloud Hypervisor argv generator consumes evaluated
configuration, not shell fragments. For declaredRunner parity it must
map the following inputs field by field:

- `config.microvm.interfaces`: interface id, MAC address, TAP or fd
  source, bridge attachment intent, and optional per-interface CH
  network arguments.
- `config.microvm.shares`: virtiofs tags, source paths after manifest
  validation, mount tags, readonly/readwrite mode, and socket paths for
  separately supervised `virtiofsd` roles.
- `config.microvm.vsock`: declared vsock enablement and requested CID,
  with daemon-owned allocation permitted where the declared value is
  absent or conflicts with live state.
- `config.microvm.cloud-hypervisor.extraArgs`: explicit extra CH argv
  entries that remain after validation and policy filtering.
- DeclaredRunner-derived paths: kernel, initrd, disk or rootfs images,
  console/log sockets, CH API socket, pid/state paths, and sidecar
  socket paths are derived from the same evaluated runner inputs but are
  materialized under daemon-owned runtime/state directories.
- Role inputs: CH, GPU helper, audio/video helper, TPM, virtiofsd, TAP,
  and relay roles each receive only the paths, fds, uid/gid,
  capabilities, and sockets required for that role.
- W1 manifest bundle fields: VM identity, closure paths, role uid/gid
  assignments, stable resource names, network intent, state/runtime
  directories, hashes, and version fields land in the W1 trusted bundle
  and become the broker/daemon ABI for argv generation.

## Future Rust crate reservations

W0b does NOT add the future `nixling-priv-broker`,
`nixling-sandbox`, `nixling-supervisor`, `nixling-ch-api`,
`nixling-host` crate stubs; those are reserved for W2 and named here to
prevent drift.

## References

- [Runner-shape audit](../reference/runner-shape-audit.md)
- Portability plan slice, "Virtualization model": Cloud Hypervisor is
  the first-milestone VMM; Firecracker feature parity and
  crosvm-as-full-VMM are deferred targets.
- [`nixos-modules/host.nix` `microvm.vms = lib.mapAttrs ...` block](../../nixos-modules/host.nix#L228)
- [ADR 0000](0000-repository-layout-and-rust-bootstrap.md)
- [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md)
