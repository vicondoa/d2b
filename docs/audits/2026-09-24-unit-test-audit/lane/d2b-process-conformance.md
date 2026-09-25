# d2b-process-conformance - unit-test audit
tests: 30 · src files: 13
net: -0 tests, -0 lines

## Findings (biggest net first)
- gap: zero-`assignment_epoch` rejection is untested - `with_assignment_binding` (src/ticket.rs:714) and `GuestExecutionBinding::new` (src/ticket.rs:307) both refuse epoch 0, but the test named `assignment_binding_requires_nonzero_session_epoch_and_client_commitment` only builds the accepted path (epoch​7) and never asserts the refusal. The assignment fence is the fail-closed boundary the name promises. No test anywhere in the crate exercises it. (Also `validate()` re-checks it at src/ticket.rs:808.)
- gap: `ProcessOutcome::exited` bounds 0..=255 untested (src/terminal.rs:52) - out-of-range exit codes only exercise `validate()` via a hand-built `Crash + Some(1)` case (`terminal_class_and_exit_code_are_bound_together`); the constructor's own rejection path is never pinned, so a regression letting `exited(999)` through would pass the suite.
- gap: `ParentWaitEvidence::verified_by` zero-identity / zero-token rejection untested (src/terminal.rs:135) - `verified_by` is the only minting path for terminal evidence and its fail-closed guard (zero digest coron token) is never exercised by any test in the crate. `is_zero()` (src/identity.rs:96) has no direct test either. 

Nothing to cut. Ship. - all 30 unit tests pin distinct product branches; zero true duplicates or trivial plumbing. Checked every `#[test]` fn against its product code branch and against the shared suite helpers in `suite.rs` (which run from sibling Provider crates): overlapping assertions (LaunchTicket Debug redaction, controller-launch-no-resource-client, stop-proof fail-closed) each pin additional behavior in their home crate, so kept per the over-lap rule.

## Keep
- `every_code_is_unique_and_matches_the_frozen_grammar` (src/error.rs:115) - 18 codes unique, grammared `[a-z][a-z0-9-]*`, length∈1..=64.
- `digests_render_hex_but_redact_their_debug` (src/identity.rs:185) - `to_hex` lowercase render (all 32 bytes)and Debug redaction for `ProcessIdentityDigest` + `ConfigurationDigest`.
- `pidfd_evidence_is_opaque_in_diagnostics` (src/identity.rs:196) - `PidfdEvidence` Debug is forced `<redacted>`;the only vent for the sealed evidence type.

- `observed_identity_covers_only_a_verified_superset` (src/identity.rs:204) - `covers()` is strict subset semantics (missing binding → false).
- `host_execution_derives_the_launch_vm_from_the_target_or_owner` (src/launch_identity.rs:339) - vm derivation branches: Guest target names VM, binding-worker splits to host `launch_vm`, bare host worker has no VM scope.

- `incomplete_rows_fail_with_the_missing_input_named`（src/launch_identity.rs:378) - 4 construction errors with exact codes + named `MissingTargetRef` payload; owner-uid-without-owner-ref guard.

- `guest_execution_names_its_own_vm`（src/launch_identity.rs:439) - Guest execution ref names its own VM (execution-name branch, no target needed.

- `owner_and_target_updates_rederive_the_vm`（src/launch_identity.rs:454) - `with_target_ref` / `with_owner` rederive the VM via `rederive_vm` (target map vs owner fallbackspective).
- `a_bundle_node_may_name_its_own_vm`（src/launch_identity.rs:487) - `with_vm` override wins for VM + `launch_vm`.
- `sandbox_digest_is_opaque_and_domain_bound`（src/sandbox.rs:160) - compiled plan digest is domain-bound (System≠User)and redacted in Debug.


- `root_and_stop_proofs_fail_closed`（src/sandbox.rs:176) - rooted sandbox spec refused for System;`StopProof::default()` fails for Local owner.


- `exit_observations_never_encode_a_signal_in_the_exit_code`（src/status.rs:147) - signal exit carries no code; code 0→Success, 3→Failure;JSON omits `exitCode` for signaled.


