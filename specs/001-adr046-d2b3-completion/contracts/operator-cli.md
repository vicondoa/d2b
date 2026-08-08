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
classifier from `data-model.md`; every hierarchy-creation/write/file-sync/link/final-reopen/
parent/ancestor/final-directory-sync boundary and response-loss replay is tested
independently per record class.

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
`RestoreHostGenerationImmutableAuditMemberRequestV1` and the closed nested
`Completed | Refused | Pending | Degraded` response DTO plus the typed broker op.
`Refused` distinguishes root from every other unauthorized caller. `Pending` carries one
closed pending-reason variant with its total publication-failure projection; neither
`Pending` nor `Degraded` exposes the broker-private restoration attempt id. Authorization,
request-shape, artifact, conflict, retention, publication-degraded, and
publication-pending are closed variants; this operation never uses a free-form broker
message fallback.
Only the consumed public-socket `Admin` capability is authorized. Launcher, workload, Zone,
`HostShutdown`, root, nonmember, unauthenticated-local, direct-broker, and remote callers
are denied before coordinator access; a valid signature is integrity only.

The release-sealed client constructs only the exact valid shared request DTO, so a broker
`invalid-request` response is local client/broker contract skew or corruption rather than
operator syntax. All nineteen closed classes are projected distinctly by their literal
`failure-class`:
`missing-schema-version`, `wrong-schema-version`, `missing-kind`, `wrong-kind`,
`unknown-field`, `missing-operation-id`, `operation-id-length`,
`operation-id-digest-mismatch`, `missing-artifact-bytes`, `empty-artifact-bytes`,
`over-limit-artifact-bytes`, `path-field`, `selector-field`, `uid-field`, `pid-field`,
`authority-token-field`, `member-override-field`, `failure-override-field`, and
`free-form-field`. Each exits `4` with exactly:

```text
host generation handoff audit restoration request rejected
failure-class: <INVALID_REQUEST_CLASS>
action: repair-restoration-client-broker-contract
```

JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-restoration-invalid-request","failureClass":"<INVALID_REQUEST_CLASS>","action":"repair-restoration-client-broker-contract"}`.
There is one real-binary golden per literal class. This is distinct from local parser
invalid invocation, which remains exit `2` and action `restore-with-one-artifact`.

Only canonical parsing, digesting, size, and signature verification may occur before the
coordinator lock. Under the lock, the broker fd-relative reopens and identity-revalidates
the coordinator and backup, rechecks the exact artifact/backup/state binding, and reserves
capacity immediately before mutation. The broker first durably appends fixed-field
restoration pre-mutation audit containing only typed domain-separated digests and closed
enums. Only after that pre record is durable may it publish broker-private non-observable
preparatory evidence carrying the required bodies, followed by digest/enum-only effective
audit provenance and a matching fixed outcome. Private evidence alone changes no
coordinator, audit view, census, or member. A mismatched, unauthenticated, or noncontiguous
original remains preserved and is superseded only by that complete authenticated chain.
No restoration body becomes durable before pre-audit.

A process death after pre-audit but before private evidence loses the request frame. That
pre-only prefix is a blocked pending state, not caller-free recovery: the broker admits no
later coordinator mutation and cannot publish evidence until the same unprivileged
public-socket `Admin` resubmits the byte-identical signed artifact. Resubmission repeats
authorization and every under-lock binding check, reconstructs the same operation and
private attempt, and then continues at evidence publication. A different artifact or
binding conflicts and cannot settle the prefix. Once private evidence is durable, restart
may continue from that durable body without caller input. Fresh-process fault tests pin
pre-only blocking, identical resubmission, different-artifact conflict, and every
post-evidence publication boundary.

Settlement is convergent under the same deterministic operation id. Pending renders
`settlement: restart-settlement-pending`; the operator immediately resubmits the
byte-identical artifact, and that authorized resubmission drives pre-only or later pending
settlement. There is no automatic-settlement prerequisite and no restoration status command.
The closed pending reason carries the exact publication failure class. A pre-only crash has
no response to replay; its first byte-identical resubmission continues the same attempt and
returns completed, conflict, or an actual typed publication failure rather than inventing a
synthetic failure class. Durable degraded renders
`settlement: repair-required`; the site backup administrator runs
`host-generation-restoration-storage-repair-v1`, then the operator resubmits that exact
artifact. The broker resumes the same append-only attempt, emits a fixed repair-resume
pre-audit event, completes only the missing publication records, and returns restored or
already-restored. A degraded event remains history but is not terminal current state after
the repaired settlement. A different artifact or operation binding conflicts rather than
opening a new attempt. Completed replay, including response loss, returns
`already-restored` with zero write. The degraded-repaired-restored, restart-after-pending,
restart-after-degraded, pre-only identical-resubmission, pre-only conflicting-resubmission,
and response-loss paths each have exact human/JSON goldens and success or refusal tests.

