# `d2b shell <target> kill --name <name> --json`

> Diataxis: reference. JSON output contract for persistent shell kill.

Schema: [`shell-kill.schema.json`](./shell-kill.schema.json).

## Shape

```json
{
  "command": "shell kill",
  "vm": "work",
  "name": "build",
  "result": "killed",
  "state": "killed"
}
```

## Fields

| Field | Meaning |
| --- | --- |
| `command` | Stable command discriminator, always `shell kill`. |
| `vm` | Current schema field for the routed target. Local targets report the resolved VM name; gateway-backed management commands forward the target through the selected gateway, whose response keeps this field name until a future output-version bump can rename it to `target`. |
| `name` | Explicit shell session name supplied with `--name`. Kill never defaults to `default`. |
| `result` | `killed` when the session was terminated; otherwise `already-absent`. |
| `state` | Terminal shell state reported by the daemon, normally `killed` for this command. |

Kill is destructive, so `--name` is required.
