---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
plan_type: refactor
origin: docs/audits/2026-09-23-ponytail-audit/README.md
created: 2026-09-24
---

# refactor: Execute ponytail-audit remediation — all executable findings

## Goal Capsule

Execute every executable recommendation from the 2026-09-23 ponytail audit (≈215 finding rows, ≈ −18,930 lines, −70 deps) in the audit report's own wave order: leaf deletes first, shared-types consolidations, family merges, then the daemon/broker/xtask islands — each wave gated on the audit's green-gate list, one commit per crate, refusals untouched.

**Origin of record.** `docs/audits/2026-09-23-ponytail-audit/README.md` (consolidated report) and the 99 lane files under `docs/audits/2026-09-23-ponytail-audit/lane/` are the unit of record. Where the report's summary tables and a lane file disagree on U-ID↔crate labels (known drift, e.g. §2 row 2 "U91 d2b-provider-host"), **the lane file wins**. This plan does not re-enumerate findings; each unit names its lane set and the executor reads the lanes for exact paths and caller-verification methods.

**Base:** branch `feat/ponytail-audit` @ `6259d5cf0` (audit lanes + U104 realm-core deletion + merged PR #600). Land as a single PR from this branch; waves are stacked commits, one commit per crate.

## Requirements

- **R1 — Execute all executable findings.** Every lane row with verdict class (a) applied/ship or (c) refused is a no-op; every class (b) executable-as-is finding is cut. Findings whose lane marks them needing regeneration first (U99 generator work, wire-shape) are executed together with their regeneration step.
- **R2 — Refusals stay refused.** The refusal ledger (`docs/explanation/over-engineering-audit-record.md`, `docs/audits/2026-09-23-ponytail-audit/U1-constraints.md`) is binding; no refused item is cut without new evidence recorded in the lane. Policy-required scaffolds (`integration/*.rs`, README, registrations.json per the README-only ratchet) are never deleted.
- **R3 — Gates green per wave.** After each unit: `cargo check --workspace`, `cargo test -p` for every touched crate, `cargo run -p xtask -- check-provider-crate-layout`, `cargo run -p xtask -- blocking-census`, `make test-host-integration`, and `make check` (the complete Bazel Layer-1 gate, which subsumes per-target `bazel build`). A red gate stops the wave. (session-settled: user-approved — host-integration and `make check` must pass between waves.)
- **R4 — Re-verify before cutting.** Every deletion re-confirms the lane's zero-caller claim at current HEAD first (PR #600 annotations may have shifted line numbers; locate by symbol, not line). If a claimed-dead symbol now has a caller, skip the finding and record why in the lane.
- **R5 — No behavior change.** Deletions remove unreachable surface only. Existing tests are the behavioral net; a test is deleted only when its subject is deleted (per the lane's own test guidance). Wire-shape items (KTD2) are the sole exception and carry their own test updates.
- **R6 — Ledger updated.** When remediation completes, `docs/explanation/over-engineering-audit-record.md` gains a section recording what became of the 2026-09-23 findings (applied / skipped-with-reason), and the consolidated report's verdicts are refreshed.

## Key Technical Decisions

- **KTD1 — Lanes are the unit of record; the plan defines only wave structure and gates.** Chosen over re-enumerating 215 findings in the plan: lanes carry paths, caller-verification methods, and refusal verdicts; duplicating them here would fork two sources of truth.
- **KTD2 — vsock framing aligns to session-unix (4-byte u32 length header).** (session-settled: user-approved — chosen over leaving the skew documented-only: the user confirmed the ADR-046 baseline default in scoping.) `packages/d2b-provider-transport-vsock/src/framing.rs` moves from the 2-byte u16 header to the session-unix 4-byte u32 shape; consumers in guest-cloud-hypervisor and tests update with it. This is the one deliberate wire change in the plan.
- **KTD3 — Single PR from `feat/ponytail-audit`, waves as stacked commits, one commit per crate.** (session-settled: user-approved — chosen over per-wave PRs: the user confirmed scoping without redirect; per-crate commits keep rollback granular while one PR keeps the evidence lanes adjacent to the deletions they justify.)

## Scope Boundaries

**In scope:** all class-(b) findings from the 99 lanes, per the wave units below; U99 generator/authority cuts including their regeneration steps; U100 workspace dep drops.

**Out of scope (true non-goals):**
- Refused findings and policy-required scaffolds (R2).
- `packages/xtask/src/async_gate.rs` and `data/async-gate-inventory.json` — unaudited new surface from PR #600; cutting xtask findings must not touch them (their own audit is follow-up work).
- `labs/` (ADR 0047 disposition), including the pre-existing `decoration.rs` dead-code denies in the lab proxy.
- Correctness, security, performance items — out of the audit's judgment scope entirely.
- Wave 0 (U104 realm-core deletion) — already landed as `fcb241985`.

### Deferred to Follow-Up Work
- Audit of `xtask/src/async_gate.rs` (new PR-#600 surface).
- The `labs/window-chrome/proxy` dead-code cleanup (ADR 0047 lane).
- Any IsolationPosture/ZoneLink cross-crate *rename* consolidation — U97 marks these refused/deferred as wire-shape changes; only byte-identical dedups with a stated canonical home are in scope (U97 rows 1–6).

## Implementation Units

### U1. Wave 1a — provider-family leaf deletes

**Goal:** cut all leaf-classified, crate-local findings in the provider crates.
**Requirements:** R1, R3, R4, R5.
**Dependencies:** none (first executable wave).
**Files:** per lane — `packages/d2b-provider-config-nixos/`, `packages/d2b-provider-audio-pipewire/`, `packages/d2b-provider-display-wayland/`, `packages/d2b-provider-guest/`, `packages/d2b-provider-guest-azure-container-apps/`, `packages/d2b-provider-shell-terminal/`, `packages/d2b-provider-transport-vsock/` (except `framing.rs` — KTD2 lands in U5), `packages/d2b-provider-process-minijail/`, `packages/d2b-provider-volume-virtiofs/`, `packages/d2b-provider-device/`, `packages/d2b-provider-device-gpu/`, `packages/d2b-provider-device-tpm/`, `packages/d2b-provider-host/`, `packages/d2b-provider-user/`, `packages/d2b-provider-endpoint/`, `packages/d2b-provider-telemetry-service/`, `packages/d2b-provider-audio-service/`, `packages/d2b-provider-role/`, `packages/d2b-provider-quota/`, `packages/d2b-provider-seccomp-profile/` (Cargo.toml + BUILD.bazel included where the lane says dep drops).
**Approach:**
1. For each lane: re-verify zero-caller claims at HEAD (R4), cut the finding, delete only the tests the lane names, run `cargo test -p <crate>`.
2. Dep drops (U49, U81, U89) also update the crate's `BUILD.bazel` dep lists.
3. Commit per crate: `refactor(<crate>): <one-line finding summary>`.
**Patterns to follow:** the audit record's applied-finding commits (e.g. `366791c80` ACA deletions) — deletion commits carry the lane's evidence in the body.
**Test scenarios:**
- Touched crate's full `cargo test -p` suite passes unchanged after each cut.
- Any test the lane marks subject-deleted is removed in the same commit; no other test is touched.
- `cargo check --workspace` stays green after each crate commit (a cut breaking another crate = the caller claim was stale; revert that finding, record in lane).
**Verification:** all Wave-1a lanes report their findings executed or skipped-with-reason; gates green.

### U2. Wave 1b — CLI / host / helper leaf deletes

**Goal:** same treatment for the CLI/host/helper crates.
**Requirements:** R1, R3, R4, R5.
**Dependencies:** none; parallel-safe with U1 (disjoint crates).
**Files:** `packages/d2b/`, `packages/d2b-host-activation-helper/`, `packages/d2b-unsafe-local-helper/`, `packages/d2b-sk-frontend/`, `packages/d2b-process-conformance/`, `packages/d2b-core-controller/`, `packages/d2b-broker-composition/`, `packages/d2b-broker-fixture-handlers/`, `packages/d2b-telemetry/`.
**Approach:** as U1. U92's twin-tree deletion removes `nixos-modules/host-activation-helper/` wholesale (lane-verified zero references). U90's SHA-256 repoint keeps the two FIPS known-vector tests, adjusted to the `sha256:` prefix.
**Test scenarios:**
- Per-crate suites pass; U90's repointed vectors still assert the operator-signature digest format.
- `nixos-modules/host-activation-helper/` removal leaves zero workspace references (re-run the U92 lane's census grep).
**Verification:** gates green; lanes updated.

### U3. Wave 1c — runtime/session/contracts leaf deletes

**Goal:** leaf-classified cuts inside the runtime, session, and resource crates (blast radius leaf even though the crates are shared).
**Requirements:** R1, R3, R4, R5.
**Dependencies:** none; parallel-safe with U1/U2 (disjoint files).
**Files:** `packages/d2b-zone-routing/`, `packages/d2b-resource-runtime/`, `packages/d2b-resource-client/`, `packages/d2b-session/`, `packages/d2b-session-unix/`, `packages/d2bd-runtime/`, `packages/d2bd/`, `packages/d2b-contracts-resource/`, `packages/d2b-contracts-zone-session/`.
**Approach:** as U1. U20 (d2bd) cuts only the 19 zero-caller fns + the Wave6RealBoundary fixture the lane names; d2bd's live composition paths are untouched.
**Test scenarios:**
- `cargo test -p d2bd -p d2bd-runtime -p d2b-session -p d2b-session-unix` suites pass.
- d2bd binary target builds (`cargo check -p d2bd --bins`).
**Verification:** gates green; lanes updated.

### U4. Wave 2 — shared-types consolidations (wide)

**Goal:** execute the wide blast-radius contracts cuts and the executable U97/U98 dedups — after the leaf surface they reference has settled.
**Requirements:** R1, R3, R4, R5.
**Dependencies:** U1–U3 (leaf surface settled first, per the audit's wave order).
**Files:** `packages/d2b-contracts/`, `packages/d2b-contracts-resource/`, `packages/d2b-contracts-zone-session/`, `packages/d2b-resource-api/`, plus the consumer sites the U97 rows name (credential family crates, process, provider, device-usbip, d2bd, d2bd-runtime, d2b-broker, guest*, volume-binding) for the UUIDv4-renderer migration.
**Approach:**
1. U3/U5 contracts dead-surface cuts (−1,688 combined) with their re-export arms.
2. U97 rows 1–6: migrate the 13 UUIDv4-renderer sites to `ResourceUid::from_bytes`; dedup `SecurityKeySessionId` onto the contracts transparent-String form; delete the contracts-side `UsbipClaimSource` copy (canonical = provider-usbip); fold the byte-identical helper doubles onto their stated canonical homes.
3. U98 classes whose merge target is stated and not refusal-blocked.
4. Skip U97 row 8 naming-drift renames (refused/deferred — see Scope Boundaries).
**Patterns to follow:** prior migration `8667ed18c` (verb-list consolidation) — one canonical home, importers re-pointed, no shims.
**Test scenarios:**
- Contracts schema/wire tests (`tests/schema.rs` per contracts crate) pass unchanged — consolidations must not alter wire bytes.
- `deserialize_*` admission-gate tests stay (R2) and pass.
- Post-migration workspace grep shows zero remaining hand-rolled UUIDv4 `format!` renderers outside `ResourceUid::from_bytes` (re-run the U97 lane's census).
**Verification:** gates green; the U97/U98 lane rows marked applied; no new dep edges except the U97-stated `d2b-resource-runtime → d2b-resource-types` edge.

### U5. Wave 3 — family merges and the vsock framing alignment

**Goal:** cross-crate family dedups with a shared home, and the single deliberate wire change.
**Requirements:** R1, R3, R4, R5; KTD2.
**Dependencies:** U1 (provider leaf surface settled), U4 (uuid migration overlaps the credential family).
**Files:** `packages/d2b-provider-toolkit/src/credential.rs` (shared home), `packages/d2b-provider-credential-secret-service/`, `packages/d2b-provider-credential-entra/`, `packages/d2b-provider-credential-managed-identity/`, `packages/d2b-provider-credential/`; `packages/d2b-provider-transport-vsock/src/framing.rs` + its consumers (`packages/d2b-provider-guest-cloud-hypervisor/`, vsock tests); `packages/d2b-provider-telemetry-binding/` verb-list straggler; `packages/d2b-session-unix/src/vsock.rs` (baseline, unchanged).
**Approach:**
1. Move the deadline/absolute-unix-ms helper trio and the `reject_process_environment_credential_chain` env-scan into the toolkit credential module; the three realizers call it (their public error surfaces stay per the U53 lane).
2. vsock: re-shape `transport-vsock/src/framing.rs` to the session-unix 4-byte u32 length header (KTD2); update the framing round-trip tests and the consumer call sites; complete the ADR-046 "adapt" migration note.
3. `TELEMETRY_SERVICE_VERBS` → import `CONVERTED_TYPE_VERBS` (U68 row 1).
**Test scenarios:**
- Credential family: existing lifecycle/fail-fast tests pass unchanged (the trio move is byte-identical logic).
- vsock framing: round-trip encode/decode test at the u32 header passes; guest-cloud-hypervisor consumer tests pass; session-unix framing tests untouched and green (baseline unchanged).
- ADR-046 migration-map row for vsock framing updated to "done".
**Verification:** gates green; both `FramedVsockTransport` copies now byte-compatible in wire shape (or one delegated to the other if the lane's merge target says so).

### U6. Wave 4a — d2b-bus relay island removal

**Goal:** delete the 7-module relay island (−4,214 lines, −1 dep) — the single largest cut.
**Requirements:** R1, R3, R4, R5.
**Dependencies:** U1–U5 merged (clean base; the island is independent but sequencing puts it after the wide waves).
**Files:** `packages/d2b-bus/src/` (relay.rs, zone_route.rs, service_router.rs, audit.rs, routing.rs, transport/), `packages/d2b-bus/src/lib.rs` (four duplicate-path re-export shims), `packages/d2b-bus/Cargo.toml` + `BUILD.bazel` (retire the `d2b_audit` dep — the workspace's last consumer).
**Approach:**
1. Re-run the U12 lane's workspace-zero-caller sweep at HEAD before cutting (R4).
2. Delete the island modules, the four shims, the `d2b_audit` dep edge.
3. Confirm `d2b-audit` crate still has its other consumers (it does per the U12 lane — only d2b-bus imported it *from bus*; verify the crate itself keeps live users before dropping nothing).
**Test scenarios:**
- `cargo test -p d2b-bus` passes; bus router/streams/registry/operations/wire/session suites untouched.
- Workspace-wide grep: zero references to any `d2b_bus::{relay,zone_route,service_router,audit,routing,transport}` path.
- `blocking-census` gate green (census baseline unaffected — island was never censused as live).
**Verification:** island gone; `d2b_audit` import gone from bus; gates green.

### U7. Wave 4b — d2b-host legacy surface and broker family cuts

**Goal:** the d2b-host museum (~−2,950) and the broker's −437.
**Requirements:** R1, R3, R4, R5.
**Dependencies:** U1–U3 (leaf patterns established); independent of U6.
**Files:** `packages/d2b-host/` (hardlink_farm legacy path, dnsmasq.rs, ssh_keygen.rs, netlink fake trailer, ifname duplicate, routes preflight, atomic-write helper twins — per the U91 lane's ranked rows), `packages/d2b-broker/` (per U21 lane).
**Approach:** per-lane cuts with the U91 lane's blast-radius tags: rows tagged `leaf` land directly; rows tagged `family` (routes preflight, hardlink_farm) land only after their cited consumers are confirmed still-live-or-dead at HEAD. U91's consistency feed items (host/activation-helper shared ResourceUid seams) route to U4's migration, not this unit.
**Test scenarios:**
- `cargo test -p d2b-host -p d2b-broker` suites pass.
- Host-integration suite green after d2b-host cuts (`make test-host-integration`), with `make check` covering the BUILD-touched targets.
- Any d2b-host module the lane cut that a broker op references — verified absent from the U21 lane's findings (re-check at HEAD).
**Verification:** gates green; lanes updated.

### U8. Wave 5 — workspace deps, generator authority, ledger close-out

**Goal:** U100 dep hygiene, U99 generator/authority cuts with their regeneration steps, and the audit ledger update (R6).
**Requirements:** R1, R3, R6.
**Dependencies:** U1–U7 (all code cuts landed; deps counted after).
**Files:** root `Cargo.toml` + per-crate `Cargo.toml`/`BUILD.bazel` (U100 rows); `packages/xtask/src/gen_broker_operations.rs`, `operation_row_authority.rs`, `provider_registration_authority.rs`, `resource_type_authority.rs`, `blocking_census.rs`, `zone_schema.rs` (U99 rows — not `async_gate.rs`); `nixos-modules/resource-schemas/` orphan wrappers; `packages/d2b-resource-api/src/generated/mod.rs` fold; `docs/explanation/over-engineering-audit-record.md`; `docs/audits/2026-09-23-ponytail-audit/README.md`.
**Approach:**
1. U99: replace gen-daemon-api's hand-rolled parser with `syn` (byte-identical output proven by regenerating and diffing against committed `docs/reference/daemon-api.md`); delete `gen-resource-schemas` + its six orphan nix wrappers; collapse the `collect_rs_files`/`verify_committed` triplication; remove zone_schema's duplicate `resource-types.nix` emission (canonical = resource_type_authority); fold the un-gated `d2b-resource-api/src/generated/mod.rs` into `gen_resource_ttrpc`.
2. U100: apply the −61 dep rows + 15 normal→dev-dep moves + workspace-pin rewrites; the census/lockfile must re-verify after.
3. Regenerate all xtask outputs the U99 cuts touch; commit regenerated artifacts with the generator change.
4. Append the remediation section to the audit record; refresh report verdicts (R6).
**Test scenarios:**
- `cargo run -p xtask -- gen-daemon-api` output byte-identical to the committed reference (pre/post `syn` swap).
- `cargo run -p xtask -- gen-resource-types` (resource_type_authority) emits the committed `nixos-modules/generated/resource-types.nix` unchanged.
- Blocking-census baseline regenerates without drift after dep drops.
- Full gate suite (R3) green; `cargo check --workspace` + touched-crate tests.
**Verification:** report and audit record reflect final applied/skipped state; all gates green.

## Verification Contract

1. **Per-unit gates (R3):** `cargo check --workspace`, touched-crate `cargo test -p`, `cargo run -p xtask -- check-provider-crate-layout`, `cargo run -p xtask -- blocking-census`, `make test-host-integration`, `make check` — after every unit, before its commits are considered done. `make check` is the authoritative full gate; individual `bazel build` invocations within units are opportunistic pre-checks, not the gate itself.
2. **Caller re-verification (R4):** every cut re-runs its lane's stated search at HEAD; a stale claim is skipped and recorded, never cut blind.
3. **Refusal integrity (R2):** post-remediation grep set — the ratchet paths (`integration/*.rs`, README, registrations.json), the declared-provider crates (transport-unix, transport-vsock core), and the pinned wire surfaces are all still present.
4. **Final ledger (R6):** audit record carries per-lane applied/skipped outcomes; report metadata refreshed.
5. **Behavioral net (R5):** workspace test suite green at the end; no test deleted except subjects deleted.

## Definition of Done

- Every one of the 99 lanes has each class-(b) finding either applied (commit reference) or skipped with a recorded reason.
- All R3 gates green at HEAD; `feat/ponytail-audit` ready to open as one PR.
- Audit record + report updated (R6).
- No refused item cut; no policy scaffold touched; `async_gate.rs` untouched.

## Risks & Dependencies

- **Stale caller claims** (PR #600 shifted lines; tree moved since lanes were written): mitigated by R4's re-verification; the expected skip rate is low but nonzero.
- **d2b-bus island is the riskiest cut** (largest; retires a dep edge): isolated in U6 with its own pre-cut sweep and the boundary note from the audit ("boundary pages must land before island cut" — the U12 lane's evidence section serves as that record).
- **vsock wire change (KTD2)** touches a declared provider transport: bounded to framing + its named consumers, with round-trip tests as the gate.
- **U99 generator swaps** can silently change generated artifacts: mitigated by the byte-identical regeneration diffs required in U8's test scenarios.
- **Wave ordering matters:** wide types cuts (U4) after leaf settles (U1–U3) so consumer surface is final before renames/dedups; islands last (U6/U7) on the most-settled base.

## Open Questions

- None blocking. Executor-level unknowns deferred by design: exact per-finding line numbers (drifted; locate by symbol), and whether any individual leaf claim went stale since its lane was written (R4 handles both).
