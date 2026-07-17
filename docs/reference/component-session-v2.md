# ComponentSession v2 contract

ComponentSession is the authenticated record boundary for d2b-owned live IPC.
This document describes the wire contract. It does not define transport I/O,
Noise state-machine execution, scheduling, or descriptor ownership.

The Rust source of truth is
`d2b_contracts::v2_component_session`. The strict generated schema fixture is
[`component-session-v2-schema.json`](component-session-v2-schema.json), and the
fixed cryptographic vectors are
[`component-session-v2-vectors.json`](component-session-v2-vectors.json).

## Preface

Every connection starts with exactly 16 network-order bytes:

| Offset | Size | Value |
| ---: | ---: | --- |
| 0 | 8 | `44 32 42 43 53 32 0d 0a` (`D2BCS2\r\n`) |
| 8 | 2 | major `2` |
| 10 | 2 | minor `0` |
| 12 | 4 | canonical offer byte length, `1..=16384` |

Short input, long input, bad magic, unsupported major, unsupported minor,
empty offer, and over-limit offer are distinct errors. No version range,
preference list, feature intersection, legacy preface, or fallback exists.

## Guest session credential

`GuestSessionCredentialV1` is the single broker-to-guestd credential codec. It
is a secret-bearing canonical binary contract and is intentionally excluded
from JSON Schema and serde. All integers and length fields are big-endian.

The base credential is exactly 156 bytes:

| Offset | Size | Value |
| ---: | ---: | --- |
| 0 | 8 | `D2BGSV2\0` magic |
| 8 | 2 | schema version `1` |
| 10 | 2 | codec version `1` |
| 12 | 2 | flags; bit 0 means bootstrap is present |
| 14 | 2 | reserved, zero |
| 16 | 4 | total credential byte length |
| 20 | 8 | nonzero ComponentSession generation |
| 28 | 32 | parent X25519 static public key |
| 60 | 32 | channel binding |
| 92 | 32 | guest identity digest |
| 124 | 32 | guest X25519 static public key |

When bootstrap is present, a two-byte block length follows the base credential.
The canonical current block is 98 bytes:

| Offset | Size | Value |
| ---: | ---: | --- |
| 156 | 2 | bootstrap block length, `98` |
| 158 | 2 | operation-ID length, `16` |
| 160 | 16 | operation ID |
| 176 | 32 | replay nonce |
| 208 | 8 | nonzero issue time in Unix milliseconds |
| 216 | 8 | expiry time in Unix milliseconds |
| 224 | 32 | bootstrap PSK |

The decoder applies the shared 64-byte operation-ID ceiling before constructing
the stricter current `OperationId`. The canonical total is therefore 256 bytes;
the absolute defensive maximum is 304 bytes. Expiry must be later than issue
time and the checked lifetime is capped at five minutes. Admission before issue
time or at/after expiry fails closed without timestamp arithmetic.

Decode rejects truncation at every byte, trailing bytes, unknown schema or
codec versions, unknown flags, nonzero reserved fields, inconsistent or
over-limit lengths, generation zero, zero bindings/digests/public keys/nonces/
issue time/expiry/PSK, invalid or overflowing lifetimes, and malformed operation
IDs. No legacy magic or alternate layout is accepted. Secret Debug output is
fully redacted. PSKs are copied directly from the decoder input into stable
heap-backed zeroizing storage and only the owning pointer moves afterward.
`GuestBootstrapPsk::generate_with` lets a broker fill that storage directly;
callers that generate material in any other source or scratch buffer remain
responsible for wiping those copies. Encoded credentials are returned as opaque,
non-cloneable `GuestSessionCredentialBytes` with redacted Debug, bounded
`as_slice`/`write_to` access, and guaranteed backing-buffer wipe on release.

## Handshake contract

The offer's canonical binary encoding starts with encoding version `1`, then
encodes fixed-width integers in network order in this sequence:

1. endpoint purpose and purpose class;
2. initiator and responder roles;
3. service package;
4. 32-byte schema fingerprint;
5. Noise profile;
6. the complete limit profile;
7. transport class, locality, 32-byte channel binding, and required identity
   evidence;
8. reconnect generation;
9. the complete attachment policy.

The accept carries the exact canonical offer and a nonzero 32-byte transcript
binding. The reject carries one closed reason and remediation. Decoders reject
unknown tags, truncation, trailing bytes, noncanonical booleans, invalid
contracts, and over-limit input. The canonical offer is 148 bytes and must fit
both the global 16 KiB ceiling and its own selected
`limits.handshakeOfferBytes`; a self-declared smaller profile is invalid on
encode and decode. An endpoint compares every offer field for equality with its
policy; it never selects a weaker value.

