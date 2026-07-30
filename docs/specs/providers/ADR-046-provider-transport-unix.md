# ADR 0046 Provider dossier: transport-unix

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-transport-unix` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 3 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-transport-unix` crate; d2b-bus Unix transport layer |
| Depends on | `ADR-046-terminology-and-identities`, `ADR-046-componentsession-and-bus`, `ADR-046-zone-routing`, `ADR-046-resource-object-model`, `ADR-046-resource-api-and-authorization`, `ADR-046-provider-model-and-packaging`, `ADR-046-resources-zone-control`, `ADR-046-resources-host-guest-process-user`, `ADR-046-provider-state`, `ADR-046-core-controllers`, `ADR-046-components-processes-and-sandbox` |
| Supersedes | v1 of this dossier (ADR-046-provider-transport-unix v1; incorrect ownership model); `InheritedSocketTransport` SD_LISTEN_FDS path (`packages/d2b-session-unix/src/systemd.rs`) |

---

## Source and reuse policy

The pre-ADR-0045 v3 baseline (`b5ddbed6`) has no Provider model, no `ZoneLink`
resource, and no production Unix-transport ComponentSession. The primary
current-state evidence for peer identity, FD credit, and atomic seqpacket
behavior is the v3 `d2b-realm-transport` and `d2b-realm-router` crates, which
are implemented-but-unwired for general routing.

The primary **reuse source** is main commit
`a1cc0b2da4a08ca3240a770a972fe4da6f912bef`. The per-crate inventory below
records exact file, symbol, and test selections, the v3 destination, and the
ADR 0045 assumptions excluded.

### d2b-session-unix - Unix transport and FD credit (primary reuse)

**Crate**: `packages/d2b-session-unix/` at main `a1cc0b2d`

#### `src/adapter.rs`

| Symbol | Selected behavior |
| --- | --- |
| `UnixSeqpacketTransport` | Atomic packet + ancillary FDs via `sendmsg`/`recvmsg` with `SCM_RIGHTS`; each `send` is one `sendmsg` call - packet and FD ancillary data are delivered atomically or not at all; receive reads the full packet and collects all attached FDs before returning |
| `UnixStreamTransport` | Frame-based: 2-byte big-endian record-length prefix on every write; reader reads the prefix, allocates a buf, reads the body; no FD ancillary support |
| `PeerIdentityPolicy` | Three variants: `Accepted` (no further check), `Pathname { uid, gid }` (SO_PEERCRED verified against expected uid/gid from the endpoint config), `InheritedSocketpair` (credentials extracted from kernel SO_PEERCRED without connecting; verified not overwritten by peer) |
| `UnixAttachmentPayload` | Attachment payload for seqpacket-received FDs; holds an `OwnedFd` with CLOEXEC enforced; `validate_descriptor` called after the protected record is decrypted and before the FD is delivered to any service handler |
| `OwnedUnixAttachment` | RAII wrapper over `UnixAttachmentPayload`; calls `close()` exactly once on drop; `into_payload()` transfers without close |

#### `src/credit.rs`

| Symbol | Selected behavior |
| --- | --- |
| `CreditPool` | Bounded per-scope integer counter; `reserve(n)` atomic, returns `Err` if pool would go negative; `release(n)` restores, never above initial capacity |
| `CreditScopeSet` | Six independent `CreditPool` instances, one per `CreditScope` variant; acquires/releases as a unit with rollback on any per-scope failure |
| `CreditBundle` | Multi-scope reservation RAII; holds one `CreditScopeSet` ref plus reserved counts per scope; `rollback()` releases all scopes; `commit()` consumes the bundle without rollback |
| `ProcessCreditLimit` | Derives per-process FD capacity from `/proc/self/fd` open-FD count; reserves `EMERGENCY_HEADROOM` FDs for Process and Host scopes; fails closed if /proc read fails |
| `CreditScope` | Six variants: `Packet`, `Request`, `Operation`, `Session`, `Process`, `Host` |

#### `src/descriptor.rs`

| Symbol | Selected behavior |
| --- | --- |
| `PeerCredentials` | Wraps kernel-supplied `ucred` (`uid`, `gid`, `pid`); never derived from payload or peer assertion |
| `PidfdIdentityPolicy` | Verifies live process identity: opens pidfd via `pidfd_open(pid, 0)`, reads `/proc/<pid>/fdinfo/<fd>`, checks `pos` and `flags`; rejects on any I/O error or kernel-object mismatch |
| `DescriptorPolicy` | Combines `PeerCredentials`, `PidfdIdentityPolicy`, and object identity into one admission decision; all three must pass |
| `VerifiedPacket` | Output of `DescriptorPolicy::verify`; carries verified `PeerCredentials`, verified object identities, and scavenged `OwnedFd` set - guaranteed close on drop |
| `ObjectIdentity` | `st_dev`, `st_ino`, `file_type`; kernel-supplied; same-kernel-object check rejects duplicate FDs |
| `AcceptedAttachment` | Binding of `VerifiedPacket` and `UnixAttachmentPayload` after decrypted descriptor matches verified object |

#### `src/socket.rs`

| Symbol | Selected behavior |
| --- | --- |
| `SeqpacketSocket` | Async wrapper over `AF_UNIX SOCK_SEQPACKET`; exposes `send_vectored_with_ancillary` / `recv_vectored_with_ancillary`; `SO_PASSCRED` is enabled on the listener before `accept` so the kernel fills `cmsg_type=SCM_CREDENTIALS` on the first received packet |
| `StreamSocket` | Async wrapper over `AF_UNIX SOCK_STREAM`; exposes `read_exact` / `write_all`; no ancillary support |

#### `src/systemd.rs`

| Symbol | Disposition |
| --- | --- |
| `InheritedSocketTransport` (SD_LISTEN_FDS) | **Excluded**: tied to ADR 0045 fixed 4-unit PID1 socket activation. In v3, Zone local sockets are pre-bound by the allocator/core and handed to the transport-unix service as opaque FD attachments via the `OpenTransport` portal call; not from `SD_LISTEN_FDS`. |

#### `src/vsock.rs`

| Symbol | Disposition |
| --- | --- |
| vsock transports | **Excluded**: belong to `Provider/transport-vsock`. |

#### Tests (`packages/d2b-session-unix/tests/unix_session.rs`)

| Test function | Covers | Action |
| --- | --- | --- |
| `ancillary_capacity_is_derived_from_closed_hard_bounds` | CreditScopeSet capacity derivation | copy |
| `process_limit_preserves_emergency_headroom` | ProcessCreditLimit headroom | copy |
| `failed_multiscope_reservation_rolls_back_every_prior_scope` | CreditBundle rollback | copy |
| `staged_credit_reservations_release_once_at_each_scope` | Release idempotency | copy |
| `inherited_passcred_is_verified_but_never_repaired` | SO_PASSCRED passthrough | copy |
| `first_packet_has_exact_directional_credentials` | First-packet SO_PEERCRED direction check | copy |
| `seqpacket_transfer_is_atomic_cloexec_and_object_exact` | FD transfer atomicity and CLOEXEC | copy |
| `duplicate_kernel_objects_are_rejected_and_cleaned_up` | Duplicate FD rejection and scavenge | copy |
| `owned_transport_adapters_transfer_packets_and_owned_files_end_to_end` | End-to-end seqpacket | copy |
| `stream_transport_reassembles_partial_and_coalesced_records` | Stream framing | copy |
| `pidfd_identity_requires_live_launch_evidence_and_rejects_unrelated_process` | Pidfd liveness | copy |
| `payload_and_control_truncation_scavenge_received_files` | FD scavenge on truncation | copy |

**V3 destination**: `packages/d2b-provider-transport-unix/src/{seqpacket,stream,identity,credit,descriptor,socket}.rs`
and `packages/d2b-provider-transport-unix/tests/{portal,identity,credit,admission}.rs`.

**Excluded ADR 0045 assumptions**:
- SD_LISTEN_FDS / `InheritedSocketTransport` startup excluded; v3 uses allocator/core-supplied FD attachment.
- `EndpointPurpose::LocalDaemon`/`GuestBootstrap` purpose taxonomy excluded; v3 uses Zone purpose class.
- `CONTROLLER_PIDFD_ATTACHMENT_INDEX`/`BROKER_PIDFD_ATTACHMENT_INDEX` attachment conventions excluded.

---

### d2b-session - ComponentSession transport abstraction (ancillary reuse)

From main `a1cc0b2d`, `packages/d2b-session/src/transport.rs`:
`OwnedTransport`, `TransportDescriptor`, `TransportError` are the trait/error
types this Provider's `UnixSeqpacketTransport` and `UnixStreamTransport`
implement. They are owned by ADR046-session-001 in `packages/d2b-bus/src/transport.rs`.

---

## Overview

`Provider/transport-unix` is a **transport service Provider**. It exposes one
long-lived `service` Process in the child Zone that owns the ZoneLink. That
child Zone's core ZoneLink controller invokes the Provider over a typed portal
to obtain, close, and observe Unix-domain socket transports. It owns no
ResourceTypes, reconciles no resources, and holds no session state. The
compiler-only child `parentZone` setting selects the allocator that pre-binds
the edge; the parent retains only sealed allocator/route state and no reciprocal
ZoneLink, Provider, Process, or Endpoint resource.

Its boundaries:

| What it owns | What it does not own |
| --- | --- |
| FD validation (socket kind, CLOEXEC, SO_PASSCRED, attachment policy) | ZoneLink spec / status / finalizers / conditions |
| Per-transport monitoring FD dup and `ObserveTransport` event stream | Session Noise handshake or record protection |
| `PeerCredentials` from first-packet SO_PASSCRED | Route advertisement or RouteTreeEngine updates |
| FD credit accounting for SCM_RIGHTS attachments (within-Zone only) | Queued-intent accumulation or drain |
| `PidfdIdentityPolicy` for attached pidfds (within-Zone portals only) | Session reconnect or generation tracking |
| Stable transport-layer error codes | Admission of ZoneLink or child Zone identity |
| `route_class` enforcement: ZoneLink transports are always FD-attachment-free | SCM_RIGHTS grants across Zone boundaries (prohibited by ZoneLink contract) |

The child Zone's core ZoneLink controller (ADR-046-core-controllers) is the sole
reconciler for its local `ZoneLink` resource, including watching, status
updates, condition transitions, finalizer management, route advertisement, and
queued-intent lifecycle. It calls
`OpenTransport`/`CloseTransport`/`ObserveTransport` on the selected Provider in
that same child Zone to obtain the transport primitive; everything else is
child core's and ComponentSession/d2b-bus's responsibility. No parent-side
ZoneLink handler exists.

---

## Provider identity

| Field | Value |
| --- | --- |
| `providerRef` | `Provider/transport-unix` |
| Crate | `d2b-provider-transport-unix` |
| Workspace path | `packages/d2b-provider-transport-unix/` |
| Implements | Child-local transport service for Unix-socket ZoneLink sessions and within-Zone bus connections |
| Artifact type | `transport` |
| Component type | `service` |
| Process domains | `system` only |
| ResourceTypes owned | None |
| ResourceTypes consumed | `Host` (executionRef target); `Provider/system-minijail` (Process Provider); `Volume` (view only: component mounts its ProviderDeployment-created Volume view; `Provider/transport-unix` does not own, reconcile, or create Volume resources) |

