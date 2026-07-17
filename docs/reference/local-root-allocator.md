# Local-root allocator contract

**Diataxis category:** reference.

This page documents the declarative inputs emitted for the local-root allocator
and the `d2b-realm-core` typed lease contract. The emitted records do not bind
listeners, allocate leases, spawn children, or supervise processes.

## Purpose

The local-root allocator is the local authority for host resources
that cannot safely be created independently by each realm broker. Realm
brokers request typed leases from the allocator instead of inventing
interface names, writing host files, creating their own lock files, or
asking the host daemon to proxy raw mutation bytes. The allocator decides
whether the requested resource set can be granted, denied, quarantined, or
reclaimed while preserving a single fail-closed ownership ledger.

The contract is intentionally host-local. It does not grant relay
identity, remote-provider credentials, cross-realm policy authority, or
permission to bypass `SO_PEERCRED`-based local authorization. Gateway
realms may ask for local host resources only through this contract once a
runtime allocator exists.

## Contract terms

| Term | Meaning |
| --- | --- |
| Local-root allocator | The local authority that owns host-resource allocation decisions and the persisted allocation ledger. |
| Typed lease | A `LeaseAllocationRequest` / `LeaseAllocationResponse` pair for closed `HostResourceKind` values rather than free-form shell or path mutations. |
| Resource id | An opaque `HostResourceId` used for matching, conflict reporting, and delegation. It is not a raw interface name, filesystem path, nftables expression, cgroup path, or credential. |
| Lease owner | The realm path, controller generation, and optional node identity that requested and owns a lease. |
| Delegation | The allocator's opaque handoff shape for a grant: opaque name, file-descriptor placeholder, partition id, or namespace handle. |
| Reconciliation | Comparing persisted lease state with observed kernel/host state and producing explicit decisions. |
| Quarantine | A fail-closed state that prevents automatic reuse until a later repair path resolves ambiguity. |
| Reclaim | A terminal cleanup decision for resources that can be safely retired from the ledger. |
| Ledger generation | An opaque compare-and-swap token binding a decision to the consistent durable snapshot from which it was computed. |
| Atomic grant commit | One durable ledger transaction that reserves the lease id, inserts the lease, stores the idempotency result, and advances the ledger generation together. |

## Resource kinds

The allocator contract only covers the closed `HostResourceKind` enum:

- bridge
- tap
- veth-pair
- nftables-table
- nftables-partition
- cgroup-subtree
- host-file-partition
- namespace-boundary

Adding a new kind is a schema change. It must carry clear ownership,
reconciliation, metric-cardinality, and security semantics before any
runtime implementation grants it.

## Typed leases

A mutating allocation request carries an operation id, correlation id,
mandatory idempotency key, lease owner, and one to thirty-two requested
resources. Each requested resource declares:

- an opaque `resource_id`;
- a closed `kind`;
- `exclusive` or `shared-partition` sharing semantics;
- explicit acquisition-order metadata.

The response is either `granted`, with an `AllocatorLease`, or `denied`,
with a low-cardinality `AllocatorReasonCode` and bounded conflict list.
Conflicts expose only resource ids, kinds, closed reasons, and optional
lease ids. They do not expose host paths, command output, credentials,
provider endpoints, or raw interface names.

Lease states are `granted`, `reconciled`, `quarantined`, `reclaimed`, and
`denied`. `denied` and `reclaimed` are terminal. Active states may move
through reconciliation, quarantine, or reclamation only along the state
transitions encoded in `AllocatorLeaseState::can_transition_to`.

## Total acquisition order

Every multi-resource request carries a deterministic total acquisition
order. Callers provide a coarse `phase` and `ordinal`; ties are broken by
closed resource kind and opaque resource id. All participants must sort
by the resulting `ResourceAcquisitionKey` before acquiring or releasing
resources. This prevents two realm brokers from deadlocking by acquiring
the same resource set in different orders.

A request that cannot be expressed in this total order must be denied
rather than partially applied. Future runtime code must not acquire a
resource out of order and then rely on cleanup to recover correctness.

## Generated child-realm inputs

Every enabled host-local child realm contributes a deterministic set of rows:

- one public and one broker `SOCK_SEQPACKET` listener request under
  `/run/d2b/r/<realm-id>/`;
- one bounded typed lease-request template containing cgroup, namespace,
  host-file partition, and listener resources;
- separate controller and broker launch records, each naming its principal,
  listener, cgroup leaf, namespace inputs, and resource references;
- controller and broker identity configurations with deterministic UID/GID
  maps, including only the realm's internal cgroup group;
- cgroup layout and ownership rows for the process-free realm/workload roots
  and the controller, broker, and workload role leaves; and
- dedicated user, mount, network, IPC, PID, and cgroup namespace rows for each
  child process.

The rows are sorted by canonical realm path. Resource acquisition is ordered by
phase and ordinal, and process launch order is explicit. These records describe
inputs to the runtime owned by the local-root controller and broker; booleans in
the records state that binding, spawning, supervision, adoption, and lease
execution have not occurred.

## Immutable host-file boundary

`host-file-partition` is a typed resource kind, not permission for a
realm broker to edit arbitrary host files. A grant delegates only an
opaque partition id or handle that the allocator recognizes. Realm
brokers must not create ad-hoc ownership markers, take independent lock
files, chmod/chown paths, or mutate host files outside a granted
partition.

