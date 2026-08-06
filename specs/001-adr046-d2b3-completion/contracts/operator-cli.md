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
| CLI-1 | Resource inspection: list and inspect resources, exact owning Provider, status, and the reason for any degraded or failed condition; committed-pending-audit status follows the exact commands, flags, exits, and human/JSON forms below and exposes only the safe operation ID; Zone readiness renders the actual handler-list names from T605 rather than a map-shaped alias | FR-016, FR-069, FR-070, SC-005, SC-032, SC-033 | W5 |
| CLI-2 | Every failure names a specific cause and an actionable next step | FR-017, SC-004 | W5 |
| CLI-3 | Cutover verbs: a non-mutating preview, and an apply gated on explicit intent plus exact content-bound consent | FR-020, FR-021 | W7 |
| CLI-4 | The apply path refuses to pass the rollback boundary without a recorded recovery-point attestation | FR-043, SC-025 | W7 |
| CLI-5 | Retired verbs are removed with a removal proof, in their own commit, after the successor is integrated | FR-023 | W5-W7 |
| CLI-6 | `d2b userd` is removed only after parity with the fixed user supervisor Process | FR-041 | W5 |
| CLI-7 | Desktop-wrapper, companion, audio, USB, security-key, and resource reference pages match exact emitted help, JSON, capabilities, typed refusals, and wire fields; absent behavior is not promised | FR-019, FR-074 | W5 |

## Committed-pending-audit recovery

The only operator-supplied replay handle is the opaque lowercase 32-hex operation ID emitted
by the original mutation. Every generic and typed `create`, `update-spec`, and `delete`
command accepts `--operation-id <OPAQUE_ID>`. Omitting it on the first attempt generates a new
ID. Supplying it uses that value for both the operation identity and its idempotency binding;
there is no separate public idempotency flag.

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

The sole status command is:

```text
d2b --zone <ZONE> op inspect --operation-id <OPAQUE_ID> [--watch] [--human | --json]
```

`--watch` waits only for export completion or the request deadline; it never reapplies the
mutation. The exit contract for both mutation and inspection is closed:

| Exit | Mutation | `op inspect` |
| --- | --- | --- |
| `0` | Ordinary or stored final success | Stored final result returned |
| `75` | Mutation committed; authoritative audit export remains pending | Operation remains committed-pending-audit |
| `76` | Operation ID exists but replay binding differs | Not emitted |
| `2` | Invalid ID or invocation | Invalid, unknown, or replay-binding-denied ID, rendered identically |
| `1` | Other typed authorization, transport, or Resource API failure | Other typed authorization, transport, or Resource API failure |

Human pending output is exactly four newline-terminated lines, with no payload or sink detail:

```text
committed; audit export pending
operation: <OPAQUE_ID>
retry: repeat the identical mutation with --operation-id <OPAQUE_ID>
status: d2b --zone <ZONE> op inspect --operation-id <OPAQUE_ID>
```

Both a pending mutation and pending inspection emit this JSON field shape; only `command`
differs. `resourceStatus` is the exact bounded CLI recovery projection shown here; the
complete canonical `ResourceStatus` remains in the protobuf response, and no other status
fields are added to this CLI envelope:

```json
{
  "ok": false,
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
  "status": {
    "argv": ["d2b", "--zone", "<ZONE>", "op", "inspect", "--operation-id", "<OPAQUE_ID>"]
  }
}
```

For inspection, `command` is `op inspect`. A final inspection exits `0`; human mode renders
the stored original final result, and JSON uses
`{"ok":true,"command":"op inspect","state":"final","operationId":"<OPAQUE_ID>","result":...}`.
Pending output never says success, rollback, or safe to use a new ID.

A mutation replay-binding mismatch exits `76` and uses this exact human form:

```text
operation replay refused
operation: <OPAQUE_ID>
remediation: repeat the identical mutation with --operation-id <OPAQUE_ID>, or start a new mutation without that flag
```

Its JSON form is
`{"ok":false,"kind":"operation-replay-mismatch","operationId":"<OPAQUE_ID>","remediation":"repeat the identical mutation with --operation-id <OPAQUE_ID>, or start a new mutation without that flag"}`.
An inspection under the wrong subject/Zone binding is deliberately indistinguishable from an
unknown ID: both exit `2`, human mode prints `operation not found` followed by
`remediation: verify --zone and operation ID`, and JSON is exactly
`{"ok":false,"kind":"operation-not-found","message":"operation not found","remediation":"verify --zone and operation ID"}`.

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
  `committed-pending-audit` with exactly the command, flags, exits, and human/JSON forms above.
  A mutation replay mismatch exits `76`; inspection under the wrong subject/Zone binding exits
  `2` with the same form as an unknown ID. Neither exposes the original operation. Human and
  machine output never call it success, rollback, or safe to repeat with a new ID and expose
  no mutation payload or raw sink error.
- Zone readiness names `Provider/system-core` and the actual failing
  `Zone.status.handlers[]` record: `system-core-host` or `system-core-user`, with its `phase`
  and `lastReconciledAt`. Exactly one of each is required; duplicate, missing, wrong-name, or
  `provider-lifecycle` substitution is reported as an actionable refusal rather than a vague
  Provider path or boolean failure. T599 must match T605's paired contract/reference evidence.
- The cutover preview modifies nothing, and the apply path is unreachable without both consent
  and attestation.
- No retired verb remains, verified by its removal proof.
