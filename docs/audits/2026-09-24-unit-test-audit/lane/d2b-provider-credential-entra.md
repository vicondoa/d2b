# d2b-provider-credential-entra - unit-test audit
tests: 8 · src files: 6
net: -3 tests, -32 lines

## Findings (biggest net first)
- duplicate: `host_system_placement_is_rejected` (src/lib.rs:1368) - covered by `guest_user_and_system_domains_are_accepted_but_host_system_is_rejected` (tests/placement.rs:5). Both pin `EntraPlacement::new` with `PlacementBinding::HostSystem` + `Host/workstation` → `Err(InvalidPlacement)` with identical inputs; the integration test additionally pins UserAgent/GuestAgent acceptance, so it is the strictly stronger keeper.
- trivial: `cleanup_metadata_preserves_invalid_grant_fences` (src/service.rs:783) - asserts `rotation_generation` and `expires_at_unix_ms` survive `cleanup_metadata_from_grant`, a verbatim struct-literal copy of the grant (service.rs:729); constructor-stores-its-arguments echo, copy visible in source. Nothing lost.
- trivial: `exact_consumer_guard_is_independent_of_request_fields` (src/lib.rs:1361) - only parses two `ResourceRef`s and asserts `assert_ne!` (contracts-crate derived `PartialEq`); never calls the guard it names (`authorizes_consumer`, lib.rs:950). Type-checking already proves it. Nothing lost.
- gap: `EntraConfig::new` lease-bound rejection (`max_leases` outside 1..=MAX_LOCAL_LEASES → `InvalidConfig`, src/lib.rs:505) - unit test pins only tenant-id validation; integration tests use valid bounds (1, 64, 256). Real closed-construction boundary, untested.
- gap: `EntraPlacement::new` `InvalidEndpoint` path (non-Guest identity ref, non-Endpoint login ref, or `endpoint_generation == 0`, src/lib.rs:591) - tests/placement.rs covers only the HostSystem binding and `validate_zone`; constructor's endpoint validation untested.
- gap: `EntraCredentialProviderFactory::new` `InvalidConsumer` (consumer_ref not a Provider ref, src/lib.rs:854) - no test anywhere in the crate.

## Keep
- `tenant_id_reuses_opaque_cloud_reference_validation` - pins `EntraConfig::new` accepting a plain tenant id and rejecting a shared-access-key string via `OpaqueAzureRef` validation.
- `operation_deadline_accepts_absolute_unix_milliseconds` - pins `operation_deadline` accepting future ms and mapping past ms to `DeadlineExceeded`.
- `client_state_projects_interaction_required_without_a_denial` - pins `reconcile` mapping `InteractionRequired` → `CredentialInteractionState::Required` + `Degraded` health and `Ready` → `NotRequired` (not exercised by integration tests, which go through `project_for_subject`).
- `process_unique_token_and_identity_canaries_never_render` - pins Entra audit records (wire + Debug) never rendering token/identity canaries. cross-check: sibling `d2b-provider-toolkit/src/credential.rs::an_authorized_call_never_renders_the_presented_identity` pins the identical assertions on the same helper; Entra-specific delta is only the provider-kind parameter. C2 to resolve.
- `process_unique_entra_canary_is_rejected_from_closed_values` - pins Entra telemetry frames passing closed-value validation and canary values being rejected. cross-check: both assertions pinned by sibling `d2b-provider-toolkit/src/credential.rs::a_frame_carries_only_closed_values` and `d2b-contracts-provider/src/v3/credential_controller.rs::audit_denial_is_identity_silent_and_telemetry_values_are_closed`. C2 to resolve.
