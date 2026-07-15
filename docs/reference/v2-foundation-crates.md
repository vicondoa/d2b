# d2b 2.0 foundation crates

The d2b 2.0 runtime foundation is split into six canonical in-tree crates. All
six are versioned with the workspace, use the workspace lockfile, set
`publish = false`, and have an empty default feature set.

| Crate | Role | Optional host feature |
| --- | --- | --- |
| `d2b-session-unix` | Linux Unix stream/seqpacket, peer identity, ancillary data, descriptor validation, and attachment credits | `host-socket` |
| `d2b-session` | Portable authenticated ComponentSession handshake, record, lifecycle, cancellation, and named-stream runtime | none |
| `d2b-provider` | Provider traits, registry generations, operation admission, lifecycle, and authenticated RPC proxy | none |
| `d2b-provider-toolkit` | Provider-agent adapter, exact registration, fixtures, redaction, and shared conformance | none |
| `d2b-state` | Atomic JSON, quarantine, generations, anchored paths, locks, leases, and audit segments | `host-fs` (Linux), `tokio` |
| `d2b-client` | Typed target resolution, session connection, generated service clients, retries, cancellation, attachments, and named streams | `host-socket` (Linux) |

## Dependency and authority boundaries

`d2b-contracts` remains the only owner of serialized DTOs and generated service
bindings. The foundation crates consume exact no-default contract features and
do not redefine wire enums, identities, provider records, state records, or
protobuf messages.

The runtime crates expose owned integration seams:

- `d2b-session::OwnedTransport` accepts an already-selected transport without
  selecting a fallback or learning a raw endpoint.
- `d2b-provider::AuthenticatedProviderRpc` carries provider calls that have
  already passed ComponentSession authentication and method authorization.
- `d2b-client::ComponentSessionConnector` connects one exact typed route and
  transport selection.
- `d2b-state::AtomicFilesystem` makes durability phases injectable and
  testable; the real implementation operates below a trusted anchored
  directory.

Provider toolkit conformance runs against the same `ProviderInstance` surface
used by in-process adapters and authenticated RPC proxies. It does not load
dynamic libraries, access ambient credentials, or publish a second provider
contract.

## Session and descriptor invariants

The portable session runtime implements the fixed preface and offer contract,
the selected `snow` profiles, transcript binding, bounded protected records,
fragmentation, replay rejection, keepalive, close, reconnect generation,
request cancellation, and fair named-stream scheduling. Cryptographic state is
process-local and is never persisted.

The Linux Unix substrate verifies parent-prearmed `SO_PASSCRED`, distinguishes
directional peer identity from responder provenance, receives payload and
ancillary data atomically, rejects truncated or unknown control messages,
scavenges every received descriptor on failure, enforces `CLOEXEC` and exact
object identity, and rolls attachment credits back through every reserved
scope.

For inherited or socketpair endpoints, `SCM_CREDENTIALS` is phase-aware
transport identity evidence. The Unix transport requires and consumes the
first-packet credentials before the session accepts the preface, then verifies
stable credentials on subsequent packets while `SO_PASSCRED` remains enabled.
Credentials never become semantic `OwnedAttachment` values on Unix because
automatic and explicitly sent credentials are indistinguishable. Only
`SCM_RIGHTS` objects enter the two-phase attachment path: the transport owns
them without descriptors, then `d2b-session` decrypts and authenticates the
descriptor batch before binding and invoking object-specific validation.

`SessionEngine` drives handshake, protected records, control RPC, cancellation,
attachments, lifecycle, and named streams through one owned transport.
`SessionDriverHandle` is the clonable object-safe seam consumed by clients and
provider-agent servers. Outbound ttrpc requests are registered and sent through
`start_ttrpc`, while raw responses are received separately and correlated by
the ttrpc adapter before `complete_ttrpc` retires request state. The portable
session layer never guesses response ordering or parses ttrpc headers. Logical
named-stream messages remain bounded at 1 MiB and are fragmented, scheduled,
and reassembled internally under a 256 KiB credit window. Final-fragment credit
remains withheld until the application consumes the logical message and
explicitly grants its length; the driver maps that grant to the exact withheld
transport bytes. Terminal stream state is removed after reset or two-sided
close. The canonical client owns one session-level receive dispatcher and
routes events into byte-bounded per-stream queues under the 4 MiB aggregate
session limit, so concurrent streams cannot consume or discard each other's
events.

## State invariants

Host filesystem access is absent unless `host-fs` is selected. Atomic state
writes use a same-directory temporary file, complete writes, file fsync,
atomic rename, and parent-directory fsync. Reads are bounded and validate
schema, generation, writer, metadata, and checksum before returning authority.
Invalid or ambiguous state produces a typed error or quarantine record, never
a success-shaped default.

Path operations are relative to a caller-supplied anchored directory and accept
only validated relative components. OFD locks are ordered, deadline-bound, and
`CLOEXEC`. Transfer is two-phase: after the recipient duplicates the open-file
description, the sender commits the transfer so dropping its local guard closes
the descriptor without issuing `F_UNLCK`. Leases and audit segments retain the
exact typed identity and generation bindings from `d2b-contracts`.

The `tokio` feature adds async atomic-state, lock, and audit adapters. Blocking
filesystem and kernel lock operations run only through
`tokio::task::spawn_blocking`; they are never executed directly on a Tokio
worker. Dropping or timing out a contended async lock acquisition signals its
blocking cancellation token so the worker terminates. Async lock operations use
a cloneable `AsyncLockSet` handle so cancelling a future cannot lose or release
locks acquired by an earlier operation. Each acquisition reserves a set-local
handoff slot until the result is claimed; an unclaimed blocking result rolls
back only its newly acquired final guard before reopening the set. The
Linux-only `host-fs` feature remains usable without Tokio for synchronous
broker-side composition and fails explicitly when selected on another
platform.

## Client invariants

The client exposes async Tokio-compatible connect, invoke, cancellation,
attachment, and named-stream APIs. It resolves a typed target through an
explicit route table, selects one declared transport, and never retries through
another transport. Mutating retries reuse one bounded idempotency identity.
Response outcomes, remote errors, attachment indexes, cancellation, and
named-stream transitions are validated before being exposed to a caller. The
local ttrpc bridge multiplexes registered invocations, continues admitting
cancellation when normal work is saturated, serializes response writes, and
correlates out-of-order response frames by ttrpc stream id without tearing down
unrelated calls. Debug and error output omits target values, endpoints,
payloads, credentials, and attachment contents while retaining closed contract,
session, remote-kind/retry, and errno diagnostics.

These crates provide foundations, not compatibility adapters. Concrete
first-party providers and control-plane service migration are separate runtime
integration work.
