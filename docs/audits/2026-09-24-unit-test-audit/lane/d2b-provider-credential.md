# d2b-provider-credential - unit-test audit
tests: 26 · src files: 6
net: -2 tests, -58 lines

## Findings (biggest net first)
- trivial: `uncertain_revocation_never_unblocks_cleanup` (src/session.rs:478) - exercises only the test-only `UncertainCredentialSession` double, returning the scripted `Uncertain` outcome; no product code is reached, and the driver-side fail-closed guarantee is already pinned by `delete_fails_closed_without_a_live_session_generation` (src/driver.rs:1698) and `delete_fails_closed_on_a_stale_session_generation` (src/driver.rs:1730). Nothing lost. Dead helper `UncertainCredentialSession` (src/session.rs:460-474) goes too.
- trivial: `factory_builds_the_service_over_the_same_facet_set` (src/effects_service.rs:271) - asserts the one-line `CredentialEffectsServiceFactory::build()` wiring answers the same constant `inspect-credential` payload already pinned byte-identically by `inspect_credential_answers_the_committed_family_surface` (src/effects_service.rs:239); the response is a pure constant (no facets read), so only theater factory construction plumbing is exercised. Nothing lost.
- gap: `CredentialRevocationRequest::new` rejects `session_generation == 0` (`credential-revoke-operation-id` validation at src/session.rs:190-192) - the zero-rotation and foreign-provider branches are tested in `revocation_request_rejects_zero_rotation_or_foreign_providers` (src/session.rs:397), but the zero-session-generation branch is not; a malformed binding could slip to a Provider.
.



## Keep
- `factory_registers_only_the_credential_resource_type` - pins CredentialDriverFactory exposes exactly the one Credential type.

- `factory_created_driver_validates_through_the_erased_boundary` - pins factory-created driver validates a valid Credential through DynResourceDriver boundary.


- `validate_rejects_a_provider_outside_the_credential_family` - pins unsupported provider → `credential-provider-unsupported` + Terminal class. 
- `validate_rejects_scope_mismatches_per_provider_kind` - pins entra-on-host and secret-service-without-user-domain rejections; user-domain scope accepted. 
- `reconcile_reports_provider_unavailable_and_never_goes_ready` - pins missing facts and unready provider → Retryable + `ProviderUnavailable` status, no Ready. 
- `managed_identity_reconcile_gates_the_agent_on_the_execution_target` - pins unready execution target → `AgentPending`, no child minted before gate. 
- `managed_identity_reconcile_ensures_the_agent_until_it_is_ready` - pins Process child `mi-agent-relay` ensured, status transitions `AgentPending`→`AgentUnavailable`→`Ready`. 
- `managed_identity_agent_spec_is_owner_bound_egress_denied_and_annotation_tagged` - pins child Process spec shape: minijail Provider, template, Guest/gateway execution ref, egress denied, ownerRef+controller Provider/uid/generation annotations. 
- `non_managed_identity_credentials_reconcile_without_children` - pins secret-service reconcile → Satisfied + `Ready` with no children. 
- `managed_identity_recover_adopts_a_ready_agent_and_waits_otherwise` - pins recover: ready child → `Adopted`; unready/no child → `Missing`. 
- `finalize_finalizes_the_owned_agent_child_before_the_revocation` - pins live child → Retryable + child-delete nudge, no revocation running; retirement the same pass converges without effects. 
- `delete_revokes_before_marking_the_agent_child_deleting` - pins revocation (`session`) call precedes child deletion, `LeaseRevoked` status, child marked deleting. 
- `delete_fails_closed_without_a_live_session_generation` - pins no live session generation → fail closed Retryable, `RevocationUncertain` status, no child deletion, session retained. 
- `delete_fails_closed_on_a_stale_session_generation` - pins session generation mismatch → `credential-revocation-unconfirmed`, uncertain evidence recorded, no child deletion. 
- `delete_skips_revocation_without_lease_facts_and_retires_the_child` - pins absent lease facts → no revoke call, child still deleted, no status written. 
- `delete_is_idempotent_under_retry` - pins same durable operation id across passes, second pass → `already-revoked`, child deleted exactly once. 
- `delete_converges_without_children` - pins childless delete converges with no children ensured. 
- `reconcile_reports_agent_draining_for_a_deleting_child` - pins deleting child → `AgentDraining` status. 
- `reconcile_deletes_a_drifted_agent_child_before_recreating_it` - pins drifted child spec → child deleted, not re-ensured, `AgentPending`. 
- `the_provider_kind_mapping_follows_the_declaring_realizers` - pins provider→kind mapping for all three realizers + negative admission. 
- `revoke_session_deduplicates_the_fenced_operation_identity` - pins durable operation id/idempotency key stable across session generations (not session-generation-derived)and Debug redaction of credential ref/uid/base64 material. 
- `revocation_request_rejects_zero_rotation_or_foreign_providers` - pins `CredentialRevocationRequest::new` closed validation: zero rotation and non-credential Provider → `InvalidResource`. 
- `confirmed_revocation_evidence_is_redacted_and_bounded` - pins evidence binds operation id + session generation + outcome code; Debug redacts identity material. 
- `inspect_credential_answers_the_committed_family_surface` - pins hosted `inspect-credential` answers the committed family surface (three Provider refs + agent binary),hermetic (no host state, no descriptors). 



route-out: none spotted.