- `process_status_uses_the_v3_common_field_names`（src/status.rs:162) - projection serializes v3 names (`providerImplementation`, `processIdentityDigest`, `waitReapOwner: "d2b"`,digests hex.


- `controller_launch_and_assignment_authority_are_separate`（src/suite.rs:334) - controller-launch ticket has no resource client;assignment ticket is session-fenced (`validate_assignment` ok, exact session/epoch);the two tickets differ.


- `reconnect_and_finalizer_proofs_fail_closed_on_stale_evidence`（src/suite.rs:366) - reconnect changes session generation + epoch; finalizer требует verified stop proof for both Local and ServiceManager owners (default fails, complete passes.


- `terminal_results_require_parent_reap_and_matching_operation_evidence`（src/terminal.rs:247) - `from_parent` rejects non-Local wait/reap owner （TerminalEvidenceMismatch);relay happy-path returns the outcome for a matching minijail ticket.


- `terminal_class_and_exit_code_are_bound_together`（src/terminal.rs:294) - `ProcessOutcome::validate` accepts crashed/unknown (no code)and refuses Crash-with-code。
Rules class/code orthogonality bound.


- `a_user_domain_ticket_without_an_exact_user_ref_is_rejected`（src/ticket.rs:1193) - User domain + no `userRef` → `UserRefRequired`;System domain + `userRef` rejected via folded-refs case.


- `folded_references_are_type_checked`（src/ticket.rs:1203) - non-Process process ref, non-Host/Guest execution ref, non-User user ref all rejected at construction;Guest execution accepted逢。


- `guest_process_tickets_require_target_execution_binding`（src/ticket.rs:1235) - Guest execution without `GuestExecutionBinding` fails `validate()` (no target-side authority座.


- `guest_execution_binding_matches_assignment_identity`（src/ticket.rs:1248) - Guest ticket with matching guest+assignment generations (provider/session/epoch) passes `validate_assignment`。


- `cross_target_process_reference_is_guest_bound_and_single_use`（src/ticket.rs:1279) - `with_target_ref` accepts Guest once,rejects Host refs and repeat binds（single-use crossing。
- `the_deadline_is_bounded_and_the_ticket_debug_is_redacted`（src/ticket.rs:1303) - `OperationBinding::new` bounds: 0 and MAX+1 refused,MAX accepted;LaunchTicket Debug forced `<redacted>`。
- `controller_launch_proof_cannot_carry_assignment_authority`（src/ticket.rs:1313) - controller-launch ticket validates as launch-only (`validate_controller_launch` ok with binding, fails without),carries no assignment/resource-client,refuses a later `with_assignment_binding`。
- `runtime_scope_commitment_is_incarnation_and_zone_bound`（src/ticket.rs:1351) - commitment is deterministicand sensitive to Zone, incarnation（generation),and process UID。
。
- `assignment_binding_requires_nonzero_session_epoch_and_client_commitment`（src/ticket.rs:1416) - assignment binding round-trips через getters: provider generation 2, session 3, epoch 7, exact client-binding digest (kept: the exact `provider_generation` and client-commitment transmission is pinned nowhere else;gap above covers the untested refusal half of its name)。
- `launch_identity_tracks_the_ticket_owner_target_and_vm`（src/ticket.rs:1440) - ticket-derived identity carries owner/UID/target/vm/role/binding-worker flags;`with_launch_identity` refuses a mismatched execution/role row。

- `malformed_readiness_is_rejected_by_ticket_validation`（src/ticket.rs:1487) - decoded-style `Condition{timeout_ms:0}` fails `validate()`（fail-closed on untrusted input）。。
- `an_expected_process_identity_seal_rejects_a_reused_identity`（src/ticket.rs:1499) - `validate_process_identity` accepts matching digest,rejects differing digest（TerminalEvidenceMismatch)--identity reuse/fencing seal往来。
- `runtime_identity_binding_is_private_and_validated`（src/ticket.rs:1517) - runtime identity storage（zone UID,owner ref,scope digest）validated and redacted in Debug。
。

remainingtests pin the 30 behaviors above;no test is pure plumbing or derivative echo. 

route-out: none（product code read alongside;no bug surfaced).
