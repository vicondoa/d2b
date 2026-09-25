# d2b-provider-credential-managed-identity — unit-test audit
tests: 8 · src files: 6
net: -6 tests, -100 lines

## Findings (biggest net first)
- duplicate: `agent_is_planned_only_after_admission_and_dependency_readiness` (src/controller.rs:337) — covered by `admitted_ready_credentials_spawn_a_co_located_agent_without_egress` (tests/topology.rs:20) + `unresolved_or_unadmitted_credentials_do_not_spawn_an_agent` (tests/topology.rs:43). Both pin plan_agent gating: admitted+deps-ready → Some(agent) with AGENT_BINARY, no egress, effect-port client required; not-admitted or deps-not-ready → None. Integration tests pin strictly more (both bindings, execution_ref/placement/owner_ref projection).
- duplicate: `ready_and_unavailable_are_closed_status_observations` (src/controller.rs:311) — trivial plumbing: pins only that `reconcile` passes the given `client_state` through into the projection (metadata=None path; `CredentialStatus::new` cannot fail here). No metadata/lease projection is exercised, so nothing behavioral is lost.
- duplicate: `collector_allowlist_rejects_nonclosed_values_for_allowed_keys` (src/telemetry.rs:33) — covered by `process_unique_managed_identity_canaries_are_absent_from_rendered_surfaces` (tests/canary.rs:21). Both assert `validate_collector_fields` rejects a non-closed value for key "outcome" and accepts `all_fields()` of a valid frame; the extra `credential_frame` wrapper exercise is delegation plumbing (type-checked args + inline `env!` version).
- duplicate: `user_agent_placement_is_rejected` (src/lib.rs:1518) — covered by `machine_placements_are_accepted_and_user_agent_is_rejected` (tests/placement.rs:8). Both pin UserAgent binding → `InvalidPlacement`; integration test also pins HostSystem/GuestAgent acceptance (strictly more).
- duplicate: `client_id_is_redacted_from_debug` (src/lib.rs:1530) — covered by `process_unique_managed_identity_canaries_are_absent_from_rendered_surfaces` (tests/canary.rs:21). Both pin `ManagedIdentityClientConfig` Debug never renders the client id; the extra `client_id()` getter-echo assertion is plumbing (returns the stored field).
- duplicate: `process_unique_managed_identity_canary_never_renders` (src/audit.rs:38) — covered by `process_unique_managed_identity_canaries_are_absent_from_rendered_surfaces` (tests/canary.rs:21). Both pin `CredentialAuditRecord` Debug + `to_wire_record()` redacting identity/secret markers; canary covers strictly more surfaces and marker classes. The `authorized_service_record` wrapper path is type-checked delegation.

## Keep
- `client_id_and_alias_validation_fail_closed` — pins closed config validation: valid client id accepted, client id containing `SharedAccessKey=…` rejected, `http://`-prefixed endpoint alias rejected. No integration test exercises rejection paths.
- `poll_client_accepts_ready_result_at_deadline` — pins the ready-branch of the sync poll helper (`poll_client_sync` returns the future's value); only test touching that machinery.

## gap
- gap: `poll_client_sync` deadline expiry — Pending future past deadline must return `DeadlineExceeded` (src/lib.rs:1171-1176) — the parking loop's only exit besides Ready; a regression hangs the revoke path. Unit test covers only the ready branch; no integration test drives a pending future past deadline.
- gap: `ManagedIdentityClientConfig::new` lease-ceiling rejection — `max_leases` outside `1..=MAX_LOCAL_LEASES` must fail closed (src/lib.rs:520-522) — unit and integration tests only construct with in-range values (1, 64).
- gap: `ManagedIdentityCredentialProviderFactory::new` consumer check — non-`Provider` `consumer_ref` must return `InvalidConsumer` (src/lib.rs:801-804) — every test constructs with a valid Provider ref; the closed-construction error path is unpinned.