If the real CLI writes the restoration request but the public-socket transport closes
before one complete typed broker response, including a pre-only broker crash, it exits `4`
and prints exactly:

```text
host generation handoff immutable audit restoration response lost
action: resubmit-same-restoration-artifact
```

With `--json`, it prints exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-restoration-response-lost","action":"resubmit-same-restoration-artifact"}`.
There is no `failureClass` or `settlement` field because transport loss does not reveal a
publication class or durable broker prefix. The operator immediately resubmits the
byte-identical artifact through the same unprivileged local Admin path; no restart wait,
automatic settlement, or status command is a prerequisite. Dedicated real-binary
human/JSON goldens pin these bytes and exit.

The command accepts one path and optional `--json`; selector, authority/key/token,
member/failure override, `--force`, or extra input exits `2`. Root instead exits `4` with
the distinct `use-unprivileged-local-admin-restoration-session` action. Unauthorized,
invalid artifact, conflict, retention capacity/admission/degradation, and restoration publication
pending/degraded exits are `4` with only the fixed actions and closed failure classes in
`data-model.md`; success exits `0` and directs the operator to rerun
`--repair-authorized-handoff`.

The additional exit-`4` forms are exact. Root is:

```text
host generation handoff audit restoration requires unprivileged local Admin
action: use-unprivileged-local-admin-restoration-session
```

Its JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-restoration-root-refused","action":"use-unprivileged-local-admin-restoration-session"}`.
Retention capacity is exactly:

```text
host generation handoff immutable audit retention capacity unavailable
failure-class: <CLOSED_RETENTION_CAPACITY_CLASS>
action: <ACTION_FROM_RETENTION_CAPACITY_TABLE>
```

Retention degradation is exactly:

```text
host generation handoff immutable audit retention degraded
failure-class: <CLOSED_RETENTION_DEGRADED_CLASS>
action: <ACTION_FROM_RETENTION_TABLE>
```

Capacity JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-backup-retention-capacity","failureClass":"<CLOSED_RETENTION_CAPACITY_CLASS>","action":"<ACTION_FROM_RETENTION_CAPACITY_TABLE>"}`.
This audited refusal has zero ledger and restoration mutation but retains its exact capacity
pre/outcome pair. Standing-reserve exhaustion is the separate no-write pre-audit admission
form:

```text
host generation handoff immutable audit capacity admission unavailable
failure-class: standing-reserve-exhausted
action: repair-retention-audit-and-reconcile
```

Its JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-capacity-admission-refused","failureClass":"standing-reserve-exhausted","action":"repair-retention-audit-and-reconcile"}`.
It carries no capacity attempt digest or generation transition.
Degradation JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-backup-retention-degraded","failureClass":"<CLOSED_RETENTION_DEGRADED_CLASS>","action":"<ACTION_FROM_RETENTION_TABLE>"}`.
Restoration publication pending/degraded uses the exact
human/JSON bytes in `data-model.md` and action `resubmit-same-restoration-artifact` or
`repair-restoration-storage-and-resubmit`. Every form has a dedicated golden; no generic
renderer string is accepted.

The private backup capability has no construction, clone/copy/default, conversion,
serialization, or post-transfer reuse surface. A set is capped at 256 members and
16,777,216 encoded bytes; the root is capped at 64 retained intents, 4,096 members, and
268,435,456 bytes. Capacity is reserved before handoff. A current set is unprunable. The replacement transition durably binds one fixed
`CLOCK_REALTIME+CLOCK_BOOTTIME` age anchor before it becomes effective, and replay never
resamples it. Same-boot age follows boot time with at most 300 seconds of wall-clock skew;
unsafe forward discontinuity or a changed-boot continuity gap quarantines age and uses
`repair-retention-clock-discontinuity`. A replaced set is pruneable from day 30 through the
hard day-90 deadline. The existing broker runs startup and internal idle-wake catch-up
without Admin traffic and either performs the audited mandatory prune or fails closed in
typed degradation. `PruneHostGenerationImmutableAuditBackupsV1` consumes the private-field
sealed lifetime-bound permit with no clone/copy/default/conversion/serde/accessor surface and
emits immutable fixed-field pre/outcome audit. Prune/limit/clock/settlement failure returns
the typed redacted report/action and blocks later mutation.

Reserved continuity-subset refusals are not rendered as generic retention failures.
`continuity-evidence-record-limit | continuity-evidence-byte-limit` use the exact
retention-capacity form above, exit `4`, error `audit-backup-retention-capacity`, and
action `repair-continuity-authoritative-source-contract`.
`continuity-repair-attempt-limit` is the private trigger-only capacity classification for
ordered cleanup. It is excluded from `CLOSED_RETENTION_CAPACITY_CLASS` and is never valid as
a public `failureClass`; its paired internal continuation label
`resume-oldest-continuity-cleanup` is never valid as a public `action`. On that
classification, the broker first drives the oldest broker-target compaction, source
release, and attempt-slice release. If cleanup blocks, the CLI renders that exact broker,
source-lifecycle, or ledger failure; it never renders the trigger, its internal continuation
label, a generic limit, or a prune-only action.
A 257th source live-pair admission uses the same ordered cleanup and exact-blocker rule.
`source-capacity` below is reserved for an authoritative-source record/byte contract
violation, not live-slot exhaustion.