**D089 desired-spec shape.** This transport Provider owns no ResourceType; child
core reconciles the exact canonical ZoneLink base fields:
`childZoneName`, `transportProviderRef`, `transportSettings`,
`transportCredentials`, `disabled`, and `limits`. Unix-specific desired input
is carried only by `spec.transportSettings`, whose deny-unknown schema is
registered and signed by the Provider selected through
`spec.transportProviderRef`. Unix uses an empty
`spec.transportCredentials` list. No desired-state provider envelope or schema
metadata appears in ZoneLink spec.
`status.provider` remains the D088 implementation-observation layer and does not
mirror desired spec. The `Provider` resource itself keeps the D075
`spec.{artifactId, config}` exception.

---

## Crate layout

Workspace policy rejects a Provider crate missing any required path.

```
packages/d2b-provider-transport-unix/
  src/
    lib.rs              - Provider entry point; component registry; portal service declaration
    service.rs          - Service binary entry: portal listener, request dispatch loop
    portal.rs           - OpenTransport / CloseTransport / ObserveTransport handlers
    seqpacket.rs        - UnixSeqpacketTransport (OwnedTransport impl)
    stream.rs           - UnixStreamTransport (OwnedTransport impl)
    identity.rs         - PeerIdentityPolicy, PeerCredentials, SO_PEERCRED/SO_PASSCRED
    credit.rs           - CreditPool, CreditScopeSet, CreditBundle, ProcessCreditLimit
    descriptor.rs       - PidfdIdentityPolicy, ObjectIdentity, AcceptedAttachment, VerifiedPacket
    admission.rs        - Socket-kind and attachment-policy validation (not Noise enforcement)
    error.rs            - TransportUnixError, stable error codes
    audit.rs            - Bounded transport-observation audit records
    metrics.rs          - OTEL metric label structs and bounded emitter
  tests/
    portal.rs           - OpenTransport / CloseTransport / ObserveTransport hermetic tests
    identity.rs         - SO_PEERCRED, SO_PASSCRED, pathname, inherited-socketpair tests
    credit.rs           - CreditPool / CreditBundle rollback / ProcessCreditLimit headroom
    admission.rs        - Socket-kind and attachment-policy validation tests
    conformance.rs      - Provider dossier conformance assertions
  integration/
    README.md           - Scenario descriptions, prerequisites, and invocation
    transport_open.rs   - OpenTransport end-to-end (allocator FD attachment → OwnedTransport)
    fd_transfer.rs      - SCM_RIGHTS FD transfer through an opened seqpacket transport
    reconnect.rs        - CloseTransport + re-OpenTransport with backoff simulation
    observation_stream.rs - ObserveTransport named-stream events
  README.md
```

`src/` holds the implementation, component binaries, and colocated unit tests
(`#[cfg(test)]`). `tests/` holds hermetic Cargo integration tests that spawn
the real binary against fake Zone endpoints via `CARGO_BIN_EXE_*` and
in-process doubles. `integration/` holds heavier container/Host cross-process
scenarios invoked by `make test-integration` and `make test-host-integration`.
`README.md` is the Provider identity, config, components, build/test, and
consumption reference.

Workspace policy requires only the four root entries: `src/`, `tests/`,
`integration/`, and `README.md`. A nested `integration/README.md` is optional
and may be added to document scenario prerequisites and invocation, but its
absence does not fail the workspace policy check.

---

## ProviderStateSet

Per `ADR-046-provider-state`, a **ProviderStateSet** is the optional, query-time
grouping of the *declared* Volume resources in a Zone whose
`metadata.ownerRef` resolves to `Provider/<name>`. It is empty for a Provider
that declares no state Volume.

`Provider/transport-unix` declares **no** Provider state Volume; its
`ProviderStateSet` is empty. Its bounded non-secret operational state - service
readiness, transport-open/close reconcile stage, bounded connection counters,
and closed-enum error detail - lives in the owning resource's `status`
subresource and the core Operation ledger (D087). All ZoneLink session state
(generation, reconnect count, queued intents, route revision) is owned by the
child Zone's core ZoneLink controller in that child's redb store under its local
`ZoneLink` resource.
The transport service holds only opaque byte-stream handles in its own process
memory (`OwnedTransport`, per D081), which are never persisted.

Because this Provider's operational state is fully derivable from spec,
`status`, the core Operation ledger, and its live process memory, it fails the
storage-need test and declares no state namespace, no state Volume, no
state-view mount, and no dedicated state-layout `User/<name>` principal. There
is no empty identity-only Volume.

---

## ZoneLink `spec.transportSettings` schema

`ZoneLink.spec.transportSettings` is validated at build time against the
settings schema signed by the Provider selected through
`spec.transportProviderRef`. For
`Provider/transport-unix` this schema is committed at:

```
docs/reference/schemas/v3/providers/transport-unix.transport-binding.json
```

### Schema

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "UnixTransportSettings",
  "description": "ZoneLink spec.transportSettings for Provider/transport-unix. Normally empty ({}). The ZoneLink contract prohibits FD/resource grants across Zone boundaries; FD attachment (SCM_RIGHTS) is therefore not configurable here. Child core always opens ZoneLink transports with attachments_enabled=false. The selected parent allocator pre-binds the edge, retains its endpoint only as sealed route state, and injects the child endpoint through sealed bootstrap; child core supplies that endpoint locally to the same-Zone Provider's OpenTransport portal. No parent resource, cross-Zone FD transfer, path, or credential is needed here.",
  "type": "object",
  "properties": {
    "socketKind": {
      "type": "string",
      "enum": ["seqpacket", "stream"],
      "default": "seqpacket",
      "description": "'seqpacket' provides atomic packet-boundary semantics (no SCM_RIGHTS on ZoneLink paths). 'stream' provides framed byte streams. Both carry Noise-protected records only across Zone boundaries."
    }
  },
  "additionalProperties": false,
  "not": {
    "anyOf": [
      { "required": ["attachmentsEnabled"] },
      { "required": ["socketPath"] },
      { "required": ["hostPath"] },
      { "required": ["password"] },
      { "required": ["token"] },
      { "required": ["key"] }
    ]
  }
}
```

The `not/anyOf` block is structurally redundant under `additionalProperties: false`
(those keys are all rejected as unknown fields) but is kept as explicit documentation
signal - `attachmentsEnabled` is listed first to make the prohibition visible at a
glance. Runtime validation also enforces the schema independently of the build step.

### Rules

- `spec.transportSettings: {}` is the normal and recommended value. `socketKind`
  is optional and defaults to `"seqpacket"`.
- **`attachmentsEnabled` is not a ZoneLink `spec.transportSettings` field.** The
  ZoneLink contract prohibits FD and resource grants across Zone boundaries -
  even for same-kernel parent/child links. Child core always opens ZoneLink
  transports with `attachments_enabled=false`; a cross-Zone SCM_RIGHTS attempt
  fails structurally at the child-core boundary before `OpenTransport` is
  called.
- `socketKind: "seqpacket"` over a ZoneLink provides packet-boundary semantics
  only. No SCM_RIGHTS ancillary data is sent or received; only Noise-protected
  record payloads traverse the link.
- SCM_RIGHTS FD attachment is supported when `route_class=local-portal`:
  within-Zone ComponentSessions and portal connections where both endpoints
  are in the same Zone. The distinction is enforced by the `route_class`
  parameter in `OpenTransport` (see Service API section).
- No socket path, host path, password, token, or key appears in any
  ResourceSpec, resource store entry, revision log, or audit record.
- The Provider settings-schema digest is pinned in the Provider resource `spec`
  during catalog publication; the drift gate (`make test-drift`) enforces
  `xtask gen-zone-schemas && git diff --exit-code`.

---

## Transport variants

### Seqpacket transport (`socketKind: "seqpacket"`)

`AF_UNIX SOCK_SEQPACKET` is the default and preferred variant.

**Packet atomicity**: each `send` is one `sendmsg` call. The kernel delivers
the payload and all ancillary data atomically or not at all. `recvmsg` with
`MSG_WAITALL` receives the full datagram or returns an error.

**ZoneLink paths - no SCM_RIGHTS**: when `route_class=zone-link`, the socket
carries only Noise-protected record payloads. No `SCM_RIGHTS` ancillary data
is sent or received; any ancillary data arriving on a zone-link transport is
a protocol error and the session is terminated. `TransportPacket.attachments`
is always empty for ZoneLink transports.

**Local-portal paths - SCM_RIGHTS enabled**: when `route_class=local-portal`
(within-Zone ComponentSessions and portals), SCM_RIGHTS FD attachment is active.
`TransportPacket.attachments` holds exactly the FDs received in that one
`recvmsg` call. Maximum FD count per packet is bounded by
`MAX_PACKET_ATTACHMENTS=32`.

**SO_PASSCRED**: the listener enables `SO_PASSCRED` on the bound socket before
`accept`. The kernel fills a `SCM_CREDENTIALS` control message on the first
incoming packet. `SeqpacketSocket::accept` reads this credential immediately;
it is stored as `PeerCredentials` for that socket and is never re-read or
overwritten by peer data.

**CLOEXEC**: every `OwnedFd` extracted from `SCM_RIGHTS` has `O_CLOEXEC` set
immediately via `fcntl(fd, F_SETFD, FD_CLOEXEC)` before being exposed to any
higher layer. A CLOEXEC failure is a fatal transport error; the FD is closed and
the packet rejected.

**Scavenge on error**: if any validation step fails after FDs are received,
every received FD in the packet is closed immediately. The `VerifiedPacket`
drop impl is the final scavenge fence.

### Stream transport (`socketKind: "stream"`)

`AF_UNIX SOCK_STREAM` is used when packet-boundary semantics are not required.
Stream transport never enables `SCM_RIGHTS` regardless of `route_class`.

**Framing**: each logical record has a 2-byte big-endian length prefix followed
by exactly that many payload bytes. The reader reads the prefix first, then
reads the exact body length using `read_exact`. This handles partial and
coalesced reads.

**No FD attachment**: `TransportDescriptor` for the stream transport has
`attachment_support: false`. Any ComponentSession `AttachmentPolicy` that
requires attachments is incompatible with stream transport and fails the
handshake during offer validation (enforced by ComponentSession, not this
Provider).

---

## Peer identity and provenance

`PeerIdentityPolicy` governs how the transport maps the connected peer to an
authenticated subject. The endpoint configuration selects exactly one policy;
the transport reports `PeerCredentials` upward to ComponentSession for
subject mapping.

### `Accepted`

No further peer check. `PeerCredentials` is still populated from `SO_PEERCRED`
but is not asserted against any config-expected uid/gid. Used only when
identity comes entirely from Noise credential (KK static key or IKpsk2 PSK).

### `Pathname { expected_uid: u32, expected_gid: u32 }`

Used for pathname socket connections. The kernel fills `SO_PEERCRED` with
the connecting process's uid/gid/pid. The transport verifies
`ucred.uid == expected_uid && ucred.gid == expected_gid` on the first data
packet; mismatch closes the connection without response.

The socket path itself never appears in `spec.transportSettings`; the path is
encoded in the allocator-issued FD and never surfaced in the ResourceSpec.

### `InheritedSocketpair`

Used when the allocator creates a `socketpair(AF_UNIX, SOCK_SEQPACKET, 0)` and
retains the parent route endpoint only in the selected allocator's sealed state
while injecting the other endpoint into the child runtime's sealed bootstrap.
The child core passes its endpoint as a **same-Zone portal attachment** to its
local `Provider/transport-unix`; neither endpoint is represented by a parent
resource and no FD is transferred across the ZoneLink. The kernel fills
`SO_PEERCRED` at `socketpair` time; the transport reads `SO_PEERCRED` on the
child endpoint immediately.

`InheritedSocketpair` is the default policy for a child-local ZoneLink whose
compiler-only `parentZone` selects a local parent allocator.

### SO_PASSCRED on first packet

When `SO_PASSCRED` is enabled (seqpacket only), the kernel includes
`SCM_CREDENTIALS` in the ancillary data of the first incoming packet. The
`SeqpacketSocket` reads these credentials from the first `recvmsg` call and
stores them as an immutable `PeerCredentials`. Subsequent packets on the same
connection are bound to the same credentials; the kernel does not allow the
peer to change them after connection.

```rust
pub struct PeerCredentials {
    pub uid: u32,
    pub gid: u32,
    pub pid: i32,  // used only for PidfdIdentityPolicy; never stored or logged
}
```

---

## FD credit pools and descriptor policy

### Credit scopes

FD credits prevent a peer from exhausting the process's open-FD budget. A
reservation succeeds only if all six scopes have sufficient capacity:

| Scope | Default capacity | Emergency headroom |
| --- | --- | --- |
| `Packet` | `MAX_PACKET_ATTACHMENTS=32` | 0 |
| `Request` | `MAX_REQUEST_ATTACHMENTS=64` | 0 |
| `Operation` | `MAX_OPERATION_ATTACHMENTS=128` | 0 |
| `Session` | `MAX_SESSION_ATTACHMENTS=256` | 0 |
| `Process` | derived from `/proc/self/fd` open count | `RESERVED_CONTROL_FDS=64` |
| `Host` | `MAX_HOST_ATTACHMENT_CREDITS=8192` | `RESERVED_CONTROL_FDS=64` |

### Reservation and rollback

`CreditBundle::reserve` attempts all six scopes in order. If any scope fails,
`rollback()` releases every scope already acquired. The calling site receives
`InsufficientCredit { scope }` and is responsible for closing any FDs already
extracted from the packet.

A `CreditBundle` not explicitly committed is automatically rolled back on drop.
`commit()` consumes the bundle and returns per-scope tokens for later release.

### Release

`CreditPool::release(n)` returns credits. It never raises the pool above its
initial capacity. Credits are released when the attachment is closed:
`UnixAttachmentPayload::close()` calls `release()` on each scope exactly once.

### Object identity and duplicate rejection

After extracting FDs from `SCM_RIGHTS`, the descriptor policy calls `fstat` on
each FD and computes `ObjectIdentity { st_dev, st_ino, file_type }`. Duplicates
are rejected (closed and scavenged) unless the endpoint policy explicitly allows
duplicate objects.

---

## Pidfd identity policy

`PidfdIdentityPolicy` provides liveness proof for pidfd attachments. It is
invoked when an attachment descriptor declares `KernelObjectType::Pidfd`:

1. Open a pidfd via `pidfd_open(peer_credentials.pid, 0)`.
2. Read `/proc/<pid>/fdinfo/<attachment_fd>`.
3. Verify `pos` and `flags` are consistent with a live pidfd.
4. Use `kcmp(getpid(), peer_pid, KCMP_FILE, self_fd, peer_fd)` to confirm the
   received FD refers to the same open-file-description as the peer.
5. Reject on any step failure, pid reuse, or FD absence under `/proc`.

The result is an `ObjectIdentity` binding with a `live_at` monotonic timestamp
stored in `AcceptedAttachment`. `pid` is never stored beyond the liveness check
and never appears in any log, audit record, or metric label.

Pidfds validated here are returned to callers (core or d2b-bus) as attachment
payloads. After delivery, the caller owns the FD entirely; the Provider has no
further knowledge of the pidfd.

---

## Blocking adapter requirement

`pidfd_open(2)`, `/proc` reads (`fdinfo`, `status`), `kcmp(2)`, `fstat(2)`,
`getsockopt(2)`, and `fcntl(2)` are all potentially blocking or slow syscalls.
Running them directly on an async task thread stalls the Tokio executor and
delays portal responsiveness and `ObserveTransport` event delivery.

All such calls **must** be dispatched through a bounded blocking adapter:

```rust
// Correct: spawn_blocking isolates the blocking syscall
let identity = tokio::task::spawn_blocking(move || {
    PidfdIdentityPolicy::verify(pid, attachment_fd)
}).await??;

