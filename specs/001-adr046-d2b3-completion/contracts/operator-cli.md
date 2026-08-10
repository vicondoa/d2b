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
owns migration guidance, DTO/schema and contract tests, reference and release treatment plus
`changelog.d/cli-operation-recovery.md`; T220 reconciles the generated manifests and folds
that fragment. The implementation
MUST NOT ship this surface under the accepted Version 1 contract.

The replay handle is an exact 16-byte UUIDv7-layout operation ID rendered as lowercase
32-hex without separators. It remains opaque to operators. Every
generic and typed `create`, `update-spec`, and `delete` command accepts
`--operation-id <OPAQUE_ID>`. If the flag is omitted, the CLI generates the ID client-side
before opening the daemon transport and retains it through request encoding and response
handling. Supplying the flag uses that value for both operation identity and idempotency
binding; there is no separate public idempotency flag. The daemon never chooses the public
ID, so loss of a response cannot lose the only inspection handle.

Operation identity is exactly `(Zone, operation_id)`. The same ID is explicitly permitted as
an independent operation in different Zones; no host-global reservation or lookup exists.
UUIDv7 issuance time plus the fixed 30-day operation recovery retention defines checked
`expiresAt`.
Malformed, future, expired, overflowed, or clock-discontinuous IDs are denied before
observation or mutation. Once expired, an operation record may be pruned, but the old ID
always returns `operation-expired` and can never become a new mutation.

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
d2b --zone <ZONE> op inspect --operation-id <OPAQUE_ID> [--watch] [--deadline <DURATION> | --no-deadline] [--human | --json]
```

Zone is required. A selector-free or empty-Zone inspection is invalid invocation exit `2`;
the CLI never scans Zones or resolves ambiguity through a host-global index.
`--watch` waits only for export completion or the request deadline; it never reapplies the
mutation. It traverses the typed store, ResourceService, method-catalogue/router, daemon/client,
and CLI path owned by T589, T592, T593, T595, and T599; an in-memory or CLI-synthesized status
map is forbidden. The accepted command retains both common Zone-runtime deadline controls:
`--deadline <DURATION>` supplies the bounded wall deadline and `--no-deadline` suppresses the
default deadline while preserving signal cancellation. They are mutually exclusive; using
both is an invalid invocation. The Version 2 exit contract for mutation and inspection is
closed:

| Exit | Mutation | `op inspect` |
| --- | --- | --- |
| `0` | Ordinary or stored final success | Stored final result returned |
| `75` | Mutation committed; authoritative audit export remains pending | Operation remains committed-pending-audit |
| `76` | Operation ID exists but replay binding differs | Not emitted |
| `2` | Invalid, expired ID or invocation | Invalid, expired, unknown, or replay-binding-denied ID |
| `1` | Other typed authorization, transport, or Resource API failure | Other typed authorization, transport, or Resource API failure |

Human pending mutation output is exactly three newline-terminated lines, with no payload,
sink detail, argv, Zone value in guidance, or operation ID embedded in guidance. The final
line is safe static prose and may name only the literal command noun `d2b op inspect`, never
flags or substituted values:

```text
committed; audit export pending
operation: <OPAQUE_ID>
next: run d2b op inspect with the operation ID shown above
```

Pending inspection uses the same first two lines and the static final line
`next: wait for audit export or rerun d2b op inspect`. Both a pending mutation and pending
inspection emit the JSON field shape below; only `command` and the closed remediation action
differ. Human guidance is not copied into JSON. Direct Version 2 operator CLI/JSON status and
recovery responses are the sole raw-identity output exception: `zoneRef` and `operationId`
are bounded recovery coordinates supplied, generated, or received by that operator, never
executable text. They remain confined to the direct response and MUST NOT become telemetry
labels, span attributes, exported audit identities, or unrelated error context.
`resourceStatus` is the exact bounded CLI recovery projection shown here; the complete
canonical `ResourceStatus` remains in the protobuf response, and no other status fields are
added to this CLI envelope:

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

If the CLI has submitted a mutation but loses the transport before decoding one complete
response, the outcome is ambiguous and it exits `1`. It MUST NOT generate or submit a second
ID. Human output is exactly:

```text
operation response lost; outcome unknown
zone: Zone/<ZONE>
operation: <OPAQUE_ID>
next: run d2b op inspect with the operation ID shown above
```

JSON is exactly
`{"ok":false,"zoneRef":"Zone/<ZONE>","schemaVersion":2,"kind":"operation-response-lost","operationId":"<OPAQUE_ID>","remediation":{"action":"inspect-operation"}}`.
The ID is the client-generated value used for the possibly committed request. A
commit-then-response-loss integration test MUST inspect that same ID to its pending or final
stored result and prove that the recovery path performs no second mutation.

A mutation replay-binding mismatch exits `76` and uses this exact human form:

```text
operation replay refused
operation: <OPAQUE_ID>
next: retry the identical mutation or start a new operation
```

Its JSON form is
`{"ok":false,"zoneRef":"Zone/<ZONE>","schemaVersion":2,"kind":"operation-replay-mismatch","operationId":"<OPAQUE_ID>","remediation":{"action":"retry-identical-operation"}}`.
The `retry-identical-operation` action means retry the same semantic mutation; a caller that
cannot do so starts a new mutation without reusing the ID. No rendered command is part of the
machine contract, and human guidance never renders flags, arguments, Zone, operation ID, or
shell text.
An inspection under the wrong subject, or in a selected Zone with no matching operation, is
deliberately indistinguishable from an unknown ID: both exit `2`, human mode prints
`operation not found` followed by
`next: verify the operation ID, Zone, and authorization context`, and JSON is exactly
`{"ok":false,"zoneRef":"Zone/<ZONE>","schemaVersion":2,"kind":"operation-not-found","message":"operation not found","remediation":{"action":"verify-operation-context"}}`.

An intrinsically expired UUIDv7 ID is safe to distinguish because no retained operation is
observed. Mutation and inspection both exit `2`, perform no lookup outside the selected Zone
and no mutation, and use this exact human form:

```text
operation expired
next: start a new operation with a newly generated operation ID
```

JSON is exactly
`{"ok":false,"zoneRef":"Zone/<ZONE>","schemaVersion":2,"kind":"operation-expired","operationId":"<OPAQUE_ID>","remediation":{"action":"start-new-operation"}}`.

The closed remediation-action set is `inspect-operation`, `wait-for-audit-export`,
`retry-identical-operation`, `start-new-operation`, and `verify-operation-context`. No action
object accepts arguments, argv, shell text, Zone, or operation ID. IDs appear only in bounded
`operationId` and `resourceStatus.update.operationId` fields; Zone appears only in bounded
`zoneRef`. An envelope that omits `operationId`, such as the deliberately indistinguishable
not-found response, MUST NOT add it as unrelated context. Every success or error envelope
MUST keep the exact field set specified for its case above, and tests MUST prove the
permitted recovery coordinates are not copied into telemetry, spans, exported audit, or
unrelated errors.

Version 1 consumers must require `schemaVersion` and upgrade before using recovery. A missing
version or `schemaVersion: 1` retains the old 0/1/2 behavior and MUST NOT be interpreted as
Version 2. Arbitrary Version 1 operation IDs are not converted to the 16-byte Version 2 form.
The d2b 3.0 clean cutover imports no persisted Version 1 recovery state.

## Host-generation handoff recovery

This is a planned T595/T599 surface, not behavior available merely because this feature
contract names it. SC-002 protocol authority belongs solely to accepted Version 2
`docs/specs/ADR-046-validation-and-delivery.md` and its generated
`docs/specs/ADR-046-validation-and-delivery-traceability.{json,md}` artifacts. The generated
JSON is machine authority and the generated Markdown is its review view. FR-056 requires that
Version 2, regenerated manifests, Gate 0, and the generated rows be valid on an ancestor of
the implementation base before T589 or any downstream consumer proceeds.

The CLI consumes these stable generated rows rather than copying their protocol:

| Generated identifier | CLI use |
| --- | --- |
| `VD2-SC002-RECEIPT` | consume the assigned activation and close-stage evidence references |
| `VD2-SC002-PUBLICATION` | consume publication, locking, retention, capacity, and crash-recovery decisions without restating them |
| `VD2-SC002-INCIDENT` | consume incident classification, redaction, and escalation decisions |
| `VD2-SC002-DISPOSITION` | consume accepted authority and recovery disposition only through its generated owner |
| `VD2-SC002-RECOVERY` | consume inspectable states, emitted actions, exact invocations, exits, and convergence |
| `VD2-SC002-SOURCE-FLOOR` | consume the installed source-generation compatibility result |
| `VD2-SC002-REGISTRIES` | consume independently authored fixture and poison assignments |
| `VD2-SC002-TRACEABILITY` | bind every consumed identifier to its schema, implementation owner, task, fixture, and enforcing gate |

Missing, duplicate, extra, stale, wrong-owner, non-ancestor, or failing generated rows block
the surface. The refusal names the remediation: accept Version 2, regenerate its traceability,
and pass Gate 0. This contract defines no host-generation protocol shape, state machinery, or
test matrix. None may be inferred from the operator commands or action procedures below.

### Operator commands

| Purpose | Exact invocation | Actor and input boundary |
| --- | --- | --- |
| Inspect the authorized handoff | `d2b host-generation inspect-authorized-handoff [--json]` | unprivileged local public-socket `Admin`; no selector or positional input |
| Repair the authorized handoff | `d2b host-generation repair-authorized-handoff [--json]` | unprivileged local public-socket `Admin`; no selector or positional input |
| Restore one immutable audit backup | `d2b host-generation restore-immutable-audit-backup PATH [--json]` | unprivileged local public-socket `Admin`; exactly one no-follow artifact path |

These are subcommands of the sole public `d2b` binary. No
`d2b-host-generation-deploy` executable, alias, wrapper, or compatibility command is
published. The commands traverse the existing public socket and typed broker operations. They never
run as root, connect directly to the broker, add a daemon repair path, or create a new unit.
The inspect and repair forms reject intent, generation, path, token, authority, selector,
extra positional, and `--force` input. The restoration form rejects every additional path,
selector, override, authority/key/token, extra positional, and `--force` input. The accepted
`VD2-SC002-RECOVERY` row alone supplies response states, exact human and JSON rendering, exits,
and convergence. The CLI must not synthesize a replacement status or translate an unknown
variant into a generic success or repair instruction.

### Public action owners

The following rows define operator resolution for generated public actions. They do not
define the emitted action enum or map protocol states and failures to actions; accepted
Version 2 and generated `VD2-SC002-RECOVERY`, `VD2-SC002-INCIDENT`,
`VD2-SC002-REGISTRIES`, and `VD2-SC002-TRACEABILITY` own those decisions.

| Action | Owner and exact procedure |
| --- | --- |
| `inspect-without-selectors` | local public-socket `Admin` runs `d2b host-generation inspect-authorized-handoff [--json]` |
| `begin-host-generation-deploy` | operator runs public runbook procedure `host-generation-deploy-bootstrap-v1` |
| `repair-authorized-handoff` | local public-socket `Admin` runs `d2b host-generation repair-authorized-handoff [--json]` |
| `repair-without-selectors` | local public-socket `Admin` reruns the repair command with no argument except optional `--json` |
| `restore-immutable-audit-backup` | disposition-pinned backup authority runs `host-generation-immutable-audit-backup-acquisition-v1`, then the local public-socket `Admin` runs `d2b host-generation restore-immutable-audit-backup PATH [--json]` |
| `restore-with-one-artifact` | local public-socket `Admin` reruns the restoration command with exactly the same one artifact path and optional `--json` |
| `reacquire-immutable-audit-backup` | disposition-pinned backup authority reruns `host-generation-immutable-audit-backup-acquisition-v1`, then the local public-socket `Admin` resubmits |
| `rerun-repair-authorized-handoff` | local public-socket `Admin` runs `d2b host-generation repair-authorized-handoff [--json]` |
| `use-local-admin-public-socket` | site access administrator runs `host-generation-local-admin-session-v1`, then the resulting local public-socket `Admin` reruns the command |
| `use-unprivileged-local-admin-restoration-session` | site access administrator runs `host-generation-unprivileged-local-admin-restoration-session-v1`, then the resulting unprivileged local public-socket `Admin` reruns the one-artifact command |
| `reconcile-immutable-audit-retention` | site backup administrator runs `host-generation-immutable-audit-retention-reconciliation-v1` |
| `repair-retention-clock-discontinuity` | site backup administrator repairs the configured authoritative time source and runs `host-generation-retention-clock-discontinuity-repair-v1` through an unprivileged local public-socket `Admin` |
| `repair-continuity-replay-key-generation` | site package administrator runs `host-generation-continuity-replay-key-generation-repair-v1` |
| `repair-continuity-authoritative-source` | site backup administrator runs `host-generation-continuity-authoritative-source-repair-v1`, then an unprivileged local public-socket `Admin` runs `host-generation-retention-clock-discontinuity-repair-v1` |
| `repair-continuity-authoritative-source-contract` | site package administrator runs `host-generation-continuity-authoritative-source-contract-repair-v1`, then an unprivileged local public-socket `Admin` runs `d2b host-generation repair-authorized-handoff [--json]` |
| `repair-continuity-source-storage-and-reconcile` | site backup administrator runs `host-generation-continuity-source-storage-repair-v1`, then an unprivileged local public-socket `Admin` runs `d2b host-generation repair-authorized-handoff [--json]` |
| `preserve-and-escalate-continuity-source-conflict` | site security authority runs `host-generation-continuity-source-conflict-escalation-v1` and preserves the named evidence until an accepted disposition authorizes a next step |
| `preserve-and-escalate-continuity-publication-conflict` | site security authority runs `host-generation-continuity-publication-conflict-escalation-v1` and preserves the named evidence until an accepted disposition authorizes a next step |
| `preserve-and-escalate-retention-clock-overflow` | site security authority runs `host-generation-retention-clock-overflow-escalation-v1`; no prune or handoff retry is authorized |
| `repair-retention-storage-and-reconcile` | site backup administrator runs `host-generation-retention-storage-repair-v1` and then its typed reconciliation step |
| `repair-retention-census-and-reconcile` | site backup administrator runs `host-generation-retention-census-repair-v1` and then its typed reconciliation step |
| `repair-retention-audit-and-reconcile` | site backup administrator runs `host-generation-retention-audit-repair-v1` and then its typed reconciliation step |
| `resubmit-same-restoration-artifact` | the same unprivileged local public-socket `Admin` reruns the restoration command with the byte-identical artifact |
| `repair-restoration-storage-and-resubmit` | site backup administrator runs `host-generation-restoration-storage-repair-v1`, then the operator resubmits the same artifact |
| `repair-restoration-client-broker-contract` | site package administrator runs `host-generation-restoration-client-broker-contract-repair-v1`, then the operator reruns the one-artifact command |
| `preserve-and-escalate-invalid-coordinator` | site security authority runs `host-generation-invalid-coordinator-escalation-v1` |
| `preserve-and-escalate-pointer-conflict` | site security authority runs `host-generation-pointer-conflict-escalation-v1` |
| `preserve-and-escalate-audit-restoration-conflict` | site security authority runs `host-generation-audit-restoration-conflict-escalation-v1` |
| `preserve-and-escalate-audit-integrity-incident` | site security authority runs `host-generation-audit-integrity-escalation-v1` |

Each named `host-generation-*-v1` procedure is an identically named anchor in
`docs/how-to/host-generation-recovery-v1.md`. The historical plan assigned that public
runbook and generated action map to T599 and historical link/agreement verification to T220. Those rows
are read-only history and supply no current gate. A missing, extra, duplicate,
unowned, or broken action mapping blocks release. Machine output carries only the generated
action token, never an argv array, shell fragment, free-form command, Zone, operation ID, or
artifact path. Escalation procedures preserve the affected evidence and authorize no repair,
replacement, deletion, retry, or force action unless the accepted disposition explicitly
permits it.

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
  forms above. A mutation replay mismatch exits `76`; inspection requires Zone, and a wrong
  subject/Zone binding exits `2` with the same form as an unknown ID. The same ID may commit
  independently in two Zones. UUIDv7 issuance and per-Zone retention tests prove that
  expired/pruned IDs return `operation-expired` and cannot become new mutations. Neither
  wrong-binding case exposes the original operation. `op inspect` accepts and tests
  `--deadline` and `--no-deadline`, rejects their
  simultaneous use, and preserves cancellation in no-deadline mode. Human and machine output
  never call pending state success, rollback, or safe to repeat with a new ID, expose no
  mutation payload or raw sink error, and contain no Zone/ID-bearing argv, command vector,
  shell fragment, or free-form JSON remediation. Human output carries only the exact static
  identifier-free guidance above; JSON retains the closed action enum. The bounded Version 2
  `zoneRef` and `operationId` recovery coordinates stay confined to direct operator responses
  and occur zero times in telemetry labels, spans, exported audit identities, or unrelated
  error context.
- T599 bumps the accepted CLI specification to Version 2 and owns migration guidance,
  DTO/schema, contract tests, references, and
  `changelog.d/cli-operation-recovery.md`. The former T220 verification/fold is historical.
  Missing or Version 1 envelopes
  are never interpreted as Version 2, and arbitrary Version 1 IDs are never silently migrated.
- Host-generation handoff commands consume only accepted generated `VD2-SC002-*` rows for
  protocol states, publication, capacity, rendering, exits, and transitions. Every generated
  public action resolves to exactly one command or named owner and public runbook procedure
  above; the feature-local CLI contract does not redefine the SC-002 protocol.
- `d2b --help`, host-generation subcommand help, packaging, completions, and policy tests
  expose only the `d2b` binary. The retired standalone executable name occurs in no emitted
  command, package output, runbook invocation, or compatibility alias.
- Zone readiness names `Provider/system-core` and the actual failing
  `Zone.status.handlers[]` record: `system-core-host` or `system-core-user`, with its `phase`
  and `lastReconciledAt`. Exactly one of each is required; duplicate, missing, wrong-name, or
  `provider-lifecycle` substitution is reported as an actionable refusal rather than a vague
  Provider path or boolean failure. T599 must match T605's paired contract/reference evidence.
- The cutover preview modifies nothing, and the apply path is unreachable without both consent
  and attestation.
- No retired verb remains, verified by its removal proof.
