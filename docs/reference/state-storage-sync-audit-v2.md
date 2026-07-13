# State, storage, synchronization, and audit v2 contract

This reference defines the serialized d2b 2.0 contract implemented by
`d2b_contracts::v2_state`. The complete machine-readable example is
[`state-storage-sync-audit-v2-fixture.json`](./state-storage-sync-audit-v2-fixture.json).
It is a clean v2 surface: no current-generation or v1 aliases are accepted.

## Version and bounds

The schema version is `2`; its initial schema generation is `1`. A generated
contract and each storage, synchronization, and audit inventory carry the same
64-character lowercase hexadecimal fingerprint.

| Bound | Value |
| --- | ---: |
| Authoritative JSON document | 1,048,576 bytes |
| Storage inventory | 4,096 rows |
| Synchronization inventory | 1,024 locks |
| Dependencies per lock | 32 |
| Restart runner/resource observations | 4,096 of each |
| Status projection | 4,096 entries |
| Audit record | 8,192 bytes |
| Audit segment | 16,384 records / 67,108,864 bytes |
| Audit retention | 1–14 days |
| Lock deadline | 1–300,000 ms |
| Opaque resource/correlation ID | 1–64 ASCII bytes |
| Serialized generation or timestamp | `1..=9007199254740991` for generations; timestamps do not exceed the same JSON-safe ceiling |

Opaque IDs match `^[a-z][a-z0-9-]{0,63}$`. Digests are exactly 64 lowercase
hexadecimal characters. All structures deny unknown fields, and all operation,
outcome, reason, remediation, policy, and state values are closed enums.

## Identity and layout

`IdentityScope` is the only dynamic scope input. It uses `RealmId`,
`WorkloadId`, `ProviderId`, and `RoleId`; local root is a closed singleton.
`AuthorityRef` uses the same typed identities. Workload authorities carry both
the realm and workload IDs; role and provider authorities carry every ID in
their scope. Every creation, reconciliation, repair, deletion, ownership, lock
owner, and lock release authority must match the resource or lock
`IdentityScope` exactly. An authority from another realm, workload, provider,
or role is rejected even when its authority kind is otherwise correct.
Human realm/workload names,
configured provider labels, device or bus IDs, endpoints, commands, and
user-supplied strings cannot enter a path contract.

`LogicalLocation` is a closed relative location vocabulary. It contains no
absolute path or arbitrary segment. Generated storage rows pair a location with
one of these mandatory categories:

| Category | Initial location classes |
| --- | --- |
| `local-root` | host allocator or broker state |
| `realm` | realm controller or broker state |
| `workload` | state, disks, store view, TPM, media, audio, or keys |
| `provider` | provider state |
| `runtime` | realm, workload, or role runtime |
| `lock` | runtime locks |
| `lease` | runtime leases |
| `quarantine` | typed quarantine state |
| `audit` | local-root or realm audit |
| `projection` | regenerable status projection |

`applicableScopes` is the closed identity set for one generated inventory. For
each listed identity, the mandatory catalog requires exactly one matching
category/location row:

| Applicable scope | Mandatory category/location rows |
| --- | --- |
| local root | `local-root` / `host-broker` |
| realm | `realm` / `realm-controller`; `lock` / `runtime-locks`; `quarantine` / `quarantine`; `audit` / `realm-audit`; `projection` / `projection` |
| workload | `workload` / `workload-state`; `lease` / `runtime-leases` |
| provider | `provider` / `provider-state` |
| role | `runtime` / `runtime-role` |

An omitted row, duplicate category/location/identity key, undeclared scope, or
duplicate applicable scope fails validation. A row has one opaque resource ID,
kind, identity scope, logical location, creation/reconcile/repair/delete
policy and authority, persistence and secret class, exact mode/group policy,
and restart/adoption policy. There is exactly one inode owner and one repair
authority, and they must match. A diagnostic projection cannot be a repair
authority.

## Authoritative JSON

`StateEnvelope<T>` validation consumes the actual canonical raw payload bytes
and a canonical decoder. It requires a non-empty payload no larger than
1,048,576 bytes, exact `encodedBytes`, and equality between the decoded value
and `payload`. `checksum` is SHA-256 over the ASCII domain
`d2b.v2.state-envelope.payload.sha256\0`, the payload length as an unsigned
64-bit big-endian integer, and the exact raw payload bytes. A bit flip, length
lie, noncanonical encoding, checksum mismatch, or decoded-value mismatch fails
closed; validating structural envelope metadata alone is not an integrity
decision.

The durability state machine is ordered:

1. `initial`
2. `temporary-created`
3. `complete-document-written`
4. `temporary-file-synced`
5. `renamed`
6. `parent-directory-synced`

