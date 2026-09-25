# d2b-provider-supervisor — unit-test audit
tests: 21 · src files: 4
net: -0 tests, -0 lines

## Findings
Nothing to cut. Ship.

Checked all 21 test fns across adapter.rs (10), broker.rs (8), systemd.rs (3) plus the crate's `tests/production_adapter.rs` integration suite as covering reference: every test pins a distinct behavior branch, and the two structurally parallel "observations bounded and consumed" tests exercise separate backend implementations (broker.rs:1182 vs systemd.rs:242 `record`/`take_observation`), so neither is deletable. No `#[ignore]`d tests, no plumbing/derive-echo tests — the Debug-redaction and digest-binding tests pin hand-written security invariants, not derives.

- gap: `BrokerSystemdEffectOwner` (systemd.rs:456-707) — the daemon's fixed supervisor owner has zero tests in this crate (unit or tests/): its launch/observe/probe/reopen/stop/finalize envelope legs, identity fence (`identity()`), scope-mismatch checks, unit-request ledger (remember/request_for/take_request), and error paths (envelope refusal, ledger miss → `IdentityChanged`, stop refused → `StopFailed`) are all unpinned. Integration tests exercise only the local `SystemdOwner` mock, never the broker-backed owner.
- gap: `wait_pidfd_exit` / `wait_pidfd_observer` (broker.rs:1586-1663) — pidfd wait loops with their timeout boundary (`DeadlineExceeded` on a never-exiting pidfd) are untested anywhere in the crate; they are the wait path both broker backends rely on.
- gap: `map_probe_error` `DeadlineExceeded` branch (adapter.rs:905-911) — `a_starved_probe_is_transient_and_never_quarantined` pins the `Busy` → `LaunchFailed` projection only; a probe whose blocking call overruns its deadline (the other documented "never produced a result" case) is not pinned.

## Keep
- `resolution_refusals_are_not_adoption_ambiguity` (adapter.rs:958) — pins `map_error` projection: ResolutionFailed → `resolution-failed`, IdentityChanged → AdoptionAmbiguous.
- `timed_out_launch_is_quarantined_until_late_cleanup_succeeds` (adapter.rs:1069) — timed-out launch → AdoptionAmbiguous, tracked; late cleanup success retires launch+handle.
- `a_late_launch_cleanup_failure_stays_quarantined_and_tracked` (adapter.rs:1115) — cleanup `StopFailed` keeps launch+handle tracked (complementary branch to the above).
- `hung_late_launch_cleanup_is_bounded_and_quarantined` (adapter.rs:1211) — hung cleanup stop bounded by deadline; launch+handle stay tracked.
- `hung_terminate_is_bounded_and_quarantined` (adapter.rs:1255) — hung explicit terminate bounded; handle retained as adoption-ambiguous.
- `terminal_stops_retire_retained_handles` (adapter.rs:1295) — repeated launch+terminate retires handles each time.
- `terminal_finalization_retires_a_naturally_exited_handle` (adapter.rs:1315) — `finalize_identity` retires the retained handle.
- `probe_uses_the_non_mutating_backend_seam` (adapter.rs:1379) — probe routes to backend `probe`, never `observe`.
- `a_parked_deadline_registration_still_bounds_a_hung_effect` (adapter.rs:1452) — deferred (queue-full) deadline registration still bounds a hung effect and wakes the poller.
- `a_starved_probe_is_transient_and_never_quarantined` (adapter.rs:1538) — saturated-pool probe → transient LaunchFailed; after drain → genuine `Ok(None)`.
- `pending_broker_observations_are_bounded_and_consumed` (broker.rs:1975) — BrokerProcessBackend observation ring caps at MAX_PENDING_OBSERVATIONS; take_observation consumes.
- `executable_mismatch_remains_observable_as_incomplete_identity` (broker.rs:2001) — unverified executable stays observable with Executable binding absent, Cgroup present.
- `open_pidfd_dispatch_failure_is_ambiguous_only_after_identity_drift` (broker.rs:2038) — dispatch refusal classifies by observed state: drift → IdentityChanged, unchanged → PidfdUnavailable, vanished → Vanished, admission refusal never classified.
- `launch_fence_separates_start_time_drift_from_a_gone_process` (broker.rs:2087) — `launch_adoption_error`: match → None, drift → IdentityChanged, gone → Vanished.
- `generic_process_roles_map_only_to_closed_broker_roles` (broker.rs:2100) — `runner_role_for_process_role` closed mapping incl. None cases.
- `device_owned_worker_row_resolves_through_its_declared_row` (broker.rs:2470) — BundleBackedLaunchResolver resolves declared Device-owned row over shared-template generic lookup; template fence and undeclared-row refusal.
- `broker_diagnostics_redact_process_identity_values` (broker.rs:2541) — hand-written Debug impls redact broker identity values.
- `broker_process_identity_digest_binds_resource_incarnation` (broker.rs:2551) — digest changes with resource_uid (identity fence).
- `pending_systemd_observations_are_bounded_and_consumed` (systemd.rs:341) — SystemdProcessBackend observation ring caps at MAX_PENDING_OBSERVATIONS; take_observation consumes (separate impl from broker's).
- `systemd_identity_diagnostics_are_redacted` (systemd.rs:359) — SystemdInvocationIdentity Debug redacts.
- `systemd_adoption_identity_binds_bundle_content_identity` (systemd.rs:367) — digest changes with bundle_content_identity.