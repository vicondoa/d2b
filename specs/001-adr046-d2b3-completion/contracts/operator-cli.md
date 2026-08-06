# Contract: Operator CLI

**Owning spec**: `ADR-046-cli-and-operations` | **Wave**: W5 (surface), W7 (cutover verbs)

## What this surface is

The `d2b` binary is the only operator surface. There is no bash fallback and no env-knob
escape hatch. Companion tools consume it, so its shape is a public contract, not an
implementation detail.

## The clean break

`ADR-046-cli-and-operations` defines a "v2 command surface removed at 3.0 clean break". This
is the single change with the widest blast radius outside the host itself: every desktop
companion reads this surface or the socket beside it.

## Obligations

| # | Obligation | Requirement | Wave |
| --- | --- | --- | --- |
| CLI-1 | Resource inspection: list and inspect resources, exact owning Provider, status, and the reason for any degraded or failed condition; committed-pending-audit status lands only with the coordinated accepted-spec Version 2 amendment and follows the exact commands, flags, exits, mandatory envelope fields, closed remediation actions, and human/JSON forms below; Zone readiness renders the actual handler-list names from T605 rather than a map-shaped alias | FR-016, FR-069, FR-070, SC-005, SC-032, SC-033 | W5 |
| CLI-2 | Every failure names a specific cause and an actionable next step | FR-017, SC-004 | W5 |
| CLI-3 | Cutover verbs: a non-mutating preview, and an apply gated on explicit intent plus exact content-bound consent | FR-020, FR-021 | W7 |
| CLI-4 | The apply path refuses to pass the rollback boundary without a recorded recovery-point attestation | FR-043, SC-025 | W7 |
| CLI-5 | Retired verbs are removed with a removal proof, in their own commit, after the successor is integrated | FR-023 | W5-W7 |
| CLI-6 | `d2b userd` is removed only after parity with the fixed user supervisor Process | FR-041 | W5 |
| CLI-7 | Desktop-wrapper, companion, audio, USB, security-key, and resource reference pages match exact emitted help, JSON, capabilities, typed refusals, and wire fields; absent behavior is not promised | FR-019, FR-074 | W5 |

## Committed-pending-audit recovery

This recovery surface requires T599's coordinated amendment of the accepted
`ADR-046-cli-and-operations` specification from Version 1 to Version 2. Version 2 assigns the
resource-recovery meanings of exits 75 and 76, makes `zoneRef` and `schemaVersion: 2`
mandatory in every recovery JSON envelope, and pins the ID and remediation contracts below.
The existing meanings of 75 and 76 for unrelated exec commands remain command-scoped. T599
owns migration guidance, DTO/schema and contract tests, reference and release treatment;
T220 reconciles the generated manifests and folds the changelog fragment. The implementation
MUST NOT ship this surface under the accepted Version 1 contract.

The only operator-supplied replay handle is an exact 16-byte operation ID rendered as
lowercase 32-hex and emitted by the original mutation. Every generic and typed `create`,
`update-spec`, and `delete` command accepts `--operation-id <OPAQUE_ID>`. Omitting it on the
first attempt generates a new ID. Supplying it uses that value for both the operation identity
and its idempotency binding; there is no separate public idempotency flag.

The exact generic retry forms are:

```text
d2b --zone <ZONE> create <RESOURCE_TYPE> --operation-id <OPAQUE_ID> (--spec-file <PATH> | --spec-stdin) [--wait-for-reconcile] [--reconcile-deadline <DURATION>] [--human | --json]
d2b --zone <ZONE> update-spec <RESOURCE_REF> --revision <REVISION> --operation-id <OPAQUE_ID> (--spec-file <PATH> | --spec-stdin) [--wait-for-reconcile] [--reconcile-deadline <DURATION>] [--human | --json]
d2b --zone <ZONE> delete <RESOURCE_REF> --revision <REVISION> --operation-id <OPAQUE_ID> [--wait-for-reconcile] [--reconcile-deadline <DURATION>] [--human | --json]
```

Typed noun commands use the same leaf verb and `--operation-id` flag. A retry MUST repeat the
same Zone, verb, target, expected revision, canonical request body, and other semantic inputs.
Changing any binding while reusing the ID exits `76` with a typed
`operation-replay-mismatch` refusal and does not reveal or resume the original operation.

The sole status command remains the accepted command:

```text
d2b op inspect --operation-id <OPAQUE_ID> [--zone <ZONE>] [--watch] [--human | --json]
```

`--watch` waits only for export completion or the request deadline; it never reapplies the
mutation. It traverses the typed store, ResourceService, method-catalogue/router, daemon/client,
and CLI path owned by T589, T592, T593, T595, and T599; an in-memory or CLI-synthesized status
map is forbidden. The Version 2 exit contract for mutation and inspection is closed:

| Exit | Mutation | `op inspect` |
| --- | --- | --- |
| `0` | Ordinary or stored final success | Stored final result returned |
| `75` | Mutation committed; authoritative audit export remains pending | Operation remains committed-pending-audit |
| `76` | Operation ID exists but replay binding differs | Not emitted |
| `2` | Invalid ID or invocation | Invalid, unknown, or replay-binding-denied ID, rendered identically |
| `1` | Other typed authorization, transport, or Resource API failure | Other typed authorization, transport, or Resource API failure |

Human pending mutation output is exactly three newline-terminated lines, with no payload,
sink detail, executable command, argv, Zone value in remediation, or operation ID embedded in
remediation:

```text
committed; audit export pending
operation: <OPAQUE_ID>
remediation: inspect-operation
```

