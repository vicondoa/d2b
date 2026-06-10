# `guest-control.json` schema (`v2` companion)

Schema: [`guest-control.json`](./guest-control.json)

`guest-control.json` is the typed guest-control contract for ADR 0026's
ttRPC/protobuf surface. It snapshots the guest daemon control messages, health
states, exec lifecycle messages, and chunked stdio RPC shapes that host
`nixlingd` and guest `nixling-guestd` must keep aligned. The protobuf service
source lives at
[`packages/nixling-ipc/proto/guest_control.proto`](../../../../packages/nixling-ipc/proto/guest_control.proto).

## Top-level sections

- `schemaVersion` / `protocolVersion` - guest-control schema and wire version.
- `transport` - virtio-vsock ttRPC transport, including the Cloud Hypervisor
  `CONNECT <port>` handshake and opaque `OK <local-port>` acknowledgement.
- `limits` - bounded frame, chunk, live-buffer, detached-log, and concurrency
  limits exposed through `Hello` / `Capabilities`.
- `hello`, `health`, and `capabilities` - readiness and version negotiation.
- `exec*`, `writeStdin`, `readOutput`, `closeStdin`, `ttyWinResize`, and
  signal/cancel messages - Docker-like exec lifecycle and chunked stdio.

## Contract notes

- The schema describes the protocol contract; it is not a commitment to use
  JSON as the runtime transport.
- Any wire-breaking change belongs in a new schema version and matching ADR /
  reference documentation update.
