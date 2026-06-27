# 0008. Supported platforms and rejected targets

- Status: Accepted
- Date: 2026-05-25
- Wave: W0b
- Plan slice: "Supported platform scope: define the first non-NixOS target, later tiers, explicit rejected targets, kernel floor, minijail provisioning, and telemetry posture."
- Companion ADRs: ADR 0002, ADR 0003, [ADR 0007](0007-bash-coexistence-and-migration.md)

## Context

The portability plan keeps the current NixOS host path as Tier 0 while
adding a Rust control plane that can run on selected non-NixOS hosts.
The first milestone is intentionally narrow: prove one non-NixOS host
with a glibc distribution, Nix daemon install, KVM, cgroup v2, nftables,
NetworkManager, Cloud Hypervisor, and a Nix-built minijail before adding
more host families or VMM feature parity.

The plan also carries a long unsupported list. macOS with vfkit, WSL,
containers as hosts, Alpine or other musl hosts, non-systemd autostart,
rootless Nix, Firecracker feature parity, crosvm as a full VMM, and
runtime graphics or audio on aarch64 are all outside the first
milestone. Aarch64 cross-evaluation for Rust crates remains supported
per W0a; this ADR rejects only aarch64 runtime graphics and audio for
the first milestone.

## Decision

1. Tier 0 SUPPORTED is NixOS unstable on x86_64 using the existing bash
   and systemd backend. This path is current behavior and must never
   regress.
2. Tier 1 ALPHA is Ubuntu 24.04 LTS on x86_64, glibc rather than musl,
   kernel `>= 6.6`, Nix daemon install, KVM, cgroup v2 unified
   hierarchy, nftables, NetworkManager, Cloud Hypervisor, and a
   Nix-built minijail. This is the first non-NixOS target.
3. Tier 1 LATER, after Ubuntu is green, is Fedora 40+ on x86_64 with the
   same kernel and control-plane requirements.
4. Tier 2 BEST-EFFORT, after Tier 1, is Arch rolling on x86_64.
5. The following are UNSUPPORTED at the first milestone and require a
   separate ADR plus panel review to add: macOS with vfkit, WSL,
   containers as hosts, Alpine or other musl hosts, non-systemd
   autostart, rootless Nix, Firecracker feature parity, crosvm as a full
   VMM, and aarch64 runtime graphics or audio.
6. The kernel floor is `6.6`. The mechanisms behind that floor include
   `cgroup.kill` in Linux 5.14 or newer, `pidfd_open` in 5.3 or newer,
   io_uring `openat2` support in 5.6 or newer, and `fchmodat2` in 6.6 or
   newer. The highest required floor wins.

   **v1.1 kernel-floor uplift (effective from `phase-daemon-only` HEAD
   `00b24c5` + the v1.1 plan).** The v1.0 floor of `6.6` documented
   above is preserved as the v1.0 historical baseline; v1.1+ requires
   `>= 6.9` because the BootedNotify pidfd-identity contract specified
   in [ADR 0018 § set-booted race-free serialization](0018-microvm-nix-removal.md#set-booted-race-free-serialization)
   relies on **pidfs** (the per-process pidfd inode filesystem) which
   landed in Linux 6.9. Pre-6.9 kernels use anon_inode-backed pidfds
   where all pidfds share the same `(st_dev, st_ino)`, so the
   fstat-based identity proof is structurally impossible to satisfy.
   v1.1 operators MUST upgrade their kernel to `>= 6.9` before bumping
   the d2b flake input; the v1.1 migration guide opens with this
   requirement, the eval-time gate
   `tests/v1.1-kernel-floor-eval.sh` (future, v1.1-P10) asserts the
   uplift on consumer hosts, and the daemon's pidfs runtime self-probe
   at startup is the runtime fail-closed defence. The Tier 1 Ubuntu
   floor is correspondingly uplifted: Ubuntu 24.04 LTS users need a
   HWE kernel `>= 6.9` (default 24.04 ships 6.8 at GA but the HWE
   stack tracks newer point releases) or must downgrade to v1.0.
7. Telemetry posture is none. `d2bd` makes no outbound network
   connections by default. Any future diagnostics must be explicit
   opt-in and documented in `SECURITY.md`.
8. Minijail is Nix-built and pinned through the d2b closure. Distro
   minijail packages are not relied upon.

## Consequences

1. Positive: W3 can ship `docs/reference/support-matrix.md` as the
   canonical operator-facing support table with clear tiers.
2. Positive: `CHANGELOG.md` and the W9 README install split can point to
   one ADR for platform scope, rejected targets, and telemetry posture.
3. Positive: Rejecting Alpine, musl, non-systemd autostart, and rootless
   Nix at v1 narrows the first-milestone implementation and test
   surface.
4. Negative: Operators on otherwise attractive targets such as Fedora,
   Arch, macOS, WSL, rootless Nix, Firecracker, or crosvm-as-VMM must
   wait for follow-up ADRs and panel gates.
5. Neutral: Aarch64 remains part of the cross-evaluation story, but not
   a first-milestone runtime graphics or audio target.

## Alternatives considered

- Support every glibc Linux distribution immediately: rejected because
  host-prep, networking, init integration, and packaging need one green
  non-NixOS baseline before broadening the matrix.
- Lower the kernel floor below 6.6: rejected because `fchmodat2` is part
  of the intended host and broker implementation surface, and 6.6 is a
  practical Ubuntu 24.04-compatible floor.
- Depend on distro minijail packages: rejected because minijail version,
  patches, and CVE response must be tied to the d2b closure.
- Add passive telemetry for alpha diagnostics: rejected because the
  product posture is no outbound network connections by default.

## Deferred kernel surfaces

| Kernel surface | W0b posture | Future ownership rule | Reason |
| --- | --- | --- | --- |
| `/dev/vhost-vsock` device node / fd | Deferred: W0b ships no jail-visible device node and no long-lived payload access. | Broker-only `SCM_RIGHTS` access may be enabled only by a later ADR. | Kernel `vhost_vsock` is out of scope for the first milestone per the plan's Supported platform scope. |

## References

- plan.md, "Supported platform scope"
- plan.md, "Architecture"
- plan.md, "Supervision and lifecycle invariants"
- plan.md, "W3 Host prepare and network reconcile"
- [ADR 0007](0007-bash-coexistence-and-migration.md)
