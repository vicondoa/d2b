# `d2b-provider-execution-policy`

This is the crate root for the `ExecutionPolicy` resource type. It owns the
type's identity and the driver declaration the v3 resource plane registers the
type by; the driver itself, its spec decoder, and its factory are the shared
declaration-only metadata driver of `d2b-resource-runtime`.

`ExecutionPolicy` states reusable confinement and nothing else: the namespace
classes an instance runs behind, the capability ceiling, privilege and root
restrictions, the identity rules, the selected syscall filter, and the umask.
It names no host path, no mount, no device node, and no resource other than
the identity and syscall-filter rows it selects, so it can never become a
second access grant beside the typed binding relationships that own
attachment.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | none - this crate declares no Provider identity on any surface |
| ResourceType | `ExecutionPolicy` |
| Package | `packages/d2b-provider-execution-policy/` |
| Driver declaration | `execution_policy_descriptor` -> `DriverDescriptor` |

There is no Provider identity to name here. The crate owns a ResourceType
vocabulary and nothing else: its `provider-identity.json` states a null on
all three identity surfaces with the reason `no-identity-owned`, because no
production source names a Provider for it. A role row's `Provider/<name>`
reference resolves against that same authority at generation, so a reference
to an identity no crate declares is refused there rather than published as a
dangling name.

## Config schema

The type declares no provider config schema: an `ExecutionPolicy` row carries
the JSON spec object the core rows store, and nothing outside it decodes at
validate. The spec's shape is the canonical `ExecutionPolicySpec` in
`d2b-contracts-resource`; this crate does not restate it.

## Exported resource types

`ExecutionPolicy` is not exportable. `ResourceExport` admits only qualified
`*.d2bus.org.*Service` types, so an `ExecutionPolicy` row is never an export
subject. The declaration carries `exportable: false`.

## Controllers / services / workers / binaries

The crate ships the type's declaration, not a standalone process. The shared
driver serves `ExecutionPolicy` rows through the `ResourceDriver` verbs:
`validate` decodes the stored spec envelope, `recover` adopts the converged
row, `reconcile` converges it as metadata, `finalize` drains owned children,
and `delete` retires the row.

The registration surface is `execution_policy_descriptor`. Alongside it the
crate exposes the two boundaries a committed row is read through:
`decode_policy_row`, which reads the row as the canonical closed contract and
refuses a row carrying a field this resource does not define, and
`admit_policy_execution`, which checks the selected reference against the
contract's own ResourceType constant and then runs the contract's one pure
evaluator.

## Placement and dependencies

`ExecutionPolicy` names no placement anchor, so an `ExecutionPolicy` row is
reconciled on its containing Zone's Host.

The crate depends only on `d2b-resource-types`, which carries the type's
declaration, on `d2b-contracts-resource`, which owns the policy contract and
the evaluator, and on `serde_json` for the row decode. Its registration test
adds `tokio`.

## RBAC requirements

The driver holds no broker authority of its own: it runs inside the daemon's
per-zone plane, and every row it reads or writes is reached through the
manager with the plane's own caller identity. Selecting a policy row is a
request to use it, never a grant: `admit_policy_execution` refuses a
selection the accepted graph did not authorize before it compares any
confinement facet.

## Security posture

The driver invents no confinement from spec text. A row that still carries a
mount, a device-node bind, or a host path is refused at decode rather than
decoded with that grant dropped, because a deployment that cannot honor a
declared restriction must be refused at admission. A backend that cannot
enforce one of the policy's own mandatory facets is refused instead of
launched without it, and the admitted result is fenced against the policy
fingerprint rather than a status generation.

## State and telemetry

The type publishes no durable status: the old plane's phase and
`observedGeneration` projections have no successor on the v3 surface, and the
driver keeps no in-memory status either. Failures travel as the registered
core failure kinds on the structured failure surface, which is what the
daemon logs and what tests assert.

## Build and test

```bash
cargo test -p d2b-provider-execution-policy
```

The `registration` suite proves the declaration registers the type through
the provider registry with its decoder and factory, that the registered name
is the contract's canonical ResourceType, and that the decode boundary reads
the canonical spec and refuses a row carrying retired access authority.
