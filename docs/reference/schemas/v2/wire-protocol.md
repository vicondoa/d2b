# `wire-protocol.json` schema (`v2` companion)

Schema: [`wire-protocol.json`](./wire-protocol.json)

`wire-protocol.json` is the typed daemon/broker/public-socket wire contract.
It snapshots the request/response envelopes and handshake shapes the Rust
control plane accepts.

## Top-level sections

- `schemaVersion` — schema directory/version for this artifact.
- `framing` — length-prefixed AF_UNIX seqpacket frame rules.
- `hello`, `helloOk`, `helloRejected` — version negotiation handshake.
- `publicSocket` / `publicRequest` / `publicResponse` — CLI ↔ daemon wire.
- `brokerSocket` / `brokerRequest` / `brokerResponse` — daemon ↔ broker wire.

## Contract notes

- `hello`/`helloOk` are intentionally split so version-range negotiation is
  explicit and testable.
- qemu-media media fds are broker-internal: the daemon names only VM/slot/ref
  identifiers, while physical USB opens resolve the root-only enrollment
  registry and image-file opens resolve trusted bundle paths inside broker
  spawn/QMP handling.
- `UsbipProbeStatus` may report `direct-config` for qemu-media image-file
  slots; those rows intentionally have no enrollment follow-up command.
- Any wire-breaking change belongs in a new schema version and matching docs.
