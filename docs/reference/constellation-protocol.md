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

## Non-goals

This skeleton does not implement Azure Relay, remote full-host transport,
stream lifecycle beyond the existing semantic frame/mux primitives, or
durable execution. Those are later ADR 0032 waves.
