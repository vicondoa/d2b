# d2b-provider-credential-secret-service — unit-test audit
tests: 14 · src files: 6
net: -2 tests, -36 lines

## Findings (biggest net first)
- duplicate: `placement_is_user_agent_only` (src/lib.rs:1974) — covered by `only_user_agent_on_host_or_guest_is_accepted` (tests/placement.rs:6). Both pin UserAgent placement on Host accepted and non-UserAgent bindings rejected with `InvalidPlacement`; the integration test covers strictly more (UserAgent on Host *and* Guest, HostSystem *and* GuestAgent rejected). 23 lines.
- duplicate: `runtime_provider_accepts_missing_controller_user_scope_claim` (src/lib.rs:1953) — covered by `runtime_provider_starts_without_controller_user_scope_claim` (src/lib.rs:1937). Both pin that `runtime_provider` accepts a provider-control route with no controller user-scope claim (identical setup, same call); the keeper adds the placement assertions (user_ref None, zone "dev"). 13 lines.
- cross-check: `ambient_sdk_chain_names_are_rejected_without_reading_values` (src/lib.rs:2148) — `reject_ambient_credential_chain` is a one-line wrapper over `d2b_contracts_provider`; the contracts crate pins the identical assertions (PATH/RUST_LOG ok, AZURE_CLIENT_SECRET → error) at d2b-contracts-provider/src/v3/credential_controller.rs:1877-1888. C2 candidate.
- cross-check: `process_unique_identity_canary_is_hashed_only_after_authorization` (src/audit.rs:38) — wrapper over toolkit `authorized_service_record`; toolkit pins denied→no record (`a_denied_call_yields_no_identity_bearing_record`, d2b-provider-toolkit/src/credential.rs:163) and authorized→identity absent from wire+Debug (`an_authorized_call_never_renders_the_presented_identity`, credential.rs:178). The `resource_name_digest=sha256:` wire-format assertion is extra here. C2 candidate.
- cross-check: `process_unique_canary_is_rejected_as_an_allowed_key_value` (src/telemetry.rs:32) — wrapper over toolkit `credential_frame`; contracts pins the canary-value rejection (`validate_collector_fields` → `ForbiddenTelemetryField`, d2b-contracts-provider/src/v3/credential_controller.rs:1918-1923) and toolkit pins that a built frame validates (`a_frame_carries_only_closed_values`, credential.rs:193). C2 candidate.
- gap: `SecretServicePlacement::new` InvalidScope path — execution_ref not Host/Guest or user_ref not User (src/lib.rs:610-625) — placement tests cover only binding rejection, never the scope error.
- gap: `runtime_provider` route-rejection paths — missing provider ref, non-Guest execution ref, subject≠provider, missing provider generation → `SessionUnauthenticated` (src/lib.rs:102-160) — both unit tests exercise only the success path; no test anywhere pins a rejection.
- gap: `SecretServiceConfig::new` bounds — max_leases outside 1..=MAX_LOCAL_LEASES or alias over MAX_COLLECTION_ALIAS_BYTES → `InvalidConfig` (src/lib.rs:523-540) — the unit test covers only empty/control/quote/backslash aliases.

## Keep
- `runtime_provider_starts_without_controller_user_scope_claim` — pins runtime_provider accepts a provider-control route lacking a controller user-scope claim; dynamic placement carries no user_ref and zone "dev".
- `collection_alias_accepts_spaces_and_rejects_unsafe_text` — pins config alias validation: spaces accepted, empty/control/backslash/quote rejected.
- `configuration_debug_redacts_collection_alias` — pins config Debug never renders the collection alias while the getter still returns it.
- `session_key_and_capability_debug_are_redacted` — pins exact `<redacted>` Debug rendering of SessionKey and SecretServiceSessionCapability.
- `same_presentation_concurrent_first_admission_is_idempotent` — pins concurrent first admission of the same capability returns equal results (thread-safe session registry).
- `counter_exhaustion_is_fallible` — pins next_counter at u64::MAX → `SessionAuthorityError::Exhausted`.
- `absolute_deadlines_use_unix_milliseconds` — pins toolkit `operation_deadline`: absolute unix-ms accepted, exhausted deadline rejected (only test of that toolkit fn).
- `poll_port_sync_does_not_start_after_deadline` — pins the deadline guard: past deadline → `Deadline` without polling the port future.
- `lock_policy_drives_closed_controller_health` — pins reconcile health mapping: FailClosed+Locked→Unavailable, FailDegraded+Locked→Degraded, Unlocked→Ready.
- remaining 3 tests (ambient-chain, audit, telemetry) pin real redaction/closed-vocabulary invariants but duplicate sibling-crate coverage — see cross-check flags above for C2.