Replay-key publication failure exits `4`. Its human form is exactly:

```text
host generation handoff immutable audit continuity replay key unavailable
failure-class: <CLOSED_REPLAY_KEY_FAILURE_CLASS>
action: <ACTION_FROM_REPLAY_KEY_TABLE>
```

Its JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-continuity-replay-key-unavailable","failureClass":"<CLOSED_REPLAY_KEY_FAILURE_CLASS>","action":"<ACTION_FROM_REPLAY_KEY_TABLE>"}`.
`CLOSED_REPLAY_KEY_FAILURE_CLASS` is exactly `entropy-unavailable | hierarchy | write |
file-sync | link | reopen | directory-sync | conflict |
replay-key-missing-after-parent-durable | posture | audit-publication`. Actions are
`repair-continuity-replay-key-generation` for `entropy-unavailable`,
`repair-retention-storage-and-reconcile` for the six storage boundaries,
`preserve-and-escalate-continuity-publication-conflict` for `conflict`, and
`preserve-and-escalate-audit-integrity-incident` for
`replay-key-missing-after-parent-durable | posture`, and
`repair-retention-audit-and-reconcile` for `audit-publication`.
Replay-key reservation failure uses the existing exact
`root-publication-record-limit | root-publication-byte-limit` retention-capacity form, or
the standing-reserve admission form when applicable; it never enters
`CLOSED_REPLAY_KEY_FAILURE_CLASS`.

Source pin/binding, audited source release, or source-prefix reconciliation failure exits
`4`. Its human form is exactly:

```text
host generation handoff immutable audit continuity source lifecycle unavailable
stage: <CLOSED_SOURCE_LIFECYCLE_STAGE>
failure-class: <CLOSED_SOURCE_LIFECYCLE_FAILURE_CLASS>
action: <ACTION_FROM_SOURCE_LIFECYCLE_TABLE>
```

Its JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-continuity-source-lifecycle-unavailable","stage":"<CLOSED_SOURCE_LIFECYCLE_STAGE>","failureClass":"<CLOSED_SOURCE_LIFECYCLE_FAILURE_CLASS>","action":"<ACTION_FROM_SOURCE_LIFECYCLE_TABLE>"}`.
`CLOSED_SOURCE_LIFECYCLE_STAGE` is exactly `pin-acquisition | replay-binding |
source-release | source-prefix-reconciliation`.
`CLOSED_SOURCE_LIFECYCLE_FAILURE_CLASS` is exactly `source-capacity |
source-unavailable | source-conflict | hierarchy | write | file-sync | link | reopen |
unlink | directory-sync | census | conflict | audit-publication |
recovery-generation-overflow`. `source-capacity` caused
by evidence count or size maps to `repair-continuity-authoritative-source-contract`;
`source-unavailable` maps to
`repair-continuity-authoritative-source`; `source-conflict | conflict` map to
`preserve-and-escalate-continuity-source-conflict`; the six storage boundaries map to
`repair-continuity-source-storage-and-reconcile`; and `audit-publication` maps to
`repair-retention-audit-and-reconcile`. Source-release `unlink` maps to
`repair-continuity-source-storage-and-reconcile`, and source-release `census` maps to
`repair-retention-census-and-reconcile`.
`source-prefix-reconciliation/recovery-generation-overflow` maps to
`preserve-and-escalate-audit-integrity-incident`; it authorizes no wrap, retry, source
mutation, pair recycle, or next pre-audit.
`source-capacity` is emitted only for the authoritative source's exact record/byte contract,
including refusal of a 141,313-byte combined attempt after acceptance at 141,312 bytes; it
always maps to `repair-continuity-authoritative-source-contract`. A 257th live pair never
uses `source-capacity`: it drives ordered broker compaction, source release, and
attempt-slice release and returns the first exact cleanup blocker from the closed matrices
below. Human/JSON goldens pin the exact-bound source-contract refusal and every ordered
cleanup blocker so neither case can fall back to
`reconcile-immutable-audit-retention`.

