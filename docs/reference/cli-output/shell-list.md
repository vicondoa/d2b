# `d2b shell <target> list --json`

> Diataxis: reference. JSON output contract for persistent shell listing.

Schema: [`shell-list.schema.json`](./shell-list.schema.json).

> Gateway-backed management forms remain historical parser compatibility
> behavior. They are unsupported and do not define a routing or output
> contract.

## Shape

```json
{
  "command": "shell list",
  "vm": "work",
  "default_name": "default",
  "sessions": [
    {
      "name": "default",
      "state": "detached",
      "attached": false,
      "is_default": true
    }
  ]
}
```

## Fields

| Field | Meaning |
| --- | --- |
| `command` | Stable command discriminator, always `shell list`. |
| `vm` | Current schema field for the local target. Local VM targets report the resolved VM name; unsafe-local targets report their configured canonical target. |
| `default_name` | Configured default shell session name for the target workload. Present even when `sessions` is empty. |
| `sessions[]` | Bounded session rows reported by guestd. |
| `sessions[].name` | Validated shell session name. |
| `sessions[].state` | One of `attached`, `detached`, `killed`, `pool-unavailable`, `feature-disabled`, or `output-gap`. |
| `sessions[].attached` | Whether a client is currently attached. |
| `sessions[].is_default` | Whether the row is the configured default session. |

Shell names are operational identifiers and may appear in CLI output. They are
not metric labels or raw daemon audit fields.
