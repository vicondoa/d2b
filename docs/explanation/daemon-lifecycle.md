# Daemon lifecycle

This document captures the daemon-owned VM lifecycle. It is the
source-of-truth explanation for the per-VM DAG, the readiness
predicates, the supervisor budgets, the state persistence contract,
the restart reconciliation rules, and the daemon-level
`[pending restart]` semantics.

The `SpawnRunner` broker path described here is part of the live
control plane.

## Per-VM process DAG

Each VM declared in the public manifest gets its own
[`VmProcessDag`](../reference/manifest-schema.md) under
`processes.json`. The headless shape is a linear 5-node DAG:

```text
host-reconcile
   └─→ store-preflight
         └─→ virtiofsd-ro-store
               └─→ ch
                     └─→ guest-control-health
```

Roles, from
[`d2b_core::processes::ProcessRole`](../../packages/d2b-core/src/processes.rs):

- `host-reconcile` — bundle-derived host state catch-up (cgroup
  delegation, nft chain, route entries, sysctl ordering).
- `store-virtiofs-preflight` — validates the per-VM virtiofs share
  set against the trusted bundle's
  [`runner_shape`](../reference/runner-shape-audit.md) preflight.
- `virtiofsd` — one instance per `microvm.shares` row. The current
  headless shape uses four shares: `ro-store`, `d2b-meta`,
  `d2b-hkeys`, and `d2b-ssh-host`.
- `cloud-hypervisor-runner` — the CH binary launched against the
  argv emitted by [`d2b_host::ch_argv`](../../packages/d2b-host/src/ch_argv.rs).
- `guest-control-health` — daemon-side authenticated guest-control
  Health probe (full Hello + token challenge-response + Health over the
  guest-control vsock). It is the framework readiness gate on
  guest-control-capable VMs (`d2b.vms.<vm>.guest.control.enable =
  true`) and fails **closed**: never ready for an old-generation,
  unreachable, auth-failed, or timed-out guest. Per-VM sshd/host-keys
  are retained as a compat surface but never gate readiness: the
  legacy raw TCP-22 `ssh-ready` / `guest-ssh-readiness` DAG node was
  removed and is no longer emitted for any VM.

Optional roles wired by per-VM features:

- `swtpm` + `swtpm-pre-start-flush` when
  `d2b.vms.<vm>.tpm.enable = true` — TPM 2.0 socket sidecar
  with the documented `swtpm_ioctl -i --unix <ctrl>` pre-start
  flush.
- `vsock-relay` when `d2b.observability.enable = true` — the
  guest→host OTLP relay sidecar.
- `gpu` / `video` / `audio` — feature-gated roles not present in the
  headless shape described here.

## Topological execution + fail-fast

The supervisor uses Kahn's algorithm to topo-sort the DAG, then
walks the order issuing one `SpawnRunner` broker call per node.
The relevant pure-Rust surface lives in
[`d2bd::supervisor::dag`](../../packages/d2bd/src/supervisor/dag.rs):

- `topo_sort(VmProcessDag)` — deterministic source-pop ordering;
  cycles surface as `DagError::Cycle { residual_nodes }`. Self-loops
  count as cycles. Edges referencing an unknown node id fail
  closed with `DagError::UnknownNode { edge }`.
- `DagExecutor<R: NodeRunner>` — drives the topo-sorted DAG through
  an async-trait `NodeRunner`. The production daemon wires the
  `SpawnRunner` broker variant behind the trait; tests use an
  in-memory `FakeRunner`.
- On the first node failure the executor stops issuing spawn calls
  and marks every remaining node as
  `NodeOutcome::Skipped { predecessor }`. The returned
  `DagRunReport` always lists every node in topo order so callers
  can render `ready` / `failed` / `skipped` exhaustively in the
  typed-error envelope.

## Readiness predicates

Each [`ProcessNode`](../../packages/d2b-core/src/processes.rs)
declares zero or more `ReadinessPredicate` entries. The supervisor
treats the node as ready when every predicate fires before its
budget expires.

Supported predicate kinds (per
[`ReadinessPredicate`](../../packages/d2b-core/src/processes.rs)):

- `api-socket-info: <path>` — daemon connects to the CH API socket
  and reads `GET /api/v1/vm.info`. Pinned to `mode=0660` +
  non-empty owner per ADR 0014 §"runner-shape preflight".
- `vsock-notify: <component>` — guest or sidecar sent a
  notify-style frame on the vsock CH listens on.
- `unix-socket-exists: <path>` — daemon-side stat of the path.
  Used for virtiofsd / swtpm sockets.
- `tcp-port: { host, port }` — TCP `connect()` against
  `host:port`. A generic predicate kind retained for old-generation
  compatibility; the framework no longer emits it as the readiness
  signal (see `guest-control-health` below).
- `guest-control-health: { vm }` — daemon-side authenticated
  guest-control Health probe. Fails **closed**: ready only when the
  daemon completes the authenticated Hello + token challenge-response +
  Health exchange over the guest-control vsock. This is the framework
  readiness gate for guest-control-capable VMs.
