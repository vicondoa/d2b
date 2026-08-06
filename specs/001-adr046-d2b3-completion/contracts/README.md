# Contracts: Complete the ADR-046 Provider Control Plane (d2b 3.0)

**Feature**: `001-adr046-d2b3-completion` | **Date**: 2026-07-29

d2b exposes contracts to operators, to guest components, to Nix consumers, and to sibling
desktop companions. This directory records **which contract surfaces this program must
deliver, change, or retire**, and what "done" means for each.

These files are an index and an obligation list. They do not restate field-level schemas: the
normative definitions live in the ADR-046 specification set, and the machine-readable form is
generated into `docs/reference/schemas/v3/` by `xtask`. Duplicating them here would create a
third source of truth that the drift gates do not check.

## Contract surfaces

| File | Surface | Consumers | Wave |
| --- | --- | --- | --- |
| [resource-api.md](./resource-api.md) | The `d2b.resource.v3` service and ComponentSession admission | In-Zone components, controllers, Providers | W2-W5 |
| [operator-cli.md](./operator-cli.md) | The `d2b` command surface | Human operators, companion tools | W5 |
| [nix-configuration.md](./nix-configuration.md) | `d2b.zones.<zone>.resources.*` option schema | Host configurations | W2, W5 |
| [generated-artifacts.md](./generated-artifacts.md) | Schemas, per-Zone bundles, UI colors, delivery artifacts | Broker, daemon, companions, drift gates | W2-W7 |
| [companion-contracts.md](./companion-contracts.md) | What the five desktop companions consume | Sibling repositories | W5 publish, W8 verify |

## Rules that apply to every surface

1. **Versioned, not silently changed.** Adding, removing, or renaming a field or operation
   bumps the relevant version, updates the paired schema and prose, and lands in the same
   change as the emitter (FR-031, constitution Principle IV).
2. **Generated artifacts are the contract.** The committed bytes under
   `docs/reference/schemas/v3/` are authoritative; `make test-drift` regenerates and requires
   a clean diff. A hand-edited schema is a gate failure, not a shortcut.
3. **Documentation ships with behavior.** A reference page affected by a change lands in the
   same wave, never deferred (FR-019).
4. **No compatibility layer.** 3.0 is a clean break. A surface being removed is removed, with
   a removal proof, in its own commit, after its successor is integrated and tested (FR-023).
5. **Retirement is explicit.** A capability may only disappear if it is on the retirement list
   with a justification and named in the release notes (FR-042).
6. **Nothing leaks.** No secret, credential, command output, raw host path, or PII crosses any
   of these surfaces into telemetry, audit, logs, or errors (FR-018).
7. **Production evidence crosses production boundaries.** Resource-plane acceptance enters
   through a registrar-admitted ComponentSession and the published ZoneBus route. A direct
   ResourceService or `WatchService` call, fixed subject, fake endpoint, independent readiness
   flag, disabled audit callback, or result from another commit cannot satisfy the
   `adr046w5` gate
   (FR-066-FR-072).
8. **One readiness projection, no partial publication.** Store, matching policy, session/router,
   controller endpoint, watch admission, audit catch-up, and the
   `d2b-core-controller`-owned `Provider/system-core` registration publish together with
   exactly one `Zone.status.handlers[]` record named `system-core-host` and one named
   `system-core-user`, each carrying `phase` and `lastReconciledAt` from its live handler, or
   the affected Zone refuses with remediation. `ProviderLifecycle` is distinct and cannot
   substitute. No other Wave 6 Provider dossier gates this wave (FR-069).
9. **Policy bootstrap is private and one-shot.** `ZoneResourceRuntime` may consume one sealed,
   non-`Clone` `PolicyBootstrapRead` for the first exact-revision policy-envelope snapshot.
   It carries no public Resource API subject or general read/mutation surface; all later
   policy access uses authenticated Resource API revision rules, and both store crates remain
   policy-neutral (FR-067, FR-073).
10. **A committed mutation is never reported as rolled back.** Until authoritative audit
    append and outbox completion finish, return operation-bound semantic
    `CommittedPendingAudit` through the existing layered `ResourceStatus`:
    `phase = ResourcePhase::Degraded`,
    `outcome.code = StatusCode("committed-pending-audit")`,
    `update.state = UpdateState::Blocked`, and
    `update.operation_id = Some(original_operation_id)`. Existing condition, outcome, and
    update detail stays bounded and redacted. `ResourceUpdateStatus` gains no phase/code
    member, and no enum variant, field, or schema version is added. Keep the Zone unpublished
    and make same-ID retry observe rather than reapply the mutation (FR-070).
11. **Amended-plan resume is receipt-bound.** T603 is the sole direct prerequisite of T589
    and writes the closed external reconciliation receipt. If all rows are satisfied and the
    analysis/panel identities pass, only an identity-verifying `/d2b-spec-edit` batch may
    check T073-T218 and T603. T589 also requires that editor progress receipt and the checked
    task set (FR-072, SC-034).
12. **Operator activation is acceptance evidence.** T604 starts from the emitted Nix
    resource declaration and per-Zone bundle, activates through the production daemon,
    observes a real owned effect/readiness or a precise actionable refusal for the
    representative Guest, Volume, Network, and Device, then removes one declaration and
    proves dependency-safe cleanup without disturbing unrelated resources. The exact
    candidate result is required by T600, T601, T602, and T219.
13. **C1 is a coordinated unreleased-v3 correction.** Constitution 2.2.0 authorizes T605 to
    add `ZoneHandlerName::SystemCoreHost` and `ZoneHandlerName::SystemCoreUser`, serialized by
    the existing kebab-case rule, with targeted Rust/contract tests, compiler-derived public
    and private API snapshots, paired reference status text, and byte-identical Zone
    desired-schema proof. T595 consumes the variants and T599 reconciles other consumers in
    the same Wave 5 PR. No `apiVersion`, `schemaVersion`, `manifestVersion`, `bundleVersion`,
    or wire-field version changes because no field, operation, or desired Zone schema
    changes and v3 is unreleased. Implementation remains pending.