### Local generation discovery

A local command client may know the daemon endpoint identity while not knowing
the daemon's restart generation. For `unix-stream` and `unix-seqpacket`
endpoints using directional Unix identity evidence, it may begin with the
bounded generation-discovery exchange. The query carries the canonical
140-byte `EndpointPolicyIdentity`: every offer field except generation. The
responder compares every field with its endpoint policy before returning a
nonzero generation and a SHA-256 binding to the exact query.

The Unix transport authenticates peer credentials and endpoint provenance
before this exchange. The client treats the returned generation as negotiated
only after completing the normal Noise handshake whose exact offer contains
that generation. A modified reply therefore causes either exact-offer
generation rejection or transcript failure. Generation discovery is not
available to enrolled, bootstrap, vsock, provider-stream, or direct-configured
endpoints. It does not make generation zero valid, does not weaken
`EndpointPolicy` equality, and is not a separate discovery socket or legacy
fallback.

### Service packages

The closed inventory is:

```text
d2b.daemon.v2
d2b.realm.v2
d2b.guest.v2
d2b.provider.v2
d2b.broker.v2
d2b.user.v2
d2b.runtime.systemd-user.v2
d2b.shell.v2
d2b.clipboard.v2
d2b.clipboard.picker.v2
d2b.notify.v2
d2b.security-key.v2
d2b.wayland.v2
d2b.activation.v2
d2b.tty.v2
```

No `.v1` package or arbitrary package string is representable.

### Purposes and identity profiles

The closed purposes are `daemon-local`, `daemon-remote`, `realm-peer`,
`realm-bootstrap`, `guest-control`, `guest-bootstrap`, `provider-agent`,
`privileged-broker`, `user-agent`, `runtime-systemd-user`,
`shell-supervisor`, `clipboard-control`, `clipboard-picker`,
`clipboard-bridge`, `desktop-observer`, `security-key`,
`activation-helper`, `tty-helper`, and `wayland-proxy`.

Purpose class is explicit because a `provider-agent` can be local or enrolled:

| Purpose class | Noise profile | Required identity evidence |
| --- | --- | --- |
| `local` | `Noise_NN_25519_ChaChaPoly_SHA256` | directional Unix evidence |
| `enrolled` | `Noise_KK_25519_ChaChaPoly_SHA256` | enrolled static keys |
| `bootstrap` | `Noise_IKpsk2_25519_ChaChaPoly_SHA256` | expected parent static key and single-use operation-bound PSK |

Roles, locality, and transport are closed enums. Locality is `process-local`,
`host-local`, `guest-local`, or `remote`. Transport is `unix-stream`,
`unix-seqpacket`, `inherited-socketpair`, `native-vsock`,
`cloud-hypervisor-vsock`, `provider-stream`, or `direct-configured`.
Every closed enum uses the spelling listed by this contract identically in JSON
and generated JSON Schema; variant-name case conversion is not part of the
wire contract.

## Limits

All additions and subtractions use checked arithmetic before allocation.
Selected profiles may lower, never raise, these ceilings:

| Resource | Hard maximum |
| --- | ---: |
| Offer | 16 KiB |
| Protected ciphertext | 65,535 bytes |
| Protected plaintext after Noise tag | 65,519 bytes |
| Logical ttrpc or named-stream message | 1 MiB |
| Active named streams | 128 |
| Attachments per packet / request / operation / session | 32 / 64 / 128 / 256 |
| Process / host attachment credits | 2,048 / 8,192 |
| Reserved nonattachment descriptors | 64 |
| Named-stream queue, per stream / aggregate | 256 KiB / 4 MiB |
| ttrpc / session-control queue | 2 MiB / 64 KiB |
| Keepalive interval / timeout | 60 s / 30 s |
| Local / remote handshake deadline | 5 s / 15 s |
| Local / remote reconnect objective | 5 s / 30 s |
| Reconnect attempts / window | 10 / 5 min |

Ciphertext allocation includes the two-byte wire length, 16-byte Noise tag,
and component header. A peer length is never used in unchecked allocation
arithmetic.

## Records and fragments

The 24-byte record header is:

```text
kind:u8 flags:u8 channel:u16 sequence:u64 generation:u64 payload_len:u32
```

