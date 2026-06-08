# 0011. cgroup v2 delegation slice naming, 8-step algorithm, and non-root delegation

- Status: Accepted
- Date: 2026-05-27
- Wave: W3 (scope s1)
- Companion ADRs: [ADR 0002](0002-non-root-daemon-and-privileged-broker.md), [ADR 0010](0010-wire-protocol-and-typed-errors.md)
- Reference: [docs/reference/cgroup-delegation.md](../reference/cgroup-delegation.md)
- Operator how-to: [docs/how-to/host-prepare.d/cgroup.md](../how-to/host-prepare.d/cgroup.md)

## Context

`nixlingd` must run per-VM payloads inside their own cgroup leaves so
the daemon can enforce CPU/memory/io/pids quotas, attach pidfds for
authoritative control, and kill stuck payloads at teardown — but
[ADR 0002](0002-non-root-daemon-and-privileged-broker.md) commits to
running the daemon as a non-root system user. Linux's cgroup v2
"delegation" model is exactly the right primitive for this: a root
process performs the one-time `+controllers` enable on the ancestor
chain and chowns a subtree to a non-root uid, after which the non-root
process owns every mutation inside that subtree.

W3 must pick:

1. the slice name and shape (one root slice vs many env-scoped slices);
2. the exact ordered sequence of writes that constitutes "delegation";
3. the kill scope (leaf-only vs ancestor-allowed);
4. whether to allow threaded cgroups, cpuset partition roots, and
   internal processes;
5. the failure-closed code path for hosts that can't support this
   (legacy cgroup v1, missing controllers, root-only environments).

## Decision

1. **Single slice, fixed name.** The delegated subtree is
   `/sys/fs/cgroup/nixling.slice` — literally that name, not
   configurable. Per-VM leaves are `<vm-id>.scope` beneath it. This
   keeps the operator mental model and the broker's bundle-derived
   path resolution trivial.

2. **8-step algorithm.** The broker performs delegation in the exact
   sequence documented in
   [docs/reference/cgroup-delegation.md](../reference/cgroup-delegation.md):
   probe → controller floor → cpuset inheritance → ordered
   `+cpu/+memory/+io/+pids/+cpuset` enable with per-step verification
   → slice mkdir → `member`-partition assertion → fd-based subtree
   chown → leaf-only kill enforcement → uid-0 refusal. Each step has
   a stable kebab-case error code that flows through the CLI
   contract (`docs/reference/error-codes.md`) and the broker audit
   record `error_kind` field.

3. **`partition=member` everywhere.** `cpuset.cpus.partition` is never
   written by W3 code. `assert_partition_member_only` panics in
   debug builds and returns `cgroup-partition-root-forbidden` in
   release builds. Partition-root creation would require a separate
   panel-approved ADR override.

4. **Threaded cgroups forbidden.** `cgroup.type=threaded` is refused
   with `cgroup-threaded-forbidden`. W3 has no use case that requires
   thread-granularity cgroups.

5. **No internal processes.** `nixling.slice` and intermediate VM
   cgroup directories MUST stay process-free; leaf role cgroups are
   the only directories that carry processes.

6. **Kill scope is leaf-only.** `cgroup.kill` is allowed only on
   broker/daemon-owned VM or role leaves during declared
   teardown/cleanup; ancestor `cgroup.kill` is refused with
   `cgroup-kill-on-ancestor-refused`. The supervisor uses
   `pidfd_send_signal(SIGTERM)` first and only escalates to
   `cgroup.kill` on the leaf as a last resort.

7. **Non-root delegation is mandatory.** The broker delegates as the
   target uid (`nixlingd`), never as uid 0. `require_non_root_delegation`
   asserts `getuid() != 0` and returns `cgroup-delegation-refused`
   otherwise.

8. **pidfd handoff over SCM_RIGHTS.** The broker forks payloads via
   `clone3(CLONE_PIDFD)` (preferred) or `fork + pidfd_open` (fallback)
   and transports the resulting CLOEXEC pidfd to `nixlingd` over
   `priv.sock` via SCM_RIGHTS. The daemon takes ownership in
   `PidfdTable` and registers each pidfd in its tokio epoll/poll loop.
   Raw-pid kill/wait is forbidden except in the reconciliation path
   where pid + `/proc/<pid>/stat` field 22 are both validated.
   `nixlingd` sets `PR_SET_CHILD_SUBREAPER` at startup with a self-test
   on `PR_GET_CHILD_SUBREAPER`.

