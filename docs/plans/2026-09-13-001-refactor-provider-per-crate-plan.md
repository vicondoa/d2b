---
title: "Provider-per-crate resource restructure - Plan"
created_at: 2026-09-13
topic: provider-per-crate-restructure
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-brainstorm
execution: code
---

# Provider-per-crate resource restructure - Plan

## Goal Capsule

- **Objective:** Implement issue #516 off `v3` - every resource type's driver moves out of d2bd into its own `d2b-provider-<type>` crate behind a `DriverDescriptor` registry; dependencies, broker operations, authority, readiness, and host integration become committed declaration rows; the CLI, daemon, broker, and Nix layers consume the declarations generically; the toolkit becomes the standard `ProviderBase` framework. Delivery is the whole program as six phased milestones.
- **Product authority:** The design of record is issue vicondoa/d2b#516 - the main design comment (§1-§9, incl. §5.1-§5.12) plus the D1-D23 change-log comment, both by vicondoa. This plan binds to those documents and cites them instead of restating their mechanisms; where a line anchor drifted during review, the verified anchors are: `DispatchBackend` at `packages/d2b-broker/src/runtime.rs:8052`, `runner_role_for_process_role` at `:10555`, `device_worker_posture` at `packages/d2b-core/src/bundle_resolver.rs:4660`.
- **Open blockers:** none. The `ZoneBootstrap`/`ZoneEnroll` handlers (unlanded) are a sequenced in-scope prerequisite (R24), not a blocker.

## Product Contract

### Summary

Restructure the resource plane so that resource knowledge exists only inside per-type provider crates: drivers declare themselves (`DriverDescriptor`), dependencies and children are declared (`ChildCreation`), broker operations are committed `Operation` rows with handler code on the declaring driver, authority is `Role`/`RoleBinding` rows, and the CLI, daemon, broker, compiler, audit, and Nix layers consume generated catalogs instead of hand-maintained tables. Done = all families migrated, all gates green, and the policy-check allowlist empty.

### Problem Frame

Adding or changing one resource type today requires edits in four to six places across at least four crates, and resource knowledge lives in shared code: the daemon's plane builder and decoder table (`packages/d2bd/src/resource_plane_v3.rs:1323/1701/1763`), the broker's dispatch monolith and five parallel op catalogs, the resolver's posture and intent tables (`packages/d2b-core/src/bundle_resolver.rs:3273/4660`), Nix registries copied five times, and audit lists that have already drifted. The v3 rewrite's own conversion (U17) needed a compiler projection, a resolver posture table, a supervisor branch, a broker ACL grant, a state-provisioning change, two provider-declaration fixes, and a new VM fixture - each discovered separately. Issue #516 records the full leak inventory; the remedy is this restructure.

### Key Decisions

- KD1. **One driver crate per resource type** - realizer crates stay separate, type-prefixed (`d2b-provider-<type>-<variant>`). (session-settled: user-directed - chosen over combining minijail/systemd and 1:1 drivers into shared crates: separation by type is the organizing principle and crate count is not a cost.)
- KD2. **Operation grants live inline on Role rows** (`operationRefs`/`commandRefs`); no separate policy-row intermediary. (session-settled: user-directed - chosen over the proposed `BrokerPolicy` rows: a (role, op) pair has no independent lifecycle.)
- KD3. **Readiness is driver-managed** via reconcile causes and the reverse-target index; no declared readiness rules or platform readiness gates. (session-settled: user-directed - chosen over the proposed `ReadinessRule` declarations: platform layers should not encode driver readiness semantics.)
- KD4. **Ref naming is `<semantic>Ref(s)`** - `roleRef`, `subjectRefs`, `resourceRefs`, `zoneRefs`, `executionRefs`, `seccompRef`, `principalRef`, `commandRef`, `operationRefs`, `providerRef(s)`, `ownerRef`; no `target`/`targetRefs` anywhere. (session-settled: user-directed - chosen over the proposed `TargetRef` scheme: the semantic prefix already says what is referenced.)
- KD5. **Command/Ops parameter validation is JSON Schema** - no invented parameter kinds. (session-settled: user-directed - chosen over an ad-hoc `kind` enum: JSON Schema is the standard already in the stack, with `writeOnly` for secrets and `default` for optional params.)
- KD6. **Principal uid/gid are generator-allocated and persisted in the committed catalog**; manifests declare principals by name only. (session-settled: user-directed - chosen over hand-picked static uids: collision-free by construction, stable across restarts and hosts.)
- KD7. **Broker operations carry their executable on the declaring driver's descriptor** (`OperationDef.handler`); the registry is the handler table. (session-settled: user-approved - proposed with the shape surfaced in D4; the user directed that the declaration provide the code, which this satisfies.)
- KD8. **Delivery boundary: whole program, phased** - six milestones per §8 sequencing, not a first slice. (session-settled: user-directed - chosen over first-slice and broker-deferred options.)
- KD9. **Done = full migration with the policy-check allowlist empty.** (session-settled: user-directed - chosen over documented-exceptions.)
- KD10. **Work happens on `v3`** (in-flight rewrite changes already merged), **gates suffice** with milestone reviews after §8 phases 2/4/6, **phase-revertible commits** on gate failure, **Process pilots** steps 1-3. (session-settled: user-directed/user-approved across the scoping questions.)