// Correct: getsockopt dispatched via blocking adapter
let sock_type = tokio::task::spawn_blocking(move || {
    admission::get_socket_type(fd)
}).await??;
```

The blocking thread pool is bounded by `TRANSPORT_UNIX_BLOCKING_THREADS=4`
(configurable via component descriptor environment). Each blocking call has a
deadline of `BLOCKING_SYSCALL_TIMEOUT_MS=500` ms; exceeding this fails
`open-transport-bad-attachment` for the affected `OpenTransport` call.

This requirement covers:
- `PidfdIdentityPolicy::verify` (pidfd_open + /proc/fdinfo + kcmp)
- `DescriptorPolicy::fstat_all` (fstat per attachment FD)
- `admission::get_socket_type` (getsockopt SO_TYPE)
- `admission::set_passcred` (setsockopt SO_PASSCRED)
- `admission::set_cloexec` (fcntl F_SETFD)
- `ProcessCreditLimit::measure` (/proc/self/fd readdir)

---

## Service component and Process resource

`Provider/transport-unix` declares one component of type `service`:

```yaml
component:
  id: unix-transport-service
  type: service
  binary: d2b-transport-unix-service
  template: unix-transport-service
  exportedMethods:
    - OpenTransport
    - CloseTransport
    - ObserveTransport
  supportedHostProviders: [system-minijail]
  allowedDomains: [system]
  cardinality: one-per-zone
  stateNamespaces: []          # no Provider state Volume; operational state in status/core ledger (D087)
  permissionClaims:
    - verb: receive-attachment  # receives allocator-issued FD attachments via OpenTransport
    - verb: invoke              # called by the child Zone's core handler over its local portal
  readiness:
    class: provider-defined
    initialDelay: "0s"
    timeout: "5s"
    failureThreshold: 1
    successThreshold: 1
  budget:
    memory:
      limit: "16Mi"
    cpu:
      limit: "50m"
```

The child Zone's core ProviderDeployment (not a Provider controller - no such
controller exists for this Provider) creates the following `Process` resource when
`Provider/transport-unix` is installed (not authored by Nix; emitted by the
ProviderDeployment reconciler):

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: transport-unix-service
  zone: k1
  ownerRef: Provider/transport-unix   # owned; deleted when Provider is removed
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system
  domain: system
  processClass: service
  template: unix-transport-service    # resolved by Provider/transport-unix against its signed package

  sandbox:
    namespaceClasses: [mount]
    capabilityClasses: []             # zero host capabilities
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: minimal
    umask: "0022"

  budget:
    memory:
      request: "4Mi"
      limit: "16Mi"
    cpu:
      request: "10m"
      limit: "200m"
    pids:
      limit: 64
    fds:
      limit: 512                      # bounded; credit system reserves headroom

  readiness:
    class: provider-defined            # service declares ready after portal endpoint handshake
    initialDelay: "0s"
    timeout: "5s"
    failureThreshold: 1
    successThreshold: 1

  restartPolicy:
    class: always
    backoffBase: "2s"
    backoffMax: "60s"
    backoffMultiplierMilli: 2000
    maxRestarts: 10
    resetAfter: "1h"

  telemetry:
    metricsEnabled: true
    tracingEnabled: true
    logLevel: warn
    sensitiveLabels: false

  mounts: []                          # no Provider state Volume; operational state in status/core ledger (D087)
  networkUsage: null                  # no network interface needed
  credentialRefs: []
```

The `Process` resource has `ownerRef: Provider/transport-unix`. The child Zone's
core ProviderDeployment aggregates `Provider/transport-unix` status from the
Process phase and portal readiness condition. If the Process spec drifts, that
child-local ProviderDeployment reconciler restores it.

The service's stable local portal is not an inline `ProcessSpec` field.
ProviderDeployment creates this owned standard `Endpoint` child when the
transport service Process is visible:

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: transport-unix-portal
  zone: k1
  ownerRef: Process/transport-unix-service
spec:
  providerRef: Provider/transport-unix
  producerRef: Process/transport-unix-service
  endpointClass: transport
  transport: unix
  purpose: transport-unix.d2bus.org/portal
  serviceFingerprint: transport-unix.d2bus.org/portal.v1
  locality: host-local
  visibility: zone
  attachmentPolicy: none
  consumerPolicy:
    allowedProviderComponents: [core-controller.d2bus.org/zonelink]
    allowedOperations: [resolve]
  lifecyclePolicy: producer-owned
status:
  phase: Ready
  resource:
    readiness: Ready
    observedProducerGeneration: 1
    observedResourceGeneration: 1
    endpointGeneration: 1
    connectionAvailability: Available
    leaseAvailability: NotRequired
  conditions: []
  update:
    state: Current
    reasons: []
    observedGeneration: 1
    targetGeneration: 1
    disruption: None
    preserveState: true
    operationId: null
    lastAssessedAt: null
    owned: { count: 0, refs: [] }
    dependencies: { count: 0, refs: [] }