Valid stage/class pairs are closed. `pin-acquisition` admits `source-capacity |
source-unavailable | source-conflict | hierarchy | write | file-sync | link | reopen |
directory-sync | conflict | audit-publication`; `replay-binding` admits the same set
without `source-capacity`; and `source-release` admits only `source-unavailable |
source-conflict | hierarchy | reopen | unlink | directory-sync | census | conflict |
audit-publication`; `source-prefix-reconciliation` admits only
`recovery-generation-overflow`. Replay-key `audit-publication` is valid only for fixed outcome-audit
publication after parent durability. Strict schemas, wire snapshots, constructors,
deserializers, and human/JSON goldens accept every listed pair and reject every other
stage/class pair or action substitution. The overflow golden uses `u32::MAX`, exits `4`,
and carries no intended outcome, terminal failure, next generation, or mutable identifier.

Replay-key candidate recycling, broker compaction/recovery, or a resumable
attempt-capacity release failure exits `4` and never masquerades as a settled continuity
repair:

```text
host generation handoff immutable audit continuity cleanup pending
stage: <CLOSED_CONTINUITY_CLEANUP_STAGE>
failure-class: <CLOSED_CONTINUITY_CLEANUP_FAILURE_CLASS>
action: <ACTION_FROM_CONTINUITY_CLEANUP_TABLE>
```

Its JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-continuity-cleanup-pending","stage":"<CLOSED_CONTINUITY_CLEANUP_STAGE>","failureClass":"<CLOSED_CONTINUITY_CLEANUP_FAILURE_CLASS>","action":"<ACTION_FROM_CONTINUITY_CLEANUP_TABLE>"}`.
Stages are exactly `replay-key-candidate-recycling | broker-compaction |
capacity-release`. Classes are exactly `head-changed | target-changed | hierarchy | write |
file-sync | link | reopen | unlink | directory-sync | census | conflict |
audit-publication`.
`head-changed | target-changed | conflict` map to
`preserve-and-escalate-continuity-publication-conflict`;
`hierarchy | write | file-sync | link | reopen | unlink | directory-sync` to
`repair-retention-storage-and-reconcile`, `census` to
`repair-retention-census-and-reconcile`, and `audit-publication` to
`repair-retention-audit-and-reconcile`. Valid pending pairs are exactly:

| Stage | Admitted classes |
| --- | --- |
| `replay-key-candidate-recycling` | `hierarchy | write | file-sync | link | reopen | unlink | directory-sync | census | conflict | audit-publication` |
| `broker-compaction` | `head-changed | target-changed | hierarchy | write | file-sync | link | reopen | unlink | directory-sync | census | conflict | audit-publication` |
| `capacity-release` | `census | audit-publication` |

Candidate-recycler and broker-compaction storage, census, conflict, and audit-publication
failures after successful unlink carry the original durable mutation intent and resume
that same operation; pre-unlink candidate-recycler failure likewise keeps the original
fixed identity pending and retries it after repair rather than publishing a failed outcome
or successor identity. These are not settled degraded repair results. Strict schemas, wire
snapshots, constructors, deserializers, and one human and JSON golden for every exact
stage/class/action triple reject every other pair and every action substitution. For
capacity release, `census` and `audit-publication` preserve and resume the original
ledger-safe release prefix after their named repair.

`ledger-conflict` and the four standing-reserve corruption classes are terminal integrity
incidents, not pending cleanup. They exit `4` with the distinct human form:

```text
host generation handoff immutable audit continuity capacity integrity incident
stage: capacity-release
failure-class: <CLOSED_CAPACITY_INTEGRITY_INCIDENT_CLASS>
action: preserve-and-escalate-audit-integrity-incident
```

Their JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-continuity-capacity-integrity-incident","stage":"capacity-release","failureClass":"<CLOSED_CAPACITY_INTEGRITY_INCIDENT_CLASS>","action":"preserve-and-escalate-audit-integrity-incident"}`.
`CLOSED_CAPACITY_INTEGRITY_INCIDENT_CLASS` is exactly `ledger-conflict |
standing-reserve-missing | standing-reserve-overdrawn |
standing-reserve-duplicated | standing-reserve-unaccounted`. This shape contains no
`pending`, `settlement`, successor, or retry field. It preserves the ledger, keeps the
charged slice unavailable, and renders no successor release identity or retry command.
Strict schema and human/JSON goldens independently reject every terminal class in the
cleanup-pending shape and every resumable class in the integrity-incident shape.
Source-release failure always uses the source-lifecycle form above, never this cleanup
class.

The selector-free `host-generation-retention-clock-discontinuity-repair-v1` procedure
returns only the following continuity-repair forms. It accepts no caller evidence,
timestamp, boot identity, anchor, deadline, member, digest, path, selector, or force input.
A repaired result exits `0`. Its human form is exactly two newline-terminated lines:

```text
host generation handoff immutable audit continuity repaired
outcome: <REPAIRED_OUTCOME>
```

`REPAIRED_OUTCOME` is exactly `repaired-before-day-90 |
repaired-after-mandatory-prune`. JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-continuity-repair","ok":true,"outcome":"<REPAIRED_OUTCOME>"}`.

Decision basis write-ahead publication pending exits `4` before any selected outcome is
available for public projection. Its human form is exactly:

