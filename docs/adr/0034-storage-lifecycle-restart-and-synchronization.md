# ADR 0034: Storage lifecycle, restart adoption, and synchronization

- Status: Accepted
- Date: 2026-06-19
- Related: ADR 0002 (non-root daemon and privileged broker), ADR 0011
  (cgroup v2 delegation and pidfd handoff), ADR 0015 (daemon-only clean
  break), ADR 0021 (broker user namespace for virtiofsd), ADR 0023
  (runner-role lifecycle matrix), ADR 0027 (hardlink-backed store-view
  live pool), ADR 0032 (nixling v2 constellation control plane)

## Context

Nixling's daemon-only architecture intentionally lets `nixlingd` restart
without tearing down every running VM. The daemon restores durable state,
re-adopts live runner processes when it can prove identity, quarantines
ambiguous state, and reports degraded status when recovery is not safe.

That lifecycle depends on host files and directories that are currently
spread across several ownership models:

- NixOS tmpfiles and `environment.etc` create base directories and bundle
  artifacts under `/etc/nixling`, `/var/lib/nixling`, and `/run/nixling`.
- NixOS activation scripts perform many root-owned `mkdir`, `chown`,
  `chmod`, and `setfacl` repairs, including per-role ACL grants.
- `nixlingd` binds public sockets, manages daemon locks, writes daemon
  reports, persists adoption metadata, and drives lifecycle DAGs.
- `nixling-priv-broker` performs privileged host mutations, spawns runners,
  prepares state such as swtpm and store-view directories, and writes audit
  records.
- Broker-spawned runners create or consume role sockets, vsock paths, disk
  images, and persistent per-VM state.
- Guest helpers own guest-local state such as detached exec records.

The result is a recurring class of bugs: access is sometimes repaired by a
manual chmod or setfacl, state files disagree with daemon memory, sockets
and lock files survive past their owners, and different code paths restamp
the same inode with different expectations. File locks are part of the same
problem: a parent can hold a lock while a child accidentally inherits the
fd or appears to own the protected resource, leaving restart recovery
ambiguous.

The existing per-VM ownership matrix is useful but too narrow. It covers
selected `/var/lib/nixling/vms/<vm>/` entries, while many root-visible
artifacts, runtime sockets, lock files, degraded-state records, external
ACL grants, and future realm scopes remain outside one contract.

## Decision

Nixling will introduce a versioned storage lifecycle contract that covers
managed host paths, process restart/adoption behavior, synchronization
resources, and degraded-state reporting. The contract is generated from
Nix, consumed by Rust DTOs in `nixling-core`, resolved by the privileged
broker from trusted bundle data, and enforced by `nixlingd` lifecycle DAGs.

The implementation may use a one-time planned-downtime cutover to move
existing hosts into the new layout. After that cutover, daemon and process
restart behavior returns to the normal continuation invariant: do not clear
runtime state just because a daemon process restarted.

### Storage contract

A generated artifact, tentatively `storage.json`, is the canonical
inventory of nixling-managed paths. Each entry includes at least:

- stable storage id and scope;
- path template plus typed variables;
- path kind (`directory`, `regular-file`, `socket`, `symlink`, external
  grant-only target, and similar closed-set variants);
- lifecycle and persistence class;
- owner, group, mode, access ACLs, and default ACLs;
- creator, writers, readers, and release authority;
- cleanup, repair, restart, and adoption policies;
- sensitivity class and observability posture;
- `noFollow`, recursion, hardlink-farm, same-filesystem, and
  filesystem-crossing invariants.

The layout is stratified by lifecycle:

| Root | Meaning |
| --- | --- |
| `/etc/nixling` | NixOS-generated configuration and bundle artifacts. Root-owned, read-only to `nixlingd`, never runtime state. |
| `/var/lib/nixling` | Persistent framework state: daemon metadata, broker audit/degraded ledgers, per-VM persistent state, store-view metadata, swtpm markers, host-runtime metadata, disk images. |
| `/run/nixling` | Boot-scoped runtime: public/broker sockets, role sockets, locks, leases, and process runtime files. Preserved across normal daemon restarts when a live owner is proven; cleaned on boot or on VM/process lifecycle only when stale-owner proof exists. |
| `/var/cache/nixling` | Regenerable cache and intermediate data. Never authority-bearing state. |
| External roots (`/run/user/<uid>`, `/dev/*`, `/sys/fs/cgroup/*`) | Observe/grant-only or kernel-owned surfaces. Nixling may grant/revoke access or validate posture; it does not own these roots. |

