# Local-root allocator contract

**Diataxis category:** reference.

This page documents the committed `d2b-realm-core` contract for
arbitrating host resources that are shared by local realm controllers. It
is a contract and schema foundation only: the current implementation does
not yet provide a runtime allocator service, live host mutation, or
realm-broker dispatch for these DTOs.

## Purpose

The local-root allocator is the future local authority for host resources
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
| Local-root allocator | The planned local authority that owns host-resource allocation decisions and the persisted allocation ledger. |
| Typed lease | A `LeaseAllocationRequest` / `LeaseAllocationResponse` pair for closed `HostResourceKind` values rather than free-form shell or path mutations. |
| Resource id | An opaque `HostResourceId` used for matching, conflict reporting, and delegation. It is not a raw interface name, filesystem path, nftables expression, cgroup path, or credential. |
| Lease owner | The realm path, controller generation, and optional node identity that requested and owns a lease. |
| Delegation | The allocator's opaque handoff shape for a grant: opaque name, file-descriptor placeholder, partition id, or namespace handle. |
| Reconciliation | Comparing persisted lease state with observed kernel/host state and producing explicit decisions. |
| Quarantine | A fail-closed state that prevents automatic reuse until a later repair path resolves ambiguity. |
| Reclaim | A terminal cleanup decision for resources that can be safely retired from the ledger. |

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

## Immutable host-file boundary

`host-file-partition` is a typed resource kind, not permission for a
realm broker to edit arbitrary host files. A grant delegates only an
opaque partition id or handle that the allocator recognizes. Realm
brokers must not create ad-hoc ownership markers, take independent lock
files, chmod/chown paths, or mutate host files outside a granted
partition.

This boundary keeps host-file mutation auditable and reconciliable. If a
future allocator cannot prove that a partition is owned by the expected
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

The current crate defines these reports and bounds them to keep audit and
metric metadata small. It does not yet observe the live kernel, mutate
nftables, create cgroups, or repair host files.

`/etc/d2b/realm-controllers.json` may reference allocator resource ids for a
realm, but those references remain metadata until a runtime allocator exists.
They do not grant host mutation authority by themselves.

Future operator repair should be explicit and evidence-driven rather than
automatic cleanup. A future CLI or daemon repair path for states such as
`DriftDetected`, `ReconcileMismatch`, or quarantine should inspect the
bounded `correlation_id`, `operation_id`, `resource_id`, and `lease_id`
metadata from allocator events and reconciliation reports, compare the
ledger entry with the observed host resource, then either reconcile the
lease, retire the stale record, or clear a quarantine only when ownership is
unambiguous. This document does not define a live repair command yet; until
such a command exists, these fail-closed states are contract signals for
future operator tooling, not permission for realm brokers to guess, delete,
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

## Current implementation status

The committed foundation consists of Rust DTOs, validation helpers, tests,
and generated schema coverage in `d2b-realm-core`. It does not install a
local-root allocator daemon, expose a public or broker socket operation,
or change the live behavior of existing `d2b.envs` networking, cgroup,
nftables, host-file, or namespace management. Runtime allocation, live
mutation, and migration from today's local host-resource paths are future
implementation work.