### Requirements

**Declaration model**

- R1. Every resource type's driver lives in its own `d2b-provider-<type>` crate and is declared by exactly one `DriverDescriptor` (single `resource_type`); d2bd retains no driver code.
- R2. `DriverDescriptor` carries: `allowedSources` (BUILTIN | STARTUP | RUNTIME bitmask; absence of RUNTIME = required at plane open), `operations` (each an `OperationDef` carrying `operationRef` + handler code), `creations`, `decoder`, `factory`, `startup` steps, `services` (full `ServiceDecl` metadata), supported resource `verbs`, `execution` domains, `exportable`, and declared `reads`.
- R3. Each provider declares a `ProviderDeclaration`: `providerRef`, structurally scoped `self_bindings`, `required`, `plane_adapters`, principals by name, `storage_roots` (confined to the provider's declared subtree; the generator refuses overlaps and path escapes), `cardinality`, and `isolation_posture`.
- R4. The `DriverRegistry` enforces: duplicate-type refusal, duplicate/foreign `operationRef` refusal on the handler table, mask-gated registration source, and the presence obligation at plane open (types without a RUNTIME bit must be registered or startup fails, naming the driver).
- R5. `ChildCreation` carries `{ child, providerRef, custody (DriverOwned | ControllerOwned), order }`; `create_child` takes the declaration handle and terminally refuses any undeclared creation, naming the family and child type.

**Committed rows**

- R6. `Operation` rows declare: `payloadSchema` (JSON Schema; `writeOnly` fields must not carry `default`/`enum`/`const`/`examples` values, and validation errors redact their values), `destructive`, `secretAccess` ceiling, `audit` facet (required/mode/retainedFields/redaction keys + `targetLabel`), `auditJoin` (canonical-JSON over declared authoritative fields, or null), `profile`/`authority` (surface cli|broker, domain host|guest, caller authority, brokerRequirement), `fds` (request/response/preopened), `bounds`, `payloadProvenance`, and `wireTag` for W3-inherited ops.
- R7. Every `BrokerRequest` variant is owned via the three-way triage - family row, broker-generic row, or explicitly transport-excluded - with the triage recorded in §5.3.1's ownership table before phase 4 closes.
- R8. `Command` rows declare `exec`, `argv` with placeholder slots validated against `params` at seed, `params` as JSON Schema (`writeOnly` secrets; `default` shrinks `required`), `roleRef`, and an `intent` facet (grammar + mint rule) replacing the resolver's per-family intent tables; `Command` rows are BUILTIN and seed-fixed.
- R9. `Role` rows carry the authority facet (rules constrained to the type's declared verbs, `operationRefs`, `commandRefs`, `subresources`) and the posture facet (`seccompRef`, `principalRef`, caps, namespaces, mounts, umask, userNs).
- R10. `RoleBinding` rows carry `roleRef`, `subjectRefs` (declared subject set Zone|User|Provider|Host|Guest|Process|Group - one declaration replacing four copies), `resourceRefs`, `zoneRefs`, `executionRefs` (per-field type-gates), and `relayAuthority`.
- R11. `SeccompProfile` rows carry device-node binds; `Principal` rows carry generator-allocated uid/gid persisted in the committed catalog, with `{prefix}-{range}` templates for per-device principals; `host-users.nix` is generated from them.

**Migration**

- R12. All drivers move per the §4 crate map: new crates `d2b-provider-volume`, `-endpoint`, `-guest`, `-credential`, `-process` (folding in `d2b-process`), `-ephemeral-process`; renames `runtime-*` → `d2b-provider-guest-*` and `system-minijail`/`system-systemd` → `d2b-provider-process-minijail`/`-systemd`; Usb/SecurityKey/Telemetry drivers into per-type crates with existing realizers unchanged; controller-family drivers into nine per-type crates plus `d2b-provider-command`/`-operation`/`-seccomp-profile`; Host/User into `d2b-provider-host`/`-user`. d2bd retains no driver code. Every new or renamed crate registers in the Bazel graph (`bazel/checks/BUILD.bazel` suite and the root srcs list) - the policy check reads that registration.
- R13. `d2b-core-controller` slims to non-resource controller-session machinery (assignment transport, coordinator, migration); its domain modules move to the type crates that own them.
- R14. d2bd composition consumes only generic mechanisms: startup steps in derived order, effects keyed (driver, providerRef), plane adapters from `ProviderDeclaration`, watch-hub row-change invalidation, and setup projections as declared startup hooks. All per-family branches, planners, path adapters, and provider literals are deleted.
- R15. `resource_runtime` consumes reconcile causes and the reverse-target index; per-family readiness gates, presence scanners, the CH child list, and the `desired_lifecycle` dispatch are deleted.

**Broker seam**

- R16. Broker dispatch is the generic envelope (resolve Operation row → validate payload against its schema → authorize against committed grants → audit → dispatch to `OperationDef.handler`); `DispatchBackend` generalizes and the per-variant match shrinks to zero family arms.
- R17. The five op catalogs (wire enum check, `BrokerProfile`, `W3BrokerOperation`, privileges rows, audit `OperationFields`) are generated views of the committed rows, and completeness gates fail CI on any variant/row/profile/authz/audit mismatch.

**Layer adoption**

- R18. The CLI consumes the services/verbs/execution/exportable/isolation catalogs: typed-noun dispatch, per-type gating literals, attach-kind/stream enums, share exportability heuristics, and unsafe-local literals are deleted; completion extends the existing `cliProjection` discovery to drivers.
- R19. Nix/xtask generate every closed inventory from manifests: the five type-registry copies, three projection-owner tables, `PROVIDER_MATRIX` packaging columns, schema-pointer tables, subject/relayBound vocabularies, controlTypes, and the hand field vocabularies - including the fourth subject-vocabulary copy in `options-zones-resources.nix`.
- R20. `d2b-resource-compiler` projections are catalog-driven from `creations`/`commands`/`operations`; the per-family arms, `ADDITIONAL_RESOURCE_TYPES`, owner/provider literals, bootstrap external-reference list, and hand secret-shape policy are deleted.
- R21. Audit and telemetry type/provider lists and op vocabularies are generated from the catalog; per-class audit value domains key to catalog ids.

**Toolkit**

- R22. The toolkit owns the provider lifecycle via `ProviderBase` + `run()`/`run_guest()`: fd10 bootstrap (generalized off the CredentialProvider-bound entrypoint), readiness marker, admission, plane-attach orchestration, startup-step execution, operation envelope, audit ring, drain ordering, and the test harness; adopt-or-delete resolves every zero-user module.
- R23. The test harness runs the real fences (schema, creations, envelope, audit) with only effect ports faked, and standardizes five test kinds - reconcile, operations, creation fence, fault, conformance - replacing the eight hand-rolled per-crate drivers.
- R24. The guest target runs guest agents on the same base over vsock, with the `ZoneBootstrap`/`ZoneEnroll` handlers landed as a sequenced prerequisite; guest agents live in their type crates under a guest build target.

**Program gates**

- R25. The §7 policy check fails CI on any resource knowledge outside provider crates (type tables, provider-id strings, per-family branches, seccomp/authz tables, launch-intent tables), on manifest↔descriptor disagreement, on self-binding scope escapes (a binding whose subject is not the declaring provider or whose role is not self-declared), and on system-zone writes from non-foundation planes; its allowlist shrinks monotonically to **empty** at done.
- R26. Every §8 phase lands phase-revertible (a failed gate rolls the phase back via git revert) with all four gates green: `make check`, the host-integration lane, the grep gates, and no fixture weakened.
- R27. Milestone reviews happen after §8 phases 2, 4, and 6; the work proceeds on `v3` with no dependency on further rewrite merges.

### Actors

- **Operators** (`User` rows): author roles/bindings via the API, review milestones.
- **Foundation seed**: commits the system-zone vocabulary (roles, commands, operations, profiles, principals, self-bindings, operator bindings from nix).
- **Drivers** (per-type, in provider crates): reconcile resources, declare creations/operations/services.
- **Providers** (session principals): admit per-zone sessions, realize children, own plane adapters.
- **Process controller**: materializes `Operation` rows from `Command` rows; executes spawns.
- **Broker**: the generic envelope - admission, validation, authz, audit, dispatch.

### Key Flows

- F1. **Seed order** (§5.6): system zone → provider identities → roles → commands → self-bindings → controller materializes Operations → operator bindings from nix → plane opens with presence obligations checked.
- F2. **Binding-gated guest start**: bindings target the Guest; each Ready independently; binding changes fire `DependentChanged`; the guest driver reads `targeted_by(guestRef)`, gates the start on its own policy, publishes the blocking binding in status conditions, launches when all Ready. Covered by R15, R24.
- F3. **Operation invocation**: authenticated session → envelope (row resolution → schema validation → grant check → audit with `invocationId`/join) → descriptor-resolved handler. Covered by R6, R16.
- F4. **Family migration unit** (repeated per family in phase 3): move driver → declare descriptor/creations/operations → delete that family's broker tables and d2bd wiring → gates green.

### Acceptance Examples

- AE1. **New family, zero outside edits** - Covers R1, R25
  - **Given:** a new resource family "Foo" as a crate with its manifest and driver.
  - **When:** it is registered and the workspace member line is added.
  - **Then:** `make check` and the policy check pass with no edit to d2bd, d2b-broker, d2b-core, or Nix hand tables, including the regenerated catalog/artifacts passing their golden-hash gates.
- AE2. **Resource knowledge in shared code fails CI** - Covers R25
  - **Given:** a type-name match arm or provider-id string is introduced into `d2b-broker`, `d2bd`, `d2b-core`, or `d2b-contracts*`.
  - **When:** the policy check runs.
  - **Then:** CI fails naming the file and the offending symbol.
- AE3. **Undeclared creation refused** - Covers R5
  - **Given:** a driver calls `create_child` with a (child, provider) pair absent from its `creations`.
  - **When:** the call is made.
  - **Then:** terminal refusal naming the family and child type; no row is written.
- AE4. **Operation deny-by-default** - Covers R6, R16
  - **Given:** an ungranted caller invokes a committed operation.
  - **When:** the envelope authorizes.
  - **Then:** refusal with an audit record carrying the `invocationId`; a granted caller's invocation produces the auditJoin key declared by the row.
- AE5. **Guest start gated on bindings** - Covers R15, R24
  - **Given:** a Guest with two bindings not yet Ready.
  - **When:** each binding reaches Ready.
  - **Then:** the Guest stays NotYet naming the pending binding, then launches; the platform layers hold no binding knowledge.
- AE6. **Command placeholder drift fails closed** - Covers R8
  - **Given:** a Command argv placeholder names no `params` property (or an instance param is unknown).
  - **When:** seed or spec decode runs.
  - **Then:** refusal naming the command and the offending placeholder/param.
- AE7. **Phase revert on gate failure** - Covers R26
  - **Given:** a phase whose host-integration lane fails after landing.
  - **When:** the rollback policy executes.
  - **Then:** `git revert` restores a green mainline; the phase re-lands only with gates green.

### Scope Boundaries

**Outside this product's identity:**
- Any operator-visible behavior change - the same resources, rows, launcher vocabulary, and audit records must keep working.
- Dynamic provider/plugin loading - all drivers link at build time; "runtime" provisioning means late in-process registration.

**Deferred for later:**
- `d2b-core-controller` rename or further split beyond slimming it to session machinery - follow-up issue.
- Guest OS account management - the guest image owns its accounts; only the enrollment contract is in scope (R24).

### How This Work Fits Together

<!-- ce-section: work-relationships -->
- This plan owns the per-provider-crate restructure: declaration model, committed rows, driver extraction, broker seam, layer adoption, toolkit framework, and the program gates.
- **Depends on:** the v3 resource-runtime rewrite (docs/plans/2026-09-09-001-refactor-v3-resource-runtime-rewrite-plan.md) - already merged to `v3`; this plan starts from that state.
- **Enables:** closing #498 (drivers declare what they own, create, and read - the descriptor carries it).
- **Shares:** #506 (provider-specified resources - the manifest/Operation model is its implementation vehicle) and #505 (Volume/VolumeBinding authorship - binding rows are its model).
- **Still to decide:** `d2b-core-controller` rename/further split (follow-up issue).
- **Can proceed independently of:** nothing in the current rewrite - this plan is the follow-up the issue designates.

### Outstanding Questions

- *Deferred to Planning:* exact `TestHarness` API surface, manifest file schema details, envelope dispatch internals, and the golden-shutdown-order test mechanics.
- *Resolve Before Planning:* none.


## Planning Contract

*Product Contract preservation: unchanged (meanings, R-IDs, and scope boundaries carried verbatim).*

- KTD1. **Milestone unit granularity** - one implementation unit per family move; framework work is its own units. (session-settled: user-approved - chosen over cluster-grouped family moves: each family lands atomically with its own gate run. Same-shape sibling crates (the six interaction types, the 1:1 trio) may share one unit with per-crate checklists.)
- KTD2. **Harness sequencing** - the toolkit `TestHarness` lands in Milestone 2; the Process pilot (Milestone 1) runs on existing per-crate test infrastructure and is migrated to the harness in Milestone 2. (session-settled: user-approved - chosen over harness-first: the pilot calibrates the harness design.)
- KTD3. **R7 variant-triage source** - the 86-disposition-row table (97 lines) at `docs/reference/broker-w2-dispositions.md` seeds the variant-ownership triage; compile-time-only and stubbed rows retire, the rest assign to families; variants absent from the table (e.g. `ConsumeLifecycleLease`) triage directly as transport-excluded.
- KTD4. **Golden-order guards** - startup-step derived order and adapter-derived drain order are each pinned by a one-time golden test against today's known-good sequences (mirrors the v3 plan's no-fixture-weakened discipline; `docs/plans/2026-09-09-001-refactor-v3-resource-runtime-rewrite-plan.md:944`).
- KTD5. **ADR-046 register compliance** - the declaration model inherits frozen decisions D101 (canonical JSON), D102 (store-generated UUIDv4 uid), D119 (single digest chain), D121 (no floats) from `docs/specs/ADR-046-decision-register.md`; declaration rows use canonical JSON and store-generated uids.
- KTD6. **Generated-view mechanics** - xtask `gen-` commands emit the five broker catalog views, the Nix registries/projection tables, `host-users.nix`, and the principal-allocation file; each is golden-hash gated, and CI fails on hand-edit.
- KTD8. **Handler execution placement** - the broker is a standalone binary that links no provider crate (preserving the issue's dependency constraint); `OperationDef.handler` code executes in the declaring crate's process, and the broker forwards validated, authorized calls over the bus to it. The descriptor handler table lives in d2bd's registry; the broker holds only the committed rows. (session-settled: user-approved - surfaced by the adversarial review; chosen over linking provider crates into the broker.)
- KTD9. **Seed declare-then-validate** - the seed writes all rows, then runs ref resolution over the committed set, so the Command↔Role reference cycle resolves without ordering hacks.
- KTD7. **Policy-check allowlist mechanics** - the §7 allowlist lives in `packages/xtask/src/provider_crate_policy.rs` beside the existing `README_ONLY_INTEGRATION_RATCHET` (`:319`); entries require a named owner and a retirement milestone; the ratchet fails CI on any growth.

### Sources & Research

- Six read-only sweeps (CLI, d2bd wiring, broker, core/compiler, api/runtime/audit, Nix/xtask) + a five-scout verification pass - ~190 hardcoded sites classified, ~25 round-2 residuals closed in-design (issue #516, §5.10 round-2 table).
- Claim verification: 12/13 confirmed; `docs/plans/` existence refuted-and-incorporated (work-relationships); anchors corrected as cited in Goal Capsule.
- Institutional learnings: no `docs/solutions/` corpus exists (confirmed repo-wide); nearest stores swept - v3 rewrite plan disciplines, ADR-046 decision register (D101/D102/D119/D121), `docs/reference/broker-w2-dispositions.md` (86-row triage seed), `xtask` ratchets. External research: skipped - settled design, strong local patterns (announced).

## Implementation Units

### U1. Invert the Process effects port (pilot)
- **Milestone:** M1
- **Goal:** the Process effects port moves to the Process family crate; d2bd provides the production impl; blocked accessors become public.
- **Requirements:** R2, R12 (the §8 step-1 pilot; Product Contract KD10 calibration).
- **Dependencies:** none.
- **Files:** `packages/d2b-process/src/` (port trait; the crate folds into `d2b-provider-process` per R12 at U5), `packages/d2bd/src/process_driver.rs` (impl stays), `packages/d2b-provider-process-minijail/src/`, `packages/d2b-provider-process-systemd/src/` (exported consts).
- **Approach:** move the trait, keep the impl in d2bd; measure the diff - it calibrates every later family.
- **Patterns to follow:** existing `ProcessEffectBackend` split in `packages/d2b-process/src/`.
- **Test scenarios:**
  - Existing process driver tests pass unchanged against the moved trait.
  - A d2bd compile without the impl fails (inversion proven).
- **Verification:** `make check` green; diff size recorded in the plan status note.

### U2. `d2b-resource-types` crate
- **Milestone:** M2
- **Goal:** shared declaration vocabulary.
- **Requirements:** R1, R2, R3, R5.
- **Dependencies:** none.
- **Files:** new `packages/d2b-resource-types/src/` (descriptor, allowed sources, child creation, provider declaration, service decl, principal/storage types), workspace member.
- **Approach:** types only - no runtime logic; depends on `d2b-contracts-resource` + `d2b-resource-runtime`.
- **Test scenarios:**
  - Type-shape tests: mask semantics (no RUNTIME bit = required), creation field round-trip.
- **Verification:** crate compiles standalone; no dependencies beyond the declared two.

### U3. `DriverRegistry` + seed order + generated catalog
- **Milestone:** M2
- **Goal:** runtime registry replacing `build_providers`/`decoders` aggregation; generated const catalog; seed order 0-7.
- **Requirements:** R4; §5.6.
- **Dependencies:** U2 (`d2b-resource-runtime` gains its dependency on `d2b-resource-types`).
- **Files:** `packages/d2b-resource-runtime/src/provider.rs` (generalize), `packages/d2bd/src/resource_plane_v3.rs` (consume registry), `packages/xtask/src/` (gen command), `packages/d2b-contracts/src/generated/` (emitted catalog).
- **Approach:** descriptors from per-crate `contribute()`; xtask-generated aggregator invokes them; startup cross-check registry ↔ catalog; seed order per §5.6.
- **Test scenarios:**
  - Duplicate type registration fails named.
  - Registry ↔ catalog mismatch fails startup named.
  - A type without RUNTIME bit missing at plane open fails named.
- **Verification:** the Process family registers through the registry and both `build_providers()` and `decoders()` lose their Process arms (full deletion lands with each family's move).

### U4. Toolkit `ProviderBase` + `run()`/`run_guest()` + harness
- **Milestone:** M2
- **Goal:** the framework base and the five-kind test harness (§5.12).
- **Requirements:** R22, R23.
- **Dependencies:** U2, U3.
- **Files:** `packages/d2b-provider-toolkit/src/` (base/, server/ merge, testing/), delete `agent.rs:389/:438` duplicates, `fd10.rs:1491` op list → declared ops.
- **Approach:** generalize `run_from_fd10` off the credential binding; adopt-or-delete zero-user modules per §5.12.
- **Test scenarios:**
  - Credential provider (canonical example) migrates to the base and passes its existing suite through the harness.
  - Harness drives reconcile/operations/creations/fault kinds against fake ports.
  - Malicious-provider negative suite (`tests/malicious_provider.rs`) passes.
- **Verification:** toolkit module list equals the base; zero-user modules gone.

### U5. Process family end-to-end (template move)
- **Milestone:** M3
- **Goal:** the full per-family migration pattern, executed once.
- **Requirements:** R1, R8, R12, R14; Product Contract KD10 pilot.
- **Dependencies:** U1-U4.
- **Files:** `packages/d2b-provider-process/` (new - folds in `d2b-process` primitives; descriptor + Command type + spawn executor), `packages/d2b-provider-process-minijail/`+`-systemd/` (renames, exported consts), `packages/d2bd/src/process_driver.rs` (deleted), `packages/xtask/src/provider_crate_policy.rs` (minimal ratchet entry).
- **Approach:** follow the F4 unit flow - move, declare, delete the d2bd wiring, register via the descriptor. Broker process arms stay until the U16 cutover (dual-run); the minimal allowlist-ratchet check lands here so the shrink is monotone from the first family move.
- **Test scenarios:**
  - Process/EphemeralProcess reconcile + launch through the registry.
  - Command placeholder/param seed validation (AE6).
  - Spawn authorization: ungranted caller refused + audited (AE4 shape).
- **Verification:** gates green; the family's broker tables and d2bd wiring deleted.

### U6. Volume + VolumeBinding (multi-provider proof)
- **Milestone:** M3
- **Goal:** prove the multi-provider inversion (two realizers behind one driver).
- **Requirements:** R1, R5 (custody), R12.
- **Dependencies:** U5.
- **Files:** new `packages/d2b-provider-volume/`, `-volume-binding/`; realizers unchanged.
- **Approach:** creations pin per-child providerRef; binding-gated flows ride causes.
- **Test scenarios:**
  - Binding-gated flows via causes + targeted_by (F2 shape).
  - Multi-provider creation ordering (custody ranks).
- **Verification:** gates green; `volume_driver.rs`/`binding_driver.rs` deleted.

### U7. Endpoint family move
- **Milestone:** M3
- **Goal/Requirements/Dependencies:** as U6 pattern; R1, R12.
- **Files:** `packages/d2b-provider-endpoint/`.
- **Test scenarios:** endpoint reconcile + purpose derivation through ports.
- **Verification:** gates green; `endpoint_driver.rs` deleted.

### U8. Credential family move
- **Milestone:** M3
- **Goal:** credential driver + backend unification onto the base.
- **Requirements:** R1, R12; R10 (subjectRefs Credential targets).
- **Dependencies:** U5.
- **Files:** `packages/d2b-provider-credential/`; backends stay as realizers.
- **Test scenarios:** credential mint (Process child via minijail) through declared creation; use-credential rule path.
- **Verification:** gates green; `credential_driver.rs` deleted.

### U9. Activation + Telemetry moves
- **Milestone:** M3
- **Goal:** single-provider families onto the base.
- **Requirements:** R1, R12.
- **Dependencies:** U5.
- **Files:** `d2b-provider-activation-nixos/`, `d2b-provider-observability-otel/` + per-type telemetry crates.
- **Test scenarios:** activation runner creation; telemetry binding status.
- **Verification:** gates green; activation/telemetry driver code deleted from d2bd.

### U10. Guest family move
- **Milestone:** M3
- **Goal:** guest driver + per-provider effect keying + controller-owned custody.
- **Requirements:** R1, R5, R12, R15.
- **Dependencies:** U6 (multi-provider pattern).
- **Files:** new `packages/d2b-provider-guest/`; runtime realizers renamed to `d2b-provider-guest-*`.
- **Test scenarios:**
  - Guest reconcile on DependentChanged/TargetChanged causes.
  - Controller-owned custody: CH children relist/finalize via custody declaration.
  - Per-provider effect keying (driver, providerRef) dispatch.
- **Verification:** gates green; `guest_driver.rs`/`guest_effects.rs` kind matches deleted.

### U11. Interaction six per-type crates
- **Milestone:** M3
- **Goal:** split the interaction driver by type.
- **Requirements:** R1, R12; D2 `ServiceDecl` metadata.
- **Dependencies:** U5.
- **Files:** six new crates (wayland-policy/wayland-session/audio-service/audio-binding/shell-pool/shell-session).
- **Test scenarios:** per-type reconcile; ComponentSession endpoint policy from ServiceDecl.
- **Verification:** gates green; `interaction_driver.rs` deleted.

### U12. Network + Usb + SecurityKey moves (1:1)
- **Milestone:** M3
- **Goal:** drivers into existing realizer crates.
- **Requirements:** R1, R12.
- **Dependencies:** U5.
- **Files:** `d2b-provider-network/` (renamed), `d2b-provider-device-usbip/`, `d2b-provider-device-security-key/`.
- **Test scenarios:** per-family reconcile + creations; usbip scope from declaration.
- **Verification:** gates green; `shared_provider_driver.rs` deleted.

### U13. Core controller family + policy-type drivers
- **Milestone:** M3
- **Goal:** nine controller-family per-type crates + command/operation/seccomp-profile drivers; core-controller slims.
- **Requirements:** R1, R12, R13.
- **Dependencies:** U5.
- **Files:** nine new crates + `d2b-provider-command/-operation/-seccomp-profile`; `d2b-core-controller/` slimmed.
- **Test scenarios:** per-type driver tests moved; rbac/zone-links/providers domain tests pass in new homes.
- **Verification:** gates green; `core_driver.rs` and the extracted domain modules deleted from core-controller.

### U14. Host/User split
- **Milestone:** M3
- **Goal:** Host/User drivers out of system-core.
- **Requirements:** R1, R12.
- **Dependencies:** U5.
- **Files:** `d2b-provider-host/`, `d2b-provider-user/`; system-core keeps the reconciler realizer.
- **Test scenarios:** host/user reconcile through the registry.
- **Verification:** gates green; `system_core_driver.rs` deleted.

### U15. Committed policy rows (Operation/Command/Role/RoleBinding/SeccompProfile/Principal)
- **Milestone:** M4
- **Goal:** the seed commits all policy rows per §5.6 order; controller materialization live.
- **Requirements:** R6, R8, R9, R10, R11.
- **Dependencies:** U3, U5, U13.
- **Files:** seed/foundation plane code in d2bd, `d2b-provider-command/-operation/-seccomp-profile` drivers, principal allocation file.
- **Approach:** the seed uses declare-then-validate (write all rows, then run ref resolution over the committed set) so the Command↔Role reference cycle resolves; the process controller materializes Operations from Commands (its self-binding authorizes it); principal ids come from the committed allocation file; the system-zone write fence (foundation-plane writes only) is enforced from this unit onward.
- **Test scenarios:**
  - Seed commit order refuses unresolved refs (roleRef/principalRef/seccompRef).
  - Materialization is authorized by the controller self-binding only.
  - Principal allocation reuse across restarts (ids stable).
- **Verification:** gates green; system-zone rows queryable per zone projection.

### U16. Broker envelope + variant triage + generated views
- **Milestone:** M4
- **Goal:** generic dispatch live; every variant owned; five catalogs generated with completeness gates.
- **Requirements:** R7, R16, R17.
- **Dependencies:** U15.
- **Files:** `packages/d2b-broker/src/runtime.rs`, `packages/d2b-contracts-broker/src/`, `packages/xtask/src/` (view generators), `packages/d2b-core/src/privileges.rs` (rows replace matrices).
- **Approach:** family-by-family arm shrink; triage seeds from `docs/reference/broker-w2-dispositions.md` (86 rows; absent variants triage directly); transport-excluded variants documented. Handler execution placement: the broker is a standalone binary and links no provider crate - validated, authorized calls forward over the bus to the declaring crate's process, where the descriptor handler table lives (KTD8).
- **Test scenarios:**
  - Completeness gates: variant ↔ row ↔ profile ↔ authz ↔ audit-shape exact match.
  - Deny-by-default: unknown op, ungranted caller, uncommitted row (AE4).
  - Envelope op invocation end-to-end (new-op path, no enum edit).
- **Verification:** dispatch match contains zero family names; spot-check tests replaced by full-catalog gates.

### U17. Nix/xtask generated inventories + host-contract aggregation
- **Milestone:** M5
- **Goal:** every Nix closed inventory generated; host contract (bindings section, principals, storage roots) aggregated.
- **Requirements:** R19; R11.
- **Dependencies:** U15, U16.
- **Files:** `packages/xtask/src/provider_packaging.rs`, `nixos-modules/generated/*`, `nixos-modules/host-users.nix`, `host-daemon.nix`, `options-zones-resources.nix`, `resources-zone-control.nix`.
- **Test scenarios:** golden-hash gating on every generated file; fourth-copy subject vocabulary consumes the declaration.
- **Verification:** hand inventories deleted; generation idempotent.
- **Execution note:** mostly packaging/generation - prefer generation-idempotence verification over unit coverage.

### U18. CLI + compiler + audit/telemetry adoption
- **Milestone:** M5
- **Goal:** the client-facing layers consume the catalogs.
- **Requirements:** R18, R20, R21.
- **Dependencies:** U16, U17.
- **Files:** `packages/d2b/src/` (dispatch/resource/exec/shell/guest/share/endpoint gating), `packages/d2b-resource-compiler/src/`, `packages/d2b-audit/src/record_types.rs`, `packages/d2b-contracts-provider/src/v3/telemetry_policy.rs`.
- **Test scenarios:** typed-noun dispatch from services/verbs catalogs; compiler projection golden outputs byte-stable; audit type lists match catalog.
- **Verification:** gates green; hand lists deleted.

### U19. Policy-check allowlist ratchet to empty
- **Milestone:** M6
- **Goal:** done-state enforcement.
- **Requirements:** R25.
- **Dependencies:** U5-U18 complete.
- **Files:** `packages/xtask/src/provider_crate_policy.rs`.
- **Test scenarios:** ratchet fails on growth; empty allowlist passes; shared-crate probes (type tables, provider strings, launch-intent tables) all trip.
- **Verification:** allowlist empty; the check runs in CI permanently.

### U20. Guest target (enrollment handlers + guest agents)
- **Milestone:** M5
- **Goal:** guest agents on the toolkit guest target.
- **Requirements:** R24.
- **Dependencies:** U4, U16.
- **Files:** `packages/d2b-zone-routing/src/service.rs` (land bootstrap/enroll handlers), `packages/d2b-session-unix/`, type crates' guest targets.
- **Test scenarios:**
  - ZoneBootstrap/ZoneEnroll admission + fail-closed refusal shapes.
  - A guest agent (security-key frontend) end-to-end over faked vsock.
- **Verification:** guest agents run on the base; no guest-side hand-rolled loops.

## Verification Contract

| Gate | Applies | Command/check | Done signal |
|---|---|---|---|
| `make check` | every unit | repo standard | green |
| Host-integration lane | every phase end | repo standard lane | green |
| Grep gates (§7 policy) | minimal ratchet from U5 (first family move); full §7 probes from U19 | `xtask check-provider-crate-layout` + §7 probes | pass; allowlist monotone ↓ |
| Fixtures | every unit | existing fixture suite | none weakened |
| Broker completeness gates | from U16 | generated-view equality checks | exact match |
| Generated-artifact hashes | from U3/U17 onward | golden hashes on generated files | stable or regenerated-by-command |
| Milestone review | after §8 phases 2, 4, 6 | operator review | sign-off recorded |

## Definition of Done

- All resource families migrated (R12); d2bd retains no driver code; the §4 kill list is fully deleted.
- All four gates green on `v3` with no fixture weakened (R26).
- The §7 policy-check allowlist is **empty** and the check runs permanently in CI (R25, KD9).
- Every provider runs on the toolkit `ProviderBase` - no hand-rolled bootstrap/service-loop/test infrastructure remains (R22).
- Milestone reviews recorded for §8 phases 2, 4, and 6.
- The guest target ships with `ZoneBootstrap`/`ZoneEnroll` handlers landed and guest agents on the base (R24).
