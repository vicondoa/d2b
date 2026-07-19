# Allocator service API

**Diataxis category:** reference.

The local-root allocator runtime is exposed through the frozen
`d2b.broker.v2` `BrokerService` methods `Allocate` and `Spawn`. This page owns
the runtime service boundary. Declarative endpoint, process, namespace, and
resource emission is documented separately in
[`local-root-allocator.md`](./local-root-allocator.md).

`Allocate` accepts only the bounded typed lease request from the committed
service contract. The runtime must preserve total acquisition order,
idempotency, exclusive-resource conflict detection, and fail-closed
reconciliation before returning any delegation attachment.

`Spawn` consumes the validated child-realm launch record and its exact
attachment table. The controller and broker are separate children with
dedicated UIDs, namespaces, listeners, cgroup leaves, bootstrap sessions, and
pidfds. Runtime dispatch must pass only declared descriptors, return controller
and broker pidfds at their fixed attachment indexes, and retain no path-based or
ambient-capability fallback.

The implementation entrypoints are
`d2b_priv_broker::allocator_service`,
`d2b_host::realm_children`, and
`d2bd::realm_child_supervisor`. The immutable allocator DTO and validation
rules remain in `d2b-realm-core` and `d2b-contracts`.

## Allocate

The service validates the protobuf with `StrictWireMessage`, converts it to the
typed allocator request, and reconciles persisted and observed state before
calling `LocalRootAllocatorEngine::allocate`. Quarantined, preserved, or
conflicting reconciliation state produces a denial without host mutation.
Successful resources remain in acquisition order. A materialized descriptor is
returned only when its resource row has an exact attachment index; opaque and
partition delegations never acquire an attachment by implication. Idempotent
replay duplicates the held descriptor with `CLOEXEC` rather than consuming the
allocator's copy.

Ledger reads, reconciliation, allocation, and atomic commits are fallible. The
service returns a single closed `allocator transaction failed` error for any
such failure; it does not expose lock, generation, path, or storage details.
No granted response or descriptor is materialized until the engine returns a
durably committed result. An idempotent replay reads the committed result and
does not perform a second commit.

## Spawn

The service resolves the launch record by realm and controller generation and
requires exact realm, generation, process-ID, and digest equality. Attachment
metadata and `SCM_RIGHTS` descriptors have equal cardinality and a one-to-one
index mapping. Singleton authority cannot carry a resource ID; resource and
lease bindings require one.

Both roles require a pre-bound listener, a connected bootstrap-session socket,
and a cgroup-v2 leaf dirfd. Listener descriptors must be `AF_UNIX`
`SOCK_SEQPACKET` listeners, bootstrap descriptors must be connected
`SOCK_SEQPACKET` sockets, namespace handles must be nsfs descriptors, storage
roots must be directories, and every descriptor must be `CLOEXEC`.
`prebind_realm_listeners_for_identities` creates both sockets before spawn,
refuses existing entries, and applies separate non-root owner/group modes. The
derived principals are `d2bd-r-<realm-id>` and `d2bbr-r-<realm-id>`; the shared
cgroup group is `d2bcg-r-<realm-id>`, distinct from the
`d2b-r-<realm-id>` public access group.

The broker creates each child through `clone3` with a pidfd, a dedicated user,
mount, network, IPC, PID, and cgroup namespace, and
`CLONE_INTO_CGROUP`. The two children use distinct non-root host identities.
Declared descriptors are installed from fd 10 upward and named only through
fixed environment keys. The child receives no `SD_LISTEN_FDS`, path lookup, or
undeclared descriptor. Namespace or cgroup creation failure has no fork or
post-exec placement fallback. If the second child fails, the first is killed
through its pidfd and no partial pair is returned.

`spawn_pair` also receives the parent-owned ends of the two pre-armed bootstrap
sessions. These endpoints are out-of-band launch authority, not additional
wire attachments, and must correspond exactly to the child ends installed in
the two processes. Before cloning either child, the broker compares each
bootstrap attachment's kernel device/inode identity with the child endpoint
bound into the corresponding parent-side object. The broker retains the
parent ends until each child sends its first bounded packet. It compares that
packet's kernel-supplied `SCM_CREDENTIALS` with the connected peer through
`ReceivedPacket::verify_first_packet_credentials`, then constructs
`PidfdEvidence` from that credential, the clone-returned PID, and the trusted
launch record's executable and cgroup digests.

No Spawn success or pidfd attachment is returned before both evidence objects
exist. A credential mismatch, truncated first packet, missing digest, or
timeout kills both children through their pidfds and closes both bootstrap
endpoints. The implementation has no public raw-credential constructor, PID
guess, path lookup, or unverified fallback.

Internally, each returned pidfd remains inside an owning
`VerifiedPidfdAttachment` with its evidence, role, process ID, PID, and fixed
attachment index. The pair has structural controller and broker fields rather
than a variable-length collection, so missing or duplicate evidence cannot be
represented. Before transport, the broker service consumes each attachment to
construct its `PidfdIdentityPolicy`; failure consumes and closes the pair, so
unverified authority cannot be retried. The evidence is never serialized and
its `Debug` representations are redacted.

The successful response always places the controller attachment at index 0 and
the broker attachment at index 1. Exact role, process-ID, PID, evidence, and
`CLOEXEC` correlation is checked before either attachment is accepted.

## Supervision and adoption

`RealmChildSupervisor` registers only a complete controller/broker pair with
distinct PIDs and process IDs, one controller generation, and the canonical
`d2b.slice/r-<realm-id>/{controller,broker}` leaves. Registration is atomic per
realm.

Restart adoption first verifies both candidates' executable, process ID,
controller generation, and cgroup membership from `/proc`. Only after both
verify does it open fresh pidfds and register the pair. Missing, ambiguous, or
mismatched state is rejected; pidfds are never persisted.
