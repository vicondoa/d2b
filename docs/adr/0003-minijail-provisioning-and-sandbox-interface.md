# 0003. Minijail provisioning and sandbox interface

- Status: Accepted
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

## References

- plan.md, "Supported platform scope"
- plan.md, "Rust control plane"
- plan.md, "Cargo workspace"
- plan.md, "Kernel resource model"
- plan.md, "Required test families"
- AGENTS.md, "VM lifecycle policy (v0.1.5+)"
- [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md)