```

### Endpoint resources (D092)

Stable managed listener identities that survive long enough for another
component to consume are standard `Endpoint` resources. Consumers refer to
`Endpoint/<name>`; `Endpoint.spec` and `Endpoint.status` carry no raw Unix path,
address, CID, port, FD number, or credential. Authorized resolution occurs only
through the EffectPort/LaunchTicket path; unauthorized callers receive
`endpoint-resolve-denied`. A transport service Process restart bumps the
Endpoint `endpointGeneration`, and dependents observe `dependency-changed`.
ZoneLink session state remains owned by the child Zone's core ZoneLink
controller. The parent store contains no reciprocal Endpoint, Process,
Provider, or ZoneLink resource.

### Retained opaque handles (D092)

Per-session `OpenTransport` named streams, `OwnedTransport` byte-stream handles,
transport connection handles, pidfds, FD attachment indexes, and `operationId`
values remain controller-internal or high-churn opaque handles under the D092
promotion test. They are not `Endpoint` resources and are never used as stable
resource identities.

**Install ordering**: because the Provider declares no state Volume,
ProviderDeployment creates the `Process` resource directly; the service reaches
readiness from its portal condition and its bounded `status`, with no
state-Volume prerequisite.

**Delete ordering**: When `Provider/transport-unix` is deleted, ProviderDeployment
requests Process deletion (setting the Process finalizer) and waits until the
Process reaches terminal status. ProviderDeployment clears its own finalizer only
after the Process has been fully deleted; the service's `status` disappears with
the resource row and its revision. The Provider itself
holds no finalizer on ZoneLink resources; ZoneLink finalizer and status transitions
remain exclusively with the child Zone's core ZoneLink controller.

---

## Service API

The transport-unix service process exposes three typed portal methods. All
methods are invoked by the child Zone's core ZoneLink controller via d2b-bus
over a same-Zone ComponentSession authenticated to the service's enrolled
`portal` endpoint. No other caller may invoke these methods (see RBAC section).
For a ZoneLink call, child core first resolves `spec.transportProviderRef`,
validates `spec.transportSettings`, requires `spec.transportCredentials = []`,
checks `spec.disabled`, enforces `spec.limits`, and derives `socket_kind` from
the validated settings. The service never receives a ZoneLink spec or legacy
provider envelope.

### `OpenTransport`

**Direction**: child Zone's core ZoneLink controller → same-child-Zone
transport-unix service

**Input**:
```text
OpenTransportRequest {
    socket_kind: "seqpacket" | "stream",   // must match the actual socket type
   attachments_enabled: bool,             // must be false for route_class=zone-link
   route_class: "zone-link" | "local-portal",
   // "zone-link": inter-Zone link; cross-Zone FD grants are prohibited
   // "local-portal": within-Zone ComponentSession or portal; seqpacket may carry SCM_RIGHTS
}
// Attachment index 0: the child endpoint from sealed allocator bootstrap.
// This is a connected socketpair end or a just-accepted listener connection.
// Child core supplies it locally as an OwnedAttachment to its same-Zone Provider.
// No FD crosses a Zone boundary.
```

**Provider behavior**:
1. Extract the FD from attachment index 0. Fail `open-transport-bad-attachment`
   if absent or more than one FD.
2. Call `getsockopt(SO_TYPE)` (blocking adapter). Fail `socket-kind-mismatch`
   if the socket type does not match `socket_kind`.
3. **Route class gate**: if `route_class = "zone-link"` and `attachments_enabled
   = true`: fail `attachment-policy-conflict` with detail
   `cross-zone-attachments-forbidden`. This is a belt-and-suspenders check;
   the ZoneLink contract structurally prevents core from ever passing
   `attachments_enabled=true` for a ZoneLink. Core's pre-call validation fires
   before this line is reached.
4. If `socket_kind = "stream"` and `attachments_enabled = true`:
   fail `attachment-policy-conflict`.
5. Set `O_CLOEXEC` on the FD (blocking adapter). Fail `cloexec-set-failed`
   if the syscall fails; close the FD.
6. If `socket_kind = "seqpacket"`: enable `SO_PASSCRED` (blocking adapter).
7. Dup the FD: keep the dup internally for `ObserveTransport` monitoring;
   the original FD is returned to core as the `OwnedTransport` attachment.
8. Allocate an opaque `transport_handle` (a random 16-byte token; redacted in
   all logs and audits).
9. Store `{ dup_fd, transport_handle, route_class, attachments_enabled }` in a
   bounded in-memory handle table (max `MAX_OPEN_TRANSPORTS=256`; fail
   `handle-table-full` if exceeded).

**Output**:
```text
OpenTransportResponse {
    transport_handle: "<opaque-16-byte-token-hex>",
    transport_class: "zone-link-seqpacket" | "zone-link-stream"
                   | "local-seqpacket"     | "local-stream",
    max_packet_size: u32,              // 0 for stream (no packet boundary)
    attachments_enabled: bool,         // always false for zone-link-*; may be true for local-seqpacket
}
// Attachment index 0: the validated OwnedTransport FD (the original received FD).
// Core passes this to d2b-bus which wraps it as UnixSeqpacketTransport or
// UnixStreamTransport.
```

After this call, d2b-bus owns the returned FD entirely. The Provider's dup FD
is used only for monitoring; the Provider reads no data from it and writes
nothing to it.

### `CloseTransport`

**Direction**: child Zone's core ZoneLink controller → same-child-Zone
transport-unix service

**Input**:
```text
CloseTransportRequest { transport_handle: "<opaque>" }
```

**Provider behavior**:
1. Look up `transport_handle` in the handle table. Return `unknown-handle` if
   not found (idempotent; core may call CloseTransport after a service restart).
2. Close the dup monitoring FD. Remove the entry from the handle table.
3. The observation stream (if open) receives a `PEER_DISCONNECTED` event and
   the named stream is half-closed. Core must drain the stream before calling
   CloseTransport or tolerate the close-before-drain.

**Output**: `CloseTransportResponse {}`

Child core calls `CloseTransport` when:
- The ZoneLink is deleted: the child Zone's core ZoneLink controller calls
  `CloseTransport` to release the transport handle before removing its own
  ZoneLink transport finalizer. The ProviderDeployment holds no ZoneLink
  finalizer.
- The ZoneLink spec changes in a way requiring a new child endpoint (child core
  re-calls `OpenTransport` after `CloseTransport`).
- The session engine has permanently failed on the returned FD.

### `ObserveTransport`

**Direction**: child Zone's core ZoneLink controller → same-child-Zone
transport-unix service (named stream)

**Input**:
```text
ObserveTransportRequest { transport_handle: "<opaque>" }
```

**Provider behavior**:
1. Look up `transport_handle`. Return `unknown-handle` if not found.
2. Register an async epoll watcher on the dup monitoring FD for
   `POLLHUP | POLLERR | POLLRDHUP`.
3. Return a named stream. For each epoll event:
   - `POLLHUP` or `POLLRDHUP` → emit `{ kind: PEER_DISCONNECTED, error_code: null }`.
   - `POLLERR` → emit `{ kind: ERROR, error_code: "transport-error" }`.
4. When `CloseTransport` is called: half-close the stream and stop watching.
5. The Provider reports only socket-level transport events; it does not
   interpret Noise records, session state, or resource API payloads on the FD.

**Output**: named stream of `TransportObservation`:
```text
TransportObservation {
    kind: PEER_DISCONNECTED | ERROR,
    error_code: string?,    // stable code from error table; null if not ERROR
}
```

`ObserveTransport` is optional. Core may choose not to open it if it tracks
transport health through other means (e.g., ComponentSession disconnect events
from the session engine). If not opened, the Provider's epoll watcher is still
active after `OpenTransport`; the events are silently discarded until
`ObserveTransport` is called or `CloseTransport` is called.

---

## ComponentSession profile notes

ComponentSession (ADR-046-componentsession-and-bus) and d2b-bus own all Noise
profile enforcement. The transport-unix Provider does not enforce Noise
profiles. It provides the transport FD; the session engine selects and executes
the handshake. This section notes the profile characteristics that make
certain profiles possible or impossible over Unix transports.

### NN - local identity over Unix transport

`Noise_NN_25519_ChaChaPoly_SHA256` is viable over Unix seqpacket or stream when:

- The `EndpointPurpose` class is `local` (within-Zone connections).
- The transport provides directional `PeerCredentials` from SO_PEERCRED.
- The endpoint policy maps the uid/gid to exactly one Zone-local canonical subject.

NN supplies forward-secret ephemeral record protection; peer authentication comes
from OS-enforced SO_PEERCRED, not a peer-supplied long-term key.

### KK - enrolled peers over Unix transport

`Noise_KK_25519_ChaChaPoly_SHA256` is used for enrolled ZoneLink ComponentSessions (every steady-state session after the one-time IKpsk2 bootstrap enrollment):

- Both static public keys are known before handshake.
- The child Zone's enrolled static identity is pinned in sealed
  enrollment/bootstrap state and verified by the child Zone's core ZoneLink
  controller after the handshake - not by this Provider.
- Unix seqpacket or stream carries the handshake bytes; the transport is
  unaware of the Noise content.
- SO_PEERCRED is still verified by `PeerIdentityPolicy`; for KK sessions its
  role is supplemental provenance, not primary authentication.

The child Zone's core ZoneLink controller drives the core-owned
enrollment-and-session state machine
`Unenrolled -> IKpsk2 -> EnrollmentCommitted -> KK -> Ready` (canonical in
ADR-046-zone-routing): the one-time IKpsk2 bootstrap runs only from `Unenrolled`
(and after revocation), and every enrolled steady-state session, including every
reconnect, uses KK. This Provider carries whichever Noise profile core selects
from that enrollment state on its FD; the transport never selects, negotiates,
or reorders handshake profiles.

### IKpsk2 - one-time bootstrap

`Noise_IKpsk2_25519_ChaChaPoly_SHA256` for Zone enrollment. The Provider
passes through IKpsk2 sessions transparently; it does not generate, consume,
or validate PSKs.

---

## d2b-bus and core integration

```text
child Zone K1 core ZoneLink controller
  -> K1 d2b-bus portal route  ->  K1 transport-unix service Process
       OpenTransport(socket_kind, attachments_enabled)
       + attachment[0]: child endpoint injected by sealed allocator bootstrap
       <-
       OpenTransportResponse(transport_handle, transport_class, ...)
       + attachment[0]: validated OwnedTransport FD

selected parent allocator (chosen by compiler-only K1.parentZone)
  -> retains only the peer endpoint and route binding as sealed state
  -> creates no ZoneLink/Provider/Process/Endpoint row in the parent store

child Zone K1 core ZoneLink controller
  -> K1 d2b-bus
       hands OwnedTransport to session engine
  -> ComponentSession (core-driven enrollment-and-session FSM:
       Unenrolled -> IKpsk2 -> EnrollmentCommitted -> KK -> Ready;
       one-time IKpsk2 bootstrap when Unenrolled, then enrolled KK;
       record protection, named streams, reconnect)
  -> selected parent allocator's route endpoint
