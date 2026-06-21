# Constellation peer protocol reference

**Diataxis category:** reference.

This page documents the ADR 0032 peer-protocol skeleton. It is
transport-neutral: transports provide reachability, while the peer-session
layer owns version/codec/schema negotiation, authentication binding, frame
caps, and typed semantic frames.

## Session establishment

Plain peer sessions use a length-delimited semantic `Handshake` frame and
receive either `HandshakeAccepted` or `HandshakeRejected`. A peer is refused
before any operation or stream frame is routed when any of these fields do
not match the local session policy:

- protocol version;
- codec id;
- codec schema fingerprint;
- channel-binding/peer-binding context.

Secure peer sessions bind the protocol version, codec id, schema
fingerprint, peer identity, and both nonces into the HMAC transcript. After
that handshake succeeds, frames are encrypted with directional keys derived
from the same transcript. A replayed nonce, identity mismatch, codec/schema
mismatch, or invalid MAC is rejected as an authentication failure.

## Frame format

The session layer carries a 4-byte little-endian length followed by one
codec frame. The frame cap is 1 MiB and is enforced before payload
allocation or protobuf decode.

Transport adapters must preserve byte-exact delivery, bounded pending
session queues, typed unavailable/backpressure errors, and explicit
shutdown behavior. The reusable checks are listed in the
[transport conformance matrix](./transport-conformance-matrix.md).

The protobuf codec maps bytes to the codec-neutral `ConstellationFrame`.
The router consumes only semantic frames; it never depends on protobuf
types. The schema fingerprint for the current protobuf shape is exposed by
`ProtocolCodec::schema_fingerprint()` and participates in handshake
binding.

## Operation routing

`OperationRequest` carries realm, node, principal, operation kind,
optional workload, bounded body, trace context, and a caller-generated
idempotency key for mutating operations. The required capability is derived
from `OperationKind` in trusted code; peers never supply the required
capability as a wire field.

The router owns idempotency for its node/gateway scope. It keys dedup by
realm, principal, node, operation kind, and idempotency key. A lost-reply
retry with the same request replays the recorded response, while a
same-key different request or a post-retention reuse fails closed.

Capabilities are negotiated as positive assertions; the accepted session
uses the intersection both peers understand and support. A missing
capability causes a typed `capability-denied` refusal before an operation
or stream is executed. Negotiation records carry a bounded fingerprint so
audit can cite the selected set without copying it into every event.

## Named streams

Post-handshake stream frames are typed and mux-validated before callers see
them. A stream must open with a `StreamDescriptor` and capability-derived
`StreamAuthz`; data before open, unknown streams, invalid channels, and
data after close fail closed.

Flow control is credit-based. `StreamFlow` grants a non-zero number of
frames, `StreamData.sequence` is strictly increasing per stream, and the
mux exposes deterministic sendable-stream ordering plus a round-robin
selection primitive for fair draining. Cancellation is idempotent for
already-cancelled streams so reconnect/cancel retries do not create
spurious protocol errors.

`StreamResume` carries a durable cursor and is accepted only for resumable
stream kinds, currently logs. Non-resumable streams reject cursor resume
requests before any transport replay or provider action.

## Durable execution

Durable execution uses `ExecutionId` for reconnect and retry. A start
request records bounded metadata plus an `ExecutionGeneration`; a same-id,
same-generation retry returns the retained summary, while a same id with
different metadata fails closed. Attach/reconnect validates the generation
before exposing streams, preventing a stale boot from attaching to a new
process. Logs requests carry explicit byte bounds and optional retained
cursors. Cancel is idempotent so lost replies and reconnect retries can
repeat the request safely.

Command arguments, environment, cwd, stdio, and log bytes are never part of
the routing metadata. They remain opaque operation or stream payloads owned
by the execution adapter.

## Non-goals

This skeleton does not implement Azure Relay, remote full-host transport,
provider-specific display, or live execution adapters. Those are later
ADR 0032 waves.