- `command: [argv...]` — daemon-spawned probe child exits 0.
- `component-specific: <name>` — escape hatch named by the role's
  emitter; the supervisor delegates the check.

## Per-node budget

Each node has a [`NodeBudget`](../../packages/d2bd/src/supervisor/dag.rs):

```rust
NodeBudget {
    spawn:     Duration::from_secs(10),
    readiness: Duration::from_secs(30),
}
```

Defaults match the Tier-1 headless alpha target. Per-node overrides
land via the trusted bundle row; the supervisor never accepts
caller-supplied budget tweaks (the daemon never names a
spawn-related authority field across the wire).

## Graceful stop path

Stop walks the per-VM DAG in reverse, but local primary VMM runners get a
provider-aware guest shutdown phase before pidfd signal cleanup. Cloud
Hypervisor/NixOS VMs use the CH API socket and request
`PUT /api/v1/vm.shutdown`. qemu-media VMs route QMP through the broker:
`system_powerdown` for guest shutdown, `query-status` for bounded state
polling, and `quit` only after the guest is stopped and QEMU is an empty VMM.

The wait is controlled by
`d2b.daemon.lifecycle.gracefulShutdown.timeoutSeconds` (default 90,
bounded 1–600) or
`d2b.vms.<vm>.lifecycle.gracefulShutdown.timeoutSeconds`. Per-VM
`lifecycle.gracefulShutdown.enable = false` skips the provider phase without
creating a degraded marker. Explicit `d2b vm stop <vm> --force --apply`
also skips the provider wait, but still uses the normal SIGTERM/SIGKILL and
cgroup cleanup policy; it is recorded as operator intent rather than as an
unexpected degraded condition.

While a primary VMM waits for guest shutdown, required sidecars remain in the
DAG and are monitored. A required sidecar exit interrupts the graceful wait and
escalates to forced cleanup so teardown does not wait on a guest whose runtime
substrate has already failed. Reverse-DAG sidecar teardown remains after the
primary VMM stop/cleanup decision.

## Host shutdown and reboot integration

NixOS still declares only the three ADR-0015 root-visible units:
`d2bd.service`, `d2b-priv-broker.socket`, and
`d2b-priv-broker.service`. There is no per-VM or extra guest-shutdown
systemd unit. Instead, `d2bd.service` has an `ExecStop` hook that first
checks the systemd manager state with absolute systemd helper paths. It runs
the all-VM shutdown hook only when the system manager is stopping for host
shutdown or reboot; a manual `systemctl restart d2bd.service` remains a
continuation event and does not stop all VMs.

Daemon updates are also continuation events. `d2bd.service` is a
`Type=notify` unit: systemd reports the restart complete only after the daemon
has rebound `/run/d2b/public.sock`, restored/adopted runner state, and sent
`READY=1`. The unit uses `KillMode=process` so the restart terminates only the
daemon main process; broker-spawned VM runners remain alive and are re-adopted
by PID/start-time identity. If startup does not reach readiness within the
bounded start timeout, systemd fails the unit instead of presenting an active but
unready public socket.

All-VM host shutdown runs in dependency phases: workload VMs in parallel first,
then env net VMs in parallel. `TimeoutStopSec` is computed from the maximum
enabled graceful timeout in each phase, plus bounded forced-fallback and
sidecar-cleanup budgets, and is emitted with `lib.mkDefault` so host operators
can intentionally override it. `d2bd.service` orders after the broker
socket/service and D-Bus so broker-mediated qemu-media shutdown remains
available while live VMMs are being stopped.

## State persistence + restart reconciliation

On every supervisor transition the daemon writes a
[`RunnerSnapshotRecord`](../../packages/d2bd/src/supervisor/state.rs)
to `/var/lib/d2b/daemon-state/<vm>/runtime.<role_id>.json`:

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
   `/var/lib/d2b/daemon-state/`.
2. For each snapshot, reads `/proc/<pid>/stat` and parses field 22
   (`starttime` ticks) using `parse_proc_stat_starttime` (handles
   comm with spaces and parens via the LAST-`)` split).
3. Classifies the snapshot as one of:
   - `ReconciliationOutcome::Adopt` — `(pid, start_time_ticks)`
     matches. The supervisor re-opens the pidfd via
     `nix::sys::pidfd::pidfd_open(pid)` and re-registers the slot
     in the `PidfdTable`.
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

Raw-pid kill/wait is **forbidden** outside the reconciliation path.
Reconciliation is the only context where the daemon validates
`(pid, start_time_ticks)` against the trusted snapshot before
considering raw-pid semantics for the re-adoption window — and even
there, the moment the pidfd is re-opened the daemon switches back to
`pidfd_send_signal` exclusively for signal delivery. The broker, not
the daemon, reaps `SpawnRunner` children; see
[ADR 0018](../adr/0018-microvm-nix-removal.md) § "broker-as-parent
reaping model".

## Daemon-level `[pending restart]`