Pending inspection uses the same first two lines and
`remediation: wait-for-audit-export`. Both a pending mutation and pending inspection emit this
JSON field shape; only `command` and the closed remediation action differ. `zoneRef` and
`operationId` are bounded status fields, never executable text. `resourceStatus` is the exact
bounded CLI recovery projection shown here; the complete canonical `ResourceStatus` remains
in the protobuf response, and no other status fields are added to this CLI envelope:

```json
{
  "ok": false,
  "zoneRef": "Zone/<ZONE>",
  "schemaVersion": 2,
  "command": "update-spec",
  "state": "committed-pending-audit",
  "committed": true,
  "operationId": "<OPAQUE_ID>",
  "resourceStatus": {
    "phase": "Degraded",
    "outcome": {"code": "committed-pending-audit"},
    "update": {"state": "Blocked", "operationId": "<OPAQUE_ID>"}
  },
  "retry": {"sameOperationIdRequired": true},
  "remediation": {"action": "inspect-operation"}
}
```

For inspection, `command` is `op inspect` and the pending action is
`wait-for-audit-export`. A final inspection exits `0`; human mode renders the stored original
final result, and JSON uses
`{"ok":true,"zoneRef":"Zone/<ZONE>","schemaVersion":2,"command":"op inspect","state":"final","operationId":"<OPAQUE_ID>","result":...}`.
Pending output never says success, rollback, or safe to use a new ID.

A mutation replay-binding mismatch exits `76` and uses this exact human form:

```text
operation replay refused
operation: <OPAQUE_ID>
remediation: retry-identical-operation
```

Its JSON form is
`{"ok":false,"zoneRef":"Zone/<ZONE>","schemaVersion":2,"kind":"operation-replay-mismatch","operationId":"<OPAQUE_ID>","remediation":{"action":"retry-identical-operation"}}`.
The `retry-identical-operation` action means retry the same semantic mutation; a caller that
cannot do so starts a new mutation without reusing the ID. No rendered command is part of the
machine or human contract.
An inspection under the wrong subject/Zone binding is deliberately indistinguishable from an
unknown ID: both exit `2`, human mode prints `operation not found` followed by
`remediation: verify-operation-context`, and JSON is exactly
`{"ok":false,"zoneRef":"Zone/<ZONE>","schemaVersion":2,"kind":"operation-not-found","message":"operation not found","remediation":{"action":"verify-operation-context"}}`.

The closed remediation-action set is `inspect-operation`, `wait-for-audit-export`,
`retry-identical-operation`, `start-new-operation`, and `verify-operation-context`. No action
object accepts arguments, argv, shell text, Zone, or operation ID. IDs appear only in bounded
`operationId` and `resourceStatus.update.operationId` fields; Zone appears only in bounded
`zoneRef`.

Version 1 consumers must require `schemaVersion` and upgrade before using recovery. A missing
version or `schemaVersion: 1` retains the old 0/1/2 behavior and MUST NOT be interpreted as
Version 2. Arbitrary Version 1 operation IDs are not converted to the 16-byte Version 2 form.
The d2b 3.0 clean cutover imports no persisted Version 1 recovery state.

## Retirement register

Populate as each is retired. `d2b host migrate-storage` is already classified: it served the
one-time v1-to-v2 storage layout cutover and has **no v3 successor**, so it belongs on the
FR-042 explicit retirement list rather than the parity list.

| Verb | Successor | Parity or retirement |
| --- | --- | --- |
| `d2b host migrate-storage` | none | Retirement - justified, must appear in release notes |
| `d2b userd *` | fixed user supervisor Process under `Provider/system-systemd` | Parity - gated |

## Acceptance

- Both machine-readable and human output modes behave per the CLI contract, with exit codes
  matching the documented table.
- Every command and field promised by the desktop-wrapper and companion/device references is
  present in exact emitted behavior. A typed unavailable state is acceptable only when the
  frozen contract already defines it or the same change follows the explicit parity/FR-042
  retirement path with replacement, migration, owner, release treatment, and contract
  coverage. Candidate absence alone is a W5 defect and never authorizes deleting the promise.
- A committed mutation whose authoritative audit is pending is displayed as degraded
  `committed-pending-audit` with exactly the Version 2 command, flags, exits, mandatory
  `zoneRef`/`schemaVersion`, DTO schema, ID format, closed remediation actions, and human/JSON
  forms above. A mutation replay mismatch exits `76`; inspection under the wrong subject/Zone
  binding exits `2` with the same form as an unknown ID. Neither exposes the original
  operation. Human and machine output never call it success, rollback, or safe to repeat with
  a new ID, expose no mutation payload or raw sink error, and contain no Zone/ID-bearing argv,
  command vector, shell fragment, or free-form remediation.
- T599 bumps the accepted CLI specification to Version 2 and owns migration guidance,
  DTO/schema, contract tests, references, and a changelog fragment. T220 verifies and folds the
  coordinated version/reference/test/schema/release treatment. Missing or Version 1 envelopes
  are never interpreted as Version 2, and arbitrary Version 1 IDs are never silently migrated.
- Zone readiness names `Provider/system-core` and the actual failing
  `Zone.status.handlers[]` record: `system-core-host` or `system-core-user`, with its `phase`
  and `lastReconciledAt`. Exactly one of each is required; duplicate, missing, wrong-name, or
  `provider-lifecycle` substitution is reported as an actionable refusal rather than a vague
  Provider path or boolean failure. T599 must match T605's paired contract/reference evidence.
- The cutover preview modifies nothing, and the apply path is unreachable without both consent
  and attestation.
- No retired verb remains, verified by its removal proof.
