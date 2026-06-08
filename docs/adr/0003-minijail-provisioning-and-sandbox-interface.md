# 0003. Minijail provisioning and sandbox interface

- Status: Accepted; virtiofsd `requiresStartRoot` carve-out SUPERSEDED by [ADR 0021](0021-broker-user-namespace-for-virtiofsd.md) in v1.1.2 (the broker-pre-established user namespace replaces the `--sandbox=namespace + requiresStartRoot=true` carve-out)
- Date: 2026-05-25
- Wave: W0b
- Plan slice: "Minijail is not assumed to come from a distro package. Nixling will ship a pinned, Nix-built minijail in its closure and CVE-track it like other bundled virtualization dependencies."
- Companion ADRs: [ADR 0002](0002-non-root-daemon-and-privileged-broker.md), [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md), ADR 0004

## Context

The v0.4.0 baseline hardens long-lived host processes through systemd
service directives such as `NoNewPrivileges`, `ProtectSystem`,
`RestrictAddressFamilies`, `CapabilityBoundingSet`, `DeviceAllow`, and
`ReadWritePaths`. AGENTS.md treats those units and their
`restartIfChanged = false` behavior as load-bearing policy for sidecars
and per-VM lifecycle services.

Moving orchestration into `nixlingd` removes per-VM systemd as the
primary sandbox constructor for daemon-owned VMs. The same hardening
intent therefore has to be represented as typed role profiles that the
Rust control plane and privileged broker can apply consistently on NixOS
and non-NixOS hosts.

The plan chooses minijail as the portable sandboxing substrate, but it
also requires minijail to be part of the Nixling closure instead of an
ambient distro dependency. This keeps sandbox behavior, versioning, and
CVE response under the same supply-chain rules established by ADR 0009
for the Rust workspace.

The plan also requires a strict unsafe-code posture: workspace crates
forbid unsafe code except for reviewed FFI exceptions. Because direct
libminijail integration needs FFI, W0b must define the preferred shape
without granting broad unsafe-code exemptions ahead of the follow-up ADR
that introduces the actual crates.

## Decision

1. Minijail is Nix-built, pinned in the nixling closure, and CVE-tracked like other bundled virtualization dependencies, so distro minijail packages are not relied upon.
2. The preferred integration is libminijail FFI through a thin `libminijail-sys` crate wrapped by a safe `nixling-sandbox` crate, while generated `.conf` files are debugging and audit artifacts only because the daemon constructs jails programmatically from typed profiles.
3. Every role profile declares uid, gid, supplementary groups, capabilities and ambient-capability policy, bind mounts and writable paths, device nodes, namespaces, seccomp policy, environment allowlist, rlimits, cgroup path, log handling, expected sockets, and expected pidfile.
4. `requiresStartRoot` is allowed only for explicitly named roles such as virtiofsd with `--sandbox=namespace`, and each exception documents the setup capability set, syscalls, reason, drop point, and steady-state uid, gid, and capability assertions.

   > **v1.1.1 supersession** ([ADR 0021](0021-broker-user-namespace-for-virtiofsd.md)):
   > the virtiofsd `--sandbox=namespace` + `requiresStartRoot=true` carve-out
   > described here is no longer the live model. The broker now
   > pre-establishes a single-entry user namespace via
   > `clone3(CLONE_NEWUSER)` before exec; virtiofsd profiles declare
   > `requiresStartRoot = false`, zero host capabilities
   > (`capabilities = []`), and a `userNamespace` block mapping in-NS
   > UID/GID 0 to the per-VM runner principal. virtiofsd runs fake-root
   > only inside the namespace. virtiofsd is the only role currently
   > moved to this model; future roles (gpu/audio/swtpm) may follow
   > pending device-bind compatibility analysis.
   >
   > **Updated v1.2 (D5)**: closes the 'future roles (gpu/audio/swtpm)
   > may follow' deferral — swtpm fully closed via ADR 0021
   > broker-pre-NS pattern (zero host caps, single-entry user NS,
   > swtpm principal mapping); gpu partially closed (render-node-only,
   > v1.2fu25): broker pre-opens /dev/dri/renderD128 before
   > clone3(CLONE_NEWUSER), dup2s to RENDER_NODE_INHERITED_FD=10 in
   > the user-NS child, crosvm references /proc/self/fd/10 via
   > --gpu-device-node. Render nodes bypass DRM master auth entirely
   > (no DRM_IOCTL_SET_MASTER required). Legacy gpu profile unchanged;
   > NVIDIA/non-render-node out of scope. audio fully closed (v1.2fu27,
   > Tier 2): user-NS + owned-net-NS (namespaces.net=true); the child
   > calls unshare(CLONE_NEWNET) inside the user NS so CAP_NET_RAW is
   > effective in the user-NS-owned net NS — resolves the AF_NETLINK
   > dependency without any host caps.