The CLI already surfaces a per-VM `[pending restart]` (when the
VM's `current` symlink diverges from its `booted` symlink). This
same idea also applies to the daemon binary itself.

On startup the daemon writes
[`DaemonVersionFile`](../../packages/d2bd/src/daemon_version.rs)
to `/run/d2b/version`:

```jsonc
{
  "serverVersion":   "0.4.0",
  "binaryPath":      "/nix/store/abc-d2bd-0.4.0/bin/d2bd",
  "startedAt":       "2026-05-29T03:00:00Z",
  "protocolVersion": 3
}
```

The CLI reads the file and runs `std::fs::canonicalize` against the
on-disk install path (`/run/current-system/sw/bin/d2bd` on NixOS,
the non-NixOS install path otherwise). `compute_restart_status`
returns:

- `DaemonRestartStatus::UpToDate` — paths match.
- `DaemonRestartStatus::PendingRestart { running_path, on_disk_path }`
  — paths differ. A `systemctl restart d2bd` will pick up the
  new binary.
- `DaemonRestartStatus::DaemonNotRunning` — `/run/d2b/version`
  is absent (CLI surfaces this as `daemon-down`).
- `DaemonRestartStatus::VersionFileUnreadable { detail }` — present
  but unparseable; the CLI refuses to compute the pending-restart
  signal and logs the detail.

The status command renders the banner via `restart_status_banner`
alongside the per-VM `[pending restart]` annotations.

## Virtiofsd watchdog

The old per-share `d2b-<vm>-virtiofsd@<share>.service`
ExecStopPost-style bash health check, driven by the
`d2b-vfsd-watchdog@<vm>.{timer,service}` pair, has been replaced
by daemon/broker pidfd supervision. The broker is the parent and sole
reaper of every `SpawnRunner` child (including Virtiofsd); the daemon
observes via the broker's `ChildExited` / `OneShotComplete` RPC
notifications and its own duplicated pidfd handle. The daemon's typed
state machine — not a bash one-shot — decides what happens next.

Each virtiofsd runner the broker spawns is registered in two places:

- the broker's parent-side pidfd table, where it is reaped via
  `waitid(P_PIDFD)`; and
- the daemon's pidfd table under its `(vm, role_id)` key, where the
  duplicated pidfd is used only for `pidfd_send_signal` and poll
  observability.

On `ChildExited` RPC, the daemon invokes
[`supervisor::pidfd::handle_runner_exit`](../../packages/d2bd/src/supervisor/pidfd.rs)
with the `(exit_code, signal)` from the broker's reap, NOT from a
local `waitid` (the daemon is not the parent and cannot reap;
`waitid(P_PIDFD)` would return `ECHILD`).

The handler:

1. Returns an empty outcome for clean shutdowns
   (`exit_code == 0`, no signal) — that's a stop-initiated reap, not
   a watchdog event.
2. For any other exit, emits three typed `SupervisorEvent`s onto the
   supervisor's event channel:
   - `VfsdDied { vm, role_id, exit }` — the audit-facing typed event.
   - `VfsdShareDegraded { vm, role_id }` — the per-share mount is now
     unrecoverable; `d2b status <vm>` surfaces this in the
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
   appending to `/var/lib/d2b/audit/broker-<utc-date>.jsonl`.

The in-daemon detection-and-degradation path has replaced the old
per-share systemd template/watchdog combination
(`microvm-virtiofsd@<vm>.service` /
`d2b-vfsd-watchdog@<vm>`).

## References

- [ADR 0004](../adr/0004-cloud-hypervisor-runner-shape.md) — CH
  argv shape + per-role minijail decision.
- [ADR 0011](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md)
  — pidfd handoff + cgroup-v2 delegation. The older
  `PR_SET_CHILD_SUBREAPER` contract is superseded for the
  SpawnRunner-child population per ADR 0018 § "set-booted race-free
  serialization" — neither broker nor daemon claims subreaper for
  SpawnRunner children.
- [ADR 0014](../adr/0014-w3-modules-devices-runner-shape.md) —
  runner-shape preflight + CH net-handoff probe.
- [Daemon API reference](../reference/daemon-api.md) — wire
  envelope shapes and typed-error catalog.
- [`d2b_host::ch_argv`](../../packages/d2b-host/src/ch_argv.rs)
  / [`swtpm_argv`](../../packages/d2b-host/src/swtpm_argv.rs) —
  pure argv generators feeding the broker `SpawnRunner` op.
  virtiofsd argv is emitted from `nixos-modules/processes-json.nix`
  because each share is already resolved during the VM eval.
- [`d2bd::supervisor::dag`](../../packages/d2bd/src/supervisor/dag.rs)
  / [`state`](../../packages/d2bd/src/supervisor/state.rs)
  / [`pidfd`](../../packages/d2bd/src/supervisor/pidfd.rs) — the
  supervisor surface itself.
- [`d2bd::daemon_version`](../../packages/d2bd/src/daemon_version.rs)
  — `[pending restart]` machinery.
