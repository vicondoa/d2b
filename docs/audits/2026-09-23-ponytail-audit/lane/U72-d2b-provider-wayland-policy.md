# U72 d2b-provider-wayland-policy

Measured 3,999 LOC (src 2,904 + tests 727 + integration 8; `wc -l`, not estimated). All ledger rows verified at HEAD `3f2664794`.

## Findings

- **#P7/#S1 [partial/applied, verified] — uid-to-UUIDv4 renderer migrated.** This crate's in-scope copy now calls the shared constructor: `interaction.rs:845` `ResourceUid::from_bytes(bytes).ok()`; no hand-rolled `format!("{:02x}…")` renderer remains in this crate. Twelve copies remain in other crates (out of lane).
- **#S3 [refused, stays] — per-type declaration boilerplate in the interaction family.** `*_spec_decoder()` wrappers and `*Driver`/`*Factory` aliases still present and consumed: `wayland_policy.rs:79,82` (`WaylandPolicyDriver`/`WaylandPolicyFactory` aliases), `wayland_policy_spec_decoder()` (registration test + `wayland_policy.rs:112`), `spec_decoder()` (interaction.rs:272, called in-crate). Public surface; refusal stands.
- **#S4 [partial, verified] — dead per-type constants.** Six `*_CONTROLLER_REF` constants and their re-export arms remain deleted (none present); paired `WAYLAND_POLICY_RESYNC` stays as the refused half — it is the return value of `InteractionType::resync()` (`interaction.rs:397`, `wayland_policy.rs:48`) and is asserted by `tests/registration.rs:54`. Refusal stands.
- **#S5 [refused, stays] — spec_ref duplicated in four crates.** This crate carries two copies: `vocabulary.rs:110` `fn spec_ref(value, path)` and `effects_service.rs:608` `fn spec_ref_at(bytes, path)` — both live, both in-crate; no importable shared pointer-ref parser in scope per the ledger. Refusal stands.

## New surface scan (no findings)

- `INTERACTION_VERBS`/`INTERACTION_EXECUTION_DOMAINS` (`lib.rs:77,94`) — cross-crate readers in shell-pool, shell-session, wayland-session, audio-binding (4 crates each).
- `AUDIO_SERVICE_TYPE`/`AUDIO_BINDING_TYPE` (`vocabulary.rs:19,22`) — live readers in `audio_registry.rs:171,205,219,352,501,502`.
- `WAYLAND_POLICY_TYPE`/`WAYLAND_POLICY_PROVIDER_REF` — live: `d2bd/src/resource_runtime.rs:164` reads `WAYLAND_POLICY_PROVIDER_REF`; registration test reads both.
- `shell_pool_spec`/`shell_session_execution`/`shell_session_pool_ref` — live in `effects_service.rs:353,368,370` and cross-crate (shell-pool, shell-session registration tests).
- `wayland_policy_descriptor` — live: `d2bd/src/resource_plane_v3.rs:163` + tests.
- `AudioResourceRuntime`/`AudioEffectRegistry` — full reconcile surface live: `facets.rs:116` constructs the registry; `audio_registry.rs:277,370,391` activate_promoted; `InteractionEffectsService` constructed at `d2bd/src/resource_plane_v3.rs:181` and factory at `shared_provider_effects.rs:3457`, `resource_plane_v3.rs:2286`.
- `test_support.rs` — consumed by d2b-provider-host and d2b-provider-user (shared surface, not crate-local dead code).

## Checked

Read `src/{lib,wayland_policy,interaction,effects_service,audio_registry,facets,vocabulary,test_support}.rs`, `tests/{engine,registration}.rs`, `integration/wayland_policy.rs`; caller searches across `packages/**/*.rs` (non-generated) for every ledger symbol: `ResourceUid::from_bytes`, `*_CONTROLLER_REF`, `*_RESYNC`, `spec_decoder`, `*Driver`/`*Factory` aliases, `spec_ref`, `INTERACTION_VERBS`, `INTERACTION_EXECUTION_DOMAINS`, `AUDIO_*_TYPE`, `WAYLAND_POLICY_*`, `wayland_policy_descriptor`, `AudioResourceRuntime`, `test_support`. All 5 ledger rows honored; `[refused]` rows (#S3, #S4-resync half, #S5) unchanged — no new evidence.

Lean already. Ship — the crate's migrated, deleted, and refused surfaces all match the ledger at HEAD; no new dead code found.