## Rejected alternatives

### libcgroup

libcgroup encapsulates the v2 delegation steps in a higher-level API.
Rejected because:

- it cannot enforce that the broker re-derives paths from the
  trusted bundle — it accepts caller-supplied paths;
- its kill scope is process-list-based, not cgroup-leaf-bounded;
- the C ABI surface adds a panel-relevant supply-chain dependency
  outside our existing Rust-only crate inventory;
- it does not enforce non-root delegation at the API surface.

### systemd `Slice=` delegation without the broker

Letting `nixling.slice` be a real `nixling.slice` systemd slice with
`Delegate=yes` and bypassing the broker for the delegation step.
Rejected because:

- it makes systemd the authority over which controllers are enabled,
  in what order, and on which ancestors — losing the
  bundle-derived-paths invariant;
- the cpuset-inheritance step is implicit and varies across systemd
  versions, which breaks our `cpuset-inheritance-failed` failure-mode
  contract;
- the audit record schema diverges from the broker's W3 baseline
  (`DelegateCgroupV2` row), which breaks the CLI error-code golden
  table.

### Per-env or per-VM slices instead of a single `nixling.slice`

Considered to make cross-env quota isolation more visible. Rejected
because:

- the cpuset inheritance step has to repeat for every additional
  ancestor, multiplying the failure surface;
- per-env slices would force the `OpenCgroupDir` audit `path_class`
  enum to grow with each environment, breaking the closed-enum gate;
- the existing `<vm>.scope` leaf naming already gives unambiguous
  per-VM identity, so the extra hierarchy doesn't buy isolation
  beyond what's already there.

### Partition roots (`cpuset.cpus.partition=root`)

Partition roots would let the slice own a dedicated CPU set isolated
from the host scheduler domain. Rejected for W3 because:

- the partition state machine has additional failure modes
  (`partition-root-invalid`) that aren't covered by the W3 CLI
  golden table;
- partition-root creation can fail at runtime if the requested CPUs
  aren't isolatable, which breaks the idempotent reconcile
  invariant.

A future ADR override may enable partition roots for workload VMs
that genuinely need scheduler isolation.

### Threaded cgroups

Rejected: no W3 use case requires thread-granularity cgroups, and
threaded cgroups have surprising interactions with `cgroup.kill` and
the controller advertisement set.

### Raw-pid kill/wait

Rejected: pid reuse is real, and the daemon must hold an
authoritative kernel-side handle for every payload it controls.
pidfd plus reconciliation via pid + start-time is the only path that
survives pid reuse cleanly. The reconciliation path is the single
exception: it is the only place where `pidfd_open` keyed on a raw
pid integer is acceptable, and even there the daemon refuses to
accept the fd until `/proc/<pid>/stat` field 22 matches the value
the broker captured at spawn.

## Consequences

- Positive: every cgroup mutation is auditable, bundle-derived, and
  exercised by the L1c canary matrix in
  `tests/cgroup-delegation-oracle.sh`.
- Positive: pidfd-based supervision is robust against pid reuse and
  removes the need for `setpgid`/`killpg` heuristics in the daemon.
- Positive: the closed enum + plan-named error codes give the CLI
  golden table a finite set of W3 cgroup failure rows.
- Negative: hosts that disable cgroup v2 unified hierarchy or strip a
  required controller fall off W3 entirely. The `cgroup-v2-unified-not-present`
  and `cgroup-controllers-missing` error codes plus the
  troubleshooting fragment in
  [docs/how-to/host-prepare.d/cgroup.md](../how-to/host-prepare.d/cgroup.md)
  document the remediation.
- Negative: partition-root and threaded-cgroup support is deferred to
  future panel-approved ADRs.

## Test coverage

- `tests/cgroup-delegation-oracle.sh` (L1c) — exercises every refusal
  path through the fake `nixling_host::cgroup::fake::FakeCgroupBackend`
  plus the broker audit recorder.
- `tests/pidfd-handoff.sh` (L1c) — exercises the SCM_RIGHTS transport,
  CLOEXEC preservation, supervisor poll registration, and
  reconciliation start-time validation.
- KVM-backed L2 confirmation lands in a later wave.
