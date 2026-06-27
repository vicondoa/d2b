# 0001. Systemd-free VM orchestration

- Status: Accepted
- Date: 2026-05-25
- Wave: W0b
- Plan slice: "The target product promise is systemd-free VM orchestration, not \"systemd-free installation everywhere.\""
- Companion ADRs: [ADR 0002](0002-non-root-daemon-and-privileged-broker.md), ADR 0004, ADR 0007

## Context

> **v1.0 status note (per [ADR 0015](0015-daemon-only-clean-break.md)):**
> The framework state this ADR's Context section describes is the
> pre-P6 v0.4 baseline. v1.0 retired the per-VM
> `d2b@<vm>.service` wrapper (pre-P6; retired in P6 per ADR 0015), the host-singleton bash dispatcher,
> the `d2b-launcher` polkit allowlist, and the
> `/run/d2b/locks/<vm>` filesystem lock; per-VM orchestration now
> runs entirely in `d2bd` with broker `SpawnRunner` + pidfd
> handoff as the single-writer enforcement. ADR 0001's portability
> decision (move per-VM orchestration into `d2bd`) is the
> foundation; ADR 0015 documents the v1.0 clean-break completion.
> The text below is preserved as historical record of the v0.4
> baseline this ADR was responding to.

The v0.4.0 baseline is a NixOS-host framework where the Nix module
emits per-VM `microvm@<vm>.service`, wrapper `d2b@<vm>.service` (pre-P6; retired in P6 per ADR 0015),
and framework-owned sidecar services for GPU, video, audio, swtpm,
virtiofsd, store synchronization, and net VMs. AGENTS.md documents this
as the current naming contract and calls out per-VM systemd services as
the authoritative lifecycle surface.

That baseline also has a load-bearing no-auto-restart policy:
framework-owned or framework-touched per-VM lifecycle services carry
`restartIfChanged = false`, and drift is surfaced through the `current`
and `booted` symlinks plus `[pending restart]` output. The policy exists
because restarting a sidecar can terminate Cloud Hypervisor and destroy
session state, especially for graphics VMs.

The portability plan moves per-VM orchestration into `d2bd` while
retaining Nix as the producer of guest closures, manifests, runner
metadata, and optional host bootstrap integration. This means systemd may
remain an init mechanism for starting the daemon on NixOS or other hosts,
but it must no longer be the per-VM supervisor for daemon-owned VMs.

The migration period must preserve single-writer semantics. The plan's
required test families demand that daemon-owned VMs have no active
matching per-VM `microvm@<vm>` or `d2b@<vm>` unit, that ownership is
declared in manifests, and that a filesystem lock prevents systemd and
`d2bd` from supervising the same VM concurrently.

## Decision

1. The product promise is "systemd-free VM orchestration", not "systemd-free installation everywhere".
2. A daemon-owned VM produces no active `microvm@<vm>.service` or `d2b@<vm>.service` unit, and tests treat that as the orchestration ownership invariant (the wrapper unit was retired in P6 per ADR 0015, so daemon-owned VMs trivially satisfy this invariant in v1.0).
3. `d2bd`, legacy systemd entry points, and any transitional CLI path must acquire `/run/d2b/locks/<vm>` as the single-writer filesystem lock before mutating VM lifecycle state.
4. NixOS may start `d2bd` as a systemd service, non-NixOS hosts may use their native init for daemon bootstrap, and all per-VM orchestration for daemon-owned VMs runs inside `d2bd`.
5. Any framework-emitted unit that remains carries `restartIfChanged = false`, and `d2bd` never auto-restarts a running child on config change because drift appears as `[pending restart]` in `d2b list` and `d2b status`.
6. `d2bd` updates the `booted` symlink atomically only after runner readiness, while `current` is updated only by explicit `d2b` activate flows or by NixOS activation in legacy mode.

## Consequences

1. Positive: The user-visible portability claim is precise and does not require replacing every host init system before per-VM orchestration becomes portable.
2. Positive: The no-auto-restart invariant survives the supervisor transition and remains testable through the same drift surface.
3. Positive: The filesystem lock gives legacy-systemd, daemon-experimental, and daemon-default migration modes a shared single-writer guard.
4. Negative: NixOS activation and legacy units need explicit daemon-owned no-op or fail-fast behavior during migration, which ADR 0007 owns in detail.
5. Neutral: Existing v0.4.0 unit naming remains relevant for legacy mode, AGENTS.md, and compatibility tests even though daemon-owned VMs stop emitting active per-VM units.

## Alternatives considered

- Promise a completely systemd-free installation: rejected because NixOS and many non-NixOS hosts may still use systemd or another init to bootstrap `d2bd`.
- Keep per-VM systemd units as the daemon backend: rejected because it preserves systemd as the orchestrator and blocks the portability goal.
- Auto-restart daemon children on manifest drift: rejected because it regresses the v0.4.0 session-state safety policy.
- Rely only on manifest ownership without a runtime lock: rejected because migration modes need a kernel-enforced single-writer mechanism.

## References

- plan.md, "Problem and goal"
- plan.md, "Supervision and lifecycle invariants"
- plan.md, "Migration modes"
- plan.md, "Required test families"
- AGENTS.md, "Naming conventions"
- AGENTS.md, "VM lifecycle policy (v0.1.5+)"
