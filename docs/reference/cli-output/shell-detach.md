# `nixling shell <target> detach --json`

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
| `vm` | Current local routed VM name. Local-shell-only generations resolve only declared local VM targets and reject gateway/remote/provider targets before daemon dispatch; ADR 0039 reserves future target routing. |
| `name` | Resolved shell session name. When `--name` is omitted, this is the configured default. |
| `result` | `detached` when a live client was detached; otherwise `already-detached-or-absent`. |
| `cause` | Optional close cause reported by the daemon/guest path, such as `client-detach` or `evicted-by-admin-detach`. Null when absent. |

Detach is non-destructive and retry-safe.