```

After `OpenTransport` returns, the transport FD is owned by the session engine.
K1's d2b-bus:
- wraps the FD as `UnixSeqpacketTransport` or `UnixStreamTransport`;
- establishes the ComponentSession handshake selected by core from the ZoneLink
  enrollment state (`Unenrolled -> IKpsk2 -> EnrollmentCommitted -> KK ->
  Ready`): the one-time IKpsk2 bootstrap when the link is `Unenrolled`,
  otherwise the enrolled KK handshake; the transport never selects or reorders
  the profile;
- owns per-session FD credits, named stream scheduling, reconnect generation;
- forwards resource API traffic over K1's allocator-bound uplink.

The child Zone's core ZoneLink controller:
- reconciles K1's local `ZoneLink` resource (status, conditions, finalizers);
- manages reconnect policy and intent queue;
- publishes/withdraws route advertisements to the `RouteTreeEngine`;
- calls `CloseTransport` when the ZoneLink is deleted or the transport must be replaced;
- calls `ObserveTransport` optionally to correlate socket events with session
  disconnect handling.

### Currency and upgrade (D091)

The child Zone's core ZoneLink controller, not the transport-unix service or
the parent allocator, implements
`assess_update`, `plan_upgrade`, and `execute_upgrade`. A Provider generation or
signed artifact generation/digest change updates universal `status.update` with
`state: UpdateAvailable` or `state: UpgradeRequired`, `reasons` including
`ProviderGenerationChanged` or `ArtifactChanged`, observed/target generation or
digest IDs, `disruption: Reload` or `disruption: Restart`, `preserveState:
true`, bounded `owned`/`dependencies`, and `lastAssessedAt`. Disruptive changes
MUST return `UpgradeRequired` rather than applying in place; non-disruptive
changes reconcile normally. Upgrade recycles the transport service realization;
open byte-stream handles are re-established by child-core reconnect. ZoneLink
session state remains owned in the child store by the child Zone's core
ZoneLink controller. `status.update` MUST NOT contain secrets.

### Expedited reconcile on mutation (D090)

For `Create`, `UpdateSpec`, and `Delete` with `waitForReconcile`, core MUST
perform no `OpenTransport`/`CloseTransport`, finalizer change, or status
mutation until it supplies
`CommittedRevisionProof {resourceUid,generation,revision,operationId}`. Abort
before that proof has no effect. After durable commit, the commit is never
rolled back if the reconcile pass times out. The response returns the committed
object, post-pass projected layered status, disposition
(`Converged|Progressing|Blocked|UpgradeRequired|Failed`), and
`statusPersistence: pending|committed`. Effect idempotency keys derive from
`(UID,generation,revision,operationId)` and use the same per-resource
single-flight priority lane.

**No global transport registry**: the Provider's `portal.rs` handle table is
local in-process state, bounded to `MAX_OPEN_TRANSPORTS=256`, and keyed by
opaque tokens. It is not published as a service endpoint, route table, or Zone
resource. Core addresses the handle by the opaque token returned from
`OpenTransport`.

**No fallback**: if the transport-unix service is not Ready or `OpenTransport`
returns an error, the child Zone's core ZoneLink controller sets its local
`SessionEstablished` condition to `False` with the appropriate reason code and
retries per the ZoneLink reconnect policy. There is no automatic downgrade to
stream, vsock, or TCP.

---

## RBAC and security invariants

### Role and RoleBinding requirements

The child Zone's core ZoneLink controller requires one `invoke` permission on
the transport-unix portal service in that same child Zone:

| Verb | Scope | Notes |
| --- | --- | --- |
| `invoke` | `transport-unix-portal` service | To call `OpenTransport`, `CloseTransport`, `ObserveTransport` |
| `receive-attachment` | `transport-unix-portal` | To receive the validated OwnedTransport FD attachment in the `OpenTransportResponse` |

No `watch`, `get`, `update-status`, `update-finalizers`, or any resource verb
is required by this Provider. The Provider owns no ResourceTypes and performs
no resource plane operations.

The transport-unix service Process requires only the permissions established by
Provider/system-minijail's process model:
- Receive its allocator-issued portal endpoint FD at spawn.
- Receive child-endpoint FD attachments in `OpenTransport` requests from its
  authorized same-Zone child core caller.
- Emit metrics to the Zone OTEL datagram socket.

The selected parent allocator receives no portal permission and makes no
Provider call. Its peer endpoint and route binding remain sealed allocator/route
state; no cross-Zone FD, ResourceRef, or status mutation is permitted.

### No ambient authority

The Provider holds no persistent secret, socket path, or session state.
Transport handles are opaque tokens; they are generated fresh on each
`OpenTransport` call and discarded on `CloseTransport` or service restart.
Static Noise keys used for the portal KK session are enrolled at Provider
installation and sealed by the Zone runtime; they are never in the service's
`Process.spec`.

### Peer identity is kernel-supplied

`SO_PEERCRED` and `SCM_CREDENTIALS` are kernel-filled. The transport never
trusts a uid/gid/pid from the peer payload. On seqpacket, forged
`SCM_CREDENTIALS` messages (require `CAP_SETUID`) are rejected: only the
first kernel-filled credential is used and it comes from the
accept/connect path.

### No path traversal or TOCTOU

All FD operations use `fstat`/`fcntl`/`getsockopt` relative to the received
FD, never absolute paths after accept. Pidfd liveness uses `fdinfo` and `kcmp`;
no `/proc/<pid>/exe` or path-based checks.

### Debug, log, and metric redaction

The following values are **never** logged, audited, or emitted as metric labels:

- `PeerCredentials.pid` (used only for pidfd liveness; not stored)
- Transport handles (opaque tokens; redacted in all Debug output)
- Static private keys (zeroizing; never reachable by a logger)
- ZoneLink name in observation events (opaque correlation ID only)
- FD numbers or socket addresses
- Any payload bytes from monitored FDs

### Invariants enforced fail-closed

| Invariant | Enforcement point |
| --- | --- |
| No socket path in `spec.transportSettings` | JSON Schema + eval-time assertion |
| No credential bytes in `spec.transportSettings` | JSON Schema + build-time validation |
| `attachmentsEnabled` not a ZoneLink `spec.transportSettings` field | JSON Schema (`additionalProperties:false` + `not/anyOf`) |
| `additionalProperties: false` rejects all unknown keys | JSON Schema |
| ZoneLink transports always have `attachments_enabled=false` | Child Zone's core ZoneLink controller (structural, pre-call); `admission.rs::validate_route_class` (belt-and-suspenders) |
| `route_class=zone-link` + `attachments_enabled=true` → `attachment-policy-conflict` | `portal.rs::open_transport` step 3 |
| CLOEXEC on every received FD | `portal.rs::open_transport` (blocking adapter) |
| Credit rollback on any validation failure | `CreditBundle` RAII drop |
| FD scavenge on decryption failure | `VerifiedPacket` RAII drop |
| `socketKind=stream` → `attachments_enabled` must be false | `admission.rs::validate_route_class` |
| `getsockopt(SO_TYPE)` verified against declared `socketKind` | `portal.rs::open_transport` (blocking adapter) |
| Duplicate kernel objects rejected | `ObjectIdentity` check in `DescriptorPolicy` |
| Pidfd liveness before delivery | `PidfdIdentityPolicy` at descriptor validation |
| Handle table bounded at `MAX_OPEN_TRANSPORTS=256` | `portal.rs` handle table `insert` |
| Provider never calls allocator/broker directly | No allocator API call site exists in this crate |

---

## Lifecycle and status

### Provider resource status

The child-local Provider resource `spec` contains the `artifactId` and signed
descriptor digests. Its `status` is managed by that child Zone's core
ProviderDeployment; the parent has no Provider resource or Provider status:

Per D088, child core writes the Provider universal `ResourceStatus` base at top-level
`status.*` and any Provider ResourceType-common observation in
`Provider.status.resource` (core-derived per D085). `Provider/transport-unix`
does not write Provider status directly. If this Provider ever writes bounded,
non-secret implementation-specific observation on a resource it owns, that
detail must live in `status.provider` with `providerRef:
Provider/transport-unix`, a qualified `schemaId` such as
`transport-unix.d2bus.org/<ResourceType>/status`, `schemaVersion` (semver MAJOR.MINOR),
`observedProviderGeneration`, and a strict unknown-field-denied, ≤32 KiB,
redacted `details` object registered and signed in the Provider manifest; the
writer updates all present layers atomically and never duplicates shared fields
from `status.resource`.

| Phase | Meaning |
| --- | --- |
| `Pending` | Provider resource created; package not yet verified or service Process not yet Ready |
| `Ready` | Package verified; service Process in Ready phase; portal endpoint accepting calls |
| `Degraded` | Service Process exited/restarting or portal not responsive; previous transport handles are invalidated |
| `Failed` | Package trust check failed or service Process exhausted restarts |

### Service Process status

The transport-unix service Process uses standard Process status
(`Pending|Ready|Succeeded|Degraded|Failed|Deleted`). The system-minijail
controller reports:

| Condition | True reason | False reason |
| --- | --- | --- |
| `PortalReady` | `portal-handshake-ok` | `portal-timeout`, `process-not-started`, `binary-exec-failed` |

`Succeeded` is not a steady-state phase for this long-lived service; the
process lifecycle is `Restart: Always`.

For the service Process, ResourceType-common observation belongs in
`Process.status.resource`; any transport-unix-specific bounded detail would use
the D088 `status.provider.details` extension schema rather than a copied common
field.

### Transport handle lifecycle

```
OpenTransport -> handle allocated, monitoring dup active
ObserveTransport (optional) -> named stream open, epoll watcher registered
CloseTransport -> monitoring dup closed, handle removed, observation stream half-closed
```

Transport handles are local in-process state. They do not appear in any
resource, revision log, or status field. If the service Process restarts,
all handles are lost; core detects this through its `ObserveTransport` stream
EOF or through the ZoneLink `SessionEstablished` condition becoming `False`
when the session engine reports a transport disconnect.

---

## Error codes

Stable transport-layer errors emitted by `Provider/transport-unix`. These
cover FD validation and portal mechanics only. Errors relating to ZoneLink
identity, session generation, Noise handshake, route advertisement, or resource
API operations are owned by core/ComponentSession/d2b-bus.

Wire-stable u32 tags are frozen at first release; protobuf field numbers are
never reused.

| Code | Stable tag | Meaning |
| --- | --- | --- |
| `socket-kind-mismatch` | 1 | `getsockopt(SO_TYPE)` on the received FD does not match the declared `socketKind` |
| `attachment-policy-conflict` | 2 | `route_class=zone-link` with `attachments_enabled=true` (cross-Zone FD grants are prohibited by the ZoneLink contract); or `socketKind=stream` with `attachments_enabled=true`; or `attachmentsEnabled` field present in ZoneLink `spec.transportSettings` (rejected structurally before Provider) |
| `open-transport-bad-attachment` | 3 | `OpenTransport` request has no FD attachment or carries more than one FD |
| `invalid-socket-fd` | 4 | Received FD is not a valid `AF_UNIX` socket (wrong address family or type) |
| `cloexec-set-failed` | 5 | `fcntl(F_SETFD, FD_CLOEXEC)` failed; FD closed |
| `peer-credential-policy-rejected` | 6 | SO_PEERCRED uid/gid does not match the `Pathname` policy expectation |
| `duplicate-kernel-object` | 7 | Two received FDs in one packet share `st_dev`/`st_ino` and duplicates are not permitted by policy |
| `insufficient-credit` | 8 | FD credit reservation failed a `CreditScope`; includes `scope` detail field |
| `attachment-on-stream-socket` | 9 | `SCM_RIGHTS` ancillary data received on a stream transport or on a session with `attachmentsEnabled=false` |
| `pidfd-liveness-check-failed` | 10 | `PidfdIdentityPolicy` rejected the received pidfd (process reuse, fdinfo mismatch, kcmp failure) |
| `handle-table-full` | 11 | `MAX_OPEN_TRANSPORTS=256` open transport handles already active |
| `unknown-handle` | 12 | `CloseTransport` or `ObserveTransport` supplied an unrecognized or already-closed `transport_handle`; idempotent for `CloseTransport` |
| `transport-settings-schema-violation` | 13 | `spec.transportSettings` violates the JSON Schema at runtime (belt-and-suspenders; build/eval guards should precede this) |

Retriable: `handle-table-full` (after delay; core may retry after a prior
transport is closed). All others are non-retriable from the Provider's perspective;
core decides whether to retry based on its ZoneLink reconnect policy.

---

## Audit events

The transport-unix service emits bounded transport-observation records only.
It does not audit ZoneLink resource transitions, session establishment/teardown,
child Zone identity verification, route changes, or intent queue operations -
those are core/session/broker audit responsibilities.

All records are emitted under category `transport-unix` to the Zone runtime's
audit log and committed before the operation completes.

| Event kind | Required fields | Trigger |
| --- | --- | --- |
| `transport-opened` | `transport_class` (`local-seqpacket`\|`local-stream`), `attachments_enabled` (bool), `peer_policy` (`accepted`\|`pathname`\|`inherited-socketpair`) | `OpenTransport` succeeds; no uid/gid/pid/path/handle in record |
| `transport-closed` | `transport_class` | `CloseTransport` called; no uid/gid/pid/path/handle in record |
| `peer-credential-policy-rejected` | `peer_policy="pathname"`, `socket_kind` | `peer-credential-policy-rejected` error; no uid/gid values |
| `attachment-credit-exhausted` | `scope`, `requested`, `available` | `insufficient-credit` error |
| `pidfd-identity-rejected` | `error_detail` (`"liveness"`\|`"fdinfo"`\|`"kcmp"`) | `pidfd-liveness-check-failed`; no pid in record |
| `cloexec-enforcement-failed` | (no additional fields) | `cloexec-set-failed` error |

**Redaction rules (strict)**:
- No `uid`, `gid`, or `pid` in any record.
- No FD numbers, socket paths, or host paths.
- No Noise key material or transport handles.
- No ZoneLink names or Zone-principal identifiers.
- `peer_policy` is a bounded enum string; it is the only peer-characterization field.

---

## OTEL, metrics, and performance targets

### Metric label constraints

All metrics use a closed, pre-declared label set. Labels are bounded
low-cardinality strings. No metric carries uid/gid/pid, resource names, session
bytes, FD numbers, socket paths, transport handles, or key material.

| Metric name | Labels | Description |
| --- | --- | --- |
| `d2b_transport_unix_opens_total` | `transport_class`, `outcome` | `OpenTransport` calls completed or failed |
| `d2b_transport_unix_closes_total` | `transport_class` | `CloseTransport` calls |
| `d2b_transport_unix_observations_active` | `transport_class` | Current open `ObserveTransport` named streams |
| `d2b_transport_unix_observation_events_total` | `transport_class`, `kind` | Transport observation events emitted |
| `d2b_transport_unix_packets_total` | `transport_class`, `direction` | Packets sent/received on active transports |
| `d2b_transport_unix_attachments_total` | `transport_class`, `outcome` | FD attachments accepted/rejected |
| `d2b_transport_unix_credit_exhausted_total` | `scope` | Credit exhaustion events per scope |
| `d2b_transport_unix_pidfd_rejections_total` | `reason` | Pidfd liveness rejections |
| `d2b_transport_unix_open_transport_duration_seconds` | `transport_class`, `outcome` | Histogram of `OpenTransport` latency |
| `d2b_transport_unix_handle_table_depth` | (none) | Current handle table entry count (gauge) |

Labels:
- `transport_class`: `"local-seqpacket"` or `"local-stream"`
- `direction`: `"send"` or `"recv"`
- `outcome`: `"ok"` or `"error"`
- `kind`: `"peer-disconnected"` or `"error"`
- `scope`: `"packet"`, `"request"`, `"operation"`, `"session"`, `"process"`, `"host"`
- `reason`: `"liveness"`, `"fdinfo"`, `"kcmp"`

### OTEL span attributes

Spans emitted by the service carry only:

```
d2b.transport.kind = "unix"
d2b.transport.class = "local-seqpacket" | "local-stream"
d2b.provider.component = "unix-transport-service"
```

No span attribute carries uid/gid/pid, transport handles, FD numbers, socket
addresses, key material, payload bytes, or peer-specific identity strings.

### Performance targets

These targets gate the Provider's integration tests:

| Measurement | Target | Fixture |
| --- | --- | --- |
| `OpenTransport` latency (seqpacket, socketpair) | p95 ≤ 2 ms | In-process; fake portal; pre-created socketpair |
| `OpenTransport` latency (stream) | p95 ≤ 1 ms | In-process; fake portal; pre-created socket |
| `CloseTransport` latency | p99 ≤ 0.5 ms | In-process; single handle |
| Seqpacket round-trip (no attachments, post-session-open) | p99 ≤ 0.5 ms | Same-host loopback via d2b-bus stub |
| Credit reservation (all six scopes, no contention) | p99 ≤ 100 µs | In-process, no lock contention |
| `ObserveTransport` event delivery after POLLHUP | p95 ≤ 5 ms | Controlled peer-close + epoll watcher wake |

---

## Nix configuration

### Option schema

The Nix option tree mirrors the canonical `ResourceSpec` schema directly. The
selected `Provider/transport-unix` is authored in the same child Zone as its
ZoneLink. The parent Zone has no reciprocal resource:

```nix
# local-root is the distinguished root. parentZone is forbidden and no
# reciprocal ZoneLink or transport Provider is authored in its store.
d2b.zones.local-root = {};