5. The Cargo workspace keeps `unsafe_code = "forbid"` at workspace scope, and future `nixling-sandbox` or `libminijail-sys` exceptions require per-crate overrides approved by a follow-up ADR.
6. Seccomp and ioctl policies are per-role and derived from typed device and resource requirements, and no profile may use an `ioctl: 1` catch-all.

## Consequences

1. Positive: Sandbox behavior becomes portable across NixOS and Tier-1 non-NixOS hosts instead of depending on systemd unit hardening.
2. Positive: Typed profiles create a 1:1 mapping from manifest fields to libminijail calls, which makes schema, prose, and runtime oracle tests comparable.
3. Positive: Minijail joins the same pinned supply-chain posture as the Rust workspace and bundled virtualization dependencies.
4. Negative: The FFI path requires a carefully reviewed unsafe-code exception before implementation can land.
5. Neutral: ADR 0009 remains the Rust supply-chain baseline, while this ADR adds runtime sandbox invariants for daemon, broker, runner, and sidecar roles.

## Alternatives considered

- Execute `minijail0` with generated config as the primary interface: rejected because text configs make typed manifest-to-sandbox drift harder to test and audit.
- Depend on the host distribution's minijail package: rejected because version, patch, and CVE posture would vary across Tier-1 hosts.
- Keep systemd hardening for daemon-owned VMs: rejected because daemon-owned orchestration must not require per-VM systemd units.
- Allow broad ioctl passthrough during bring-up: rejected because the plan requires device-derived ioctl allowlists and negative tests for undeclared ioctls.

## Updated v1.2

**D4/P2.1 — seccomp BPF compilation wired** (closes the v1.1.2-final deferral):

`load_runner_seccomp` in `packages/nixling-priv-broker/src/live_handlers.rs`
previously returned `Ok(None)` for non-absolute policy refs (e.g.
`w1-cloud-hypervisor-runner`), silently skipping seccomp installation.

As of v1.2fu15, the function:

1. Maps every internal `seccompPolicyRef` emitted by
   `nixos-modules/minijail-profiles.nix` to a `&[DeviceClass]` slice via
   `policy_ref_device_classes()`.
2. Compiles that slice to a BPF program using
   `nixling_host::seccomp::compile_ioctl_policy_to_bpf`, which derives
   the ioctl allowlist from the `ioctl_policy.rs` matrix (Decision §6).
3. Returns `Ok(Some(program))` for known refs; returns
   `Err(SpawnFailed { detail: "InvalidSeccompPolicy: unknown …" })` for
   unknown refs — the `Ok(None)` silent-skip path is retired.

The broker child closure is reordered: capset → umask → seccomp → execve
(previously capset → seccomp → umask), ensuring umask is not intercepted by
a restrictive BPF before the final stage [panel-kernel R0 #1].

Behavioral regression tests (fork + waitpid) in
`packages/nixling-priv-broker/src/seccomp_compile_tests.rs` verify that
a BPF compiled for `[DeviceClass::Kvm]` allows `KVM_GET_API_VERSION` and
kills with `SIGSYS` on an undeclared ioctl [panel-security R0 #3, #4].

## References

- plan.md, "Supported platform scope"
- plan.md, "Rust control plane"
- plan.md, "Cargo workspace"
- plan.md, "Kernel resource model"
- plan.md, "Required test families"
- AGENTS.md, "VM lifecycle policy (v0.1.5+)"
- [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md)
