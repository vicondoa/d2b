# Daemon autostart contract

`nixlingd` runs a single **autostart pass** on startup that brings
per-env net VMs and workload VMs up in a controlled, degraded-aware
order. The contract is intentionally narrow: it sequences the VMs,
caps how many start at once, tolerates failures, and is safe to
re-run.

This page is the reference. The Rust implementation lives in
`packages/nixlingd/src/autostart.rs`; the production starter wires
into `dispatch_broker_vm_start` so each per-VM start drives the
same host-prep DAG → process DAG → pidfd-registration sequence
that a manual `vm start` would.

## When the pass runs

`nixlingd::serve()` calls `run_startup_autostart()` once, after:

1. `PidfdTable::restore_from_disk` has loaded any previously
   supervised runners off `/var/lib/nixling/daemon-state/`.
2. `adopt_orphaned_runners_on_startup` has reconciled the table
   against `/proc` (so VMs that survived a daemon restart are
   already accounted for).

Only then does the pass dispatch the plan. The daemon's accept
loop starts immediately afterwards; autostart progress is logged
to `journalctl -u nixlingd.service` but never blocks the public
socket from serving `status` / `doctor` / `audit`.

If the trusted bundle fails to load, autostart is **skipped** with
a warning — the daemon must still come up so operators can run
`nixling doctor` against a broken bundle.

## Plan order

`build_autostart_plan(resolver)` derives the plan from the loaded
bundle. The shape is intentionally simple so two operators on the
same bundle always see the same order (Net VMs first, then
workloads):

1. **Net VMs.** Every VM where `is_net_vm = true` (i.e., the
   auto-declared `sys-<env>-net` VMs from
   `nixos-modules/network.nix`). Sorted by `(env, vm-name)`.
2. **Workloads.** Every other VM, sorted by `(env, vm-name)` so
   workloads pin to their env's net VM in plan order.

The daemon currently derives the `autostart` flag heuristically:
every VM the manifest knows about is autostart-eligible **unless**
it is a graphics VM (graphics VMs are barred from autostart by
`nixos-modules/assertions.nix` SWArch-M9 — they have no Wayland
session at boot).

VMs with `autostart = false` remain in the plan (so a future
`nixling status --plan` can surface the full picture) but
`execute_autostart` skips them with `Outcome::NotAutostart`. A
non-autostart net VM does **not** propagate as a degraded gate for
its env's workloads — opting out is an explicit operator choice,
not a failure.

## Concurrency cap

Both stages honour a single concurrency cap N
(`nixling.daemon.autostart.parallelism`, default `3`):

- up to N net VMs start in parallel in stage 1;
- once stage 1 settles (every net VM has reached a terminal
  outcome), up to N workloads start in parallel in stage 2.

Values `< 1` are clamped to `1`. The cap is enforced with a
`tokio::sync::Semaphore`; each per-VM start runs on a
`spawn_blocking` worker so the broker round-trip can use plain
sync I/O.

## Degraded mode

Failures are isolated, not abortive:

- A net VM failure (`Outcome::Failed`) does NOT block sibling net
  VMs in other envs. Once stage 1 ends, every workload whose env
  has a failed net VM is recorded as `Outcome::Degraded` with a
  human-readable `reason` pointing at the upstream failure. The
  workload's start machinery is **not** dispatched.
- A workload failure is recorded as `Outcome::Failed` but does not
  block sibling workloads (including workloads in the same env).
- A net VM whose status was `Outcome::NotAutostart` does NOT
  degrade its env. Operators routinely opt the framework-declared
  `sys-<env>-net` VM out of autostart on hosts where the env's net
  topology is managed by hand.

The daemon itself stays up regardless of autostart outcomes; the
accept loop is reachable as soon as `run_startup_autostart`
returns.

## Idempotency (Idempotent re-entry)

Every per-VM dispatch is gated on
`VmStarter::is_running(vm)`, which the production
`BrokerVmStarter` implements as
`pidfd_table.contains(vm, "ch-runner")`. A second invocation of
the autostart pass against the same live daemon reports
`Outcome::AlreadyRunning` for every VM the previous pass started
and dispatches nothing new. This is the property the SIGHUP /
bundle-reload path relies on: a future `nixlingctl reload` can
re-run the pass without double-spawning runners.

## Configuration

NixOS option set (`nixos-modules/options-daemon.nix`):

```nix
nixling.daemon.autostart = {
  # Concurrency cap N for the autostart pass. Net-VM phase and
  # workload phase each honour the cap independently. Default 3;
  # values < 1 are clamped to 1.
  parallelism = 3;
};
```

The Rust side exposes the same default via
`autostart::DEFAULT_PARALLELISM` and reads the value off
`DaemonConfig::autostart_parallelism` (camelCase
`autostartParallelism` on the wire / on disk).

## Outcomes (`Outcome` enum)

| Variant            | Meaning                                                                                       |
| ------------------ | --------------------------------------------------------------------------------------------- |
| `Started`          | The VM was not running and the per-VM start sequence succeeded.                               |
| `AlreadyRunning`   | The pidfd table already supervises this VM; nothing dispatched (idempotency short-circuit).   |
| `NotAutostart`     | Plan row carried `autostart = false`; nothing dispatched. Not propagated as degraded.         |
| `Failed { reason }`| `VmStarter::start` returned an error; sibling VMs continue.                                   |
| `Degraded { reason }` | Workload skipped because its env's net VM is `Failed` (or `Degraded`).                    |

The companion `AutostartReport` preserves plan order so the
journal record reads as the operator expects: net VMs first, then
workloads grouped by env.

## Testing

- Unit tests live in `packages/nixlingd/src/autostart.rs`
  (`cargo test --lib autostart`). They cover ordering,
  concurrency-cap enforcement, degraded-mode propagation,
  idempotent re-entry, and the `parallelism = 0` clamp.
- `tests/daemon-autostart-eval.sh` is a static + small nixpkgs
  eval gate that asserts the public Rust surface, the NixOS
  option default + override, the daemon-side wiring, and the
  documentation cross-references.
- The full Layer-2 smoke (`tests/daemon-autostart-smoke.sh`) is
  out of scope for this page — it brings up a real 2-env × 2-workload
  fixture and asserts the net-VM-first envelope on hardware-like
  state.

## Cross-references

- [`docs/reference/daemon-api.md`](daemon-api.md) — daemon
  lifecycle and where the autostart pass slots in.
- [`docs/reference/host-prep-dag.md`](host-prep-dag.md) — per-VM
  host-prep step set that `dispatch_broker_vm_start` drives once
  the autostart layer picks the VM.
- [`docs/reference/per-vm-state-ownership.md`](per-vm-state-ownership.md)
  — ownership-matrix preflight that gates every per-VM start
  (autostart or otherwise).