# K1 is the CHILD Zone; this compiler-only setting selects local-root's
# allocator and is not emitted into any resource.
d2b.zones.k1.parentZone = "local-root";

d2b.zones.k1.resources.transport-unix = {
  type = "Provider";
  spec = {
    artifactId = "provider-transport-unix";  # matches d2b.artifacts.provider-transport-unix
    config = {};
  };
};
```

The associated artifact declaration (separate from the resource spec):

```nix
d2b.artifacts.provider-transport-unix = {
  package = pkgs.d2b-provider-transport-unix;
  type    = "provider";
};
```

The `ZoneLink` spec is authored in the standard resource syntax:

```nix
# K1's sole child-local uplink (seqpacket, no cross-Zone FD attachment).
d2b.zones.k1.resources.k1-uplink = {
  type = "ZoneLink";
  spec = {
    childZoneName        = "k1";
    transportProviderRef = "Provider/transport-unix";
    transportSettings    = {};   # child bootstrap endpoint; no ZoneLink attachments
    transportCredentials = [];
    disabled = false;
    limits = {
      maxPendingIntents    = 256;
      maxActiveStreams     = 32;
      reconnectMaxAttempts = 10;
      reconnectWindowSecs  = 300;
    };
  };
};

# Independent K3 child using the stream variant. It needs its own same-Zone
# selected Provider; a Provider ref never resolves across Zones.
d2b.zones.k3.parentZone = "local-root";
d2b.zones.k3.resources.transport-unix = {
  type = "Provider";
  spec = {
    artifactId = "provider-transport-unix";
    config = {};
  };
};

d2b.zones.k3.resources.k3-uplink = {
  type = "ZoneLink";
  spec = {
    childZoneName        = "k3";
    transportProviderRef = "Provider/transport-unix";
    transportSettings    = { socketKind = "stream"; };
    transportCredentials = [];
    disabled = false;
    limits = {
      maxPendingIntents    = 256;
      maxActiveStreams     = 32;
      reconnectMaxAttempts = 10;
      reconnectWindowSecs  = 300;
    };
  };
};
```

### Canonical emitted `ZoneLink` JSON with default `spec.transportSettings`

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "type": "ZoneLink",
  "metadata": { "name": "k1-uplink", "zone": "k1" },
  "spec": {
    "childZoneName": "k1",
    "transportProviderRef": "Provider/transport-unix",
    "transportSettings": {
      "socketKind": "seqpacket"
    },
    "transportCredentials": [],
    "disabled": false,
    "limits": {
      "maxPendingIntents": 256,
      "maxActiveStreams": 32,
      "reconnectMaxAttempts": 10,
      "reconnectWindowSecs": 300
    }
  }
}
```

`attachmentsEnabled` does not appear in the canonical JSON. Core enforces
`attachments_enabled=false` for all ZoneLink transports at the OpenTransport
call site regardless of socket kind. The emitter fills the `socketKind` schema
default before computing `generationId`. `k1.parentZone` is absent because it
is compiled only into the selected allocator's sealed bootstrap state; no
reciprocal local-root resource is emitted.

### Eval-time assertions

These supplement generated per-field option type checks and live in
`nixos-modules/assertions.nix`:

| Assertion | Error message |
| --- | --- |
| ZoneLink and selected Provider are declared in the same child Zone; `spec.childZoneName` equals that Zone; compiler-only `parentZone` resolves to the allocator owner; no reciprocal parent resource is emitted | `zones.<zone>: transport-unix ZoneLink and Provider must be child-local with a self-matching childZoneName` |
| `spec.transportSettings` must not contain `attachmentsEnabled` | `zones.<zone>.resources.<name>: spec.transportSettings.attachmentsEnabled is not a valid ZoneLink transport field; FD attachment across Zone boundaries is prohibited` |
| `spec.transportSettings` contains no top-level key `socketPath`, `hostPath`, `password`, `token`, or `key` | `zones.<zone>.resources.<name>: transportSettings must not contain host paths, socket paths, or secret material. Remove the <forbidden-keys> key(s) from spec.transportSettings: transport endpoints are allocator-issued and secrets are referenced as Credential resources.`, where `<forbidden-keys>` is the comma-separated list of the forbidden keys actually present |

These are belt-and-suspenders: the JSON Schema `additionalProperties: false` and
the `not/anyOf` block already reject these keys. The eval assertion catches them
earlier (at `nix eval` time) with a clearer error message focused on the security
rationale.

### Build-time validation additions

The `xtask gen-zone-resources` step adds for `Provider/transport-unix` links:

1. Resolve `spec.transportProviderRef` to the same-Zone Provider and validate
   `spec.transportSettings` against
   `docs/reference/schemas/v3/providers/transport-unix.transport-binding.json`.
2. Reject `attachmentsEnabled` as a ZoneLink `spec.transportSettings` key
   (structurally covered by `additionalProperties:false`, and explicitly named
   in `not/anyOf`).

### Eval and build tests

| Test ID | Kind | What it proves |
| --- | --- | --- |
| `nix-unit: transport-unix-child-local-topology` | nix-unit eval | Provider and ZoneLink are in K1, `childZoneName = "k1"`, `k1.parentZone = "local-root"` selects only the allocator, and no local-root reciprocal resource is emitted |
| `nix-unit: transport-unix-exact-zonelink-spec` | nix-unit eval | Emitted spec has exactly `childZoneName`, `transportProviderRef`, `transportSettings`, `transportCredentials`, `disabled`, and `limits`; legacy provider/fingerprint/capability fields are rejected |
| `nix-unit: transport-unix-empty-settings` | nix-unit eval | `spec.transportSettings = {}` passes all assertions |
| `nix-unit: transport-unix-socket-path-rejected` | nix-unit eval | `spec.transportSettings.socketPath = "/run/..."` rejected at eval |
| `nix-unit: transport-unix-attachments-enabled-rejected` | nix-unit eval | `spec.transportSettings.attachmentsEnabled = false` rejected as unknown field at eval |
| `nix-unit: transport-unix-stream-variant` | nix-unit eval | `socketKind = "stream"` accepted; no other field needed |
| `drift: transport-unix-transport-binding-schema` | `make test-drift` | `xtask gen-zone-schemas && git diff --exit-code` for transport binding schema |
| `build: transport-unix-unknown-field-rejected` | flake check | Unknown key in `spec.transportSettings` fails build |
| `build: transport-unix-attachments-enabled-is-unknown-field` | flake check | `attachmentsEnabled` field in ZoneLink `spec.transportSettings` fails build (not in schema) |

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has an advisory wall-clock p95
diagnostic threshold of <=50 ms; gate enforcement is aggregate per-crate
process CPU only. There is no wall-clock
sleep, and `cargo test -p d2b-provider-transport-unix --lib --tests` completes in ≤3 s warm-cache
execution time (compilation excluded). They use a deterministic fake clock/RNG
and the toolkit fakes/FakeEffectPort only - no process spawn, container,
network, DBus, systemd, broker daemon, Nix eval/build, KVM, USB/GPU/TPM
hardware, or live cloud, and no filesystem tree beyond tiny temp fixtures. Any
scenario needing those lives only in `integration/`, which keeps a lane
timeout/budget, parallel isolation, and fake external services by default; such
a need is re-placed into `integration/`, never given a sleep, larger timeout,
or `#[ignore]`. Bounded crypto/property tests are the only classified
exception, each named with a capped case count and a declared higher per-test
budget.

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass -
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.

