# `nixling shell <target> kill --name <name> --json`

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
| `vm` | Current local routed VM name. Local-shell-only generations resolve only declared local VM targets and reject gateway/remote/provider targets before daemon dispatch; ADR 0039 defines the target-routing contract. |
| `name` | Explicit shell session name supplied with `--name`. Kill never defaults to `default`. |
| `result` | `killed` when the session was terminated; otherwise `already-absent`. |
| `state` | Terminal shell state reported by the daemon, normally `killed` for this command. |

Kill is destructive, so `--name` is required.