```text
host generation handoff immutable audit continuity repair decision basis pending
publication-stage: decision-basis
failure-boundary: <CLOSED_SETTLEMENT_PUBLICATION_BOUNDARY>
settlement: decision-basis-pending
action: <ACTION_FROM_SETTLEMENT_BOUNDARY>
```

Its JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-continuity-repair-decision-basis-pending","publicationStage":"decision-basis","failureBoundary":"<CLOSED_SETTLEMENT_PUBLICATION_BOUNDARY>","settlement":"decision-basis-pending","action":"<ACTION_FROM_SETTLEMENT_BOUNDARY>"}`.
This form deliberately has no `intendedOutcome` or terminal `failure`: neither is public
until the exact private `ContinuityRepairDecisionBasisV1` final and directory chain are
durable. The private `ContinuityRepairDecisionBasisIntentV1` final and directory chain are
the write-ahead decision commit; an in-memory candidate before intent durability is not a
selected outcome. Before intent parent durability, restart accepts an absent or exact final:
absence means no decision committed and replays the preceding durable repair state, while
an exact final resumes durability. After the intent directory chain is durable, every
later basis state reloads that frozen intent. Before basis parent durability, restart
accepts absence or the exact basis and recreates absence byte-identically only from the
durable intent. The closed intent and basis boundary set is exactly
`hierarchy | write | file-sync | link | reopen | directory-sync | conflict |
audit-publication`; every member has this one legal response shape and exact exit. The
boundary is derived from either the sealed `Progress` prefix or sealed `Conflict` state.
Progress prefixes exclude `Conflict`. A `conflict` state preserves its durable predecessor
plus existing and candidate private digests, publishes no intended outcome, and maps only to
`preserve-and-escalate-continuity-publication-conflict`.

Decision selection publication pending exits `4` before an intended outcome is publicly
committed. Its human form is exactly:

```text
host generation handoff immutable audit continuity repair decision selection pending
intended-outcome: <CLOSED_TERMINAL_OUTCOME>
publication-stage: decision-selection
failure-boundary: <CLOSED_SETTLEMENT_PUBLICATION_BOUNDARY>
settlement: decision-selection-pending
action: <ACTION_FROM_SETTLEMENT_BOUNDARY>
```

Its JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-continuity-repair-decision-selection-pending","intendedOutcome":"<CLOSED_TERMINAL_OUTCOME>","publicationStage":"decision-selection","failureBoundary":"<CLOSED_SETTLEMENT_PUBLICATION_BOUNDARY>","settlement":"decision-selection-pending","action":"<ACTION_FROM_SETTLEMENT_BOUNDARY>"}`.
`intendedOutcome` comes only from the durable sealed decision basis recorded before
decision-selection publication. The boundary is derived from the incomplete durable prefix
or sealed conflict state and cannot be supplied independently. `conflict` is not a progress
prefix; it carries the same durable basis predecessor plus existing and candidate private
selection digests and permits no replacement or reselection.

Settlement preparation incomplete after durable decision selection but before durable
decision-pre exits `4`. Its human form is exactly:

```text
host generation handoff immutable audit continuity repair settlement preparation incomplete
intended-outcome: <CLOSED_TERMINAL_OUTCOME>
stage: decision-pre-audit
failure-class: audit-publication
action: repair-retention-audit-and-reconcile
```

Its JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-continuity-repair-settlement-preparation-incomplete","intendedOutcome":"<CLOSED_TERMINAL_OUTCOME>","stage":"decision-pre-audit","failureClass":"audit-publication","action":"repair-retention-audit-and-reconcile"}`.
The intended outcome comes only from the exact durable decision-selection final. Restart
reclassifies that selection, performs no new repair mutation, and retries decision-pre
publication before intent or terminal publication.

Pending exact-outcome publication after durable decision-pre exits `4`. Its human form is
exactly:

```text
host generation handoff immutable audit continuity repair pending
intended-outcome: <CLOSED_TERMINAL_OUTCOME>
publication-stage: <CLOSED_SETTLEMENT_PUBLICATION_STAGE>
failure-boundary: <CLOSED_SETTLEMENT_PUBLICATION_BOUNDARY>
settlement: exact-terminal-settlement-pending
action: <ACTION_FROM_SETTLEMENT_BOUNDARY>
```

