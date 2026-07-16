# `d2b shell <target> kill --name <name> --json`

> Diataxis: reference. JSON output contract for persistent shell kill.

Schema: [`shell-kill.schema.json`](./shell-kill.schema.json).

> Gateway-backed management forms remain historical parser compatibility
> behavior. They are unsupported and do not define a routing or output
> contract.

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
| `vm` | Current schema field for the local target. Local VM targets report the resolved VM name; unsafe-local targets report their configured canonical target. |
| `name` | Explicit shell session name supplied with `--name`. Kill never defaults to `default`. |
| `result` | `killed` when the session was terminated; otherwise `already-absent`. |
| `state` | Terminal shell state reported by the daemon, normally `killed` for this command. |

Kill is destructive, so `--name` is required.
