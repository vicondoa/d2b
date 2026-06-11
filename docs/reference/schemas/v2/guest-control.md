# `guest-control.json` schema (`v2` companion)

Schema: [`guest-control.json`](./guest-control.json)

`guest-control.json` is the typed guest-control contract for ADR 0026's
ttRPC/protobuf surface. It snapshots the guest daemon control messages, health
states, exec lifecycle messages, and chunked stdio RPC shapes that host
`nixlingd` and guest `nixling-guestd` must keep aligned. The protobuf service
source lives at
[`packages/nixling-ipc/proto/guest_control.proto`](../../../../packages/nixling-ipc/proto/guest_control.proto).
The generated Rust bindings are intentionally split by role:
`packages/nixling-ipc/src/generated/guest_control.rs` is message-only and
`ttrpc`-free, while `packages/nixling-guestd/src/generated/guest_control_ttrpc.rs`
contains the guestd-local service bindings.

## Top-level sections

- `schemaVersion` / `protocolVersion` - guest-control schema and wire version.
- `transport` - host-to-guest virtio-vsock ttRPC transport on port `14318`,
  the separate guest-to-host observability port `14317`, the reserved unused
  side-channel port `14319`, the Cloud Hypervisor `CONNECT 14318\n` handshake,
  the bounded opaque `OK <decimal-local-port>\n` acknowledgement, and the
  readiness rule that socket existence alone is never readiness.
- `limits` - bounded frame, chunk, live-buffer, detached-log, and concurrency
  limits exposed only after authentication through `Authenticate` /
  `Capabilities`.
- `hello` - unauthenticated challenge/discovery only. It returns a guest nonce,
  guest boot id, and protocol version; it does not return health, capabilities,
  or a capability fingerprint.
- `authenticate` / `authenticated` - proof-of-possession boundary for the
  guest-control token. `Authenticate` carries the host HMAC over the canonical
  transcript; `authenticated` returns the guest HMAC plus bounded authenticated
  health and capabilities.
- `healthRequest`, `health`, `capabilitiesRequest`, and `capabilities` -
  authenticated readiness/version/capability RPCs. Pre-ttRPC CONNECT, Hello
  challenge, and Authenticate failures are host-synthesized status, not
  guest-returned `Health` RPC payloads.
- `exec*`, `writeStdin`, `readOutput`, `closeStdin`, `ttyWinResize`, and
  signal/cancel messages - Docker-like exec lifecycle and chunked stdio.
  `controlAck` is the shared response for resize, signal, and cancel control
  events.

## Contract notes

- The schema describes the protocol contract; it is not a commitment to use
  JSON as the runtime transport.
- Raw `argv`, environment values, stdin, stdout, and stderr bytes are
  sensitivity-bearing payload fields. Implementations must project only
  bounded enum/counter fields into logs, audit records, metrics, spans, health,
  and user-facing errors.
- Optional protobuf scalar/string fields are optional because absence changes
  behavior: `user`, `cwd`, `execId`, `knownStateGeneration`,
  `clientDeadlineMs`, and `retryAfterMs` must not be collapsed into default
  zero or empty-string sentinels.
- Any wire-breaking change belongs in a new schema version and matching ADR /
  reference documentation update.