`CLOSED_TERMINAL_OUTCOME` is exactly `repaired-before-day-90 |
repaired-after-mandatory-prune | degraded-before-day-90 |
degraded-day-90-before-prune | degraded-day-90-after-prune`. JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-continuity-repair-pending","intendedOutcome":"<CLOSED_TERMINAL_OUTCOME>","publicationStage":"<CLOSED_SETTLEMENT_PUBLICATION_STAGE>","failureBoundary":"<CLOSED_SETTLEMENT_PUBLICATION_BOUNDARY>","settlement":"exact-terminal-settlement-pending","action":"<ACTION_FROM_SETTLEMENT_BOUNDARY>"}`.
`CLOSED_SETTLEMENT_PUBLICATION_STAGE` is exactly `outcome-intent | terminal-outcome`.
`CLOSED_SETTLEMENT_PUBLICATION_BOUNDARY` is exactly `hierarchy | write | file-sync | link |
reopen | directory-sync | conflict | audit-publication`. The intended outcome is
reconstructed from broker-private durable decision-pre for the intent stage and from the
byte-identical durable intent for the terminal stage; it is never changed during
settlement. Human/JSON goldens cover every stage/boundary pair for all five intended
outcomes, and fresh-process cases prove the first missing boundary alone is retried.
`ACTION_FROM_SETTLEMENT_BOUNDARY` is
`repair-retention-storage-and-reconcile` for `hierarchy | write | file-sync | link |
reopen | directory-sync`,
`preserve-and-escalate-continuity-publication-conflict` for `conflict`, and
`repair-retention-audit-and-reconcile` for `audit-publication`.

A settled degraded result exits `4`. Its human form is exactly:

```text
host generation handoff immutable audit continuity repair degraded
outcome: <DEGRADED_OUTCOME>
failure-branch: <CLOSED_CONTINUITY_FAILURE_BRANCH>
failure-class: <CLOSED_CONTINUITY_FAILURE_CLASS>
action: <ACTION_FROM_CONTINUITY_TABLE>
```

`DEGRADED_OUTCOME` is exactly `degraded-before-day-90 |
degraded-day-90-before-prune | degraded-day-90-after-prune`.
`CLOSED_CONTINUITY_FAILURE_BRANCH` is exactly `source | publication | retention`, and its
class must belong to that branch's closed terminal enum in `data-model.md`; the retention
branch uses `ContinuityRepairTerminalRetentionFailureClassV1`, not the broader retention
lifecycle enum.
Decision selection, settlement preparation, intent publication, and terminal publication
are disjoint from these enums and are not settled degraded failure classes. JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-continuity-repair-degraded","outcome":"<DEGRADED_OUTCOME>","failure":{"branch":"<CLOSED_CONTINUITY_FAILURE_BRANCH>","class":"<CLOSED_CONTINUITY_FAILURE_CLASS>"},"action":"<ACTION_FROM_CONTINUITY_TABLE>"}`.
No continuity form contains a replay handle, attempt, watermark, prune proof, evidence,
clock, boot identity, path, selector, argv, or free-form value. Real-procedure human/JSON
goldens pin each repaired variant, every degraded branch/class/action,
decision-basis pending without an intended outcome, decision-selection pending,
preparation-incomplete, and both later pending stages for all five intended outcomes.
Constructor and deserializer negatives reject an intended outcome or terminal failure in
the basis-pending form, `Complete` in any pending form, and any independently supplied
boundary that disagrees with its state. Across the four publication-pending types -
decision basis, decision selection, outcome intent, and terminal outcome - `Conflict` is a
sealed state variant carrying predecessor, existing digest, and candidate digest, never a
progress prefix or free failure field; basis pending alone omits `intendedOutcome`, while
the other three derive it only from their required durable predecessor. Human/JSON output
never renders any of those private digests or predecessor bytes.

The independent two-edge restoration and two-edge prune audit fixtures plus the unchanged
168-case broker registry preserve their exact ids and own only their literal caller,
request, artifact, legacy backup/restoration publication, conflict, and no-write cases.
They are supplemented, not replaced, by the mandatory read-independent 216-case
durable-record/boundary registry and 88-case lifecycle registry in `data-model.md`. The 216
cases own all nine boundaries for each listed amendment record class, including reservation,
both release reasons, settlement, repair-resume, and continuity-repair
pre/evidence/watermark/outcome. The 88 cases own aggregate storage and continuity-evidence
limits, standing-reserve exhaustion and corruption taxonomy, cycle-unique capacity
success/refusal/retry, malformed release/continuity prefixes, retention-anchor conflict,
continuity evidence compaction and both permit seals, transport-loss resubmission,
private-identifier/body canaries, and family shrinkage. The 156-case status registry and
unchanged 168-case registry cannot substitute
for either supplemental registry.

An
unaudited extra mutation instead returns the separate
`preserve-and-escalate-audit-integrity-incident` action and is not restoration-eligible.
No generic copy, force flag, daemon repair path, or new unit exists.

