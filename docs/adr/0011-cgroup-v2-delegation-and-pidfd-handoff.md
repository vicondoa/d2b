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

1. **Single slice, fixed name; per-VM intermediate + per-role leaf
   hierarchy (v1.1 reconciles SUPERSEDES the v1.0 flat
   `<vm-id>.scope` model).** The delegated subtree is
   `/sys/fs/cgroup/nixling.slice` — literally that name, not
   configurable.

   - **v1.0 model (historical):** per-VM leaves at
     `nixling.slice/<vm-id>.scope`. This held for v1.0 because every
     per-VM systemd-template service mapped 1:1 to a scope unit.
   - **v1.1 model (CANONICAL after this ADR's v1.1 update):**
     per-VM **intermediate** cgroup directory at
     `nixling.slice/<vm-id>/` (process-free per ADR 0011 § Decision
     item 5 — "no internal processes" in non-leaf nodes), with
     **per-role leaves** at `nixling.slice/<vm-id>/<role>/` for
     each broker `SpawnRunner{role}` child per
     [ADR 0018](0018-microvm-nix-removal.md) § "Sidecar/template
     retirement — full role matrix". The role-leaf model is
     required because v1.1 retires the per-VM systemd-template
     scope model and replaces it with one broker-spawned child
     per role; each role needs its own pidfd / cgroup.kill scope.

   Operator-facing path-class values (referenced by audit records
   per [ADR 0010](0010-wire-protocol-and-typed-errors.md) and
   [`docs/reference/cgroup-delegation.md`](../reference/cgroup-delegation.md)):
   - `slice` — `/sys/fs/cgroup/nixling.slice` (the delegated root).
   - `vm-interior` (v1.1 only) — `nixling.slice/<vm-id>/` (process-
     free intermediate). `cgroup.kill` on this path is REFUSED
     with `cgroup-kill-on-ancestor-refused`.
   - `vm-role-leaf` (v1.1 canonical) — `nixling.slice/<vm-id>/<role>/`
     (leaf, per-SpawnRunner role). Carries processes; `cgroup.kill`
     allowed during declared teardown per item 6 below.
   - `host-scoped-leaf` — leaves for SpawnRunner roles that have no
     associated workload VM. Two host-scope path patterns are
     recognized, both carrying `path_class: host-scoped-leaf`:
     - **Per-env host roles** at `nixling.slice/sys-<env>/<role>/`
       (e.g., `usbipd-backend`, `usbipd-proxy` for each USBIP-enabled
       environment per
       [ADR 0018](0018-microvm-nix-removal.md)).
       `sys-<env>/` is the process-free interior.
     - **Host singletons** at `nixling.slice/host/<role>/` (e.g.,
       `otel-host-bridge` per
       [`docs/reference/privileges.md`](../reference/privileges.md)
       § "Per-runner-role profile catalog"). `host/` is the
       process-free interior. The `host/` interior is used for
       runner roles whose scope is "exactly one per host" with no
       env axis to substitute (resolves R12 virt-r12-2).

   The v1.1 update **supersedes** the v1.0 `<vm-id>.scope` flat
   reading of item 1; ADR 0018's role-disposition matrix is the
   normative role inventory. Cross-references in
   [ADR 0018](0018-microvm-nix-removal.md),
   [`docs/reference/cgroup-delegation.md`](../reference/cgroup-delegation.md),
   and [`docs/reference/privileges.md`](../reference/privileges.md)
   use the same path-class taxonomy. This resolves R10 security-r10-3
   (cross-ADR cgroup-hierarchy contradiction).

   v1.1-P10 migrates existing v1.0 scope-based audit records / path
   parsers (`nixling-host`, `nixlingd::supervisor::pidfd`) to the
   role-leaf taxonomy. The path-class enum lands in
   `packages/nixling-ipc/src/audit_path_class.rs` in v1.1-P10.

2. **8-step algorithm, split into Phase A (privileged setup) and
   Phase B (post-delegation runtime mutation).** The broker performs
   delegation in the exact sequence documented in
   [docs/reference/cgroup-delegation.md](../reference/cgroup-delegation.md):
   probe → controller floor → cpuset inheritance → ordered
   `+cpu/+memory/+io/+pids/+cpuset` enable with per-step verification
   → slice mkdir → `member`-partition assertion → fd-based subtree
   chown → leaf-only kill enforcement → uid-0 refusal. Each step has
   a stable kebab-case error code that flows through the CLI
   contract (`docs/reference/error-codes.md`) and the broker audit
   record `error_kind` field.

   **Phase A (privileged, uid 0) — steps 1-6** (probe, controllers,
   cpuset inheritance, `+controller` enables, slice/leaf mkdir, fd-
   based `fchown` of the delegated subtree to `nixlingd`'s uid/gid)
   run as uid 0 because they require write access above the
   delegated subtree (mkdir under cgroup root, `+controllers` on
   parent cgroups, `fchown` to a target uid). This matches
   [ADR 0015](0015-daemon-only-clean-break.md)'s "broker chowns
   before drop-priv" lifecycle.

   **Phase B (post-delegation, uid != 0) — steps 7-8** (leaf-only
   kill enforcement, uid-0 refusal guard) run after the broker has
   dropped privileges to `nixlingd`'s uid. Step 8's
   `require_non_root_delegation()` (`getuid() != 0`) gates
   **runtime mutation of the already-delegated subtree** — i.e., it
   is enforced on subsequent calls into the cgroup module **AFTER
   the initial delegation completes**, not on the Phase A setup
   path. The R9 kernel reviewer flagged the earlier flat 8-step
   reading as self-contradictory (steps 4-6 require root, step 8
   refuses root). The split makes the privilege boundary explicit:
   Phase A is one-shot at delegation time under root; Phase B is
   the steady-state invariant after drop-priv.

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

6. **Kill scope is leaf-only.** `cgroup.kill` is **broker-mediated
   only** in v1.1+ (per the v1.1-P10 `CgroupKill` broker op landing
   per [`docs/reference/cgroup-delegation.md`](../reference/cgroup-delegation.md)
   § "Broker ops on the cgroup tree"). The broker is the sole
   writer of `cgroup.kill` files; the daemon NEVER writes
   `cgroup.kill` directly. The daemon's supervisor uses
   `pidfd_send_signal(SIGTERM)` first and ONLY escalates to a
   broker-mediated `CgroupKill` op as a last resort when SIGTERM
   does not drain the leaf within the role's documented grace
   period. The op writes only on per-VM role leaves or host-scoped
   leaves during declared teardown/cleanup; ancestor `cgroup.kill`
   is refused with `cgroup-kill-on-ancestor-refused`.

7. **Non-root delegation invariant (steady state).** **Phase B
   runtime mutation** of the already-delegated subtree (post-Phase A
   setup per item 2 above) MUST run as `nixlingd`'s target uid;
   `require_non_root_delegation` asserts `getuid() != 0` and returns
   `cgroup-delegation-refused` if violated. This is the steady-state
   invariant after Phase A (which legitimately runs as root for
   `+controllers` cascade, slice/leaf `mkdir`, and `fchown` to
   `nixlingd`'s uid/gid before drop-priv per
   [ADR 0015](0015-daemon-only-clean-break.md) lifecycle). The
   R9+R10 reviews flagged the prior wording ("never as uid 0") as
   self-contradictory with Phase A privileged setup; the steady-state
   reading is the operative one for the broker process post-drop-priv.

8. **pidfd handoff over SCM_RIGHTS.** The broker forks payloads via
   `clone3(CLONE_PIDFD)` (preferred) or `fork + pidfd_open` (fallback)
   and transports the resulting CLOEXEC pidfd to `nixlingd` over
   `priv.sock` via SCM_RIGHTS. The daemon takes ownership in
   `PidfdTable` and registers each pidfd in its tokio epoll/poll loop.
   Raw-pid kill/wait is forbidden except in the reconciliation path
   where pid + `/proc/<pid>/stat` field 22 are both validated.

   **Process-into-cgroup placement primitive** (resolves R21 kernel
   blocker). A pidfd is NOT writable to `cgroup.procs` — the kernel
   only accepts a PID/tgid in `cgroup.procs`. v1.1+ implementations
   MUST use one of the following Linux primitives to place the
   broker-spawned child into its role-leaf cgroup:

   - **Preferred: `clone3(CLONE_INTO_CGROUP)`** (Linux ≥ 5.7). The
     broker passes the role-leaf cgroup directory's `O_RDONLY`
     dirfd (obtained via `openat(2)` against the delegated subtree)
     in `clone_args.cgroup` along with `CLONE_PIDFD` (to obtain
     the pidfd atomically in `clone_args.pidfd`). Atomic
     fork+place; no race window where the child runs in the wrong
     cgroup. The Linux 5.7+ kernel is well below the v1.1 floor of
     6.9 so this is universally available.
   - **Fallback (NOT used in v1.1+; documented for historical
     completeness)**: parent-side write of the post-`fork(2)`
     child PID into the role-leaf's `cgroup.procs` file BEFORE
     calling `execve` in the child. This has a tiny race window
     where the child runs briefly in the broker's cgroup before
     the parent's write completes — acceptable for some payloads
     but NOT for the v1.1 audit/lifecycle model where the child
     must be in its declared leaf from the first instruction.

   The broker's spawn helper MUST use `clone3(CLONE_INTO_CGROUP |
   CLONE_PIDFD)` for every SpawnRunner role. Audit semantics: on
   `clone3` failure the broker emits `SpawnFailed` with
   `error_kind` derived from `errno` (e.g., `ESRCH` if the
   cgroup-leaf dirfd was closed, `EPERM` if the broker lacks
   write permission to the delegated subtree's leaf). The
   `tests/broker-spawn-clone3-cgroup-eval.sh` gate (future,
   v1.1-P10) asserts every SpawnRunner role uses
   `CLONE_INTO_CGROUP` (e.g., via strace fixture or syscall
   tracer); fallback parent-side-write is denied at compile
   time. No `cgroup.procs` writes from outside the broker.

   **Subreaper note — v1.0 said `nixlingd` sets
   `PR_SET_CHILD_SUBREAPER`; v1.1 SUPERSEDES this for SpawnRunner
   children.** Per
   [ADR 0018](0018-microvm-nix-removal.md) § "set-booted race-free
   serialization" / "broker-as-parent reaping model", NEITHER the
   broker NOR `nixlingd` claims `PR_SET_CHILD_SUBREAPER` for the
   SpawnRunner-child population in v1.1. The broker is the direct
   parent of every SpawnRunner child (via `clone3(CLONE_PIDFD)`)
   and reaps via `waitid(P_PIDFD)`; making either side a
   subreaper would silently re-parent unrelated host processes
   into the daemon/broker, breaking the audit/lifecycle model.
   The v1.0 `nixlingd` PR_SET_CHILD_SUBREAPER self-test described
   here remains historically accurate for v1.0 source but is
   explicitly REMOVED in v1.1; ADR 0018's normative supersession
   note is the operative contract for v1.1+ implementations.
   `nixlingd` does NOT set `PR_SET_CHILD_SUBREAPER` in v1.1;
   the R11 kernel reviewer flagged this contradiction.

   **Updated v1.2 (D7).** The broker now runs a dedicated background
   Tokio SIGCHLD reap loop for SpawnRunner children. On SIGCHLD it
   iterates the broker's pidfd registry and calls `waitid(P_PIDFD,
   WEXITED|WNOHANG)` on each registered child pidfd, removing exited
   entries from the registry and recording a typed `ChildReaped`
   event. The event is written to the broker audit log for forensics
   and also buffered in-memory (256-entry FIFO) for daemon pickup via
   the additive `PollChildReaped` IPC request. `nixlingd` records
   those notifications in its `BrokerReapLog` and, on
   `waitid(P_PIDFD, ... WNOWAIT)` returning `ECHILD`, treats the
   broker notification as authoritative and returns immediately
   instead of re-entering `/proc` polling. Responsibility remains the
   same: the broker is the direct parent and the sole reaper for the
   SpawnRunner-child population; neither broker nor daemon becomes a
   subreaper.

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
