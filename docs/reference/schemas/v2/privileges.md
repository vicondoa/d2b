# `privileges.json` schema (`v2`)

Schema: [`privileges.json`](./privileges.json)

`privileges.json` is the authorization matrix for both the public CLI/API
surface and the private broker operations.

## Top-level fields

- `schemaVersion` — schema directory/version for this artifact.
- `publicOperations` — public command rows (`list`, `status`, `host check`,
  `vm start --dry-run`, and so on).
- `brokerOperations` — private broker rows (`ValidateBundle`,
  `RunActivation`, `SpawnRunner`, `DelegateCgroupV2`, and friends).

## Per-operation fields

Each operation row carries:

- `operation` — stable enum/command name.
- `subject` / `scope` — who and what the row targets.
- `allowedGroups` — allowlist groups.
- `brokerRequired` — whether the broker is required, conditional, or absent.
- `destructive` — whether mutation/teardown is possible.
- `secretAccess` — whether secrets are touched.
- `audit` — retained fields + success/deny/error audit requirements.
- `defaultForUnknown` — locked to `deny-and-audit`.

## Contract notes

- Unknown future operations must deny and audit by default.
- The Layer-1 privilege-oracle skeleton uses the committed
  `brokerOperations[].operation` list as its operation inventory.
