# `nixling auth status` output

Schema: [`auth-status.schema.json`](./auth-status.schema.json)

`nixling auth status --json` reports how the daemon maps the current
caller: role, socket reachability/version, the allowed subcommand set,
and explicit denial hints.

## Fields

| Field | Type | Meaning | Stability |
| --- | --- | --- | --- |
| `role` | string enum | One of `none`, `launcher`, or `admin`. | Stable wire contract. |
| `publicSocket.path` | string | Public socket path, normally `/run/nixling/public.sock`. | Stable wire contract. |
| `publicSocket.reachable` | boolean | Whether the daemon answered the reachability probe. | Stable wire contract. |
| `publicSocket.serverVersion` | string | Daemon-reported implementation version. | Stable wire contract. |
| `publicSocket.selectedVersion` | string | Protocol version selected by handshake negotiation. | Stable wire contract. |
| `allowedCommands[]` | array of string | Commands the caller may invoke. | Stable wire contract; array sorted lexicographically. |
| `deniedCommands[].command` | string | Denied command name. | Stable wire contract. |
| `deniedCommands[].reason` | string | Short denial hint suitable for operator UX. | Stable field; wording may refine between minors. |

## Ordering and null handling

- `allowedCommands[]` and `deniedCommands[]` are emitted in command-name
  order for stable diffs.
- `deniedCommands` is always present and may be an empty array.
- No documented fields are omitted in W2.

## Stability promise

The field names above and the `role` enum are frozen for W2. The reason
strings are operator-facing text and may be clarified, but they should
not silently change the underlying authorization decision.

## Human example

```text
$ nixling auth status
role: launcher
public socket: /run/nixling/public.sock (reachable, server=0.2.0-w2, selected=0.2.0)
allowed commands: auth status, host check, list, status
denied commands:
- audit: requires admin role in nixling.site.adminUsers
```

## JSON example

```json
{
  "role": "launcher",
  "publicSocket": {
    "path": "/run/nixling/public.sock",
    "reachable": true,
    "serverVersion": "0.2.0-w2",
    "selectedVersion": "0.2.0"
  },
  "allowedCommands": [
    "auth status",
    "host check",
    "list",
    "status"
  ],
  "deniedCommands": [
    {
      "command": "audit",
      "reason": "requires admin role in nixling.site.adminUsers"
    }
  ]
}
```
