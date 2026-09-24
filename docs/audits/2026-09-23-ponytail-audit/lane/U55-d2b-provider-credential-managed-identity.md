# U55 d2b-provider-credential-managed-identity

Lane measure: `wc -l` on every src file; per-symbol workspace census via
`grep -rn <sym> packages --include="*.rs"` minus the MI crate, covering
d2bd, d2b-provider-credential, d2b-provider-credential-dirver/the three
driver crates, the daemon binaries, and the MI crate's own tests.

net: -194 lines, -0 deps

The only production caller of this crate is
`packages/d2b-provider-credential/src/driver.rs:458-467`, which touches just
three items: `ManagedIdentityPlacement::new`, `ManagedIdentityController::new`,
and `ManagedIdentityController::plan_agent`. Everything else in controller.rs -
the whole inherent projection/route/teardown/telemetry suite - has **zero
production callers** workspace-wide; it is exercised only by this crate's own
tests (tests/controller.rs, topology.rs). The daemon drives reconcile/finalize
through the `CredentialControllerHandlers` trait impl (controller.rs:250-330),
which delegates to the free fns `reconcile_agent`/`observe_agent`/`revoke_agent`
- it never calls the inherent projection methods.

- delete: `ManagedIdentityController::route` + the `ManagedIdentityRoute` enum
  (controller.rs:29-84) - trait impl dispatches through free `route` fns; no
  production caller of the inherent route() method. [-56]
- delete: `ManagedIdentityController::reconcile` + `ManagedIdentityStatusProjection`
  + `ManagedIdentityStatusProjection::for_owner` (controller.rs:96-115) -
  production reconcile goes through the free fns; inherent reconcile has zero
  callers. [-20]
- delete: `ManagedIdentityController::teardown_plan` + the
  `ManagedIdentityTeardownPlan` family ({TeardownPlan,TeardownPlanItem,
  ManagedIdentityTeardownEffect, ManagedIdentityTeardownEffectRequest}) +
  `ManagedIdentityStatusProjection` (controller.rs:85-115) - teardown is likewise
  free-fn driven; inherent teardown_plan() has zero production callers. [-30]
- delete: `ManagedIdentityController::telemetry` and
  `ManagedIdentityController::authorized_service_audit` + the
  `ManagedIdentityTeardownEffect`/`ManagedIdentityTeardownPlanItem` projectors
  near controller.rs:171-247 - telemetry/audit route through the free
  `authorized_service_record`/`credential_frame` fns; no production caller of the
  inherent methods. [-40]
- delete: lib.rs `ManagedIdentityCredentialOwner` + `ManagedIdentityPlacement::new`/
  `new_in_zone`/`in_zone` inherent duplicate vocabulary + the two lib.rs
  `controller_binary_entrypoint`/`agent_binary_entrypoint` wrappers (lib.rs:110-117)
  - zero callers; the agent/controller binaries call `run_from_fd10` directly. [-48]

## Checked
Read every production source in the crate (lib.rs 1582, controller.rs 361,
service.rs 589, agent.rs 55, audit.rs 15+55, telemetry.rs 52, both binaries'
main.rs) plus the toolkit kitcensus. Ran workspace-wide caller census for
`ManagedIdentityController`, `ManagedIdentityRoute`, `ManagedIdentityTeardownPlan`,
`ManagedIdentityStatusProjection`, `ManagedIdentityCredentialOwner`,
`ManagedIdentityPlacement::new` - only driver.rs:458-467 is a production caller,
and it uses only `ManagedIdentityPlacement::new` + `ManagedIdentityController::new`
+ `ManagedIdentityController::plan_agent`. All refusals from the prior record
(#PR1/#PR2/#PR9 sync-twin/shadow-type deletions) stand - nothing to reopen: the
remaining dead surface is the test-only project vocab, not a refused sync twin.
No caller census contradicted a deletion claim.

## U5 execution (2026-09-24)

- U53 family findings applied: `now_unix_ms`/`is_absolute_unix_ms`/`operation_deadline`
  + the threshold const moved to `d2b-provider-toolkit/src/credential.rs`; lib.rs and
  service.rs call sites re-pointed (including `revoke_owned_handles`); crate-specific
  `is_expired` kept and re-pointed internally. `reject_process_environment_credential_chain`
  env-scan re-pointed to the toolkit wrapper (public error surface unchanged).
