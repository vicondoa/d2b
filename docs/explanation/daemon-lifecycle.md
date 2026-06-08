# Daemon lifecycle (W4 main wave)

This document captures the daemon-owned VM lifecycle that lands in W4
main. It is the source-of-truth explanation for the per-VM DAG, the
readiness predicates, the supervisor budgets, the state persistence
contract, the restart reconciliation rules, and the daemon-level
`[pending restart]` semantics.

The wire surface (W4-H5 `SpawnRunner` broker variant) is stable;
broker-side execution of `SpawnRunner` lands in **W4-fu** (the
follow-up wave). Every `nixling vm <verb> --apply` today returns
the documented W3 typed `daemon-down` envelope; `--dry-run` returns
the DAG the supervisor will drive when W4-fu ships.

## Per-VM process DAG

Each VM declared in the public manifest gets its own
[`VmProcessDag`](../reference/manifest-schema.md) under
`processes.json`. The W4 headless alpha Tier-1 shape is a linear
5-node DAG:

```text
host-reconcile
   └─→ store-preflight
         └─→ virtiofsd-ro-store
               └─→ ch
                     └─→ ssh-ready
```

Roles, from
[`nixling_core::processes::ProcessRole`](../../packages/nixling-core/src/processes.rs):

- `host-reconcile` — bundle-derived host state catch-up (cgroup
  delegation, nft chain, route entries, sysctl ordering).
- `store-virtiofs-preflight` — validates the per-VM virtiofs share
  set against the trusted bundle's
  [`runner_shape`](../reference/runner-shape-audit.md) preflight.
- `virtiofsd` — one instance per `microvm.shares` row. W4 alpha
  ships the four-share shape from the W0b audit
  (`ro-store`, `nl-meta`, `nl-hkeys`, `nl-ssh-host`).
- `cloud-hypervisor-runner` — the CH binary launched against the
  argv emitted by [`nixling_host::ch_argv`](../../packages/nixling-host/src/ch_argv.rs).
- `guest-ssh-readiness` — daemon-side probe that the guest is
  reachable on the allocated TAP + static IP.

Optional roles wired by per-VM features:

- `swtpm` + `swtpm-pre-start-flush` when
  `nixling.vms.<vm>.tpm.enable = true` — TPM 2.0 socket sidecar
  with the W3-invariant `swtpm_ioctl -i --unix <ctrl>` pre-start
  flush.
- `vsock-relay` when `nixling.observability.enable = true` — the
  guest→host OTLP relay sidecar.
- `gpu` / `video` / `audio` — W5 deliverables; the W4 main wave
  ships only the headless shape, no graphics/audio/USBIP roles.

## Topological execution + fail-fast

The supervisor uses Kahn's algorithm to topo-sort the DAG, then
walks the order issuing one `SpawnRunner` broker call per node.
The relevant pure-Rust surface lives in
[`nixlingd::supervisor::dag`](../../packages/nixlingd/src/supervisor/dag.rs):

- `topo_sort(VmProcessDag)` — deterministic source-pop ordering;
  cycles surface as `DagError::Cycle { residual_nodes }`. Self-loops
  count as cycles. Edges referencing an unknown node id fail
  closed with `DagError::UnknownNode { edge }`.
- `DagExecutor<R: NodeRunner>` — drives the topo-sorted DAG through
  an async-trait `NodeRunner`. The production daemon wires the
  W4-H5 `SpawnRunner` broker variant behind the trait; tests use
  an in-memory `FakeRunner`.
- On the first node failure the executor stops issuing spawn calls
  and marks every remaining node as
  `NodeOutcome::Skipped { predecessor }`. The returned
  `DagRunReport` always lists every node in topo order so callers
  can render `ready` / `failed` / `skipped` exhaustively in the
  W3 typed-error envelope.

## Readiness predicates

Each [`ProcessNode`](../../packages/nixling-core/src/processes.rs)
declares zero or more `ReadinessPredicate` entries. The supervisor
treats the node as ready when every predicate fires before its
budget expires.

Supported predicate kinds (per
[`ReadinessPredicate`](../../packages/nixling-core/src/processes.rs)):

- `api-socket-info: <path>` — daemon connects to the CH API socket
  and reads `GET /api/v1/vm.info`. Pinned to `mode=0660` +
  non-empty owner per ADR 0014 §"runner-shape preflight".
