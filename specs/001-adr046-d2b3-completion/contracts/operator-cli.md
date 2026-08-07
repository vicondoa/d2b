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
d2b op inspect --operation-id <OPAQUE_ID> [--zone <ZONE>] [--watch] [--deadline <DURATION> | --no-deadline] [--human | --json]
```

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
| `2` | Invalid ID or invocation | Invalid, unknown, or replay-binding-denied ID, rendered identically |
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
differ. Human guidance is not copied into JSON. `zoneRef` and `operationId` are bounded status
fields, never executable text. `resourceStatus` is the exact bounded CLI recovery projection
shown here; the complete canonical `ResourceStatus` remains in the protobuf response, and no
other status fields are added to this CLI envelope:

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
next: retry the identical mutation or start a new operation
```

Its JSON form is
`{"ok":false,"zoneRef":"Zone/<ZONE>","schemaVersion":2,"kind":"operation-replay-mismatch","operationId":"<OPAQUE_ID>","remediation":{"action":"retry-identical-operation"}}`.
The `retry-identical-operation` action means retry the same semantic mutation; a caller that
cannot do so starts a new mutation without reusing the ID. No rendered command is part of the
machine contract, and human guidance never renders flags, arguments, Zone, operation ID, or
shell text.
An inspection under the wrong subject/Zone binding is deliberately indistinguishable from an
unknown ID: both exit `2`, human mode prints `operation not found` followed by
`next: verify the operation ID, Zone, and authorization context`, and JSON is exactly
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

## Host-generation handoff recovery

This is a planned T595 target-closure helper contract, not behavior available at the
committed base. The unprivileged
`d2b-host-generation-deploy --inspect-authorized-handoff [--json]` command reads the sole
current-source nonterminal intent through the existing public socket. It accepts no intent
id, generation selector, path, token, or root invocation.

Its exact `HostGenerationHandoffStatusV1` schema and five-line human projection are in
`data-model.md`. It serializes only from that file's closed validated variants; arbitrary
state/phase/owner/action/successor cross-products refuse. Active source or target recovery
projects `wait-for-broker-recovery`; a failed existing broker unit projects
`restart-existing-broker`. Active and failed `transfer-pending` are distinct variants.
Active rollback projects `wait-for-broker-rollback`; failed rollback projects
`restart-existing-broker-for-rollback`. A valid `recovery-irreconcilable` state exists only
when immutable pre-mutation/outcome audit proves the complete prior
profile/service/pointer/reference tuple and one contiguous rollback. Any missing, duplicate,
reordered, or mismatched proof member is `invalid-coordinator`, not a recovery variant. No
state directs daemon repair or a new unit.

Human and JSON output contain no intent, generation, pid, uid, store path, unit path,
executable identity, or free-form remediation. The apply command and broker recovery return
the same typed projection after a valid conflict, concurrent transition, or terminal handoff.
The authenticated current-intent pointer selects active and terminal records without mtime
or caller input. Inspect exits `0` for every valid active or terminal tuple, `2` for root or
forbidden input, `3` only for an exactly empty coordinator census, and `4` for repairable
pointer absence, invalid coordinator, or incomplete rollback proof. Repairable absence has
its own exact `pointer-repair-required` human/JSON projection and action
`repair-authorized-handoff`; it is never rendered as `not-found`. Invalid coordinator uses
`preserve-and-escalate-invalid-coordinator`, not the repair command. `data-model.md` pins the
exact human and JSON error envelopes. Every state-table row, forbidden transition/input, source/target
active/failed condition, transfer-pending failure, restart, terminal pointer replacement,
incomplete rollback proof, and redaction case is independently pinned.

The paired mutation command is exactly
`d2b-host-generation-deploy --repair-authorized-handoff [--json]`. It is unprivileged,
selector-free, and traverses the same public socket. T595 owns the CLI and T592 owns the
typed `RepairHostGenerationCurrentIntentV1` broker operation. Only the accepted-socket
`Admin` capability is admitted; launcher, workload, Zone, unauthenticated, direct-broker,
and root callers are denied. Intent or generation selectors, path or token input, an extra
positional argument, and `--force` each exit `2` with zero mutation and the exact
`repair-without-selectors` human/JSON refusal from `data-model.md`.

Pointer absence is closed. `clean-absence` is an exact empty census, so inspect and repair
both return the exact exit-`3` not-found envelope without a write.
`repairable-absence` has exactly one fully valid authenticated active or terminal intent,
one contiguous sequence, and one complete immutable matrix with only its current pointer
missing. Every competing, malformed, unauthenticated, orphaned, unknown, or incomplete
census is `invalid-coordinator`. Under the broker coordinator lock,
repair durably appends the distinct `coordinator-pointer-repair/pre-mutation` audit member,
publishes the pointer from a file-synced unnamed inode directly to the final no-replace
name, syncs the final parent, and durably appends the matching outcome. A conflicting final
is preserved and exits `4`. Restart before the link sees absence; restart after it accepts
only absence or the exact complete final; a pre-only audit resumes, and a complete pair plus
exact pointer is second-run success with zero write. Dispatch, repair audit, backup, and
restoration records all use the one `HostGenerationImmutablePublicationV1` restart
classifier from `data-model.md`; every write/file-sync/link/parent/ancestor-sync boundary and
response-loss replay is tested independently per record class.

A missing, mismatched, unauthenticated, or noncontiguous immutable member exits `4` and
reports only its closed member id and failure class with
`action: restore-immutable-audit-backup`. The binding
`HostGenerationImmutableAuditBackupOwner` supplies the signed append-only
`HostGenerationImmutableAuditRestorationV1`. The named external acquisition procedure is
`host-generation-immutable-audit-backup-acquisition-v1`; it returns one canonical
current-user mode-`0600` single-link artifact no larger than 131,072 bytes. Submit it only
with:

```text
d2b-host-generation-deploy --restore-immutable-audit-backup PATH [--json]
```

The unprivileged T595 client opens the path once no-follow and sends only the bounded
canonical bytes through the existing public socket. T592 owns the shared
`RestoreHostGenerationImmutableAuditMemberRequestV1`/response DTO and typed broker op.
Only the consumed public-socket `Admin` capability is authorized. Launcher, workload, Zone,
`HostShutdown`, root, nonmember, unauthenticated-local, direct-broker, and remote callers
are denied before coordinator access; a valid signature is integrity only.

The broker validates the exact signed artifact and authenticated backup, durably appends the
fixed-field restoration pre-mutation audit, append-only provenance/restored-member record,
and matching outcome. A mismatched, unauthenticated, or noncontiguous original remains
preserved and is superseded only by that complete authenticated append-only chain. Completed
replay, including response loss, returns `already-restored` with zero write. The command
accepts one path and optional `--json`; selector, authority/key/token, member/failure
override, `--force`, root, or extra input exits `2`. Unauthorized, invalid artifact, and
conflict exits are `4` with only the fixed actions and closed failure classes in
`data-model.md`; success exits `0` and directs the operator to rerun
`--repair-authorized-handoff`.

The private backup capability has no construction, clone/copy/default, conversion,
serialization, or post-transfer reuse surface. A set is capped at 256 members and
16,777,216 encoded bytes, is unprunable while current, and is durably pruned only from day
30 through the hard day-90 deadline after replacement. Prune/limit failure returns the
typed redacted degraded report and blocks later mutation. The independent two-edge
restoration audit fixture and 62-case broker registry own caller denial,
signature/domain/key/authority/member/failure/predecessor binding, all four restoration
classes, conflicts, backup-before-mutation, retention boundaries, every publication crash
boundary, and zero-mutation refusals. The 156-case status registry is not a substitute.

An
unaudited extra mutation instead returns the separate
`preserve-and-escalate-audit-integrity-incident` action and is not restoration-eligible.
No generic copy, force flag, daemon repair path, or new unit exists.

Every action is executable or names one external procedure:

| Action | Owner and exact procedure |
| --- | --- |
| `inspect-without-selectors` | T595 command `d2b-host-generation-deploy --inspect-authorized-handoff [--json]` |
| `begin-host-generation-deploy` | operator runs the parameterized `host-generation-deploy-bootstrap-v1` procedure in `quickstart.md` |
| `repair-authorized-handoff` | T595 command `d2b-host-generation-deploy --repair-authorized-handoff [--json]` |
| `repair-without-selectors` | rerun the same T595 repair command without any other argument except optional `--json` |
| `restore-immutable-audit-backup` | external acquisition procedure above, then T595 `--restore-immutable-audit-backup PATH [--json]` |
| `restore-with-one-artifact` | rerun the T595 restoration command with exactly one artifact path and optional `--json` |
| `reacquire-immutable-audit-backup` | disposition-pinned backup authority reruns `host-generation-immutable-audit-backup-acquisition-v1`, then the operator resubmits |
| `rerun-repair-authorized-handoff` | T595 command `d2b-host-generation-deploy --repair-authorized-handoff [--json]` |
| `use-local-admin-public-socket` | site access administrator runs named external `host-generation-local-admin-session-v1`, then the local Admin reruns submission |
| `preserve-and-escalate-invalid-coordinator` | site security authority runs `host-generation-invalid-coordinator-escalation-v1` |
| `preserve-and-escalate-pointer-conflict` | site security authority runs `host-generation-pointer-conflict-escalation-v1` |
| `preserve-and-escalate-audit-restoration-conflict` | site security authority runs `host-generation-audit-restoration-conflict-escalation-v1` |
| `preserve-and-escalate-audit-integrity-incident` | site security authority runs `host-generation-audit-integrity-escalation-v1` |

Each escalation procedure preserves the coordinator and backup set, accepts only the
matching fixed error plus authenticated forensic evidence, and authorizes no repair, copy,
replace, delete, retry, or force action.

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
  operation. `op inspect` accepts and tests `--deadline` and `--no-deadline`, rejects their
  simultaneous use, and preserves cancellation in no-deadline mode. Human and machine output
  never call pending state success, rollback, or safe to repeat with a new ID, expose no
  mutation payload or raw sink error, and contain no Zone/ID-bearing argv, command vector,
  shell fragment, or free-form JSON remediation. Human output carries only the exact static
  identifier-free guidance above; JSON retains the closed action enum.
- T599 bumps the accepted CLI specification to Version 2 and owns migration guidance,
  DTO/schema, contract tests, references, and
  `changelog.d/cli-operation-recovery.md`. T220 verifies and folds the coordinated
  version/reference/test/schema/release treatment. Missing or Version 1 envelopes
  are never interpreted as Version 2, and arbitrary Version 1 IDs are never silently migrated.
- Zone readiness names `Provider/system-core` and the actual failing
  `Zone.status.handlers[]` record: `system-core-host` or `system-core-user`, with its `phase`
  and `lastReconciledAt`. Exactly one of each is required; duplicate, missing, wrong-name, or
  `provider-lifecycle` substitution is reported as an actionable refusal rather than a vague
  Provider path or boolean failure. T599 must match T605's paired contract/reference evidence.
- The cutover preview modifies nothing, and the apply path is unreachable without both consent
  and attestation.
- No retired verb remains, verified by its removal proof.
