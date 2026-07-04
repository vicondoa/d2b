# USB security-key notification and event JSON

> Reference for the machine-readable events emitted by the d2b USB
> security-key proxy through the d2b notification subsystem.
> Event files are written to `/run/d2b/usb-sk/events.jsonl` (one JSON object
> per line) and consumed by `d2b usb security-key sessions`, the Waybar
> helper, and `d2b-wlcontrol`.

## Event envelope

Every security-key event is a JSON object with a common envelope:

```json
{
  "app":      "d2b.usb.security-key",
  "severity": "info",
  "ts":       "2025-11-01T10:23:44.123456Z",
  "session":  "sk_7f3a2b91c04e",
  "vm":       "personal-dev",
  "realm":    "personal",
  "body":     { ... }
}
```

### Envelope fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `app` | `string` | yes | Always `"d2b.usb.security-key"` for this subsystem. |
| `severity` | `"info" \| "warning" \| "critical"` | yes | `"info"` for normal lifecycle events; `"warning"` for contention/queue events; `"critical"` for broker errors and unexpected failures. |
| `ts` | RFC 3339 string | yes | UTC timestamp of the event. |
| `session` | `string \| null` | yes | Opaque session identifier for this CTAP ceremony request. `null` for broker-level events not tied to a specific session. |
| `vm` | `string \| null` | yes | d2b VM name that initiated the request. `null` for host-level events. |
| `realm` | `string \| null` | yes | d2b env/realm the VM belongs to. `null` when `vm` is `null`. |
| `body` | `object` | yes | Event-type-specific payload; see [Event types](#event-types) below. |

## Event types

The `body.kind` field identifies the event type.

### `ceremony_started`

Emitted when a VM's guest frontend initiates a new CTAP ceremony and the
host broker grants the lease.

```json
{
  "kind":    "ceremony_started",
  "key_id":  "FIDO:1050:0407:XXXXXXXXXXXX",
  "rp_id":   "github.com"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `key_id` | `string` | Stable key selector string identifying the physical device. |
| `rp_id` | `string \| null` | Relying-party ID parsed from the CTAP request, if available and `notifications.showRpId = true`; `null` otherwise. |

Desktop notification summary: `personal-dev is using security key`.

### `user_presence_wait`

Emitted when the broker is waiting for user presence (physical touch) on the
key.

```json
{
  "kind":       "user_presence_wait",
  "key_id":     "FIDO:1050:0407:XXXXXXXXXXXX",
  "rp_id":      "github.com",
  "timeout_at": "2025-11-01T10:25:44.123456Z"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `key_id` | `string` | Stable key selector. |
| `rp_id` | `string \| null` | Relying-party ID, if available. |
| `timeout_at` | RFC 3339 string | UTC time when the broker will cancel the ceremony if no touch is received. |

Desktop notification summary: `Touch security key for personal-dev`.

### `ceremony_completed`

Emitted when a CTAP ceremony completes successfully.

```json
{
  "kind":        "ceremony_completed",
  "key_id":      "FIDO:1050:0407:XXXXXXXXXXXX",
  "duration_ms": 3147
}
```

| Field | Type | Description |
|-------|------|-------------|
| `key_id` | `string` | Stable key selector. |
| `duration_ms` | `integer` | Ceremony duration from lease acquisition to completion in milliseconds. |

No desktop notification emitted for successful completions (reduces noise for
normal WebAuthn use).

### `ceremony_failed`

Emitted when a CTAP ceremony fails — timeout, broker error, guest disconnect,
or `CTAPHID_ERROR` from the device.

```json
{
  "kind":        "ceremony_failed",
  "key_id":      "FIDO:1050:0407:XXXXXXXXXXXX",
  "reason":      "timeout",
  "duration_ms": 120003
}
```

| Field | Type | Description |
|-------|------|-------------|
| `key_id` | `string` | Stable key selector. |
| `reason` | `"timeout" \| "guest_disconnect" \| "ctap_error" \| "broker_error" \| "cancelled"` | Failure cause. |
| `duration_ms` | `integer` | Time from lease acquisition to failure in milliseconds. |

Desktop notification summary: `Security key request failed (personal-dev): timeout`.

### `queue_wait_started`

Emitted when a VM's request is queued because another VM holds the active
lease.

```json
{
  "kind":          "queue_wait_started",
  "key_id":        "FIDO:1050:0407:XXXXXXXXXXXX",
  "active_vm":     "personal-dev",
  "queue_timeout": "2025-11-01T10:23:59.123456Z"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `key_id` | `string` | Stable key selector. |
| `active_vm` | `string` | VM that currently holds the lease. |
| `queue_timeout` | RFC 3339 string | UTC time when the queued request will be cancelled if the active ceremony is still in progress. |

Desktop notification summary: `Security key busy: personal-dev is authenticating`.

Desktop notification actions (when supported by the notification daemon):

| Action ID | Label | Effect |
|-----------|-------|--------|
| `cancel_active` | `Cancel active request` | Sends `d2b usb security-key cancel <active-session>` with a single-use nonce bound to the session/action/expiry. |
| `open_status` | `Open status` | Opens `d2b-wlcontrol` at the USB security-key panel. |

### `queue_wait_expired`

Emitted when a queued VM's request times out while the active ceremony is
still in progress.

```json
{
  "kind":      "queue_wait_expired",
  "key_id":    "FIDO:1050:0407:XXXXXXXXXXXX",
  "active_vm": "personal-dev"
}
```

Desktop notification summary: `Security key request timed out (work-aad)`.

### `lease_revoked`

Emitted when the broker forcibly revokes a lease — for example, when a VM
stops or the guest frontend disconnects mid-ceremony.

```json
{
  "kind":   "lease_revoked",
  "key_id": "FIDO:1050:0407:XXXXXXXXXXXX",
  "reason": "guest_disconnect"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `key_id` | `string` | Stable key selector. |
| `reason` | `"vm_stop" \| "guest_disconnect" \| "daemon_restart" \| "explicit_cancel"` | Revocation reason. |

## Notification action nonce semantics

Notification actions that trigger privileged operations (such as
`cancel_active`) include a single-use, high-entropy nonce in the action
callback payload. The nonce is bound to:

- The session ID of the target operation.
- The action type (e.g., `cancel_active`).
- An expiry timestamp (defaults to 60 seconds after the notification is sent).

`d2bd` rejects action callbacks with:
- A missing nonce.
- An expired nonce (past the expiry timestamp).
- A previously consumed nonce (single-use).
- A nonce bound to a different session ID or action type.

This prevents other desktop clients from spoofing privileged security-key
actions by replaying or crafting notification callbacks.

## Lease state file

The current lease state is also available as a machine-readable JSON file at
`/run/d2b/usb-sk/lease.json`. `d2b usb security-key status` reads this file
directly when `d2bd` is not reachable, enabling offline inspection.

```json
{
  "active": {
    "session":    "sk_7f3a2b91c04e",
    "vm":         "personal-dev",
    "key_id":     "FIDO:1050:0407:XXXXXXXXXXXX",
    "started_at": "2025-11-01T10:23:44.123456Z",
    "rp_id":      "github.com"
  },
  "queued": [
    {
      "session":    "sk_9a1d4f02e77b",
      "vm":         "work-aad",
      "queued_at":  "2025-11-01T10:23:51.456789Z",
      "timeout_at": "2025-11-01T10:24:06.456789Z"
    }
  ]
}
```

`active` is `null` when no ceremony is in progress. `queued` is an empty array
when no VMs are waiting.
