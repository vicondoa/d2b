# `d2b shell <target> detach --json`

> Diataxis: reference. JSON output contract for persistent shell detach.

Schema: [`shell-detach.schema.json`](./shell-detach.schema.json).

> Gateway-backed management forms remain historical parser compatibility
> behavior. They are unsupported and do not define a routing or output
> contract.

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
| `vm` | Current schema field for the local target. Local VM targets report the resolved VM name; unsafe-local targets report their configured canonical target. |
| `name` | Resolved shell session name. When `--name` is omitted, this is the configured default. |
| `result` | `detached` when a live client was detached; otherwise `already-detached-or-absent`. |
| `cause` | Optional close cause reported by the daemon/guest path, such as `client-detach` or `evicted-by-admin-detach`. Null when absent. |

Detach is non-destructive and retry-safe.