---

## Current-code fit

| Item | Current anchor | Evidence class | Treatment |
| --- | --- | --- | --- |
| Unix seqpacket transport | `d2b-session-unix/src/adapter.rs` `UnixSeqpacketTransport` (main `a1cc0b2d`) | `implemented-but-unwired` | copy and adapt to v3 portal model |
| Unix stream transport | `d2b-session-unix/src/adapter.rs` `UnixStreamTransport` (main `a1cc0b2d`) | `implemented-but-unwired` | copy unchanged |
| FD credit pool | `d2b-session-unix/src/credit.rs` (main `a1cc0b2d`) | `implemented-but-unwired` | copy unchanged |
| SO_PEERCRED / SO_PASSCRED | `d2b-session-unix/src/adapter.rs` `PeerIdentityPolicy`, `src/socket.rs` (main `a1cc0b2d`) | `implemented-but-unwired` | copy and adapt for v3 Zone subject mapping |
| Pidfd identity policy | `d2b-session-unix/src/descriptor.rs` `PidfdIdentityPolicy` (main `a1cc0b2d`) | `implemented-but-unwired` | copy unchanged |
| Unix session tests (12) | `d2b-session-unix/tests/unix_session.rs` (main `a1cc0b2d`) | `test-only-or-preview` | copy and adapt for v3 portal model |
| OpenTransport/CloseTransport/ObserveTransport service API | none | `ADR-only` | new |
| Service Process resource with full sandbox/budget/endpoints spec | none in v3 baseline | `ADR-only` | new |
| ProviderStateSet | ownerRef=Provider/transport-unix Volume query | `ADR-only` | Empty - `Provider/transport-unix` declares no Provider state Volume; its bounded non-secret operational state lives in `status`/the core Operation ledger (D087) |
| ZoneLink `spec.transportSettings` schema | none | `ADR-only` | new in this spec |
| FD pre-bind pattern (broker FD pre-binding) | `d2b-priv-broker/src/ops/{swtpm_dir,spawn_runner}.rs` (v3 baseline) | `implemented-and-reachable` | selected parent allocator adapts the pattern, retains its peer endpoint as sealed route state, and injects only the child endpoint through sealed bootstrap; the child-local Provider is a passive recipient and no FD crosses the ZoneLink |
| Route advertisement / RouteTreeEngine | `d2b-realm-core/src/route_engine.rs` (v3 baseline) | `implemented-but-unwired` | child core owns local ZoneLink session/cursor state; selected parent allocator owns only sealed route state; not in this Provider |
| ZoneLink controller (reconcile/status/finalizer/routes) | none in v3 baseline | `ADR-only` | owned by the child Zone's core-controller (ADR-046-core-controllers); no parent-side handler and not in this Provider |
| OTEL bounded emitter | v3 `tracing` crate usage (baseline) | `implemented-and-reachable` | extend existing pattern |

---

## Implementation work items

### ADR046-transport-unix-001: v3 contract types and Unix transport constants

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-transport-unix-001` |
| Dependency/owner | ADR046-session-001 (v3 ComponentSession contracts); W0 shared contract root |
| Current source | `packages/d2b-contracts/src/v2_component_session.rs` (protocol constants, credit-class constants) at main `a1cc0b2d` |
| Reuse source | Same file; constants copied into `packages/d2b-contracts/src/v3/zone_session.rs` by ADR046-session-001 |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-transport-unix/src/credit.rs` (imports `MAX_PACKET_ATTACHMENTS=32`, `RESERVED_CONTROL_FDS=64`, credit-class constants from v3 contract); `src/portal.rs` (imports `MAX_PACKET_ATTACHMENTS` for portal validation) |
| Detailed design | Import credit scope capacities and headroom from `v3_zone_session.rs`; add `MAX_OPEN_TRANSPORTS: usize = 256` local constant for handle table bound. Primary reuse disposition: `adapt`. Preserved source-plan detail: Dependency on ADR046-session-001 output. |
| Integration | `CreditScopeSet` constructed from imported constants at session setup |
| Data migration | None; v3 constants freeze independently |
| Validation | `tests/credit.rs::ancillary_capacity_is_derived_from_closed_hard_bounds` passes against v3 constants |
| Removal proof | No current code imports v3 transport constants; new import |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

---

