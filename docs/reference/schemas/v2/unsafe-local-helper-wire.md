# `unsafe-local-helper-wire.json` schema (`v2`)

Schema: [`unsafe-local-helper-wire.json`](./unsafe-local-helper-wire.json)

This schema captures private helper protocol version 2 between `d2bd` and the
same-UID unsafe-local user helper. It is an install-together contract with no
version-1 fallback. Peer credentials, not payload fields, establish execution
identity. The dedicated terminal stream remains terminal protocol version 1.

## Contract notes

- Control frames use bounded `AF_UNIX` `SOCK_SEQPACKET` messages.
- Terminal readiness transfers exactly one connected `AF_UNIX` `SOCK_STREAM`.
- Shell management requests and responses correlate both request and operation
  ids. Each request also carries the bounded default name and session limit
  resolved from the private workload policy; callers cannot supply that policy
  through the public protocol. List, detach, and kill results map directly to
  the public shell result DTOs.
- The connected terminal stream is bound to one attachment. Its frames use a
  four-byte little-endian JSON-body length prefix, contain no client-supplied
  session handle, and cover bounded stdin writes, output reads, resize, wait,
  stdin close, attachment close, and typed rejection.
- Terminal JSON frames are limited to 128 KiB, decoded chunks to 64 KiB,
  per-stream output rings to 8 MiB, and long polls to 1000 ms.
- Persistent-shell snapshots carry only a redacted shell name, state/attachment
  posture, and a bounded opaque supervisor id in addition to common scope
  identity.
- Socket and received descriptors use the frozen CLOEXEC requirements.
- Shell and terminal requests contain no uid, argv, environment, cwd, host
  path, transcript, PID, unit name, compositor data, or terminal session
  handle.
- The user helper implements this contract. Public `d2bd` shell routing and
  feature advertisement remain unavailable until the next integration slice.