Public retention rendering is a total mapping over wire-emittable variants.
`CLOSED_RETENTION_CAPACITY_CLASS` is exactly
`intent-member-limit | intent-byte-limit | root-intent-limit | root-member-limit |
root-byte-limit | root-publication-record-limit | root-publication-byte-limit |
restoration-record-limit | restoration-byte-limit | restoration-attempt-limit |
continuity-evidence-record-limit | continuity-evidence-byte-limit |
pending-staging-record-limit | pending-staging-byte-limit`.
`continuity-repair-attempt-limit` remains a private trigger-only member of the internal
capacity class and is not a member of that public response set.
`CLOSED_CAPACITY_ADMISSION_CLASS` is exactly
`standing-reserve-exhausted`. `CLOSED_RETENTION_DEGRADED_CLASS` is exactly
`clock-rollback | clock-watermark | epoch-invalid | clock-forward-discontinuity |
clock-continuity-ambiguous | clock-overflow | unlink | directory-sync | census |
audit-publication | pending-settlement | standing-reserve-missing |
standing-reserve-overdrawn | standing-reserve-duplicated |
standing-reserve-unaccounted`:

| Closed failure class | Exact action |
| --- | --- |
| `intent-member-limit`, `intent-byte-limit`, `root-intent-limit`, `root-member-limit`, `root-byte-limit`, `root-publication-record-limit`, `root-publication-byte-limit`, `restoration-record-limit`, `restoration-byte-limit`, `restoration-attempt-limit`, `pending-staging-record-limit`, `pending-staging-byte-limit` | `reconcile-immutable-audit-retention` |
| `continuity-evidence-record-limit`, `continuity-evidence-byte-limit` | `repair-continuity-authoritative-source-contract` |
| `standing-reserve-exhausted` | `repair-retention-audit-and-reconcile` |
| `clock-rollback`, `clock-watermark`, `epoch-invalid`, `clock-forward-discontinuity`, `clock-continuity-ambiguous` | `repair-retention-clock-discontinuity` |
| `clock-overflow` | `preserve-and-escalate-retention-clock-overflow` |
| `unlink`, `directory-sync` | `repair-retention-storage-and-reconcile` |
| `census` | `repair-retention-census-and-reconcile` |
| `audit-publication`, `pending-settlement` | `repair-retention-audit-and-reconcile` |
| `standing-reserve-missing`, `standing-reserve-overdrawn`, `standing-reserve-duplicated`, `standing-reserve-unaccounted` | `preserve-and-escalate-audit-integrity-incident` |

The separate internal-only mapping
`continuity-repair-attempt-limit -> resume-oldest-continuity-cleanup` selects ordered
cleanup. It is not a public failure/action pair.

Continuity rendering uses this exact total extension; a `retention` branch uses the table
above:

| Continuity failure branch/class | Exact action |
| --- | --- |
| `source/source-unavailable` | `repair-continuity-authoritative-source` |
| `source/source-conflict` | `preserve-and-escalate-continuity-source-conflict` |
| `publication/hierarchy`, `publication/write`, `publication/file-sync`, `publication/link`, `publication/reopen`, `publication/directory-sync` | `repair-retention-storage-and-reconcile` |
| `publication/conflict` | `preserve-and-escalate-continuity-publication-conflict` |
| `publication/audit-publication` | `repair-retention-audit-and-reconcile` |
| settlement preparation `decision-pre-audit/audit-publication` | `repair-retention-audit-and-reconcile` |
| pending `decision-basis|decision-selection|outcome-intent|terminal-outcome` at `hierarchy|write|file-sync|link|reopen|directory-sync` | `repair-retention-storage-and-reconcile` |
| pending `decision-basis|decision-selection|outcome-intent|terminal-outcome` at `conflict` | `preserve-and-escalate-continuity-publication-conflict` |
| pending `decision-basis|decision-selection|outcome-intent|terminal-outcome` at `audit-publication` | `repair-retention-audit-and-reconcile` |

