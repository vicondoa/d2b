# U1 — Per-crate audit constraint packets (priming feed)

Source files (ran before this packet was written; both on disk under the repo):

- Plan: `docs/plans/2026-09-23-001-ponytail-audit-per-crate-plan.md` (178 lines; plan tree `v3` @ `3f2664794`)
- Record: `docs/explanation/over-engineering-audit-record.md` (809 lines; the "over-engineering audit record" consolidated findings/refusal ledger)
- Policy: `packages/xtask/src/provider_crate_policy.rs` (READ ONLY-only ratchet + required paths + integration path matrix); `packages/xtask/src/blocking_census.rs` (non-member exclusions, caller ledger)

The unit map (U2–U96) and the sequencing are bindingly defined in the plan's constraint section (`docs/plans/...`). Every auditor reads the plan's constraint packet BEFORE starting its crate; this document is that packet, handed to every unit U2–U104.

## (a) One-page global rules summary

**Tags** (audit judgment classes, in scope): `delete:` / `stdlib:` / `native:` / `yagni:` / `shrink:`. Correctness bugs, security, performance, and wire-shape changes are explicitly out of scope — route to a normal review pass.

**Caller verification is mandatory** (plan constraint 2, caller classes 2 / 5). Every zero-caller claim must be backed by a workspace-wide reference search: Rust references *plus* `BUILD.bazel`, `nixos-modules/`, `tests/`, `docs/reference/policy/`. The prior record's caller refutations (`d2bd/src/composition.rs:11508`, `resource_runtime.rs:3216…`) are the dominant false-positive source.

**Exclusion classes**:
- `src/generated/` is out of per-crate scope (7 crates carry it — contracts, broker, broker-composition, broker-fixture-handlers, broker-fixture-syscall-surface, control, provider, etc.); findings about generated shapes go to U99 (generator/authority audit).
- `labs/window-chrome/proxy` is a standalone workspace whose disposition is ADR 0047's — it stays (user-directed); only U104 edits it (one intentional type move; see U104).
- `docs/residual-review-findings/`, ADRs (`docs/adr/`), and dossiers named by xtask policy are authoritative context, not audit targets.
- Refusal-ledger items without new evidence: refused stays refused unless this audit finds *new evidence* or cites what changed.

