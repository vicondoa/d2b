# U20 d2bd

**Blast radius:** all `leaf` - d2bd is the daemon crate; the daemon **lib** is imported only by the daemon's own `main.rs` (the bin target) and its own tests (verified: no `packages/*/Cargo.toml` declares a d2bd dependency and no `packages/*/BUILD.bazel` lists a d2bd lib in any dep-stanza; the only workspace consumers are `packages/d2b` (the `d2bd` binary target) and the d2bd-integration tests). Every finding is a zero-caller `pub fn`/`pub async fn` on the crate's own inherent `impl Block`, `impl<S> InteractionComposition<S>`, `impl<S> ProcessComposition<S>`, `impl<S> CompositionSource{set}`, or a crate-local free fn. Deletion is a leaf cut (no trait impl, no other crate reaches the surface; `pub` visibility is unreachable external surface only inside this crate).

- delete  Zero-caller `reconcile_wave6_operator_acceptance` (121 lines) on `impl ProductionControllerBoundary` - the sole production consumer of the Wave6 acceptance vocabulary rides `select_wave6_resources`/`Wave6ProviderBoundary` (d2bd-runtime), and the inline dashed-hex uid renderer inside it is part of finding B. No caller anywhere in the workspace (reference search: `packages/*/src`, `packages/*/tests`, `packages/*/integration`, `packages/*/Cargo.toml`, `packages/*/BUILD.bazel`, `nixos-modules/`, `docs/reference/policy/`). [packages/d2bd/src/resource_runtime.rs:4601] (leaf)

- delete  Zero-caller `adopt_deployed_controller` (47) and `launch_deployed_controller` (33) free fns in `impl ProviderBinding {` - deployed-controller adoption is a provider-crate piece (broker/credential family); these two here have zero callers. [packages/d2bd/src/provider_registry.rs:481, 445] (leaf)

- delete  Zero-caller `reserve_authority` (25), `reserve_external_nic` (25), `resolve_recovered_authority_closed` (8), `resolve_recovered_authority_adopted` (8), `quarantine_recovered_authority` (6) on `impl ZoneResourceRuntime {` - recovered-authority quarantine/resolution hand-rolled here with no caller. [packages/d2bd/src/resource_runtime.rs:8309, 8343, 8372, 8383, 8393] (leaf)

- delete  Zero-caller `drain_controller_assignment` (14), `release_controller_assignment` (17), `assignment_registry` (3) on `impl ZoneResourceRuntime {` - controller-assignment drain/release rebuilt as an alternate surface nobody reads. [packages/d2bd/src/resource_runtime.rs:3748, 3765, 3659] (leaf)

- delete  Zero-caller `dispatch_component_request` (14), `dispatch_component_request_with_attachments` (15) on `impl InteractionComposition<S> {` - the `*_for_session` variants (live, used by composition.rs:3514,4643) are the used surface; these non-session twins have zero callers. [packages/d2bd/src/interaction_composition.rs:1054, 1071] (leaf)

- delete  Zero-caller `clipboard_session` (11), `notification_session` (11), `capture_guest_clipboard` (12), `capture_host_clipboard` (13) on `impl InteractionComposition<S> {` - clipboard/notification session helpers with zero callers. [packages/d2bd/src/interaction_composition.rs:1856, 1870, 2166, 2234] (leaf)

- delete  Zero-caller `has_active_resource` (7) on `impl ProductionProcessProviders {` - GPU/providers have their own `has_active_resource_in_zone`; this plain variant has nothing reading it. [packages/d2bd/src/process_provider_runtime.rs:1893] (leaf)

- delete  Zero-caller `provider_resource_type` (4) free fn - convenience wrapper with no user. [packages/d2bd/src/provider_registry.rs:849] (leaf)

- delete  Dead test fixture `Wave6RealBoundary` (~348 lines, `tests/zone_provider_acceptance.rs:1443-1790`): a `Wave6RealBoundary` struct whose `Wave6ProviderBoundary` impl is never instantiated by any acceptance test (all four acceptance tests bind in-line `ZoneProviderBoundary` fixtures); its sole reference is its own trait-impl block, and its methods are constructed nowhere. [packages/d2bd/tests/zone_provider_acceptance.rs] (leaf)

**net:** -394 production lines (19 zero-caller `pub fn` above) plus -348 test-fixture lines (Wave6RealBoundary), -0 deps.

## Consistency notes
(none - d2bd is not a contracts/types crate)

## Reopened refusals
- The dashed-hex uid renderers inside `reserve_authority` (resource_runtime.rs:8309) and `adopt_deployed_authority`-family sites (8309-8393) are in-scope remainder of the workspace-wide hand-rolled UUIDv4 renderer finding #P7/#S1 [partial]/[applied] (shared `ResourceUid::from_bytes` at d2b-contracts/src/identity.rs:621): these two production renderer copies remain inside the crate at HEAD. Flagged because the packet's own in-scope row says "twelve remain in d2bd..." - these are two of those twelve, still unreachable, and this audit re-confirms zero callers workspace-wide. They migrate to the shared `ResourceUid::from_bytes` when the cross-cutting finding is fully applied; not separately refused.

## U3 outcome (2026-09-24)
- applied: all 19 named zero-caller fns (reconcile_wave6_operator_acceptance; adopt_deployed_controller + launch_deployed_controller; reserve_authority, reserve_external_nic, resolve_recovered_authority_closed/adopted, quarantine_recovered_authority; drain_controller_assignment, release_controller_assignment, assignment_registry; dispatch_component_request, dispatch_component_request_with_attachments; clipboard_session, notification_session, capture_guest_clipboard, capture_host_clipboard; has_active_resource; provider_resource_type) + the Wave6RealBoundary fixture (tests/zone_provider_acceptance.rs). R4 at HEAD: every fn had exactly one hit (its own definition); fixture never instantiated.
- Clean-cutover orphans removed in-crate: controller_assignment_revocation + schedule_controller_assignment_revocation; adopt_target_local_controller + launch_target_local_controller; ProductionProcessProviders::{launch_controller, adopt_controller, validate_controller_target}; controller_launch_ticket + compiled_controller_digests + configuration_digest; controller_resource test helper; set_guest test helper; pinned test controller_launch_ticket_binds_target_descriptor_and_session_without_assignment; orphaned imports (authority, Wave6, ConfigurationDigest, sha2, ProviderAdoption/Launch, ControllerProcessResource, test-module imports). d2bd live composition paths untouched; 10 composition.rs in_flight() assertions reworked to behavioral try_acquire asserts.