### ADR046-transport-unix-002: seqpacket transport and peer identity

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-transport-unix-002` |
| Dependency/owner | ADR046-transport-unix-001; d2b-bus transport layer (ADR046-bus-001) |
| Current source | `packages/d2b-session-unix/src/{adapter,socket,descriptor}.rs`, `tests/unix_session.rs` at main `a1cc0b2d` |
| Reuse source | Same; `UnixSeqpacketTransport`, `PeerIdentityPolicy`, `UnixAttachmentPayload`, `OwnedUnixAttachment`, `SeqpacketSocket`, `PeerCredentials`, `ObjectIdentity`, `AcceptedAttachment`, `VerifiedPacket` |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-transport-unix/src/{seqpacket,identity,socket}.rs` |
| Detailed design | Copy transport structs verbatim; adapt `PeerIdentityPolicy` to report `PeerCredentials` upward to ComponentSession for subject mapping (not for direct resource lookup - that is core's responsibility); maintain `SO_PASSCRED` setup and first-packet credential extraction as documented; CLOEXEC enforcement uses `rustix` syscall wrappers over `libc` where available Primary reuse disposition: `adapt`. Preserved source-plan detail: copy and adapt. |
| Integration | `portal.rs::open_transport` calls `SeqpacketSocket::getsockopt(SO_TYPE)` and `setsockopt(SO_PASSCRED)`, constructs `UnixSeqpacketTransport`, hands OwnedTransport FD back to caller |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | Copy all 12 test functions; add `peercred_reported_to_componentsession_not_resolved_to_subject_here` |
| Removal proof | `d2b-realm-transport` seqpacket path retired after ZoneLink sessions migrate to child-local Providers and tests prove no reciprocal parent-store resource or cross-Zone FD transfer remains |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

---

### ADR046-transport-unix-003: stream transport

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-transport-unix-003` |
| Dependency/owner | ADR046-transport-unix-001 |
| Current source | `packages/d2b-session-unix/src/adapter.rs` `UnixStreamTransport`, `src/socket.rs` `StreamSocket` at main `a1cc0b2d` |
| Reuse source | Same |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-transport-unix/src/{stream,socket}.rs` |
| Detailed design | Copy verbatim; add `attachment_support: false` in `TransportDescriptor` (stream never carries SCM_RIGHTS regardless of route class); `admission.rs::validate_route_class` rejects `attachments_enabled=true` for stream Primary reuse disposition: `adapt`. Preserved source-plan detail: copy unchanged. |
| Integration | Same path as seqpacket but without SCM_RIGHTS paths |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | `tests/portal.rs::stream_open_transport_forces_no_attachments`; `tests/identity.rs::stream_transport_reassembles_partial_and_coalesced_records` |
| Removal proof | No current stream ZoneLink path exists; stream is net-new |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

---

### ADR046-transport-unix-004: FD credit pool system

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-transport-unix-004` |
| Dependency/owner | ADR046-transport-unix-001 |
| Current source | `packages/d2b-session-unix/src/credit.rs` at main `a1cc0b2d` |
| Reuse source | Same; `CreditPool`, `CreditScopeSet`, `CreditBundle`, `ProcessCreditLimit`, `CreditScope` |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-transport-unix/src/credit.rs` |
| Detailed design | Copy all five types verbatim; import scope-capacity constants from v3 contract; add `#[derive(Debug)]` with redacted Display (no raw counts in Debug output) Primary reuse disposition: `adapt`. Preserved source-plan detail: copy unchanged. |
| Integration | `CreditScopeSet` created per active ComponentSession; `CreditBundle` per packet receive; credits released in `UnixAttachmentPayload::close()` |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | Copy all 4 credit test functions; add `credit_released_on_attachment_close` and `emergency_headroom_constant_across_fd_counts` |
| Removal proof | No current code path uses this crate directly; new |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

---

### ADR046-transport-unix-005: pidfd identity policy

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-transport-unix-005` |
| Dependency/owner | ADR046-transport-unix-002 |
| Current source | `packages/d2b-session-unix/src/descriptor.rs` `PidfdIdentityPolicy`, `DescriptorPolicy` at main `a1cc0b2d` |
| Reuse source | Same |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-transport-unix/src/descriptor.rs` |
| Detailed design | Copy verbatim; adapt `DescriptorPolicy::verify` to produce `AcceptedAttachment` carrying `ObjectIdentity` binding for v3 ComponentSession attachment descriptor model; `pid` not stored beyond liveness check Primary reuse disposition: `adapt`. Preserved source-plan detail: copy and adapt. |
| Integration | Called by seqpacket transport after decrypting attachment descriptor |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | Copy `pidfd_identity_requires_live_launch_evidence_and_rejects_unrelated_process` and `duplicate_kernel_objects_are_rejected_and_cleaned_up` |
| Removal proof | Broker pidfd-open path in `d2b-priv-broker/src/sys.rs` serves different purpose (process supervision); no removal dependency |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

---

### ADR046-transport-unix-006: socket-kind validation and attachment admission

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-transport-unix-006` |
| Dependency/owner | ADR046-transport-unix-002 |
| Current source | No existing v3 socket-kind admission module |
| Reuse source | `getsockopt(SO_TYPE)` pattern widely used; no specific main reuse source |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-transport-unix/src/admission.rs` |
| Detailed design | `validate_route_class(route_class, socket_kind, attachments_enabled, received_fd)` calls `getsockopt(SO_TYPE)` (blocking adapter) on `received_fd`: `SOCK_SEQPACKET` must match `"seqpacket"`, `SOCK_STREAM` must match `"stream"`, any other type fails `invalid-socket-fd`; if `route_class == RouteClass::ZoneLink && attachments_enabled == true` fail `attachment-policy-conflict` with detail `cross-zone-attachments-forbidden`; if `socket_kind == "stream" && attachments_enabled == true` fail `attachment-policy-conflict`; no Noise profile enforcement (that is ComponentSession's responsibility); returns `Ok(RouteAdmission { route_class, socket_kind, attachments_enabled })` |
| Integration | Called by `portal.rs::open_transport` before the monitoring dup and handle allocation |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | `tests/admission.rs::seqpacket_fd_passes_seqpacket_kind`; `stream_fd_passes_stream_kind`; `seqpacket_fd_rejects_stream_kind_declaration`; `zone_link_with_attachments_enabled_fails`; `local_portal_seqpacket_with_attachments_accepted`; `stream_with_attachments_enabled_rejected` |
| Removal proof | No current code has this gate; new path |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

---

### ADR046-transport-unix-007: OpenTransport / CloseTransport / ObserveTransport portal

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-transport-unix-007` |
| Dependency/owner | ADR046-transport-unix-002 through 006; ADR046-bus-001 (d2b-bus ComponentSession method dispatch); ADR046-session-001 (named-stream protocol) |
| Current source | No portal service in v3 baseline; `d2b-provider-toolkit/src/server.rs` `GeneratedProviderServiceServer` dispatch pattern (main `a1cc0b2d`) for service entry pattern |
| Reuse source | main `a1cc0b2d` `d2b-provider-toolkit/src/server.rs` service dispatch pattern |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-transport-unix/src/{portal,service}.rs` |
| Detailed design | `portal.rs`: `PortalHandler` struct owns a bounded `HashMap<TransportHandle, MonitorState>` (capacity `MAX_OPEN_TRANSPORTS=256`); `open_transport(req, attachment_fd)` validates via `admission.rs`, dups FD, allocates handle, stores `MonitorState { dup_fd, observation_senders: Vec<NamedStreamSender> }`; `close_transport(handle)` closes dup FD, half-closes all observation senders, removes entry; `observe_transport(handle)` registers a new `NamedStreamSender` and spawns an async epoll-watcher task on the dup FD; `TransportHandle` is a `[u8; 16]` random token; redacted in all Debug impls; `service.rs` is the binary entry: accepts the allocator-issued portal endpoint FD at launch, runs `GeneratedTransportServiceServer` over it, dispatches to `PortalHandler` Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt dispatch pattern; implement portal methods as new. |
| Integration | The child Zone's core ZoneLink controller calls the three methods via same-Zone d2b-bus; the selected parent allocator retains its peer endpoint as sealed route state and injects the child endpoint through sealed bootstrap; the child runtime supplies that FD locally at Process spawn, not through a parent Provider, a cross-Zone attachment, or `SD_LISTEN_FDS` |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | `tests/portal.rs::open_transport_zone_link_validates_and_returns_ownedtransport`; `open_transport_uses_child_bootstrap_endpoint_only`; `parent_endpoint_never_enters_provider_portal`; `open_transport_local_portal_seqpacket_with_attachments_accepted`; `open_transport_zone_link_attachments_enabled_rejected`; `close_transport_is_idempotent_after_handle_removed`; `observe_transport_delivers_pollhup_as_peer_disconnected`; `handle_table_rejects_at_max_capacity`; `restart_clears_all_handles` |
| Removal proof | Ad-hoc IPC stubs in `d2bd/src/` retired after portal migration |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

---

### ADR046-transport-unix-008: service Process resource definition and Provider package

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-transport-unix-008` |
| Dependency/owner | ADR046-transport-unix-007; ADR046-provider-003 (system Provider framework); Provider/system-minijail (ADR046-provider-003) |
| Current source | `packages/d2b-priv-broker/src/` minijail spawn patterns (v3 baseline); `packages/d2b-host/src/` process arg patterns; current package derivations in `flake.nix` |
| Reuse source | Minijail sandbox semantic class patterns from current v3 broker; Process resource schema from ADR-046-resources-host-guest-process-user |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-transport-unix/` crate Cargo.toml binary target `d2b-transport-unix-service`; Provider component descriptor JSON committed at `packages/d2b-provider-transport-unix/descriptor/unix-transport-service.json`; Nix package derivation at `packages/d2b-provider-transport-unix/` |
| Detailed design | Component descriptor declares: `processClass=service`, `template=unix-transport-service`, `stateNamespaces=[]` (no Provider state Volume; bounded non-secret operational state in status/core ledger, D087), `sandbox.capabilityClasses=[]`, `sandbox.namespaceClasses=[mount]`, `sandbox.seccompClass=strict`, `budget.memory.limit="16Mi"`, `budget.cpu.limit="200m"`, `budget.fds.limit=512`, `endpoints=[{name:portal,transport:unix,purpose:transport-unix-portal}]`, `readiness={class:provider-defined,initialDelay:"0s",timeout:"5s",failureThreshold:1,successThreshold:1}`, `restartPolicy={class:always,backoffBase:"2s",backoffMax:"60s",backoffMultiplierMilli:2000,maxRestarts:10,resetAfter:"1h"}`; Provider package bundles descriptor digest; core ProviderDeployment creates the Process with empty `mounts` when `Provider/transport-unix` is installed Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt; no direct symbol copy. |
| Integration | Provider resource installed → core ProviderDeployment reads component descriptor → creates child `Process/transport-unix-service` (no state-Volume prerequisite) → ProviderSupervisor spawns binary with portal FD in inherited FD table. On delete: Process terminal first → ProviderDeployment finalizer cleared last; the service `status` disappears with the resource row |
| Data migration | None (fresh Provider resource) |
| Validation | `tests/conformance.rs::process_resource_matches_component_descriptor`; `tests/conformance.rs::provider_state_set_is_empty`; `tests/conformance.rs::no_state_volume_mount`; sandbox policy tests against minijail conformance kit |
| Removal proof | No current transport-service Process exists; new path |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

---

### ADR046-transport-unix-009: Nix configuration and transport binding schema

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-transport-unix-009` |
| Dependency/owner | ADR046-transport-unix-001; Nix/build integrator |
| Current source | `nixos-modules/options-realms.nix` realm options (v3 baseline); `nixos-modules/assertions.nix` |
| Reuse source | None |
| Reuse action | create |
| Destination | `docs/reference/schemas/v3/providers/transport-unix.transport-binding.json`; `nixos-modules/assertions.nix` (assertion additions); generated `nixos-modules/generated/options-zones-ZoneLink.nix` `transportSettings` submodule |
| Detailed design | Commit the JSON Schema; run `xtask gen-zone-schemas` and `xtask gen-zone-nix-options` to regenerate committed files; generate the exact six-field ZoneLink base; reject legacy provider envelopes and allocator-private fingerprint/capability fields; add assertions for stream+attachments conflict and sensitive key names; require an empty `transportCredentials` list; reuse the common topology assertions to require the ZoneLink and selected Provider in the same child Zone, self-matching `childZoneName`, compiler-only non-root `parentZone`, and no reciprocal parent resource; `xtask gen-zone-resources` adds Provider-selected `transportSettings` validation |
| Integration | Build emitter resolves `spec.transportProviderRef`, validates `spec.transportSettings` against that Provider's schema, and validates the empty `spec.transportCredentials` list before computing `generationId`; the topology compiler seals child→parent allocator selection separately and emits only the child-local Provider/ZoneLink resources; drift gate enforces sync |
| Data migration | `d2b.realms.*` Nix options superseded by `d2b.zones.*`; no compatibility bridge (v3 reset) |
| Validation | All eval/build tests in the Nix section, including `transport-unix-exact-zonelink-spec`, `transport-unix-child-local-topology`, legacy-field rejection, empty-credential enforcement, and a generated-bundle assertion that the parent store has no reciprocal Provider/ZoneLink row |
| Removal proof | `nixos-modules/options-realms.nix` realm wiring retired after Zone resource bundle activation replaces it |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

---

### ADR046-transport-unix-010: audit, OTEL, and metrics

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-transport-unix-010` |
| Dependency/owner | ADR046-transport-unix-007; ADR-046-telemetry-audit-and-support |
| Current source | v3 baseline `tracing` crate patterns; v3 `d2b-realm-router/src/service_v2.rs` audit field shapes |
| Reuse source | None |
| Reuse action | create |
| Destination | `packages/d2b-provider-transport-unix/src/{audit,metrics}.rs` |
| Detailed design | `AuditRecordKind` enum with 6 event kinds from Audit section; `AuditRecord` carries only the fields listed (no uid/gid/pid/path/handle/ZoneLink name); emit via Zone runtime `emit_audit_record()` interface; `MetricCounter`/`MetricHistogram` with closed label types per Metrics section; emit via bounded in-process ring to OTEL Provider datagram socket; `tracing::instrument` spans on `PortalHandler` methods with the 3 permitted span attributes only |
| Integration | `portal.rs` calls `audit.rs::emit_*` before returning from each portal method; `seqpacket.rs` calls `metrics.rs::record_*` on every accept/packet/attachment |
| Data migration | Existing `d2bd/src/metrics.rs` VM-label metrics superseded by v3 metrics; not migrated |
| Validation | `tests/conformance.rs::audit_records_contain_no_pid_uid_or_handle`; `tests/conformance.rs::metric_labels_are_closed_set`; `tests/conformance.rs::span_attributes_contain_no_sensitive_fields` |
| Removal proof | `d2bd/src/metrics.rs` hand-rolled registry retired after metric surface migration |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

---

### ADR046-transport-unix-011: integration tests and README

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-transport-unix-011` |
| Dependency/owner | ADR046-transport-unix-007 through 010; test orchestration owner |
| Current source | No existing integration tests for Unix portal scenarios |
| Reuse source | Test scenario shapes from `d2b-session-unix/tests/unix_session.rs` end-to-end test (main `a1cc0b2d`) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-transport-unix/integration/` and `integration/README.md` |
| Detailed design | Four scenarios: `transport_open.rs` (fixture models compiler-only `k1.parentZone = "local-root"`; fake selected allocator retains one socketpair endpoint as sealed route state and injects the other into K1 bootstrap; K1 core passes only that child endpoint to its same-Zone Provider → OwnedTransport out; verify socket kind, CLOEXEC, SO_PASSCRED, K1-local Provider/ZoneLink, and absent local-root reciprocal row; p95 latency assertion ≤2 ms); `fd_transfer.rs` (within-Zone local-portal seqpacket `SCM_RIGHTS` transfer, credit accounting, scavenge on error injection; ZoneLink route rejects the same packet); `reconnect.rs` (CloseTransport + re-OpenTransport with a fresh sealed child endpoint, verify previous handle is unknown and monitoring dup closed); `observation_stream.rs` (ObserveTransport stream receives `PEER_DISCONNECTED` when peer closes within 5 ms p95). `integration/README.md` documents prerequisites (no KVM required; all scenarios use in-process socketpairs and fake Zone API endpoint stub), invocation (`cargo test -p d2b-provider-transport-unix --test integration`), environment variables, and expected output |
| Integration | Invoked by `make test-integration`; no host mutation; each scenario creates its own socketpair, keeps parent endpoint state outside the resource stores, and exposes only the child endpoint to the child-local Provider |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | All four scenarios pass in CI; latency assertions enforced using monotonic timestamps; scavenge correctness verified by open-FD count before/after error injection |
| Removal proof | Ad-hoc IPC test stubs retired after scenario parity |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |
