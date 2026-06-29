# Clipboard picker protocol

**Diataxis category:** reference.

The picker protocol is a small, versioned, newline-delimited JSON protocol over
an anonymous AF_UNIX `socketpair()` inherited by the picker process. It is
public so the separate picker repository can implement it without depending on
d2b internal Rust crates.

## Transport

- `d2b-clipd` creates one socketpair per picker request and supervises the
  picker process.
- The inherited FD number is passed through a non-secret argument such as
  `--ipc-fd=3`.
- No launch token is needed on the socketpair path.
- If a future side-channel nonce is added, it must be at least 256 bits from a
  CSPRNG and must not be passed through argv or environment.
- Transfer FDs are never sent to the picker.
- The picker is not given `NIRI_SOCKET`; placement and destination labels arrive
  in `OpenRequest`.

All frames are UTF-8 JSON objects terminated by `\n`. Decoders are bounded.
`d2b.site.clipboard.protocol.pickerToClipdMaxFrameBytes` caps picker-to-daemon
frames, while `clipdToPickerMaxFrameBytes` caps larger daemon-to-picker
`OpenRequest` frames. Unknown message kinds are rejected. Unknown fields in
stable v1 daemon-received messages are rejected.

## Version 1 messages

### `ClientHello` picker → clipd

```json
{"type":"client_hello","protocol_version_range":{"min":1,"max":1},"picker_version":"0.1.0"}
```

### `OpenRequest` clipd → picker

Contains the selected protocol version, `clipd_version`, request id,
destination metadata, requested MIME type, expiry, and filtered candidates.
Clipboard payload bytes are not included.

### `Select` picker → clipd

```json
{"type":"select","selected_protocol_version":1,"request_id":"opaque","entry_id":"opaque"}
```

### `Cancel` picker → clipd

```json
{"type":"cancel","selected_protocol_version":1,"request_id":"opaque"}
```

### `Error` / `Close` clipd → picker

Carries a bounded reason code and optional request id. It must not include raw
clipboard contents.

## Candidate metadata

Candidates may include:

- `entry_id`
- `source_realm`
- `source_realm_kind` (`host`, `vm`)
- `source_app`
- `source_app_id`
- `source_attribution` (`exact_client`, `focused_window_guess`,
  `cache_stale_focused_window_guess`, `broker_injected_debug`)
- bounded, redacted `preview_text`
- closed-allowlist `content_type`
- `timestamp_unix_ms`
- optional capped PNG thumbnail metadata
- `confirmation_required`

Text and HTML previews are rendered as plain text only. ANSI escapes,
non-printable controls, rich HTML, Pango markup, and remote resources are not
allowed. Guest-origin image thumbnails are not sent in v1; guest image entries
use safe metadata such as byte count.

## Schema

The v1 JSON schema is committed at
[`schemas/clipboard-picker-protocol-v1.json`](./schemas/clipboard-picker-protocol-v1.json).
