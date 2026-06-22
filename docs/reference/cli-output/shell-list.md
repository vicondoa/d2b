# `nixling shell <target> list --json`

> Diataxis: reference. JSON output contract for persistent shell listing.

Schema: [`shell-list.schema.json`](./shell-list.schema.json).

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
| `vm` | Current local routed VM name. Local-shell-only generations resolve only declared local VM targets and reject gateway/remote/provider targets before daemon dispatch; ADR 0039 defines the target-routing contract. |
| `default_name` | Configured default shell session name for the target workload. Present even when `sessions` is empty. |
| `sessions[]` | Bounded session rows reported by guestd. |
| `sessions[].name` | Validated shell session name. |
| `sessions[].state` | One of `attached`, `detached`, `killed`, `pool-unavailable`, `feature-disabled`, or `output-gap`. |
| `sessions[].attached` | Whether a client is currently attached. |
| `sessions[].is_default` | Whether the row is the configured default session. |

Shell names are operational identifiers and may appear in CLI output. They are
not metric labels or raw daemon audit fields.
