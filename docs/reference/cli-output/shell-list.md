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
| `vm` | Current schema field for the routed target. Local targets report the resolved VM name; gateway-backed management commands forward the target through the selected gateway, whose response keeps this field name until a future output-version bump can rename it to `target`. |
| `default_name` | Configured default shell session name for the target workload. Present even when `sessions` is empty. |
| `sessions[]` | Bounded session rows reported by guestd. |
| `sessions[].name` | Validated shell session name. |
| `sessions[].state` | One of `attached`, `detached`, `killed`, `pool-unavailable`, `feature-disabled`, or `output-gap`. |
| `sessions[].attached` | Whether a client is currently attached. |
| `sessions[].is_default` | Whether the row is the configured default session. |

Shell names are operational identifiers and may appear in CLI output. They are
not metric labels or raw daemon audit fields.