- `vsock-notify: <component>` — guest or sidecar sent a
  notify-style frame on the vsock CH listens on.
- `unix-socket-exists: <path>` — daemon-side stat of the path.
  Used for virtiofsd / swtpm sockets.
- `tcp-port: { host, port }` — TCP `connect()` against
  `host:port`. Used for the guest SSH readiness probe.
- `command: [argv...]` — daemon-spawned probe child exits 0.
- `component-specific: <name>` — escape hatch named by the role's
  emitter; the supervisor delegates the check.

## Per-node budget

Each node has a [`NodeBudget`](../../packages/nixlingd/src/supervisor/dag.rs):

```rust
NodeBudget {
    spawn:     Duration::from_secs(10),
    readiness: Duration::from_secs(30),
}
```

Defaults match the Tier-1 headless alpha target. Per-node overrides
land via the trusted bundle row; the supervisor never accepts
caller-supplied budget tweaks (security-1: the daemon never names a
spawn-related authority field across the wire).

## State persistence + restart reconciliation

On every supervisor transition the daemon writes a
[`RunnerSnapshotRecord`](../../packages/nixlingd/src/supervisor/state.rs)
to `/var/lib/nixling/daemon-state/<vm>/runtime.<role_id>.json`:

```jsonc
{
  "vm":              "corp-vm",
  "roleId":          "ch",
  "role":            "cloud-hypervisor",
  "pid":             4242,
  "startTimeTicks":  987654321,
  "snapshottedAt":   "2026-05-29T03:00:00Z"
}
```

Writes are tmp+rename so a crash mid-write leaves the previous
snapshot intact. Snapshots are per-(vm, role_id) so updating one
role does not touch unrelated VMs.

On daemon startup the supervisor:

1. Enumerates every persisted snapshot under
   `/var/lib/nixling/daemon-state/`.
2. For each snapshot, reads `/proc/<pid>/stat` and parses field 22
   (`starttime` ticks) using `parse_proc_stat_starttime` (handles
   comm with spaces and parens via the LAST-`)` split).
3. Classifies the snapshot as one of:
   - `ReconciliationOutcome::Adopt` — `(pid, start_time_ticks)`
     matches. **In W4-fu** the supervisor re-opens the pidfd via
     `nix::sys::pidfd::pidfd_open(pid)` and re-registers the slot
     in the W3 s1 `PidfdTable`. **In W4 main** the reconciliation
     module only classifies the outcome; the actual `pidfd_open`
     call lands together with the broker-side `SpawnRunner`
     execution in W4-fu.
   - `ReconciliationOutcome::Quarantine { observed_start_time_ticks }`
     — PID still exists, but `start_time_ticks` drifted. The PID
     was reused by an unrelated process. The slot is parked with
     an audit event `quarantine-pid-drift`; the supervisor does
     NOT control the process further. Operator decides whether
     to kill (`pidfd_send_signal` after an ADR carve-out) or wait
     it out.
   - `ReconciliationOutcome::Missing` — `/proc/<pid>/` is gone.
     Snapshot is deleted; runner is treated as not-running on the
     next supervisor pass.
   - `ReconciliationOutcome::UnparseableProcStat { detail }` —
     `/proc/<pid>/stat` was present but field 22 could not be
     parsed. Treated as quarantine because we cannot prove
     safety of re-adoption.

Per the W3 s1 pidfd contract, raw-pid kill/wait is **forbidden**
outside the reconciliation path. Reconciliation is the only context
where the daemon validates `(pid, start_time_ticks)` against the
trusted snapshot before deciding to use raw-pid semantics for the
re-adoption window — and even there, the moment the pidfd is
re-opened the daemon switches back to `pidfd_send_signal` /
`waitid(P_PIDFD)` exclusively.

## Daemon-level `[pending restart]`

