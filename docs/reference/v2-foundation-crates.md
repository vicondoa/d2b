# d2b 2.0 foundation crates

The d2b 2.0 runtime foundation is split into six canonical in-tree crates. All
six are versioned with the workspace, use the workspace lockfile, set
`publish = false`, and have an empty default feature set.

| Crate | Role | Optional host feature |
| --- | --- | --- |
| `d2b-unix-session` | Linux Unix stream/seqpacket, peer identity, ancillary data, descriptor validation, and attachment credits | `host-socket` |
| `d2b-session` | Portable authenticated ComponentSession handshake, record, lifecycle, cancellation, and named-stream runtime | none |
| `d2b-provider` | Provider traits, registry generations, operation admission, lifecycle, and authenticated RPC proxy | none |
| `d2b-provider-toolkit` | Provider-agent adapter, exact registration, fixtures, redaction, and shared conformance | none |
| `d2b-state` | Atomic JSON, quarantine, generations, anchored paths, locks, leases, and audit segments | `host-fs` |
| `d2b-client` | Typed target resolution, session connection, generated service clients, retries, cancellation, attachments, and named streams | none |

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

## State invariants

Host filesystem access is absent unless `host-fs` is selected. Atomic state
writes use a same-directory temporary file, complete writes, file fsync,
atomic rename, and parent-directory fsync. Reads are bounded and validate
schema, generation, writer, metadata, and checksum before returning authority.
Invalid or ambiguous state produces a typed error or quarantine record, never
a success-shaped default.

Path operations are relative to a caller-supplied anchored directory and accept
only validated relative components. OFD locks are ordered, deadline-bound, and
`CLOEXEC`; leases and audit segments retain the exact typed identity and
generation bindings from `d2b-contracts`.

## Client invariants

The client resolves a typed target through an explicit route table, selects one
declared transport, and never retries through another transport. Mutating
retries reuse one bounded idempotency identity. Response outcomes, remote
errors, attachment indexes, cancellation, and named-stream transitions are
validated before being exposed to a caller. Debug and error output omits target
values, endpoints, payloads, credentials, and attachment contents.

These crates provide foundations, not compatibility adapters. Concrete
first-party providers and control-plane service migration are separate runtime
integration work.