Per-role runtime sockets move toward role-specific directories under
`/run/nixling/vms/<vm>/roles/<role>/...`. Shared default ACLs on broad
per-VM runtime directories are not the primary authorization mechanism.
Every cross-role socket consumer is explicit in the storage contract.

Dynamic path components are not raw strings. VM, role, environment,
generation, and bus-id values are typed identifiers validated by closed
regexes and bundle membership before any path is expanded.

### Restart and degraded-state contract

Every process DAG node has a restart contract, either embedded in
`processes.json` or generated as a companion artifact. The contract records:

- restart class;
- adoption inputs;
- persistent and runtime storage references;
- cleanup-before-restart rules;
- readiness predicate after adoption;
- degraded-state behavior;
- remediation id and operator text.

Restart classes are closed:

| Class | Meaning |
| --- | --- |
| `adoptable` | The live process may survive daemon restart. Nixling re-discovers it, opens a fresh pidfd, verifies identity, and re-registers it. |
| `recreatable` | The process can be stopped/restarted from persistent state without data loss. |
| `stateful-quarantine` | Nixling cannot prove safety. Leave the state/process alone, mark degraded, and require operator action. |
| `non-resumable` | The process cannot be resumed across the relevant owner restart. Mark degraded until explicit restart/remediation succeeds. |
| `external-observed` | Nixling does not own the process or resource but can report health and degraded state. |

Pidfds are not persisted. A pidfd is process-local fd authority and cannot
be serialized. On disk, nixling may persist logical adoption metadata:
VM, role, declared cgroup leaf, expected executable/profile identity, and
last observed PID/start-time for diagnostics. Persisted PID or fd values
are never authority.

Adoptable runner discovery is cgroup-backed. Each adoptable runner has a
declared cgroup leaf under `nixling.slice`. On restart, nixling reads the
leaf's `cgroup.procs`, immediately opens pidfds for candidates, and then
verifies cgroup membership plus executable/profile/cmdline/start-time
shape against the bundle. Ambiguous, multiple, or mismatched candidates
quarantine the role and mark the narrowest affected VM/component degraded.

Adoptable runners live in dedicated delegated scopes/slices under
`nixling.slice`; they are not children whose lifetime is tied to
`nixlingd.service` or `nixling-priv-broker.service`. The design must not
rely on deprecated `KillMode=none`.

Degraded state is a typed daemon-owned ledger. It records scope, closed
reason slug, affected storage/lock ids, adoption/restart attempt, live
owner evidence when relevant, and a static remediation id. `nixling vm
list`, `nixling vm status`, and `nixling host doctor` read this ledger
and surface inline remediation commands. Privileged broker repairs never
trust paths, owners, modes, ACLs, or commands from the ledger; repair
resolves only trusted storage/sync ids from the bundle.

The degraded-state taxonomy is closed. Initial classes include
`storage-drift`, `storage-repair-failed`, `adoption-pending`,
`adoption-quarantined`, `restart-required`, `lock-owner-ambiguous`,
`lock-acquire-timeout`, `external-dependency-unhealthy`,
`migration-required`, `migration-failed`, and role-specific component
degradations.

### Synchronization contract

Locks are managed resources, not incidental files. A generated sync
contract, either part of `storage.json` or a companion `locks.json`,
contains one row per lock:

- lock id and scope;
- lock path template or in-process resource id;
- lock kind (`ofd`, `file-record`, `in-process`, `kernel-object`, and
  other closed-set variants);
- owner process and allowed holders;
- inheritance and fd-passing policy, including exact `SCM_RIGHTS` or
  explicit fd-mapping transfers when a lock fd is intentionally handed to
  another process;