The W3 CLI already surfaces a per-VM `[pending restart]` (when the
VM's `current` symlink diverges from its `booted` symlink). W4 adds
the daemon-binary equivalent.

On startup the daemon writes
[`DaemonVersionFile`](../../packages/nixlingd/src/daemon_version.rs)
to `/run/nixling/version`:

```jsonc
{
  "serverVersion":   "0.4.0",
  "binaryPath":      "/nix/store/abc-nixlingd-0.4.0/bin/nixlingd",
  "startedAt":       "2026-05-29T03:00:00Z",
  "protocolVersion": 3
}
```

The CLI reads the file and runs `std::fs::canonicalize` against the
on-disk install path (`/run/current-system/sw/bin/nixlingd` on NixOS,
the Tier-0 install path otherwise). `compute_restart_status` returns:

- `DaemonRestartStatus::UpToDate` — paths match.
- `DaemonRestartStatus::PendingRestart { running_path, on_disk_path }`
  — paths differ. A `systemctl restart nixlingd` will pick up the
  new binary.
- `DaemonRestartStatus::DaemonNotRunning` — `/run/nixling/version`
  is absent (CLI surfaces this as `daemon-down`).
- `DaemonRestartStatus::VersionFileUnreadable { detail }` — present
  but unparseable; the CLI refuses to compute the pending-restart
  signal and logs the detail.

The status command renders the banner via `restart_status_banner`
alongside the per-VM `[pending restart]` annotations.

## Virtiofsd watchdog (P2)

Before P2 the per-share `nixling-<vm>-virtiofsd@<share>.service`
ExecStopPost-style bash health check, driven by the
`nixling-vfsd-watchdog@<vm>.{timer,service}` pair, was the only
surface that noticed a virtiofsd sidecar dying mid-run. P2 folds
that detection into the daemon's pidfd reaper so the supervisor's
typed state machine — not a bash one-shot — decides what happens
next.

Each virtiofsd runner the daemon spawns is registered in the W3 s1
pidfd table under its `(vm, role_id)` key. When the reaper observes
a pidfd exit for a slot whose `RunnerRole == Virtiofsd`, it consults
[`supervisor::pidfd::handle_runner_exit`](../../packages/nixlingd/src/supervisor/pidfd.rs)
with the exit's `(exit_code, signal)`. The handler:

1. Returns an empty outcome for clean shutdowns
   (`exit_code == 0`, no signal) — that's a stop-initiated reap, not
   a watchdog event.
2. For any other exit, emits three typed `SupervisorEvent`s onto the
   supervisor's event channel:
   - `VfsdDied { vm, role_id, exit }` — the audit-facing typed event.
   - `VfsdShareDegraded { vm, role_id }` — the per-share mount is now
     unrecoverable; `nixling status <vm>` surfaces this in the
     per-VM degraded counter.
   - `StopRunnerRequested { runner_role: CloudHypervisor, reason }` —
     drives CH down through the existing SIGTERM→SIGKILL pidfd
     ladder. Suppressed when
     `VfsdWatchdogPolicy::stop_ch_on_unexpected_exit = false`; the
     default is `true` because a dead virtiofsd leaves the guest's
     root-share FUSE path irrecoverable.
3. Returns a `VfsdDiedAuditRecord` with `event = "vfsd-died"`,
   `policy_stopped_ch`, and the classified exit — the integrator
   wraps it into the existing `OpAuditRecord` envelope before
   appending to `/var/lib/nixling/audit/broker-<utc-date>.jsonl`.

The per-share systemd template (`microvm-virtiofsd@<vm>.service` /
`nixling-vfsd-watchdog@<vm>`) stays on disk until the P6 deletion
sweep; P2 only owns the in-daemon detection-and-degradation path.

## References

- [ADR 0004](../adr/0004-cloud-hypervisor-runner-shape.md) — CH
  argv shape + per-role minijail decision.
- [ADR 0011](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md)
  — pidfd handoff + `PR_SET_CHILD_SUBREAPER` contract.
- [ADR 0014](../adr/0014-w3-modules-devices-runner-shape.md) —
  runner-shape preflight + CH net-handoff probe.
- [Daemon API reference](../reference/daemon-api.md) — wire
  envelope shapes and typed-error catalog.
- [`nixling_host::ch_argv`](../../packages/nixling-host/src/ch_argv.rs)
  / [`virtiofsd_argv`](../../packages/nixling-host/src/virtiofsd_argv.rs)
  / [`swtpm_argv`](../../packages/nixling-host/src/swtpm_argv.rs) —
  pure argv generators feeding the W4-H5 broker spawn op.
- [`nixlingd::supervisor::dag`](../../packages/nixlingd/src/supervisor/dag.rs)
  / [`state`](../../packages/nixlingd/src/supervisor/state.rs)
  / [`pidfd`](../../packages/nixlingd/src/supervisor/pidfd.rs) — the
  supervisor surface itself.
- [`nixlingd::daemon_version`](../../packages/nixlingd/src/daemon_version.rs)
  — `[pending restart]` machinery.
