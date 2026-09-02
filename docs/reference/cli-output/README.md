# CLI JSON output schemas

**Diataxis category:** reference.

The JSON schemas in this directory are generated from current Rust DTOs by:

```bash
bazel run //packages/xtask:xtask -- gen-cli-schemas
```

The drift target is `//packages/xtask:gen_cli_schemas_drift`.

## Stable surfaces

| Surface | Schema | Prose |
| --- | --- | --- |
| `list` | `list.schema.json` | [list.md](./list.md) |
| `status` | `status.schema.json` | [status.md](./status.md) |
| `usb probe` | `usb-probe.schema.json` | [usb-probe.md](./usb-probe.md) |
| `audit` | `audit.schema.json` | [audit.md](./audit.md) |
| `host check` | `host-check.schema.json` | [host-check.md](./host-check.md) |
| `host doctor` | `host-doctor.schema.json` | [host-doctor.md](./host-doctor.md) |
| `auth status` | `auth-status.schema.json` | [auth-status.md](./auth-status.md) |
| `op inspect` | `op-inspect.schema.json` | [constellation-observability.md](../constellation-observability.md) |
| `store verify` | `store-verify.schema.json` | [store-verify.md](./store-verify.md) |

All lifecycle responses identify the Zone and Resource involved through
typed, redacted fields. They do not expose host paths, credentials, argv,
pidfds, cgroup paths, namespace identifiers, or private runtime locators.

Older Realm and VM-first schemas are not current CLI surfaces and are not
regenerated. Historical compatibility material belongs in ADRs and migration
notes, not in this index.