Channel `0` is session control, `1` is ttrpc control, and `2` is attachment
control. Named streams begin at `0x0100`; the intervening range is invalid.
Record kind must match channel class. Sequence state distinguishes replay,
out-of-order input, and exhausted nonces. Sequence `u64::MAX` is reserved;
`u64::MAX - 1` is the final usable send or receive sequence.

The 24-byte fragment header is:

```text
message_id:u64 index:u32 count:u32 total_plaintext_len:u32 offset:u32
```

Message, count, total length, channel, generation, sequence, and offsets are
authenticated by the enclosing record. Fragment state distinguishes another
message, duplicate, reorder, overlap, invalid bounds, and input after
completion. Only ComponentSession fragments logical messages.

Session control includes close, keepalive ping/pong, request cancellation, and
cancellation acknowledgement. Cancellation names both reconnect generation and
the fixed 16-byte request ID.

## Request envelope and deadlines

Request IDs and trace IDs are exactly 16 bytes. Correlation IDs and idempotency
keys are `1..=64` bytes. Unknown JSON fields and out-of-bound IDs are rejected.
The envelope authenticates:

- request, correlation, trace, and optional idempotency IDs;
- `issuedAtUnixMs`;
- absolute `expiresAtUnixMs`.

The maximum future clock skew is 30 seconds and the maximum declared lifetime
is 15 minutes. Expiry before issue, issue beyond skew, expiry at or before the
receiver clock, and a lifetime beyond either global or service cap are
rejected.

At each hop, wall-clock remaining time is intersected with the existing
monotonic budget, service cap, and any peer ttrpc timeout. The result is a
relative nanosecond duration; epoch time is never placed in `timeout_nano`.
A peer timeout can only shorten the authenticated deadline.

## Packet-atomic attachments

Attachments are available only for Unix seqpacket or inherited socketpair
policies. An authenticated packet declares the exact descriptor count and an
ordered descriptor for each object. A descriptor binds:

- closed kind, kernel object type, access, and semantic purpose;
- service, method, request, optional operation, packet sequence, and session
  generation;
- duplicate-object policy and mandatory `CLOEXEC`;
- packet, request, operation, session, process, and host credit classes.

No numeric or raw file descriptor is serialized. Payload and ancillary data
arrive in one packet. Message truncation, control truncation, unknown control,
missing or extra descriptors, order mismatch, absent `CLOEXEC`, policy
mismatch, and credit exhaustion are fatal before semantic dispatch. Credit
arithmetic is checked at all six scopes. A `credentials` descriptor represents
exactly one `SCM_CREDENTIALS` control record as `process-credentials`; it is not
a pidfd or another `SCM_RIGHTS` object, has no `CLOEXEC` claim, and is accepted
only when the exact negotiated attachment policy sets
`credentialsAllowed = true`. Count and credit validation still fail first when
those bounds are violated.

## Errors and telemetry

`SessionErrorCode`, `HandshakeRejectReason`, `CloseReason`, `Remediation`, and
attachment/parser errors are closed stable enums. They distinguish malformed
framing, authentication and transcript failures, every exact-offer mismatch,
replay/order/nonce failures, fragment failures, deadline and cancellation
states, attachment validation and credit failures, control exhaustion,
keepalive, disconnect, and invariant failure.

Metric labels are represented only by closed enums for transport, purpose,
channel class, Noise profile, locality, provider type, health state, result,
and reason. Session, request, stream, realm, workload, provider, operation,
endpoint, and user identifiers are not metric fields.

## Deterministic Noise vectors

The committed vector artifact contains one case for every local purpose,
including attachment-enabled seqpacket cases; enrolled realm, guest, and
provider peers; and realm and guest bootstrap. Every case fixes:

- protocol name, purpose class, exact prologue, roles, and package;
- static and ephemeral private test keys and static public keys;
- derived static and ephemeral public keys, checked through the pinned `snow`
  resolver;
- PSK where applicable;
- both handshake payloads and messages;
- transcript hash and directional transport keys;
- first protected record in each direction;
- transcript downgrade, cross-purpose, purpose-class, role, schema, limit, and
  channel-binding mutations;
- wrong-operation, expired-PSK, and replay mutations for bootstrap.

The vectors are test data, not production keys. Contract tests verify them
with the pinned `snow = "=0.10.0"` dev dependency, reject corrupted declared
public keys, and execute bootstrap admission against the fixture's operation
ID, replay nonce, validity time, and expiry time. Successful bootstrap admission
consumes the state; wrong-operation, expiry, and second-use attempts return
their exact closed handshake rejection reasons. Production contract code
contains no cryptographic implementation.