Every public action is executable or names one external procedure. The table also documents
the one internal continuation label in an explicitly non-emittable row:

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
| `use-unprivileged-local-admin-restoration-session` | site access administrator runs `host-generation-unprivileged-local-admin-restoration-session-v1`; the resulting unprivileged local Admin reruns the same one-artifact command |
| `reconcile-immutable-audit-retention` | site backup administrator runs `host-generation-immutable-audit-retention-reconciliation-v1`, which can invoke only the typed prune op through the existing public-socket Admin path and sealed coordinator capability |
| `repair-retention-clock-discontinuity` | site backup administrator repairs the configured authoritative time source, then an unprivileged public-socket `Admin` runs selector-free `host-generation-retention-clock-discontinuity-repair-v1` only as a wake signal; the sealed broker coordinator validates non-caller authoritative continuity evidence, consumes its private repair permit, preserves the original day-90 deadline across reboot or discontinuity, and prunes before completing repair when that deadline has passed |
| `repair-continuity-replay-key-generation` | site package administrator runs `host-generation-continuity-replay-key-generation-repair-v1`, which restores the release-sealed broker CSPRNG and root posture without supplying, rotating, or replacing a key; broker startup then resumes the exact typed publication prefix |
| `repair-continuity-authoritative-source` | site backup administrator runs `host-generation-continuity-authoritative-source-repair-v1`, which restores the disposition-pinned source version, authority, and exact replay-by-private-handle contract without accepting evidence from the operator; an unprivileged local public-socket `Admin` then reruns selector-free `host-generation-retention-clock-discontinuity-repair-v1` |
| `repair-continuity-authoritative-source-contract` | site package administrator runs versioned external procedure `host-generation-continuity-authoritative-source-contract-repair-v1`, reinstalls the release-sealed authoritative-source producer/consumer contract and its evidence record/byte limits, and has the disposition-pinned authority, never the operator, republish the same authoritative fact in canonical bounded form; an unprivileged local public-socket `Admin` then runs exactly `d2b-host-generation-deploy --repair-authorized-handoff [--json]` with no path, digest, selector, force flag, or other argument as the selector-free wake |
| `resume-oldest-continuity-cleanup` | internal only and non-emittable; broker startup or the existing idle wake resumes the oldest broker-target compaction, source release, and attempt-slice release in order; a block renders the exact owning closed failure and neither this label nor `continuity-repair-attempt-limit` is emitted as a substitute |
| `repair-continuity-source-storage-and-reconcile` | site backup administrator runs versioned external procedure `host-generation-continuity-source-storage-repair-v1`, restoring only underlying availability of the disposition-pinned source filesystem/provider - mount, space, inode availability, and service reachability - and must not edit, copy, truncate, rename, unlink, recreate, or reconcile the source root or any lifecycle byte; an unprivileged local public-socket `Admin` then runs exactly `d2b-host-generation-deploy --repair-authorized-handoff [--json]` with no path, digest, selector, force flag, or other argument, letting only sealed broker operation `ReconcileHostGenerationImmutableAuditContinuitySourcePrefixV1` publish immutable pre/outcome audit and resume the retained prefix |
| `preserve-and-escalate-continuity-source-conflict` | site security authority runs `host-generation-continuity-source-conflict-escalation-v1`, preserving the source, coordinator root, and immutable prefix and permitting no replacement, fallback evidence, prune, or retry until an accepted authority disposition names the source repair; after that repair, the site backup administrator and local Admin perform the authoritative-source repair and selector-free wake above |
| `preserve-and-escalate-continuity-publication-conflict` | site security authority runs `host-generation-continuity-publication-conflict-escalation-v1`, preserving the conflicting final, parent, coordinator root, and immutable prefix and permitting no unlink, replacement, copy, compaction, or retry until an accepted authority disposition resolves the exact publication identity; the site backup administrator then runs `host-generation-retention-storage-repair-v1` and a local Admin reruns the selector-free wake |
| `preserve-and-escalate-retention-clock-overflow` | site security authority runs `host-generation-retention-clock-overflow-escalation-v1`; no pruning or handoff retry is authorized |
| `repair-retention-storage-and-reconcile` | site backup administrator runs `host-generation-retention-storage-repair-v1`, then typed reconciliation; it never unlinks directly |
| `repair-retention-census-and-reconcile` | site backup administrator runs `host-generation-retention-census-repair-v1`, then typed reconciliation; it never edits an immutable census directly |
| `repair-retention-audit-and-reconcile` | site backup administrator runs `host-generation-retention-audit-repair-v1`, settles the immutable prefix through the broker, then reconciles |
| `resubmit-same-restoration-artifact` | rerun T595's restoration command with the exact same one artifact; the same unprivileged public-socket `Admin` authorization and byte-identical resubmission drive settlement |
| `repair-restoration-storage-and-resubmit` | site backup administrator runs `host-generation-restoration-storage-repair-v1`, then the operator resubmits the exact same artifact |
| `repair-restoration-client-broker-contract` | site package administrator runs `host-generation-restoration-client-broker-contract-repair-v1`, reinstalls one matching release-sealed client/broker generation, then reruns the same one-artifact command |
| `preserve-and-escalate-invalid-coordinator` | site security authority runs `host-generation-invalid-coordinator-escalation-v1` |
| `preserve-and-escalate-pointer-conflict` | site security authority runs `host-generation-pointer-conflict-escalation-v1` |
| `preserve-and-escalate-audit-restoration-conflict` | site security authority runs `host-generation-audit-restoration-conflict-escalation-v1` |
| `preserve-and-escalate-audit-integrity-incident` | site security authority runs `host-generation-audit-integrity-escalation-v1` |

Each escalation procedure preserves the coordinator and backup set, accepts only the
matching fixed error plus authenticated forensic evidence, and authorizes no repair, copy,
replace, delete, retry, or force action.
Dedicated operator/task goldens pin the complete owner, versioned procedure, and exact
selector-free wake command for
`repair-continuity-authoritative-source-contract` and
`repair-continuity-source-storage-and-reconcile`; a missing owner/procedure, alternate
command, added selector, or action-token substitution fails independently.

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