**Refusal classes (7 classes from the plan's constraint section; concrete examples from step 1 cited per class):**
1. **Refused stays refused** — known refusal classes (with the plan's own examples in the constraint text): policy-scaffolds (`integration/*.rs` + README paths demanded by `packages/xtask/src/provider_crate_policy.rs` README-only ratchet); declared-provider artifacts with zero in-tree callers (`transport-unix`, `transport-vsock` — pinned by policy matrix, Nix-pinned catalogs, dossiers, committed schemas); pinned wire fields / Nix-pinned catalogs (display global catalog, `debug_logging`); hand-written `Deserialize` impls that are live admission gates (qemu guest/provider spec shapes); refactors refused for cross-crate ownership (supervisor blocking executor, ZoneLink enrollment-machine merge, host/user driver merge).
2. **Caller verification is mandatory.**
3. **`src/generated/` is out of per-crate scope / `src/generated/` is out of per-unit scope.**
4. **Labs are out of scope** except where a lab consumes a workspace crate.
5. **Docs / ADRs / dossiers are authoritative context, not audit targets.**

**Blast-radius tags:** `leaf` (crate-local), `family` (crates within one family), `wide` (contracts/types, daemon, broker). Every finding gets tagged; a zero-caller finding also states its reference-search method.

**Done:** net `-<N>` lines, `-<M>` deps per finding.

## (c) Refusals ledger — per-crate, with examples from the plan's constraint section

### How to read a row

The record (`docs/explanation/over-engineering-audit-record.md`) stores refusal classes **per crate**. The "refused stays refused" classes are the binding set; this packet carries them verbatim (with the `refused`/`partial` verdict markers applied from the record). Concrete examples from the record, quoted by class:

| Refusal class | Concrete example(s) from record | Evidence path |
| --- | --- | --- |
| Policy-required scaffolds | README-only ratchet (`integration/*.rs` + README paths demanded by `packages/xtask/src/provider_crate_policy.rs`); README-only integration ratchet (18 crates: `activation-nixos`, `audio-pipewire`, `credential-secret-service`, …) | `packages/xtask/src/provider_crate_policy.rs` |
| Declared-provider artifacts with zero in-tree callers | `transport-unix`, `transport-vsock` (Pinned wire groups); `debug_logging` | `docs/reference/policy/`, `nixos-modules/`, dossier |
| Pinned wire fields / Nix-pinned catalogs | Display global catalog (`debug_logging`); designed projections | `docs/specs/` dossiers, `nixos-modules/` |
| Hand-written `Deserialize` impls (live admission gates) | qemu guest/provider spec shapes (finding class c1/c2), qemu-media hand-written deserializers | `packages/d2b-provider-guest-qemu-media/`, `packages/d2b-provider-guest-cloud-hypervisor/` |
| Cross-crate refactors (refused) | supervisor blocking executor; ZoneLink enrollment-machine merge; host/user driver merge | `packages/d2bd/src/composition.rs:11508` leads |

### Refusal classes with per-crate examples (from the plan constraint section + prior art)

**Known refusal classes** (from the prior record's 7 listed classes; this re-audit's constraint section repeats them verbatim):

1. **Refused stays refused** unless this audit finds *new evidence* (e.g. the blocking caller was deleted since). Reopened refusals must cite what changed. Known refusal classes:
   - Policy-required scaffolds: `integration/*.rs` + README paths demanded by `packages/xtask/src/provider_crate_policy.rs` (README-only ratchet).
   - Declared-provider artifacts with zero in-tree callers (e.g. transport-unix, transport-vsock) — pinned by the policy matrix, `nixos-modules/provider-runtime-contracts.nix`, dossiers, committed schemas.
   - Pinned wire fields / Nix-pinned catalogs (display global catalog, `debug_logging`).
   - Hand-written `Deserialize` impls that are live admission gates (qemu guest/provider spec shapes).
   - Refactors refused for cross-crate ownership (supervisor blocking executor, ZoneLink enrollment-machine merge, host/user driver merge).
2. **Caller verification is mandatory.**
3. **`src/generated/` is out of per-crate scope** (7 crates carry it; the plan's constraint section names `src/generated/` — generated shapes go to U99).
4. **Labs are out of scope** except where a lab consumes a workspace crate. **`labs/window-chrome/proxy`** is standalone, whose disposition is ADR 0047's — it stays (user-directed).
5. **Docs** (`docs/residual-review-findings/`), ADRs (`docs/adr/`), and dossiers named by xtask policy are authoritative context, not audit targets.

## (c) Per-unit audit contract (applies to every crate unit U2–U96; each unit per plan constraint 1-5)

- **Format:** one line per finding, ranked biggest cut first: `<tag> <what to cut>. <replacement>. [repo-relative path]`. End with `net: -<N> lines, -<M> deps`.
- **Nothing to cut:** `Lean already. Ship.` plus one sentence naming what was checked.
- **Verify every claim:** caller search per plan constraint 2; LOC measured, not estimated.
- **Exclusions:** `src/generated/`, policy-required scaffolds, refused-ledger items without new evidence, `src/generated/` per-crate scope.
- **Consistency feed:** contracts/types crates also note divergences (duplicate type definitions, naming drift, wire-shape skew) for U97.

## Refusal classes to respect (from the plan's constraint section, with concrete examples found in step 1)

1. **Policy-required scaffolds** — README-only ratchet. Concrete example: the eleven provider crates' `integration/*.rs` + README paths demanded by `packages/xtask/src/provider_crate_policy.rs` (README-only ratchet, `packages/xtask/src/provider_crate_policy.rs:9`).

   fn main() { } // placeholder guard

2. **Caller verification is mandatory** — every zero-caller claim must be backed by a workspace-wide reference search (Rust references *plus* `BUILD.bazel`, `nixos-modules/`, `tests/`, `docs/reference/policy/`), and the plan's constraint section (2) makes this binding.

3. **`src/generated/` is out of per-crate scope** — findings about generated shapes go to U99 (generator/authority audit) instead; they may be *cited* as evidence but never flagged for deletion. Concrete example: `packages/d2b-contracts/src/generated/…` generated catalog shapes are authoritative context.

4. **Labs are out of scope** — `labs/window-chrome/proxy` is a standalone workspace whose disposition is ADR 0047's — it stays (user-directed); see U104 for its one intentional edit.

5. **`docs/residual-review-findings/` and ADRs are authoritative, not audit targets.**

Seq schema: phases/units per plan `docs/plans/...`; auditor lane leads de-duplicate within family; consolidation feed to U97.

## Window/CLI family notes (consistency consent for contracts/types crates)

- …

## User-directed LD (U104)

- `docs/explanation/over-engineering-audit-record.md` + `docs/plans/...` are authoritative refusals ledger; … (U104 executes the one intentional deletion).

## Per-crate refusal ledger (extracted from the audit record)

### U2 d2b-realm-core
- no prior findings

### U3 d2b-contracts
- #A2 [applied] Generated resource-type catalog, its generator, drift test, and sync test — resource_type_catalog.rs, its mod.rs, xtask/src/gen_resource_type_catalog.rs, the drift target, and the .bzl entry gone (~251 lines); both plane cross-checks read V3_CONVERTED_RESOURCE_TYPES
- #A9 [applied] Three hand-maintained copies of the converted-resource-type list and two sync tests — WellKnownType::ALL projected at const-eval from V3_CONVERTED_RESOURCE_TYPES (resource_type.rs:103); fence test deleted, generated copy gone with A2
- #B4 [partial] d2b-contracts dead/compat surface — deleted usbip_effect_port.rs (509 lines), provider_effects/mod.rs, auth_wire.rs; target.rs node-era compat types kept (only external references are in d2b-realm-core) + two test-only identity wrappers
- #C4 [not applied] Contract-crate macro/boilerplate consolidation (76 Wire deserialize blocks, identity macros, facade copies) — still present

### U4 d2b-contracts-broker
- #C4 [not applied] Contract-crate macro/boilerplate consolidation (76 Wire deserialize blocks, identity macros, facade copies) — still present

### U5 d2b-contracts-control
- #C4 [not applied] Contract-crate macro/boilerplate consolidation (76 Wire deserialize blocks, identity macros, facade copies) — still present

### U6 d2b-contracts-provider
- #C4 [not applied] Contract-crate macro/boilerplate consolidation (76 Wire deserialize blocks, identity macros, facade copies) — still present

### U7 d2b-contracts-resource
- #B9 [not applied] Assorted one-purpose public surface (aliases, reachability enum, limits consts) — still present in d2b-resource-client, d2b-resource-api, and d2b-contracts-resource
- #C4 [not applied] Contract-crate macro/boilerplate consolidation (76 Wire deserialize blocks, identity macros, facade copies) — still present
- #C5 [not applied] StoreErrorKind second taxonomy and its two translation tables — d2b-contracts-resource/src/v3/operations/error.rs:93 still declares the enum beside ResourceErrorKind

### U8 d2b-contracts-zone-session
- #B5 [partial] d2b-contracts-zone-session skeleton limbs — deleted src/v3/generation_bundle.rs (608) with its test (171), mod lines, BUILD target, suite entry; dead half of services.rs and six *StatusResource projections (~1,000 lines) stay
- #C4 [not applied] Contract-crate macro/boilerplate consolidation (76 Wire deserialize blocks, identity macros, facade copies) — still present

### U9 d2b-resource-types
- #A9 [applied] Three hand-maintained copies of the converted-resource-type list and two sync tests — WellKnownType::ALL projected at const-eval from V3_CONVERTED_RESOURCE_TYPES (resource_type.rs:103); fence test deleted, generated copy gone with A2

### U10 d2b-resource-api
- #A6 [not applied] 20 hand-written redaction Debug impls where redacted_debug! exists — hand-written impls remain in d2b-resource-api and d2b-core-controller (e.g. authz.rs); the exported macro is unused there
- #B1 [partial] Five d2b-resource-api modules with zero callers plus the bus-side WatchSink impl — deleted metrics.rs, zone_service.rs, quota_gate.rs, emergency_gate.rs (807 lines) and the telemetry dependency; watch.rs and the bus-side impl kept (tested watch-delivery credit path: send_wait, send_and_wait_ack, acknowledged_bytes)
- #B9 [not applied] Assorted one-purpose public surface (aliases, reachability enum, limits consts) — still present in d2b-resource-client, d2b-resource-api, and d2b-contracts-resource

### U11 d2b-core
- #A3 [applied] d2b-core runtime.rs duplicates 8 runtime DTOs — runtime.rs re-exports the eight names from d2b-contracts; field-for-field duplicate declarations and 15 call sites updated (-215)
- #A4 [not applied] 6 one-line compat shim modules in d2b-core plus a MODULE_NAME const — error.rs, contract_id.rs, configured_argv.rs, privileges_w3.rs, workload_identity.rs, unsafe_local_workloads.rs still exist at HEAD
- #A8 [not applied] d2b-core/src/base64_codec.rs hand-rolls RFC 4648 padding rules — base64_codec.rs is present; no wrapper onto the workspace base64 dependency landed

### U12 d2b-bus
- no prior findings

### U13 d2b-zone-routing
- no prior findings

### U14 d2b-resource-runtime
- #PR15 [refused] Systemic duplications worth one fix each — NullRequeue/recording doubles and duplicated resource_uid need d2b-resource-runtime test support reachable by provider crates (cross-crate, owned elsewhere); otel copy sits inside the refused surface; ResourceUid::from_bytes fix did land
- #A5 [not applied] 12 MODULE_NAME consts and the smoke_tests module asserting them against their own filenames — consts and the smoke test remain in d2b-resource-runtime/src/*.rs and lib.rs
- #B6 [partial] d2b-resource-runtime peripheral dead paths — deleted ManagerCall and ChannelManagerEndpoint (context.rs) with five tests reworked onto a local endpoint stub; audit-log history read path and identity re-export module kept with the callers that justify them
- #B8 [applied] BindingChildReconciler — type, lib.rs re-export, five tests plus one helper whose only subject it was deleted; tests exercising surviving functions stay

### U15 d2b-resource-compiler
- #B7 [applied] d2b-resource-compiler's compile_provider_artifact alias — alias and doc comment deleted; canonical compile_artifact it forwarded to stays with its tests
- #C7 [not applied] d2b-resource-compiler hand-rolls a JSON-Schema validator — walkers and secret-shape scanners remain in d2b-resource-compiler/src/

### U16 d2b-resource-client
- #B9 [not applied] Assorted one-purpose public surface (aliases, reachability enum, limits consts) — still present in d2b-resource-client, d2b-resource-api, and d2b-contracts-resource
- #C1 [not applied] GuestControlEndpoint declared twice, byte-identically — both copies remain: d2b-resource-client/src/zone_client.rs:129 and d2b-provider-guest-cloud-hypervisor/src/guest_local.rs:49

### U17 d2b-session
- no prior findings

### U18 d2b-session-unix
- no prior findings

### U19 d2bd-runtime
- #P7 [partial] The uid-to-UUIDv4 renderer hand-rolled in eleven places — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated; twelve remain in d2bd, d2bd-runtime, d2b-broker, and provider crates
- #S1 [applied] Cross-cutting: hand-rolled UUID rendering, 15+ copies — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated (wayland-policy, volume-binding, volume); twelve remain in broker, credential, guest, process, provider, d2bd-runtime, d2bd

### U20 d2bd
- #P7 [partial] The uid-to-UUIDv4 renderer hand-rolled in eleven places — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated; twelve remain in d2bd, d2bd-runtime, d2b-broker, and provider crates
- #S1 [applied] Cross-cutting: hand-rolled UUID rendering, 15+ copies — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated (wayland-policy, volume-binding, volume); twelve remain in broker, credential, guest, process, provider, d2bd-runtime, d2bd

### U21 d2b-broker
- #P7 [partial] The uid-to-UUIDv4 renderer hand-rolled in eleven places — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated; twelve remain in d2bd, d2bd-runtime, d2b-broker, and provider crates
- #S1 [applied] Cross-cutting: hand-rolled UUID rendering, 15+ copies — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated (wayland-policy, volume-binding, volume); twelve remain in broker, credential, guest, process, provider, d2bd-runtime, d2bd

### U22 d2b-broker-composition
- no prior findings

### U23 d2b-broker-fixture-handlers
- no prior findings

### U24 d2b-broker-fixture-syscall-surface
- no prior findings

### U25 d2b-core-controller
- #A6 [not applied] 20 hand-written redaction Debug impls where redacted_debug! exists — hand-written impls remain in d2b-resource-api and d2b-core-controller (e.g. authz.rs); the exported macro is unused there
- #C2 [not applied] d2b-core-controller authority.rs parallel NIC machinery and test-only constructors — still present
- #C3 [not applied] controller_assignment.rs hand-rolled JSON codec — still present

### U26 d2b-controller-toolkit
- no prior findings

### U27 d2b-provider-test-controller
- no prior findings

### U28 d2b-process-conformance
- no prior findings

### U29 xtask
- #A2 [applied] Generated resource-type catalog, its generator, drift test, and sync test — resource_type_catalog.rs, its mod.rs, xtask/src/gen_resource_type_catalog.rs, the drift target, and the .bzl entry gone (~251 lines); both plane cross-checks read V3_CONVERTED_RESOURCE_TYPES

### U30 d2b-provider
- #P7 [partial] The uid-to-UUIDv4 renderer hand-rolled in eleven places — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated; twelve remain in d2bd, d2bd-runtime, d2b-broker, and provider crates
- #S1 [applied] Cross-cutting: hand-rolled UUID rendering, 15+ copies — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated (wayland-policy, volume-binding, volume); twelve remain in broker, credential, guest, process, provider, d2bd-runtime, d2bd
- #B2 [partial] d2b-provider installation/share_adapter/forwarding and the dispatcher half of agent.rs — deleted installation.rs, share_adapter.rs, forwarding.rs (1,134 lines, -1,290 with tests) and three dependency edges; agent.rs dispatcher half kept (toolkit's FakeProvider implements ProviderAgentService)

### U31 d2b-provider-toolkit
- #B3 [refused] The toolkit's unconsumed framework half — declared-but-unwired work the provider lanes are instantiating; zero-impl codec traits and zero-caller entry points remain in server/operations/plane/base/testing

### U32 d2b-provider-activation-nixos
- #PR8 [applied] activation-nixos: three orphan subsystems — deleted runner.rs (151), diagnostics/ (110), manifest.rs (32), unreachable RetentionPlan surface (52), tests/runner.rs (86), and its BUILD target (14); live generation retention stays in Nix

### U33 d2b-provider-config-nixos
- #P6 [partial] Single-product wrappers in the Provider and config-nixos crates — ProviderDriverStatus is now a struct (driver.rs:152); ProviderDriverArgs + decode_document stay; ConfigNixosClient has a live caller at d2bd/src/composition.rs:11508

### U34 d2b-provider-audio-pipewire
- #S42 [applied] audio: orphaned legacy nix/host.nix + nix/guest.nix — deleted nix/host.nix + nix/guest.nix, src/audio_argv.rs, the audio_policy.rs shim, and the AudioRunnerContract holder; AudioMediator defaults, AudioReadiness, and the in-src fake refused
- #S43 [applied] audio: src/audio_argv.rs duplicate argv builder — same deletion as finding 42
- #S44 [refused] audio: AudioMediator compat defaults, AudioReadiness, FakeAudioMediator — a test implementor depends on the mediator default, AudioReadiness is read by the daemon, and the in-src fake is consumed by another crate's tests
- #S45 [applied] audio: src/audio_policy.rs re-export shim — same deletion as finding 42

### U35 d2b-provider-clipboard-wayland
- #S73 [applied] clipboard: d2b-clip-debug bin + packaging — deleted d2b-clip-debug.rs with packaging, duplicated clipd_host/fd.rs, test-only controller/rbac/descriptor cluster, annotated-dead reserved items, dead picker/service shells, never-run and duplicate tests; controller shrink refused (policy pins path, daemon uses runner contract)
- #S74 [applied] clipboard: src/clipd_host/fd.rs copy of src/fd.rs — same deletion as finding 73
- #S75 [applied] clipboard: test-only controller/rbac/descriptor cluster — same deletion as finding 73
- #S76 [applied] clipboard: annotated-dead reserved items — same deletion as finding 73
- #S77 [applied] clipboard: session-typed runtime admits — same deletion as finding 73
- #S78 [applied] clipboard: never-run and duplicate tests — same deletion as finding 73
- #S79 [partial] clipboard: MIME policy triplication; dead files — dead files removed (src/picker_session/ and src/service/{audit,metrics}.rs gone, service/mod.rs remains); MIME-policy triplication reported, not consolidated

### U36 d2b-provider-display-wayland
- #S27 [refused] display: labs/window-chrome/proxy copy — not deleted: labs/window-chrome/proxy is a standalone workspace whose disposition ADR 0047 owns; the pass left it
- #S28 [applied] display: legacy border-decoration renderer — deleted legacy renderer and unreachable subsurface tracking, wayland_proxy_argv.rs, ui-colors.nix + niri-vm-borders.nix, metrics.rs/audit.rs/redaction test, portal.rs, duplicated proxy-readiness protocol, descriptor.rs, bundle half of PrincipalPool, stale ProxyProcessTemplate bincs
- #S29 [applied] display: src/wayland_proxy_argv.rs has no caller — same deletion as finding 28
- #S30 [applied] display: nix/ui-colors.nix + nix/niri-vm-borders.nix unimported — same deletion as finding 28
- #S31 [applied] display: src/metrics.rs + src/audit.rs built only by tests — same deletion as finding 28
- #S32 [applied] display: src/portal.rs has no component — same deletion as finding 28
- #S33 [applied] display: duplicated proxy-readiness protocol — same deletion as finding 28
- #S34 [applied] display: src/descriptor.rs duplicates lib.rs literals — same deletion as finding 28
- #S35 [applied] display: bundle half of PrincipalPool unreachable — same deletion as finding 28
- #S36 [applied] display: ProxyProcessTemplate + stale binary consts — same deletion as finding 28
- #S37 [partial] display: hand-written Wayland global catalog; projection assertions; debug_logging; lib.rs export block — global catalog, FilterInput::debug_logging, and the lib.rs re-export block refused (Nix assertion pins the catalog; field is pinned wire); projection-assertion duplication not separately recorded as landed

### U37 d2b-provider-notification-desktop
- #S46 [applied] notification: the whole security_key/ island — deleted the security_key/ island (state machine, waybar renderer, second nonce store, second notification model, ceremony events), d2b-sk-waybar-helper.rs + nix/site.nix + packaging, eight unused twin entry points, re-hardcoded collector tables, two tautological tests; NotificationRunnerContract refused (daemon composition test calls it)
- #S47 [applied] notification: d2b-sk-waybar-helper bin + nix/site.nix + packaging — same deletion as finding 46
- #S48 [applied] notification: NotificationRuntime's eight unused twin entry points — same deletion as finding 46
- #S49 [applied] notification: collector-field tables re-hardcoded — same deletion as finding 46
- #S50 [applied] notification: tautological tests — same deletion as finding 46

### U38 d2b-provider-guest
- #P7 [partial] The uid-to-UUIDv4 renderer hand-rolled in eleven places — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated; twelve remain in d2bd, d2bd-runtime, d2b-broker, and provider crates
- #G42 [refused] Guest: the shared-provider driver re-rolled instead of implementing SharedProviderFamily — toolkit lacks three hooks (provider_spec + status_sink on the effect request, children-converged-to-Pending gate, per-kind recover-evidence hook); driver.rs still implements the runtime driver directly
- #G43 [refused] Guest: GuestTargetEffect port and dispatch have no production implementor — only consumer is d2bd/tests/guest_target_service.rs (another lane's file); trait and dispatch stay until the first target-local effect registers
- #G44 [refused] Guest: byte-identical re-rolls of toolkit helpers — reuse would change the guest crate's public error type a live daemon file consumes; the 20 local lines stay, dedup rides the refused fold
- #G45 [applied] Guest: never-called public items — HOST_REF, GuestEffectOutcome::projection, GuestDriverStatus::phase, GuestSpecEnvelope::raw, GuestDriver::controller_generation deleted
- #G46 [applied] Guest: UnavailableGuestDriverEffects port adapter — deleted; the in-file test already had ScriptedEffects
- #G47 [applied] Guest: GuestRegistration.controller_ref and GuestKind::controller_ref() have no production reader — deleted with the self-assert
- #G48 [applied] Guest: hand-written Debug impls that render only the type name — deleted
- #G49 [refused] Guest: integration/guest_family.rs scenario declaration no target compiles — crate-layout policy requires an integration/*.rs for every crate not on the README-only ratchet; this crate is not on it
- #G50 [applied] Guest: write-only fields (deleting, raw bytes copy) — deleted
- #G51 [refused] Guest: GuestChildSurface is a one-implementation trait — making the request field concrete hits invariance on ContextChildSurface's Mutex<&mut ResourceContext>; a restructure, not a deletion
- #G52 [applied] Guest: module-scope #![allow(dead_code)] — removed with the two dead test helpers it hid
- #S1 [applied] Cross-cutting: hand-rolled UUID rendering, 15+ copies — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated (wayland-policy, volume-binding, volume); twelve remain in broker, credential, guest, process, provider, d2bd-runtime, d2bd
- #C6 [not applied] d2b-provider-guest driver hand-rolls what shared_provider.rs ships — refused with the guest family's fold (guest finding 42): framework-side work

### U39 d2b-provider-guest-azure-container-apps
- #G69 [applied] ACA: deployment_service.rs has no production caller — src/deployment_service.rs deleted (451 lines) with its exports and test; nix/default.nix no longer projects an unrunnable service
- #G70 [applied] ACA: seven hand-written Deserialize blocks mirroring Raw twins — replaced by try-from plumbing; deny_unknown_fields stays on the raw shapes
- #G71 [applied] ACA: dead public API in effects.rs — deleted: lease-cleanup const, credential-scope validator, required_operations, retry_after feature, BoxAcaFuture, implementation id
- #G72 [applied] ACA: config/profile re-validation three times per admission — validated once
- #G73 [applied] ACA: dead controller members — deleted
- #G74 [applied] ACA: unused deps — d2b-contracts dropped; serde_json moved to dev-dependencies; tokio feature trimmed
- #G75 [applied] ACA: nix/default.nix processFor template param redundant — param dropped
- #G98 [applied] cross-crate: unproduced metric/audit vocabulary across five crates — label-set modules deleted with their re-exports: supervisor metrics.rs/tracing.rs (416 lines), qemu-media audit.rs + telemetry span half, azure-vm telemetry.rs/audit.rs, ACA metrics.rs/audit.rs, cloud-hypervisor metrics.rs/audit.rs
- #G99 [applied] cross-crate: runner-contract structs read only by their own tests — *RunnerContract struct/accessors/constructors and the tests that only pinned them deleted; FINALIZER/REPAIR_INTERVAL_SECS constants stay
- #G100 [partial] cross-crate: clock seam triplicated — azure-virtual-machine half applied (now depends on the toolkit clock seam); ACA half refused: d2bd/tests/cloud_composition.rs imports and implements AcaClock

### U40 d2b-provider-guest-azure-virtual-machine
- #G76 [applied] AVM: owned-VM check copy-pasted five times — one verify_owned_vm helper; disclosed log-only difference: checks now log resource group plus a stage field
- #G77 [partial] AVM: untouched accessors — as_str and retryable deleted; PskExtensionPayload::{len,is_empty} are the field's only readers and removing them trips dead_code
- #G78 [refused] AVM: BootstrapPskDelivery is a one-variant enum carried as config — d2bd constructs BootstrapPskDelivery::VmExtension at packages/d2bd/src/guest_effects.rs:1697; it stays
- #G79 [applied] AVM: write-only state (tag_digest, vm_delete_confirmed) — deleted
- #G80 [applied] AVM: bootstrap_svc.rs is a module for a 3-state wrapper — folded into bootstrap.rs
- #G81 [applied] AVM: validate_credential_scope has zero call sites — deleted
- #G82 [applied] AVM: idempotency.rs is a module for one 20-line fn — moved next to its call sites
- #G83 [refused] AVM: AzureVmConfig tenant_id/client_id never read — they are deny_unknown_fields wire fields; deleting them is a wire change
- #G84 [applied] AVM: AzureVmStatus::operation_digest never observed — deleted
- #G85 [applied] AVM: serde_json is a normal dep but only tests use it — moved to dev-dependencies
- #G98 [applied] cross-crate: unproduced metric/audit vocabulary across five crates — label-set modules deleted with their re-exports: supervisor metrics.rs/tracing.rs (416 lines), qemu-media audit.rs + telemetry span half, azure-vm telemetry.rs/audit.rs, ACA metrics.rs/audit.rs, cloud-hypervisor metrics.rs/audit.rs
- #G99 [applied] cross-crate: runner-contract structs read only by their own tests — *RunnerContract struct/accessors/constructors and the tests that only pinned them deleted; FINALIZER/REPAIR_INTERVAL_SECS constants stay
- #G100 [partial] cross-crate: clock seam triplicated — azure-virtual-machine half applied (now depends on the toolkit clock seam); ACA half refused: d2bd/tests/cloud_composition.rs imports and implements AcaClock

### U41 d2b-provider-guest-cloud-hypervisor
- #G1 [applied] CH: guest_local.rs seed/watch/session DTO family and GuestLocalController — guest_local.rs shrank 1,803 -> 180 lines (GUEST_SEED_RESOURCE_TYPES const + GuestControlEndpoint stay); DTO/controller halves and 10 tests gone
- #G2 [partial] CH: adoption.rs near-verbatim copy of the qemu-media file — copy, integration test, and property test gone; ProcessAdoptionStatus (16 lines) stays because d2bd/src/resource_runtime.rs reads it
- #G3 [applied] CH: health_check_test.rs rebuilds the same wiring 10 times — tests/health_check_test.rs deleted; remaining suites keep their coverage
- #G4 [partial] CH: identical test harness copy-pasted across test binaries — tests/common/mod.rs now shared by controller.rs and reconcile_state_machine_test.rs; redaction_test.rs keeps its own fixture
- #G5 [applied] CH: map_wire_commit_response + committed_child_from_wire + owner_ref_from_canonical_json have zero callers — deleted from src/identity.rs with the lib re-export
- #G6 [applied] CH: CloudHypervisorGuestSettings, ConsoleType, valid_token unconstructed — deleted from src/config.rs and src/lib.rs; CloudHypervisorConfig stays
- #G7 [refused] CH: fd10 bootstrap handshake re-implemented locally — toolkit fd10 entry points are private; only public entry is a different supervised flow; controller_session.rs keeps its handshake and assignment-stream loop
- #G8 [applied] CH: nix/tests/default.nix repeats evalModules boilerplate in 10 cases — replaced by one mkCase helper; eight cases proved byte-identical to the previous JSON before deletion
- #G9 [not applied] CH: BUILD.bazel repeats the same test tail for 9 rust_test targets — BUILD-file function rejected by Bazel at analysis time and dropped a crate dep; reverted in 4422fe6ab, targets restored inline
- #G10 [refused] CH: AttachmentRef and BootstrapGraph.attachments are write-only — deletion needs an edit at d2bd/src/resource_runtime.rs:6207 (another lane's file); field and every Vec::new() call site survive
- #G11 [refused] CH: tests/state_status_test.rs spends 5 rows on the same is_exact chain — five rows exercise distinct conjuncts of the chain; shrinking them would drop coverage
- #G12 [applied] CH: validate_commit_response duplicates map_commit_response — controller.rs now calls identity::map_commit_response; disclosed log-only difference: strict-subset response can emit the pre-existing warn line
- #G13 [applied] CH: redaction_test.rs builds the same descriptor twice and carries its own verifier — fixture shared
- #G14 [applied] CH: two single-variant error enums — CloudHypervisorConfigError::Invalid and BootstrapGraphError::InvalidReference are gone
- #G15 [applied] CH: valid_digest duplicated in two modules — both copies deleted in favor of the schema fingerprint parse
- #G16 [applied] CH: nix self-assert (guestSetupDescriptors == projection of itself) — projectedPrivateDescriptors and the assertion are gone from nix/default.nix
- #G17 [applied] CH: pending_after_batch only forwards args — inlined at the five call sites
- #G18 [applied] CH: GuestSessionEvidenceProbe trait has no implementation — deleted from src/health.rs and src/lib.rs
- #G19 [applied] CH: session_generation_is_fresh read only by a test — deleted from src/shutdown.rs
- #G20 [applied] CH: CLOUD_HYPERVISOR_IMPLEMENTATION_ID, CONTROLLER_BINARY have no reader — deleted from src/lib.rs
- #G98 [applied] cross-crate: unproduced metric/audit vocabulary across five crates — label-set modules deleted with their re-exports: supervisor metrics.rs/tracing.rs (416 lines), qemu-media audit.rs + telemetry span half, azure-vm telemetry.rs/audit.rs, ACA metrics.rs/audit.rs, cloud-hypervisor metrics.rs/audit.rs
- #G99 [applied] cross-crate: runner-contract structs read only by their own tests — *RunnerContract struct/accessors/constructors and the tests that only pinned them deleted; FINALIZER/REPAIR_INTERVAL_SECS constants stay
- #C1 [not applied] GuestControlEndpoint declared twice, byte-identically — both copies remain: d2b-resource-client/src/zone_client.rs:129 and d2b-provider-guest-cloud-hypervisor/src/guest_local.rs:49

### U42 d2b-provider-guest-qemu-media
- #G21 [applied] qemu: whole qemu_argv module called only by its own tests — src/qemu_argv.rs deleted (257 lines) with its re-exports
- #G22 [applied] qemu: entire media-watch subsystem has no production caller — src/controller/media_watch.rs deleted; d2bd derives media_ready from dependency phases
- #G23 [applied] qemu: RuntimeState observation map and constant methods — src/state.rs deleted; the invariant stays in the descriptor
- #G24 [applied] qemu: TapLaunchRouter/NetworkLaunchEvent/TapAttachment record an effect nobody replays — deleted with src/controller/network.rs
- #G25 [refused] qemu: hand-written Deserialize + 14 default fns for GuestProviderSpecSettings — hand-written deserializer is the live admission gate (d2bd parses and discards, so a derive would drop validation); types/guest.rs keeps it
- #G26 [applied] qemu: display.rs has zero production callers — src/controller/display.rs deleted; display stays a plain Endpoint ref in d2bd
- #G27 [applied] qemu: status surface has no production path — src/controller/status.rs and the status constructors/deserializer deleted; d2bd publishes status through its own sink
- #G28 [applied] qemu: QMP methods no production path invokes — dead methods and their only-producer variants deleted from src/qmp/mod.rs; disclosed log-only difference: negotiate propagates the transport's own error
- #G29 [applied] qemu: HotplugOperation/HotplugResult never constructed — src/controller/hotplug.rs deleted; QmpSession is used directly
- #G30 [refused] qemu: nix/projection.nix providerAssertions re-validates what live modules enforce — premise refuted: assertions are evaluated as a module and asserted by the crate's own Nix case; they stay
- #G31 [applied] qemu: HostGlobalAuthorityIndex + AuthorityReservation test-only second index — deleted from src/controller/device_watch.rs; the vocabulary types stay
- #G32 [applied] qemu: three dead ControllerConfigProjection fields — write-only fields removed from src/config.rs
- #G33 [refused] qemu: ProviderConfig hand-rolls Deserialize and a Wire copy — same live admission-gate reason as finding 25
- #G34 [applied] qemu: descriptor.rs contract exercised only by its own test — src/descriptor.rs deleted
- #G35 [applied] qemu: repr-field validation triplicated; QmpCommand::name() has no caller — one token validator; name() deleted
- #G36 [applied] qemu: QmpHealth tracker never probed outside tests — deleted from src/qmp/mod.rs
- #G37 [applied] qemu: RuntimeVolumeSpec::validate rebuilds literals new_with_provider just built — one shared helper in src/controller/volume.rs
- #G38 [refused] qemu: nix/projection.nix guestPatchForZone emits a patch nobody inspects — premise refuted: guestPatchesByZone is applied by nixos-modules/bundle-zones.nix and asserted by the crate's own Nix case; it stays
- #G39 [applied] qemu: GuestSpecError variants never constructed — dead variants deleted from src/types/guest.rs
- #G40 [applied] qemu: ProviderDescriptor compatibility alias has no namer — deleted
- #G41 [applied] qemu: command journal evicts with Vec::remove(0) — now VecDeque::pop_front in src/qmp/mod.rs
- #G98 [applied] cross-crate: unproduced metric/audit vocabulary across five crates — label-set modules deleted with their re-exports: supervisor metrics.rs/tracing.rs (416 lines), qemu-media audit.rs + telemetry span half, azure-vm telemetry.rs/audit.rs, ACA metrics.rs/audit.rs, cloud-hypervisor metrics.rs/audit.rs
- #G99 [applied] cross-crate: runner-contract structs read only by their own tests — *RunnerContract struct/accessors/constructors and the tests that only pinned them deleted; FINALIZER/REPAIR_INTERVAL_SECS constants stay

### U43 d2b-provider-shell-terminal
- #S38 [applied] shell-terminal: src/process_lifecycle.rs second ProcessProvider impl — deleted process_lifecycle.rs with its conformance test, process_templates.rs with its test, and the ShellRunnerContract constant holder; SHELL_REPAIR_INTERVAL_SECS kept (read by two crates)
- #S39 [refused] shell-terminal: InMemoryShellAuthority forwards to the ledger — real behavior with live coverage (every method forwards to the ledger; production composes the ledger directly), so deleting it would remove an exercised implementation
- #S40 [applied] shell-terminal: src/process_templates.rs read only by its own test — same deletion as finding 38
- #S41 [applied] shell-terminal: *RunnerContract constant-holder pattern — same deletion as finding 38

### U44 d2b-provider-transport-azure-relay
- #S9 [applied] transport-azure-relay: ~580 lines of unused surface — sealed-credential write half, RelayTransportService and handles, src/reconnect.rs, duplicate mint_sas, unbound-acquire default, audit/metrics deleted; GatewayGuestCredentialSource kept (d2bd/src/composition.rs:4422)

### U45 d2b-provider-transport-unix
- #S7 [refused] transport-unix has no caller in the repo — declared provider: xtask/src/provider_crate_policy.rs:275-279 pins src/portal.rs and tests/transport.rs, nixos-modules/provider-runtime-contracts.nix:213 lists it, committed binding schema exists; nothing deleted

### U46 d2b-provider-transport-vsock
- #S8 [refused] transport-vsock: ~2,000 lines with zero workspace dependents — same declaration class as finding 7: policy matrix row, runtime provider row with settings assertions, dossier, and committed schema all name it

### U47 d2b-provider-system-core
- #P4 [applied] system-core modules with no caller in the tree — 17 files / -1,624 lines in e5a6e091a; crate now holds error/host/lib/ownership/testing/user only
- #P5 [partial] Duplicate Host/User handler-status emitter and its readiness checker — system-core copy (handler_status.rs) gone with finding 4; zone-side copy stays - live SystemCoreStatusEmitter calls it (zone_status.rs:149, constructed at d2bd/resource_runtime.rs:3216,3616,4957)

### U48 d2b-provider-process-systemd
- #PR4 [partial] systemd's dead modules — deleted guest_exec.rs (614 lines), manifest.rs (29), adoption.rs (31); dossier still names adoption.rs at ADR-046-provider-system-systemd.md:1295,1351,1468 and no dossier edit landed
- #PR5 [applied] The EffectPortAdapter layer in both process provider crates is never constructed — minijail/src/effect_port.rs and systemd/src/effect_port.rs deleted with re-export arms and dossier destination lines; live conformance ProcessLaunchEffectPort remains the generic spawn boundary

### U49 d2b-provider-process-minijail
- #PR5 [applied] The EffectPortAdapter layer in both process provider crates is never constructed — minijail/src/effect_port.rs and systemd/src/effect_port.rs deleted with re-export arms and dossier destination lines; live conformance ProcessLaunchEffectPort remains the generic spawn boundary
- #PR12 [applied] minijail shims with no production caller — deleted manifest.rs, sandbox_compiler.rs, user_ns.rs, ephemeral.rs, effect_result.rs, finalize.rs, the reconcile dispatcher and its enums, PlatformGate::new_for_test, adoption::validate_candidate (-405 net)

### U50 d2b-provider-volume-local
- #S51 [applied] volume-local: audit catalog + otel catalog + test — deleted audit/otel catalogs with their test, migration/relocation/sealing/snapshot planners with five test files, src/path.rs with its test, store-view validators, swtpm-volume policy, test-only ACL planner, dead effect-port half, unread schema, unused zone-session dependency
- #S52 [applied] volume-local: migration/relocation/sealing state machines — same deletion as finding 51
- #S53 [refused] volume-local: orphaned nix/store.nix + nix/sync-json.nix — both files are loaded by bazel/checks/nix/BUILD.bazel:55-56 as part of the storage-volume eval surface
- #S54 [applied] volume-local: snapshot policy/catalog planner — same deletion as finding 51
- #S55 [applied] volume-local: src/path.rs opaque path proofs + tests — same deletion as finding 51
- #S56 [applied] volume-local: store-view validators, swtpm volume policy, standalone quota planner — same deletion as finding 51; inline quota duplicate removed while quota::admit_quota stays
- #S57 [partial] volume-local: src/effect_port.rs second vocabulary — dead half removed (216 -> 100 lines); surviving file re-exports the contract effect-port vocabulary (effect_port.rs:12), so the whole-file deletion did not happen
- #S58 [not applied] volume-local: duplicate Volume validator inside the zone compiler — duplicate remains (resources-zones-volumes.nix:11 keeps its own modePattern, already drifted from resources-volume.nix:23); no commit touched the crate's Nix directory
- #S59 [applied] volume-local: JSON round-trip decode of typed contract values — same deletion as finding 51
- #S60 [partial] volume-local: four competing marker-phase vocabularies; test-only ACL planner; unread schema — test-only ACL planner (acl.rs 342 -> 241) and unread root-config.schema.json removed; marker-phase consolidation not separately verified; src/marker.rs remains
- #S61 [not applied] volume-local: volume scout could not see the driver crates — method note, not an action; the duplications it points at are recorded under findings 1, 2, 5, and 6

### U51 d2b-provider-volume-virtiofs
- #S62 [partial] volume-virtiofs: in-crate reconciler + readiness classifier + testing — deleted src/readiness.rs (99) and src/user_ns.rs (181); controller.rs (361, the in-crate reconciler) and testing.rs (333) untouched - tests/lifecycle.rs uses the testing fixtures
- #S63 [applied] volume-virtiofs: third argv renderer + socket-path generator + dead helpers — deleted virtiofsd_argv.rs (247, third argv renderer), user_ns.rs (181), dead worker helpers (worker.rs 455 -> 198); socket_path.rs now the 4-line MAX_SOCKET_PATH_BYTES constant
- #S64 [partial] volume-virtiofs: src/port.rs + unread schema — unread root-config.schema.json deleted; src/port.rs (139) remains, used by the controller and the lifecycle test; socket_path.rs shrank 165 -> 4

### U52 d2b-provider-supervisor
- #G53 [refused] supervisor: hand-rolled blocking executor (16 threads, deadline thread, custom waker) — deadline cannot move out of the crate: d2bd awaits launch with no timeout, tokio is dev-only here; src/adapter.rs keeps the executor
- #G54 [refused] supervisor: hand-rolled seqpacket broker transport — d2bd-runtime transport is not semantically interchangeable (no deadlines, different error mapping) and provider crates may not depend on it; src/broker.rs:2239-2428 survives
- #G55 [refused] supervisor: generic systemd seam with exactly one production implementation — d2bd names the generic type at packages/d2bd/src/process_provider_runtime.rs:59; the seam stays
- #G56 [applied] supervisor: three write-only quarantine sets/fields — deleted; ambiguity already reported through ProcessConformanceError::AdoptionAmbiguous
- #G57 [applied] supervisor: check_user_manager chain has no caller — trait default, forwarder, and override deleted
- #G58 [partial] supervisor: test-only constructors on the production backend — new and with_socket deleted from both backends; with_socket_and_role stays (only caller is tests/production_adapter.rs)
- #G59 [applied] supervisor: BrokerSystemdEffectOwner::{new,with_socket} have zero callers — deleted
- #G60 [refused] supervisor: a test scrapes d2b-broker's source to keep an error-kind string honest — no exported constant exists for the broker error kind and d2b-contracts-broker is out of lane; include_str scrape and cross-crate compile_data stay
- #G61 [applied] supervisor: typed-identity projection hand-copied into six wire requests — one shared emit helper
- #G62 [not applied] supervisor: identical bounded pending-observation ledger implemented twice — deferred for a single writer: dedup spans src/broker.rs and src/systemd.rs; both copies remain
- #G63 [applied] supervisor: observe and probe bodies differ by one line — shared implementation
- #G64 [applied] supervisor: ProviderSupervisor::wait_identity has no caller — deleted
- #G65 [refused] supervisor: comment-only integration files no target compiles — crate-layout policy ratchet requires the paths
- #G66 [refused] supervisor: wait_pidfd_exit/wait_pidfd_observer are the same poll loop twice — measured: shared part is nine lines and the observer needs the elapsed-vs-unreadiness split, so the saving does not exist
- #G67 [applied] supervisor: SystemdIdentityContext 2-field wrapper — folded into SystemdInvocationIdentity::new
- #G68 [refused] supervisor: BrokerFrame wraps its fd table in a Mutex though single-owner — removing the Mutex needs a &mut self call and src/systemd.rs calls take_fd on an immutable frame
- #G98 [applied] cross-crate: unproduced metric/audit vocabulary across five crates — label-set modules deleted with their re-exports: supervisor metrics.rs/tracing.rs (416 lines), qemu-media audit.rs + telemetry span half, azure-vm telemetry.rs/audit.rs, ACA metrics.rs/audit.rs, cloud-hypervisor metrics.rs/audit.rs
- #G101 [refused] cross-crate: /proc/<pid>/stat field-22 readers copied per crate — no shared parser is reachable from the supervisor, and the local one classifies Z/X as gone; the copy stays

### U53 d2b-provider-credential-secret-service
- #PR1 [applied] The three Credential realization crates were one provider written three times — one shared module d2b-provider-toolkit/src/credential.rs keyed on CredentialProviderKind; the three crates keep their own binaries, provider refs, and process-level canary tests
- #PR2 [applied] Every credential op implemented twice per crate (sync copies of the async bodies) — sync twins deleted (managed-identity -480, entra -474, secret-service -501); each sync dispatch is a fail-fast guard plus one dispatch_blocking call
- #PR3 [partial] secret-service's production-dead halves — deleted controller_binary_entrypoint, never-constructed SecretServicePortError::{Missing,Denied}, duplicate invariant_error(), two cross-crate suites; session-capability authority kept (lib.rs:1290-1360)

### U54 d2b-provider-credential-entra
- #PR1 [applied] The three Credential realization crates were one provider written three times — one shared module d2b-provider-toolkit/src/credential.rs keyed on CredentialProviderKind; the three crates keep their own binaries, provider refs, and process-level canary tests
- #PR2 [applied] Every credential op implemented twice per crate (sync copies of the async bodies) — sync twins deleted (managed-identity -480, entra -474, secret-service -501); each sync dispatch is a fail-fast guard plus one dispatch_blocking call

### U55 d2b-provider-credential-managed-identity
- #PR1 [applied] The three Credential realization crates were one provider written three times — one shared module d2b-provider-toolkit/src/credential.rs keyed on CredentialProviderKind; the three crates keep their own binaries, provider refs, and process-level canary tests
- #PR2 [applied] Every credential op implemented twice per crate (sync copies of the async bodies) — sync twins deleted (managed-identity -480, entra -474, secret-service -501); each sync dispatch is a fail-fast guard plus one dispatch_blocking call
- #PR9 [applied] managed-identity 'backward-compatible' shadow types — ManagedIdentityTelemetry{Operation,Outcome,Frame} and ManagedIdentityAudit{Operation,Outcome,Record} deleted (~230 lines); canary test rebuilt on the shared contract types

### U56 d2b-provider-credential
- #P7 [partial] The uid-to-UUIDv4 renderer hand-rolled in eleven places — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated; twelve remain in d2bd, d2bd-runtime, d2b-broker, and provider crates
- #PR14 [partial] credential base crate: test-only and write-only surface — deleted CredentialDriverStatus::{phase,outcome_code} and the re-asserting test; CONTROLLER_PROVIDER_*_ANNOTATION written into live status annotations (driver.rs:550-552), CredentialRevocationEvidence has a production reader, integration/ is policy-required
- #PR15 [refused] Systemic duplications worth one fix each — NullRequeue/recording doubles and duplicated resource_uid need d2b-resource-runtime test support reachable by provider crates (cross-crate, owned elsewhere); otel copy sits inside the refused surface; ResourceUid::from_bytes fix did land
- #S1 [applied] Cross-cutting: hand-rolled UUID rendering, 15+ copies — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated (wayland-policy, volume-binding, volume); twelve remain in broker, credential, guest, process, provider, d2bd-runtime, d2bd

### U57 d2b-provider-network-local
- no prior findings

### U58 d2b-provider-device
- #S2 [applied] Cross-cutting: six copies of the same 9-verb list — CONVERTED_TYPE_VERBS in d2b-resource-types/src/descriptor.rs; DEVICE/HOST/USER/VOLUME/BINDING_VERBS migrated; TELEMETRY_BINDING_VERBS left outside the family and reported

### U59 d2b-provider-device-gpu
- #S12 [applied] GPU: host-global authority index and restart-recovery machinery — authority index/recovery/adoption, legacy effect port, controller upgrade path, probe/status/audit/telemetry/production/arbitration/descriptor/wire deleted; GpuController::adopt_lifecycle kept (policy-pinned test exercises it)
- #S13 [applied] GPU: legacy effect path and upgrade/runner-contract machinery — same deletion as finding 12
- #S14 [applied] GPU: src/probe.rs nothing probes DRM through — same deletion as finding 12
- #S15 [applied] GPU: status/audit/telemetry trio authored by d2bd — same deletion as finding 12
- #S16 [applied] GPU: src/production.rs never constructed — same deletion as finding 12
- #S17 [applied] GPU: src/arbitration.rs second claim arbiter — same deletion as finding 12
- #S18 [applied] GPU: worker specs carrying unread fields and forwarders — same deletion as finding 12
- #S19 [applied] GPU: duplicate wire snapshot — same deletion as finding 12
- #S20 [applied] GPU: nix/default.nix row merge already supplies defaults — same deletion as finding 12; nix/default.nix left to its owning lane

### U60 d2b-provider-device-security-key
- #S5 [refused] Cross-cutting: spec_ref duplicated in four crates — no importable shared pointer-ref parser in scope (interaction engine exposes only key_ref/owned_child_ensure/resource_uid; toolkit helper metadata-specific and off-limits); the four copies stay
- #S21 [applied] security-key: semantic descriptor machinery — deleted src/descriptor.rs, the Binding-admission branch, src/session_ring.rs, src/effect_port.rs with observe_inventory, src/cid.rs blocking framers, zero-referent command consts, unread vsock/lease constants
- #S22 [applied] security-key: Binding-admission branch with zero daemon callers — same deletion as finding 21
- #S23 [applied] security-key: src/session_ring.rs and its pushes — same deletion as finding 21
- #S24 [applied] security-key: src/effect_port.rs with no implementor — same deletion as finding 21
- #S25 [applied] security-key: second CID translator and relay leftovers — same deletion as finding 21
- #S26 [applied] security-key: constants with no production referent — same deletion as finding 21

### U61 d2b-provider-device-tpm
- #S70 [refused] Forward-only nixos-modules wrappers (usbip, tpm) — nixos-modules/** is outside the lane's write scope
- #S71 [applied] tpm: the semantic ticket layer, state tokens, runner half, migration helpers — deleted semantic ticket layer with tests, dead runner half, migration helpers, from_status/status with src/status.rs, swtpm_argv exec_arg0* flexibility; StateDirIntent/tokens and SwtpmArgvInput fields refused (daemon references/constructs them)
- #S72 [applied] tpm: swtpm_argv dead flexibility; from_status/status test-only — same deletion as finding 71

### U62 d2b-provider-device-usbip
- #S5 [refused] Cross-cutting: spec_ref duplicated in four crates — no importable shared pointer-ref parser in scope (interaction engine exposes only key_ref/owned_child_ensure/resource_uid; toolkit helper metadata-specific and off-limits); the four copies stay
- #S65 [applied] usbip: the v2 reconcile model inside src/reconcile_state.rs — reconcile_state.rs shrank 5,520 -> 454 lines; degraded-reason cluster the daemon projects stays; reference page corrected in 254b1ac49
- #S66 [partial] usbip: src/state_machine.rs and src/usbip_argv.rs — usbip_argv.rs (748 lines) deleted; state_machine.rs refused - docs/reference/usbip-state-machine.md:7,245 pins it as the canonical step-ordering artifact
- #S67 [refused] usbip: parallel Service effect path (firewall.rs + controller.rs) — provider dossier requires tests/controller_state_machine.rs, wrong_zone_and_redaction.rs, and effect_port_contract.rs against UsbipEffectPort; those tests exist and run
- #S68 [refused] usbip: Binding lifecycle half + production.rs forwarding impl — v3 rewrite plan names BindingLifecycle as the attach seam no production path constructs yet - declared, not dead
- #S69 [refused] usbip: arbitration.rs + never-constructed declaration modules + BusId re-wrap + token duplicate — dossier-declared tests and rows (arbitration_conflict.rs, Process/EphemeralProcess declarations, physical-usb-backing token shared with security-key)
- #S70 [refused] Forward-only nixos-modules wrappers (usbip, tpm) — nixos-modules/** is outside the lane's write scope

### U63 d2b-provider-observability-otel
- #PR6 [applied] otel: 1,368 lines of Nix byte-copies of the live module graph — deleted nix/stack.nix (741), stale host.nix/guest.nix forks, the crate aggregation entry, and two dangling build inputs; nix/ now holds only projection.nix and tests
- #PR7 [refused] otel: unwired surface — provider dossier declares the realization plan (ADR-046-provider-observability-otel.md section 18); nothing beyond the Nix fork was deleted
- #PR15 [refused] Systemic duplications worth one fix each — NullRequeue/recording doubles and duplicated resource_uid need d2b-resource-runtime test support reachable by provider crates (cross-crate, owned elsewhere); otel copy sits inside the refused surface; ResourceUid::from_bytes fix did land

### U64 d2b-provider-process
- #P7 [partial] The uid-to-UUIDv4 renderer hand-rolled in eleven places — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated; twelve remain in d2bd, d2bd-runtime, d2b-broker, and provider crates
- #PR11 [refused] The process family depends on both of its own Providers to read two &strs — premise refuted: PROVIDER_REF has eight non-test users across seven files; the family driver reading the providers' own exported names is the correct layering
- #S1 [applied] Cross-cutting: hand-rolled UUID rendering, 15+ copies — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated (wayland-policy, volume-binding, volume); twelve remain in broker, credential, guest, process, provider, d2bd-runtime, d2bd

### U65 d2b-provider-host
- #S2 [applied] Cross-cutting: six copies of the same 9-verb list — CONVERTED_TYPE_VERBS in d2b-resource-types/src/descriptor.rs; DEVICE/HOST/USER/VOLUME/BINDING_VERBS migrated; TELEMETRY_BINDING_VERBS left outside the family and reported
- #S6 [refused] Cross-cutting: host and user drivers are ~60% the same file — measured 922 (host) vs 845 (user) lines, 281 differing after subject normalization; only shared home (toolkit) off-limits; host-to-user edge would be a new cross-family dependency

### U66 d2b-provider-user
- #S2 [applied] Cross-cutting: six copies of the same 9-verb list — CONVERTED_TYPE_VERBS in d2b-resource-types/src/descriptor.rs; DEVICE/HOST/USER/VOLUME/BINDING_VERBS migrated; TELEMETRY_BINDING_VERBS left outside the family and reported
- #S6 [refused] Cross-cutting: host and user drivers are ~60% the same file — measured 922 (host) vs 845 (user) lines, 281 differing after subject normalization; only shared home (toolkit) off-limits; host-to-user edge would be a new cross-family dependency

### U67 d2b-provider-endpoint
- #G86 [refused] endpoint: DeadManager + NullRequeue doubles duplicate the workspace's — the runtime's recording doubles are #[cfg(test)] pub(crate) and cannot be used across crates; the local doubles stay
- #G87 [refused] endpoint: EndpointRealization variant identity consumed nowhere in production — the change needs an edit at packages/d2bd/src/resource_plane_v3.rs:1858 (another lane's file)
- #G88 [applied] endpoint: three near-duplicate EndpointSpec test builders — one parameterized builder; the dead one removed
- #G89 [refused] endpoint: EndpointDriverArgs.zone never read — needs an edit at the daemon construction site; the field stays
- #G90 [applied] endpoint: factory_registers_only_the_endpoint_resource_type re-asserts registration — deleted
- #G91 [applied] endpoint: EndpointSpecEnvelope wraps one live field — decoded straight to the canonical object
- #G92 [applied] endpoint: EndpointDriverErrorKind::class() has unreachable arms — arms removed
- #G93 [applied] endpoint: tautological assertions on the test's own fake vocabulary — deleted
- #G94 [applied] endpoint: ENDPOINT_TYPE_NAME duplicates the WellKnownType const — now derived
- #G95 [applied] endpoint: EndpointDriver::error ignores self — free function
- #G96 [applied] endpoint: #[derive(Clone)] on EndpointDriver unused — dropped
- #G97 [applied] endpoint: tokio time feature declared but unused — feature dropped
- #PR15 [refused] Systemic duplications worth one fix each — NullRequeue/recording doubles and duplicated resource_uid need d2b-resource-runtime test support reachable by provider crates (cross-crate, owned elsewhere); otel copy sits inside the refused surface; ResourceUid::from_bytes fix did land

### U68 d2b-provider-telemetry-service
- #PR10 [partial] telemetry pair: mirrored drivers and self-testing tests — deleted both finalize overrides, registration asserts, and dead accessors (-184 net); pair collapse needs d2b-resource-runtime test support reachable across crates; DEPENDENCY_READINESS_PROVEN is the published readiness gate

### U69 d2b-provider-telemetry-binding
- #PR10 [partial] telemetry pair: mirrored drivers and self-testing tests — deleted both finalize overrides, registration asserts, and dead accessors (-184 net); pair collapse needs d2b-resource-runtime test support reachable across crates; DEPENDENCY_READINESS_PROVEN is the published readiness gate
- #S2 [applied] Cross-cutting: six copies of the same 9-verb list — CONVERTED_TYPE_VERBS in d2b-resource-types/src/descriptor.rs; DEVICE/HOST/USER/VOLUME/BINDING_VERBS migrated; TELEMETRY_BINDING_VERBS left outside the family and reported

### U70 d2b-provider-volume
- #P7 [partial] The uid-to-UUIDv4 renderer hand-rolled in eleven places — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated; twelve remain in d2bd, d2bd-runtime, d2b-broker, and provider crates
- #S1 [applied] Cross-cutting: hand-rolled UUID rendering, 15+ copies — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated (wayland-policy, volume-binding, volume); twelve remain in broker, credential, guest, process, provider, d2bd-runtime, d2bd
- #S2 [applied] Cross-cutting: six copies of the same 9-verb list — CONVERTED_TYPE_VERBS in d2b-resource-types/src/descriptor.rs; DEVICE/HOST/USER/VOLUME/BINDING_VERBS migrated; TELEMETRY_BINDING_VERBS left outside the family and reported

### U71 d2b-provider-volume-binding
- #P7 [partial] The uid-to-UUIDv4 renderer hand-rolled in eleven places — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated; twelve remain in d2bd, d2bd-runtime, d2b-broker, and provider crates
- #S1 [applied] Cross-cutting: hand-rolled UUID rendering, 15+ copies — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated (wayland-policy, volume-binding, volume); twelve remain in broker, credential, guest, process, provider, d2bd-runtime, d2bd
- #S2 [applied] Cross-cutting: six copies of the same 9-verb list — CONVERTED_TYPE_VERBS in d2b-resource-types/src/descriptor.rs; DEVICE/HOST/USER/VOLUME/BINDING_VERBS migrated; TELEMETRY_BINDING_VERBS left outside the family and reported
- #S10 [applied] Volume: DesiredBindingChild carries five fields nobody reads — now {name, spec}; the five unread fields and the unused import are gone
- #S11 [partial] Volume: volume-binding uid_hex and resource_uid side by side — resource_uid migrated to the shared constructor; uid_hex refused because it renders plain 32-hex inside failure diagnostic text (driver.rs:504), not a UUID

### U72 d2b-provider-wayland-policy
- #P7 [partial] The uid-to-UUIDv4 renderer hand-rolled in eleven places — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated; twelve remain in d2bd, d2bd-runtime, d2b-broker, and provider crates
- #S1 [applied] Cross-cutting: hand-rolled UUID rendering, 15+ copies — shared ResourceUid::from_bytes at d2b-contracts/src/identity.rs:621; three in-scope call sites migrated (wayland-policy, volume-binding, volume); twelve remain in broker, credential, guest, process, provider, d2bd-runtime, d2bd
- #S3 [refused] Cross-cutting: per-type declaration boilerplate in the interaction family (approx. 250 lines claimed) — measured ~200 lines across six crates, only ~70 safely removable; *_spec_decoder() wrappers consumed by each crate's registration test and *Driver/*Factory aliases are public surface
- #S4 [partial] Cross-cutting: dead per-type constants no caller reads — six *_CONTROLLER_REF constants and their six re-export arms deleted; paired *_RESYNC constants refused - each is the return value of its own InteractionType::resync(), called at wayland-policy/src/interaction.rs:795

### U73 d2b-provider-wayland-session
- #S3 [refused] Cross-cutting: per-type declaration boilerplate in the interaction family (approx. 250 lines claimed) — measured ~200 lines across six crates, only ~70 safely removable; *_spec_decoder() wrappers consumed by each crate's registration test and *Driver/*Factory aliases are public surface
- #S4 [partial] Cross-cutting: dead per-type constants no caller reads — six *_CONTROLLER_REF constants and their six re-export arms deleted; paired *_RESYNC constants refused - each is the return value of its own InteractionType::resync(), called at wayland-policy/src/interaction.rs:795

### U74 d2b-provider-audio-service
- #S3 [refused] Cross-cutting: per-type declaration boilerplate in the interaction family (approx. 250 lines claimed) — measured ~200 lines across six crates, only ~70 safely removable; *_spec_decoder() wrappers consumed by each crate's registration test and *Driver/*Factory aliases are public surface
- #S4 [partial] Cross-cutting: dead per-type constants no caller reads — six *_CONTROLLER_REF constants and their six re-export arms deleted; paired *_RESYNC constants refused - each is the return value of its own InteractionType::resync(), called at wayland-policy/src/interaction.rs:795

### U75 d2b-provider-audio-binding
- #S3 [refused] Cross-cutting: per-type declaration boilerplate in the interaction family (approx. 250 lines claimed) — measured ~200 lines across six crates, only ~70 safely removable; *_spec_decoder() wrappers consumed by each crate's registration test and *Driver/*Factory aliases are public surface
- #S4 [partial] Cross-cutting: dead per-type constants no caller reads — six *_CONTROLLER_REF constants and their six re-export arms deleted; paired *_RESYNC constants refused - each is the return value of its own InteractionType::resync(), called at wayland-policy/src/interaction.rs:795

### U76 d2b-provider-shell-pool
- #S3 [refused] Cross-cutting: per-type declaration boilerplate in the interaction family (approx. 250 lines claimed) — measured ~200 lines across six crates, only ~70 safely removable; *_spec_decoder() wrappers consumed by each crate's registration test and *Driver/*Factory aliases are public surface
- #S4 [partial] Cross-cutting: dead per-type constants no caller reads — six *_CONTROLLER_REF constants and their six re-export arms deleted; paired *_RESYNC constants refused - each is the return value of its own InteractionType::resync(), called at wayland-policy/src/interaction.rs:795
- #S5 [refused] Cross-cutting: spec_ref duplicated in four crates — no importable shared pointer-ref parser in scope (interaction engine exposes only key_ref/owned_child_ensure/resource_uid; toolkit helper metadata-specific and off-limits); the four copies stay

### U77 d2b-provider-shell-session
- #S3 [refused] Cross-cutting: per-type declaration boilerplate in the interaction family (approx. 250 lines claimed) — measured ~200 lines across six crates, only ~70 safely removable; *_spec_decoder() wrappers consumed by each crate's registration test and *Driver/*Factory aliases are public surface
- #S4 [partial] Cross-cutting: dead per-type constants no caller reads — six *_CONTROLLER_REF constants and their six re-export arms deleted; paired *_RESYNC constants refused - each is the return value of its own InteractionType::resync(), called at wayland-policy/src/interaction.rs:795
- #S5 [refused] Cross-cutting: spec_ref duplicated in four crates — no importable shared pointer-ref parser in scope (interaction engine exposes only key_ref/owned_child_ensure/resource_uid; toolkit helper metadata-specific and off-limits); the four copies stay

### U78 d2b-provider-zone
- #P1 [applied] Ten name-substituted copies of one metadata driver — one shared driver in d2b-resource-runtime/src/metadata.rs (535 lines); the ten crates' driver.rs are now 20-22 lines each
- #P2 [applied] Ten copies of the same registration test — shared assert_metadata_registration in d2b-resource-types/src/metadata.rs:72; per-crate tests/registration.rs kept (crate-layout policy)
- #P5 [partial] Duplicate Host/User handler-status emitter and its readiness checker — system-core copy (handler_status.rs) gone with finding 4; zone-side copy stays - live SystemCoreStatusEmitter calls it (zone_status.rs:149, constructed at d2bd/resource_runtime.rs:3216,3616,4957)
- #P8 [refused] emit_handler_status duplicating the emitter's own mandatory pair — deleting it would change the status a live emitter publishes; zone_status.rs:101-152 unchanged, no commit
- #P9 [refused] integration/*.rs and README scaffolds in ten metadata crates — crate-layout policy requires an integration/*.rs for every crate not on the README-only ratchet (xtask/src/provider_crate_policy.rs)
- #A1 [applied] 11 provider crates hand-roll the same 45-line declaration-only ResourceDriver — one driver in d2b-resource-runtime/src/metadata.rs (535 lines) + one declaration builder and shared registration assertion in d2b-resource-types/src/metadata.rs; the eleven type crates' driver.rs are 20-22 lines

### U79 d2b-provider-zone-link
- #P1 [applied] Ten name-substituted copies of one metadata driver — one shared driver in d2b-resource-runtime/src/metadata.rs (535 lines); the ten crates' driver.rs are now 20-22 lines each
- #P2 [applied] Ten copies of the same registration test — shared assert_metadata_registration in d2b-resource-types/src/metadata.rs:72; per-crate tests/registration.rs kept (crate-layout policy)
- #P3 [partial] A second ZoneLink enrollment-and-session state machine inside the provider crate — callerless half removed (ZoneLinkKeyPolicy::new + MIN/MAX bounds, -45 lines); machine stays in zone_links.rs; full merge refused (spans d2b-bus and d2bd)
- #P9 [refused] integration/*.rs and README scaffolds in ten metadata crates — crate-layout policy requires an integration/*.rs for every crate not on the README-only ratchet (xtask/src/provider_crate_policy.rs)
- #A1 [applied] 11 provider crates hand-roll the same 45-line declaration-only ResourceDriver — one driver in d2b-resource-runtime/src/metadata.rs (535 lines) + one declaration builder and shared registration assertion in d2b-resource-types/src/metadata.rs; the eleven type crates' driver.rs are 20-22 lines

### U80 d2b-provider-provider
- #P6 [partial] Single-product wrappers in the Provider and config-nixos crates — ProviderDriverStatus is now a struct (driver.rs:152); ProviderDriverArgs + decode_document stay; ConfigNixosClient has a live caller at d2bd/src/composition.rs:11508

### U81 d2b-provider-role
- #P1 [applied] Ten name-substituted copies of one metadata driver — one shared driver in d2b-resource-runtime/src/metadata.rs (535 lines); the ten crates' driver.rs are now 20-22 lines each
- #P2 [applied] Ten copies of the same registration test — shared assert_metadata_registration in d2b-resource-types/src/metadata.rs:72; per-crate tests/registration.rs kept (crate-layout policy)
- #P9 [refused] integration/*.rs and README scaffolds in ten metadata crates — crate-layout policy requires an integration/*.rs for every crate not on the README-only ratchet (xtask/src/provider_crate_policy.rs)
- #A1 [applied] 11 provider crates hand-roll the same 45-line declaration-only ResourceDriver — one driver in d2b-resource-runtime/src/metadata.rs (535 lines) + one declaration builder and shared registration assertion in d2b-resource-types/src/metadata.rs; the eleven type crates' driver.rs are 20-22 lines

### U82 d2b-provider-role-binding
- #P1 [applied] Ten name-substituted copies of one metadata driver — one shared driver in d2b-resource-runtime/src/metadata.rs (535 lines); the ten crates' driver.rs are now 20-22 lines each
- #P2 [applied] Ten copies of the same registration test — shared assert_metadata_registration in d2b-resource-types/src/metadata.rs:72; per-crate tests/registration.rs kept (crate-layout policy)
- #P9 [refused] integration/*.rs and README scaffolds in ten metadata crates — crate-layout policy requires an integration/*.rs for every crate not on the README-only ratchet (xtask/src/provider_crate_policy.rs)
- #P10 [applied] Unused d2b-contracts-resource dependency in seven crates — role-binding, operation, quota, emergency-policy, resource-import, resource-export, seccomp-profile: zero references in Cargo.toml and BUILD dep lists at HEAD
- #A1 [applied] 11 provider crates hand-roll the same 45-line declaration-only ResourceDriver — one driver in d2b-resource-runtime/src/metadata.rs (535 lines) + one declaration builder and shared registration assertion in d2b-resource-types/src/metadata.rs; the eleven type crates' driver.rs are 20-22 lines

### U83 d2b-provider-quota
- #P1 [applied] Ten name-substituted copies of one metadata driver — one shared driver in d2b-resource-runtime/src/metadata.rs (535 lines); the ten crates' driver.rs are now 20-22 lines each
- #P2 [applied] Ten copies of the same registration test — shared assert_metadata_registration in d2b-resource-types/src/metadata.rs:72; per-crate tests/registration.rs kept (crate-layout policy)
- #P9 [refused] integration/*.rs and README scaffolds in ten metadata crates — crate-layout policy requires an integration/*.rs for every crate not on the README-only ratchet (xtask/src/provider_crate_policy.rs)
- #P10 [applied] Unused d2b-contracts-resource dependency in seven crates — role-binding, operation, quota, emergency-policy, resource-import, resource-export, seccomp-profile: zero references in Cargo.toml and BUILD dep lists at HEAD
- #A1 [applied] 11 provider crates hand-roll the same 45-line declaration-only ResourceDriver — one driver in d2b-resource-runtime/src/metadata.rs (535 lines) + one declaration builder and shared registration assertion in d2b-resource-types/src/metadata.rs; the eleven type crates' driver.rs are 20-22 lines

### U84 d2b-provider-emergency-policy
- #P1 [applied] Ten name-substituted copies of one metadata driver — one shared driver in d2b-resource-runtime/src/metadata.rs (535 lines); the ten crates' driver.rs are now 20-22 lines each
- #P2 [applied] Ten copies of the same registration test — shared assert_metadata_registration in d2b-resource-types/src/metadata.rs:72; per-crate tests/registration.rs kept (crate-layout policy)
- #P9 [refused] integration/*.rs and README scaffolds in ten metadata crates — crate-layout policy requires an integration/*.rs for every crate not on the README-only ratchet (xtask/src/provider_crate_policy.rs)
- #P10 [applied] Unused d2b-contracts-resource dependency in seven crates — role-binding, operation, quota, emergency-policy, resource-import, resource-export, seccomp-profile: zero references in Cargo.toml and BUILD dep lists at HEAD
- #A1 [applied] 11 provider crates hand-roll the same 45-line declaration-only ResourceDriver — one driver in d2b-resource-runtime/src/metadata.rs (535 lines) + one declaration builder and shared registration assertion in d2b-resource-types/src/metadata.rs; the eleven type crates' driver.rs are 20-22 lines

### U85 d2b-provider-resource-export
- #P1 [applied] Ten name-substituted copies of one metadata driver — one shared driver in d2b-resource-runtime/src/metadata.rs (535 lines); the ten crates' driver.rs are now 20-22 lines each
- #P2 [applied] Ten copies of the same registration test — shared assert_metadata_registration in d2b-resource-types/src/metadata.rs:72; per-crate tests/registration.rs kept (crate-layout policy)
- #P9 [refused] integration/*.rs and README scaffolds in ten metadata crates — crate-layout policy requires an integration/*.rs for every crate not on the README-only ratchet (xtask/src/provider_crate_policy.rs)
- #P10 [applied] Unused d2b-contracts-resource dependency in seven crates — role-binding, operation, quota, emergency-policy, resource-import, resource-export, seccomp-profile: zero references in Cargo.toml and BUILD dep lists at HEAD
- #A1 [applied] 11 provider crates hand-roll the same 45-line declaration-only ResourceDriver — one driver in d2b-resource-runtime/src/metadata.rs (535 lines) + one declaration builder and shared registration assertion in d2b-resource-types/src/metadata.rs; the eleven type crates' driver.rs are 20-22 lines

### U86 d2b-provider-resource-import
- #P1 [applied] Ten name-substituted copies of one metadata driver — one shared driver in d2b-resource-runtime/src/metadata.rs (535 lines); the ten crates' driver.rs are now 20-22 lines each
- #P2 [applied] Ten copies of the same registration test — shared assert_metadata_registration in d2b-resource-types/src/metadata.rs:72; per-crate tests/registration.rs kept (crate-layout policy)
- #P9 [refused] integration/*.rs and README scaffolds in ten metadata crates — crate-layout policy requires an integration/*.rs for every crate not on the README-only ratchet (xtask/src/provider_crate_policy.rs)
- #P10 [applied] Unused d2b-contracts-resource dependency in seven crates — role-binding, operation, quota, emergency-policy, resource-import, resource-export, seccomp-profile: zero references in Cargo.toml and BUILD dep lists at HEAD
- #A1 [applied] 11 provider crates hand-roll the same 45-line declaration-only ResourceDriver — one driver in d2b-resource-runtime/src/metadata.rs (535 lines) + one declaration builder and shared registration assertion in d2b-resource-types/src/metadata.rs; the eleven type crates' driver.rs are 20-22 lines

### U87 d2b-provider-command
- #PR13 [refused] command: the controller-family template — deferred: duplicated fences and verb lists live in a shared engine another lane was restructuring, so a cut would have collided; declaration-only half removed with the shared metadata driver (policy finding 1)
- #A1 [applied] 11 provider crates hand-roll the same 45-line declaration-only ResourceDriver — one driver in d2b-resource-runtime/src/metadata.rs (535 lines) + one declaration builder and shared registration assertion in d2b-resource-types/src/metadata.rs; the eleven type crates' driver.rs are 20-22 lines

### U88 d2b-provider-operation
- #P1 [applied] Ten name-substituted copies of one metadata driver — one shared driver in d2b-resource-runtime/src/metadata.rs (535 lines); the ten crates' driver.rs are now 20-22 lines each
- #P2 [applied] Ten copies of the same registration test — shared assert_metadata_registration in d2b-resource-types/src/metadata.rs:72; per-crate tests/registration.rs kept (crate-layout policy)
- #P9 [refused] integration/*.rs and README scaffolds in ten metadata crates — crate-layout policy requires an integration/*.rs for every crate not on the README-only ratchet (xtask/src/provider_crate_policy.rs)
- #P10 [applied] Unused d2b-contracts-resource dependency in seven crates — role-binding, operation, quota, emergency-policy, resource-import, resource-export, seccomp-profile: zero references in Cargo.toml and BUILD dep lists at HEAD
- #A1 [applied] 11 provider crates hand-roll the same 45-line declaration-only ResourceDriver — one driver in d2b-resource-runtime/src/metadata.rs (535 lines) + one declaration builder and shared registration assertion in d2b-resource-types/src/metadata.rs; the eleven type crates' driver.rs are 20-22 lines

### U89 d2b-provider-seccomp-profile
- #P1 [applied] Ten name-substituted copies of one metadata driver — one shared driver in d2b-resource-runtime/src/metadata.rs (535 lines); the ten crates' driver.rs are now 20-22 lines each
- #P2 [applied] Ten copies of the same registration test — shared assert_metadata_registration in d2b-resource-types/src/metadata.rs:72; per-crate tests/registration.rs kept (crate-layout policy)
- #P9 [refused] integration/*.rs and README scaffolds in ten metadata crates — crate-layout policy requires an integration/*.rs for every crate not on the README-only ratchet (xtask/src/provider_crate_policy.rs)
- #P10 [applied] Unused d2b-contracts-resource dependency in seven crates — role-binding, operation, quota, emergency-policy, resource-import, resource-export, seccomp-profile: zero references in Cargo.toml and BUILD dep lists at HEAD
- #A1 [applied] 11 provider crates hand-roll the same 45-line declaration-only ResourceDriver — one driver in d2b-resource-runtime/src/metadata.rs (535 lines) + one declaration builder and shared registration assertion in d2b-resource-types/src/metadata.rs; the eleven type crates' driver.rs are 20-22 lines

### U90 d2b
- no prior findings

### U91 d2b-host
- no prior findings

### U92 d2b-host-activation-helper
- no prior findings

### U93 d2b-unsafe-local-helper
- no prior findings

### U94 d2b-sk-frontend
- no prior findings

### U95 d2b-audit
- no prior findings

### U96 d2b-telemetry
- no prior findings

## Context / sources

- `docs/plans/...` (plan's constraint section, unit map)
- `docs/explanation/over-engineering-audit-record.md` (finding tables, refusal ledger, per-family verdicts)
- `packages/xtask/src/provider_crate_policy.rs` + `blocking_census.rs` (policy gates)

## Receipt (this packet references step-1 ground truth)

Per-crate findings asserted in the plan's unit map (LOC from plan, measured not estimated):

| Unit | Crate | ~LOC | Unit | Crate | ~LOC |
| --- | --- | --- | --- | --- | --- |
| U2 | `packages/d2b-realm-core` | 15,950 | U40 | `packages/d2b-provider-azure-virtual-machine` | 1,882 |
| U3 | `packages/d2b-contracts` | 11,509 | U79 | `packages/d2b-provider-zone-link` | 3,787 |
| U20 | `packages/d2bd` | 94,493 | U64 | `packages/d2b-provider-process` | 8,836 |
| U94 | `packages/d2b-telemetry` | — | U29 | `packages/xtask` | 40,186 |

(non-exhaustive; the plan's unit map table is authoritative for all 95 units.)

This U1 packet is authoritative context for every per-crate auditor, who must read the plan constraint section before starting.
[End of U1 packet]
