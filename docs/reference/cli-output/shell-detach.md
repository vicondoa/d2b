# `d2b shell <target> detach --json`

> Diataxis: reference. JSON output contract for persistent shell detach.

Schema: [`shell-detach.schema.json`](./shell-detach.schema.json).

## Shape

```json
{
  "command": "shell detach",
  "vm": "work",
  "name": "default",
  "result": "already-detached-or-absent",
  "cause": null
}
```

## Fields

| Field | Meaning |
| --- | --- |
| `command` | Stable command discriminator, always `shell detach`. |
| `vm` | Current schema field for the routed target. Local targets report the resolved VM name; gateway-backed management commands forward the target through the selected gateway, whose response keeps this field name until a future output-version bump can rename it to `target`. |
| `name` | Resolved shell session name. When `--name` is omitted, this is the configured default. |
| `result` | `detached` when a live client was detached; otherwise `already-detached-or-absent`. |
| `cause` | Optional close cause reported by the daemon/guest path, such as `client-detach` or `evicted-by-admin-detach`. Null when absent. |

Detach is non-destructive and retry-safe.