Only the final phase may report success. Before rename, crash recovery can
observe only the prior document. Around rename it may observe the prior
document or the complete new document. After parent-directory fsync it observes
the complete new document. A partial new document is never an allowed outcome.
Corrupt, ambiguous, owner-mismatched, or generation-mismatched state is
quarantined with a closed reason and remediation.

## Restart and adoption

Restart discovery records bounded runner and resource observations and a
nonzero completion timestamp. Runner evidence carries the realm, workload, and
role IDs plus candidate count, cgroup identity, executable fingerprint,
configuration fingerprint, config generation, and verdicts for every value.
Adoption compares all of them with one `RunnerAdoptionTarget`.
`PidfdPersistence` has exactly one value:
`process-local-non-persistent`. Pidfds are reopened and never serialized.

Adoption requires one candidate, exact target scope and identities, all
generation/configuration/executable/cgroup evidence matching, and a freshly
opened pidfd. Any missing, mismatched, or multiple-candidate evidence must be
quarantined.

Recovery ordering is fixed to `recover-before-cleanup`. There is no completion
boolean that can authorize deletion. A completed `RestartDiscovery` derives an
`OwnerAbsenceProof` bound to its discovery ID, completion timestamp, config
generation, and exact cleanup target. Cleanup replays validation against that
same discovery and the expected current generation. Any exact live,
mismatched, or ambiguous matching observation prohibits cleanup; missing
target observations are incomplete evidence. Cleanup targets only one declared
resource, role, or workload. There is no realm-wide or runtime-root sweep.

## Synchronization and leases

Each lock has a typed key, class, unique global order, bounded dependency set,
owner/release authority, contention policy, deadline, cancellation behavior,
and FD-transfer policy. The inventory rejects duplicate IDs/orders, unknown
dependencies, order inversions, and dependency cycles.

File locks are `ofd`, require `O_CLOEXEC`, and cannot cross a process boundary.
Other FD authority crosses a boundary only through a closed explicit policy:
ComponentSession attachment, `SCM_RIGHTS` lease handoff, or exact FD mapping.
Fork inheritance is not a transfer policy.

Lease records bind a resource, typed owner, generation, JSON-safe expiry,
revocation state, and explicit transfer policy. Generation mismatch,
revocation, and expiry fail closed.

## Audit

Audit is separate from state snapshots. `AuditStream` is either `local-root` or
one typed realm. Local-root audit has exactly the local-root broker owner; a
realm stream has exactly its matching realm broker owner.

Records contain only:

- stream, sequence, timestamp, bounded operation/session correlation, and an
  optional typed `ProviderId`;
- a typed actor;
- closed event, outcome, and reason labels;
- previous/current digests and encoded byte count.

There are no path, argv, command, endpoint, credential, proof, secret, or
payload-byte fields. Hashing is canonical, length-prefixed, SHA-256, and
domain-separated:

| Object | Domain |
| --- | --- |
| record | `d2b.v2.audit.record.sha256\0` |
| segment | `d2b.v2.audit.segment.sha256\0` |
| checkpoint | `d2b.v2.audit.checkpoint.sha256\0` |

The record digest covers every record field except `recordHash`, including the
previous hash and encoded byte count. The segment digest covers stream, owner,
segment ID, exact range, previous segment digest, generation, timestamps,
encoded size, prune status, count, and every verified record hash. Segment
validation checks record contents, contiguous sequences, internal links, and
the preceding segment link. The checkpoint digest covers stream, owner,
checkpoint ID, covered sequence, segment digest, previous checkpoint digest,
generation, and timestamp. Checkpoint validation checks segment coverage,
checkpoint-chain continuity, and the signature over the computed checkpoint
digest through `AuditCheckpointSignatureVerifier`. Syntactically valid
arbitrary digests are not evidence.

Retention is mandatory and bounded to at most 14 days. A prune decision
consumes the actual verified target segment, its verified checkpoint, the
preceding checkpoint when applicable, and the signature verifier. A stale,
unrelated, unsigned, or range-mismatched checkpoint is rejected; there is no
`checkpointPresent` boolean authority. Export selects a sequence range and either redacted JSONL or a
checkpoint bundle without naming an output path. Missing sequence numbers
create `AuditGap`; they cannot be represented as a successful continuation.
Gap expected and observed sequences use the shared JSON-safe bounded integer
type and cannot exceed `9007199254740991`.

## Projections

`StateProjection` is a bounded read model with the sole authority value
`diagnostics-only`. It can report typed status, degraded reason, remediation,
and observed generation. Its API always reports that it cannot authorize and
cannot repair. Authoritative recovery resolves opaque inventory IDs and live
evidence instead.