This boundary keeps host-file mutation auditable and reconciliable. If an
allocator cannot prove that a partition is owned by the expected
lease, it must deny, quarantine, or report a storage-contract violation
instead of repairing by guesswork.

## Reconciliation, quarantine, and reclaim

Reconciliation reports compare two facts for each resource id:

1. the persisted lease metadata from the allocator ledger;
2. the observed host state from a bounded source such as kernel netlink,
   the nftables API, cgroupfs, host filesystem inspection, a namespace
   registry, or the allocator ledger itself.

Observed states are `present`, `missing`, `foreign-owner`, `ambiguous`,
and `inaccessible`. Decisions are `reconciled`, `quarantine`, `reclaim`,
and `deny`. Non-reconciled decisions require a closed reason code.
Quarantine, reclaim, and deny are fail-closed decisions: they prevent
reuse until a later, explicit owner resolves or retires the resource.

The crate defines these reports and bounds them to keep audit and metric
metadata small. It does not observe the live kernel, mutate
nftables, create cgroups, or repair host files.

## Allocator engine adapters

`LocalRootAllocatorEngine<L, O, V>` is statically composed from three narrow
adapters:

- `AllocatorLedger::load` returns a fallible, generation-bound
  `AllocatorLedgerSnapshot` containing the current leases and opaque,
  engine-owned idempotency records;
- `AllocatorLedger::commit_allocation` accepts one engine-created transaction.
  For a grant, the adapter reserves the next lease id while holding its
  exclusive lock, materializes the engine-owned lease and idempotency result,
  and durably publishes the sequence, lease, idempotency record, and generation
  change atomically. Denial idempotency is committed through the same method;
- `ObservedAllocatorState` exposes an already-collected resource observation
  snapshot;
- `AllocatorLiveness` answers whether the exact realm/controller generation in
  a `LeaseOwner` is live.

All three traits require `Send + Sync`. The engine uses generic static dispatch;
it has no trait-object, dynamic downcast, ambient-I/O, or fallback path. Host
observation happens before the decision pass. Durable reads and commits remain
explicitly fallible so an OFD-locked adapter can report lock, I/O, stale
generation, or integrity failures. `AllocatorEngineError` is a closed,
detail-free error enum; `allocate` and `reconcile` return `Result` and propagate
these failures without producing an allocation response.

The engine computes and validates a decision against one snapshot, then commits
before returning `Granted`. A commit must expose either the complete old state
or the complete new state after any failure; it must never expose a reserved id,
lease, or idempotency record independently. A stale generation fails closed.
The caller may retry with the same idempotency key: an uncommitted failure is
recomputed from a fresh snapshot, while a durable commit whose acknowledgement
was lost replays the exact stored result. The idempotency record is constructed,
serialized, and compared by the engine, so a ledger adapter stores an opaque
value rather than reimplementing request fingerprint validation.

The in-memory `FakeAllocatorLedger`, `FakeObservedAllocatorState`, and
`FakeAllocatorLiveness` adapters are available only to crate tests or consumers
that explicitly enable the `test-support` feature. They are not default generic
parameters and are absent from the normal production API.

`/etc/d2b/realm-controllers.json` references the emitted resource ids for each
child realm. Those references are inputs, not grants, and do not confer host
mutation authority.

Operator repair should be explicit and evidence-driven rather than
automatic cleanup. A CLI or daemon repair path for states such as
`DriftDetected`, `ReconcileMismatch`, or quarantine should inspect the
bounded `correlation_id`, `operation_id`, `resource_id`, and `lease_id`
metadata from allocator events and reconciliation reports, compare the
ledger entry with the observed host resource, then either reconcile the
lease, retire the stale record, or clear a quarantine only when ownership is
unambiguous. These fail-closed states are contract signals for operator
tooling, not permission for realm brokers to guess, delete,
or recreate host resources directly.

## No realm-broker ad-hoc locks

Realm brokers must use typed allocator leases and the generated
storage/synchronization contracts for coordination. They must not add
realm-specific lock files, hidden state directories, broad `/run/d2b`
sweeps, or direct host-resource mutation paths to work around the
allocator. If the allocator contract cannot represent a resource or lock
ordering, the correct next step is to extend the contract and schema, not
to bypass it in a broker.

See [Realm controller configuration](./realm-controller-config.md) for the
metadata artifact that records each realm's allocator binding.

## Redaction and observability

Allocator events are low-cardinality metadata: grant, denial, conflict,
reconciliation, reclamation, and quarantine. Event records carry operation
and correlation ids, the lease owner, optional resource kind, optional
reason, and optional trace context. They intentionally omit paths, raw
interface names, command output, credentials, provider endpoints, and
opaque resource values that would expand label cardinality or leak host
state.

## Runtime boundary

The committed foundation consists of Rust DTOs, validation helpers, a pure
allocator decision engine with injectable state adapters, tests, generated
schema coverage, and declarative Nix rows. The local-root runtime composes
durable ledger, observed-state, and pidfd-backed liveness adapters; validates
the generated rows; binds listeners; creates namespaces and cgroups; allocates
and revokes leases; parent-spawns children; returns pidfds; and performs
adoption and supervision. No child realm `.socket` or `.service` unit is
generated, and the allocator is not a separate daemon.