- total acquisition order;
- timeout and stale-owner policy;
- adoption policy, degraded scope, and release authority.

Framework advisory file locks use Linux OFD locks (`F_OFD_SETLK`).
New BSD `flock()` or POSIX process record lock (`F_SETLK`) sites are not
allowed. Existing non-OFD sites are migration targets. Lock fds are opened
with `O_CLOEXEC` by default and do not cross `execve` into runners unless
an explicit sync-spec capability grants that transfer and records the lease
handoff through `SCM_RIGHTS` or an explicit fd mapping. Fork/exec
inheritance is not an ownership transfer.

Multi-lock operations acquire through a single ordered helper using the
sync contract's total order, such as `(scope class, anchored root,
normalized relative path, lock id)`. Ad hoc nested locking is a
test-failing policy violation.

Lease liveness is pidfd-backed where possible. The daemon or broker polls
pidfds; a readable pidfd (`POLLIN`) means the owner exited. If nixling is
the parent or responsible reaper, the event loop follows with
`waitid(P_PIDFD, ...)` or the existing reap path before clearing owner
leases. Ambiguous lock ownership quarantines the protected scope rather
than force-unlocking it.

### Broker path and privilege boundary

The privileged broker remains the only root authority for host storage
mutations. The daemon sends opaque storage and lock ids, never raw paths or
free-form modes/owners/ACLs. The broker resolves ids against its trusted
bundle copy.

All broker storage and lock path resolution uses anchored fd-relative
walking. On Linux, the required primitive is `openat2()` with
`RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`, plus
`RESOLVE_NO_XDEV` unless the storage spec explicitly permits a filesystem
boundary. Leaf operations use `O_NOFOLLOW`. A fallback `O_PATH`
fd-relative walk must provide equivalent guarantees or fail closed.

Hardlink-sensitive paths such as store-view live pools carry explicit
no-recursion and same-filesystem/cross-mount invariants. Recursive chmod,
chown, or setfacl over a hardlink farm remains forbidden.

Raw Linux APIs needed for `openat2`, `pidfd_open`, `F_OFD_SETLK`,
`waitid(P_PIDFD, ...)`, `epoll`, and any other pidfd polling are isolated
behind safe Rust wrappers, using `rustix` or `nix` where possible. Any
unavoidable unsafe code stays inside the existing unsafe quarantine with
documented safety preconditions.

### NixOS and systemd handoff

NixOS tmpfiles may create only base roots with non-recursive,
non-ACL-clobbering rules. Nested `/run/nixling/vms/**` runtime
directories, per-role socket directories, and dynamic ACLs are
broker-owned.

Per-role systemd socket units in broker-managed subdirectories are avoided.
If one is ever introduced, it must be ordered after broker bootstrap and
ACL reconciliation. `RuntimeDirectoryPreserve=yes` alone is not a race
solution.

The cutover explicitly retires storage/ACL activation hooks. Hooks that
remain for non-storage concerns are enumerated separately, and eval/policy
tests assert that no legacy storage/ACL activation hooks remain after the
cutover.

### Atomic persistence and observability

Authoritative JSON state uses the durable write sequence: create a temp
file in the same directory, write, fsync the file, rename over the target,
and fsync the parent directory. This applies to runner adoption metadata,
lease records, degraded ledger segments, and generated runtime state.

Audit and degraded records use closed reason enums. Path hashes use
scope-specific salts and may appear in structured logs, audit records,
local doctor output, and the degraded ledger. They must not appear as
Prometheus/OpenTelemetry metric labels or high-cardinality tags.

The degraded ledger uses deterministic storage classes:

| Class | Semantics |
| --- | --- |
| `tamper-evident-segmented` | Segment-internal hash chain; sealed segment summary with first/last sequence, segment hash, scope, and creation/prune metadata. Retention may delete sealed old segments only after retaining a checkpoint summary. |
| `append-only-bounded` | Append-only writer with bounded age/size retention; no tamper-evident chain claim. |
| `plain-bounded-diagnostic` | Bounded diagnostic cache; not audit authority. |

Audit/degraded ledgers are tamper-evident segmented files. Local ephemeral
diagnostics may be plain bounded files.

Undeclared, malicious, or dynamic path violations use a separate quota and
rate-limit lane from normal audit/degraded history. Repeated violations are
deduplicated by `(scope, reason, pathHash, actorClass)` over a bounded
window. If the violation lane saturates, nixling records one
`violation-audit-throttled` sealed summary instead of allowing violation
events to evict normal history.

Declared-path hash resolution is available through an authorized local
doctor/debug command. Undeclared path break-glass extraction is root-only,
local-only, scope-checked, audited, rate-limited, and never exported to
normal status surfaces or metrics.

Host-local storage reads used by status/list/doctor are audited when they
cross sensitive classes such as secrets, credential-adjacent state,
gateway-backed realm state, and audit/degraded ledgers. Gateway-backed
realm reads stay inside the gateway boundary; the host daemon must not
read or audit realm provider credentials directly.

## Migration decision

The storage lifecycle cutover may be breaking and may require planned VM
downtime. During that cutover only, nixling may clear old boot-scoped
runtime sockets and old lock/lease files after proving that VMs,
`nixlingd`, broker-spawned runners, and relevant helpers are quiesced.

The cutover preserves critical persistent data:

- swtpm NVRAM and root-owned swtpm identity markers;
- VM sshd host keys and framework-managed SSH keys;
- store-view state, metadata, and gcroots;
- daemon logical adoption metadata and degraded/audit history;
- host-runtime metadata;
- VM disk images, including writable store overlay images.

The migration command has dry-run and apply modes. Dry-run and preflight
output print the checkpoint id and exact rollback command before any apply
step begins. If the migration fails after moving persistent state, the
operator receives a typed failure plus the rollback command; broad chmod,
chown, or setfacl instructions are not an acceptable recovery path.

Repository landing remains separate from host adoption. Framework changes
land through a PR. `/etc/nixos` consumes the merged nixling result only at
the end. If `/etc/nixos` changed while the PR was open, the host update
preserves those edits and stops for operator review if they conflict with
the new migration procedure.

## Consequences

### Positive

- File ownership, ACL, cleanup, restart adoption, and lock behavior become
  explicit contracts rather than scattered side effects.
- `nixlingd` restart recovery can preserve live VMs without unsafe broad
  `/run` cleanup.
- Broker repairs are less vulnerable to confused-deputy path attacks
  because raw paths are not authority.
- Operators see typed degraded states with inline remediation commands
  instead of debugging by manual chmod/chown/setfacl.
- Future ADR 0032 gateway-backed realm work has explicit host/gateway
  storage boundaries and cannot accidentally store realm credentials in
  host-side state.

### Costs

- The bundle schema grows new storage, restart, and synchronization
  artifacts.
- The broker must centralize path-safe filesystem primitives and lock
  lease handling before activation scripts can be retired.
- The one-time cutover is intentionally disruptive and needs careful
  preflight/rollback UX.
- Tests must cover schema generation, broker path safety, restart
  adoption, lock ordering, degraded ledger integrity, and NixOS tmpfiles
  handoff.

## Rejected alternatives

### Keep adding activation repairs

Rejected. Activation scripts run at rebuild time, cannot reason about live
pidfd/cgroup ownership, and have already accumulated overlapping ACL
repair logic. They are the wrong owner for runtime state.

### Clear `/run/nixling` on daemon restart

Rejected. A daemon restart is a continuation event. Broad runtime cleanup
would destroy the evidence and sockets needed to adopt live runners and
would make restart behavior depend on data loss.

### Persist pidfds

Rejected. Pidfds are process-local fds, not serializable authority. The
restart contract persists logical adoption metadata and re-opens pidfds
from live cgroup candidates.

### Let the daemon pass raw paths to the broker

Rejected. The daemon is not the root trust anchor for host mutation.
Privileged repair must resolve opaque ids against broker-trusted bundle
data.

### Use path hashes everywhere for observability

Rejected. Path hashes are useful in structured audit and local doctor
flows, but metric labels/tags would create cardinality risk. Metrics use
closed scope/reason classes instead.
