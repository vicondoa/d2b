# Unit-Test Audit - Consolidated Findings, Verdicts, and Remediation Sequencing

**Date:** 2026-09-24 · **Branch:** `v3` @ `236705d2630c5930eb74860c9a1d43178d491090`

**Method (one paragraph).** A read-only, per-crate unit-test audit with exactly one lane per workspace crate (75 crate lanes plus two cross-cutting lanes, C1 `_cross-helper-duplication.md` and C2 `_cross-layer-overlap.md`), following the same execution model as the 2026-09-23 ponytail audit of product code (`docs/audits/2026-09-23-ponytail-audit/README.md` - same consolidated-report shape: executive summary, ranked findings table, per-family sections, reconciliation). Each lane enumerated every `#[test]`/`#[tokio::test]` fn in `packages/<crate>/src/**` with file:line, wrote a one-line behavior statement per test, and applied a **redundancy-first stance**: tests are cut only when another test already pins the same observable behavior, and every `duplicate:` finding MUST cite its covering test as `path::test_name` - no citation, no duplicate (the rule has no exceptions). Verdict tags are exactly `duplicate:` (same behavior pinned elsewhere; keeper = the test pinning strictly more behavior, or the clearer name on ties), `trivial:` (plumbing: defaults, derive/constructor echoes, mock-echo - nothing observable lost), `keep:` (everything else), and `gap:` (a real product error path/boundary with no test anywhere in the crate; additions, not deletions - max 3 per lane, none invented). Integration/contract tests (`tests/*.rs`, `tests/golden/`, sibling-crate tests) were read as *covering references* but never as audit targets; cross-crate duplicates were flagged `cross-check:` and resolved only by C2. `#[ignore]`d tests were reported as findings (a broken promise). Lanes are read-only on product/test code; the only write of this audit is this report. **Census caveat:** the `packages/*` regex census over-counts some crates (`#[test]` tokens inside comments/doc text - e.g. d2bd-runtime 458 census vs 454 real fns, d2b-telemetry 36 vs 32, d2b-provider-volume-local 40 vs 37, d2b-unsafe-local-helper 40 vs 29; d2b-broker's 663 includes one false positive while omitting the 37 fns its `roundtrip_test!` macro expands to). Lane `tests:` counts are authoritative and used everywhere below; the census inflation is noted per lane where it occurs.

---

## 1. Executive summary

| Metric | Value |
| --- | --- |
| Crates audited | **94** (75 lanes with unit tests + 19 zero-test crates, §5) |
| Total unit tests | **4,938** (sum of lane `tests:` counts; census regex would say ~4,962) |
| Verdict counts | **duplicate 159** (139 crate-lane + 20 cross-layer C2; 137 bullet rows - 2 lanes compress 2-test families into one row) · **trivial 46** · **keep 4,733** · **gap 133** (additions, not deletions). C1 adds 5 helper-family findings measured in lines only (-646), not test fns. duplicate 159 + trivial 46 = 205 = headline net ✓ |
| **Net** | **-205 tests, -3,829 lines** (reconciled in §7) |
| Lanes with nothing to cut | **17** ("Nothing to cut. Ship.") |
| Product-code bugs routed out | 6 (see §7 route-out appendix) |

### Top-10 crates by net (tests; lines as tiebreak)

| # | Crate | Net tests | Net lines | Top finding |
| --- | --- | ---: | ---: | --- |
| 1 | d2b-zone-routing | -24 | -488 | 20 engine/service/resolver unit tests re-pinned by `tests/route_engine_vectors.rs` + 2 trivial |
| 2 | d2bd | -13 | -164 | audio-dispatch `Applied`/`Failed` mapping dups + 5 fake mock-echo trivials + stale-fence dups |
| 3 | d2b-broker | -11 | -188 | 1 crash-window dup + 10 trivial mock-echo/plumbing pins |
| 4 | d2b-provider-device-security-key | -10 | -120 | relay parse/CID/lease families re-pinned by `relay.rs` keepers |
| 5 | d2b-provider-display-wayland | -9 | -164 | fd-handoff close dups, 3 VecDeque-semantics trivials, rate-limiter dup |
| 6 | d2b-contracts | -8 | -38 | `WorkloadTarget`/`RealmTarget` alias parse dups + 2 constructor echoes |
| 7 | d2b-provider-credential-managed-identity | -6 | -100 | plan-gating/canary/placement dups re-pinned by integration tests |
| 8 | d2b-contracts-broker | -6 | -100 | envelope/legacy-field deny-unknown dups |
| 9 | d2bd-runtime | -6 | -60 | readiness/proc-state parse dups + lock-parent dup |
| 10 | d2b-resource-runtime | -5 | -103 | watch `Missed` dup + 4 trivial echoes |

---

## 2. Global ranked findings table (all findings, ranked by net)

Ranked by lane net (tests, then lines), findings in each lane's stated order (lanes list biggest-first). Net column shows the lane's net; C2 rows carry the cross-layer net (-20 tests, -305 lines); C1 rows are **lines only** (-646 lines of duplicated helpers - helper families, not test fns). Full per-finding detail lives in each lane file.

| Rank | Tag | Finding | Crate | Lane | Net |
| --- | --- | --- | --- | --- | --- |
| 1 | duplicate | `a_second_advertiser_claiming_the_same_descendant_is_refused_as_multi_parent` (src/engine.rs:2879)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 2 | duplicate | `an_allocation_for_another_edge_or_generation_is_refused` (src/engine.rs:2815)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 3 | duplicate | `a_route_outside_the_allocated_prefix_or_capability_scope_is_refused` (src/engine.rs:2778)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 4 | duplicate | `a_withdrawal_removes_only_the_named_live_routes` (src/engine.rs:3189) - covered by `withdrawal_vectors` (tests/route_engine_vectors.rs:1066). | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 5 | duplicate | `relay_forwarding_never_exceeds_the_initial_protocol_budget` (src/engine.rs:3159)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 6 | duplicate | `a_relay_hop_refuses_an_exhausted_budget_a_dead_link_and_an_attachment` (src/engine.rs:3130)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 7 | duplicate | `an_expired_advertisement_and_a_future_dated_one_are_both_refused` (src/engine.rs:2728)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 8 | duplicate | `an_advertisement_that_does_not_advance_its_issue_time_is_refused_as_replay` (src/engine.rs:2704)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 9 | duplicate | `disjoint_trees_have_no_nearest_common_ancestor` (src/engine.rs:2576)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 10 | duplicate | `a_two_hop_downward_route_is_allowed_and_pays_its_hops` (src/engine.rs:2498)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 11 | duplicate | `a_replayed_advertisement_is_refused_on_the_exact_window_key` (src/engine.rs:2654)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 12 | duplicate | `an_advertisement_from_an_unknown_parent_is_refused` (src/engine.rs:2756)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 13 | duplicate | `a_hop_budget_smaller_than_the_path_is_refused` (src/engine.rs:3045) - covered by `hop_count_boundary_vectors` (tests/route_engine_vectors.rs:1277). | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 14 | duplicate | `a_future_dated_withdrawal_is_refused_as_malformed` (src/engine.rs:3274) - covered by `withdrawal_vectors` (tests/route_engine_vectors.rs:1066). | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 15 | duplicate | `the_local_root_target_asserts_no_advertised_ceiling` (src/engine.rs:3028)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 16 | duplicate | `an_unknown_target_zone_is_refused` (src/engine.rs:2566) - covered by `unknown_and_disjoint_zone_vectors` (tests/route_engine_vectors.rs:699). | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 17 | duplicate | `a_capability_the_target_never_advertised_is_refused` (src/engine.rs:3018)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 18 | duplicate | `an_exhausted_hop_budget_is_refused_before_the_walk` (src/engine.rs:3067)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 19 | duplicate | `remote_route_without_runtime_admission_is_refused` (src/engine.rs:2077)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 20 | duplicate | `policy_denial_is_reported_before_the_tree_walk` (src/engine.rs:2488)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 21 | duplicate | `projection_request_defaults_refuse` (src/service.rs:1503) - covered by `an_unauthenticated_projection_reports_every_remote_row_unreachable` (src/serv… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 22 | duplicate | `an_unauthenticated_route_projection_refuses_a_remote_entrypoint` (src/resolver.rs:765)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 23 | trivial | `every_reason_the_engine_can_produce_is_covered_by_this_suite` (src/engine.rs:3333)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 24 | trivial | `durable_exec_table_is_bounded_to_ephemeral_processes` (src/router.rs:505)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 25 | gap | `ZoneOperationRouter::begin`/`complete` error paths - `InvalidExpiry` (expiry ≤ now), `InvalidRequestDigest` (empty digest)… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 26 | gap | `DurableExecTable` bound and rejections - `WrongExecutionType` for a non-`EphemeralProcess` ref, `DuplicateOperation`… | d2b-zone-routing | `lane/d2b-zone-routing.md` | -24 t / -488 L |
| 27 | duplicate | `scm_rights_receipt_fd_does_not_inherit_across_exec` (packages/d2b-broker/src/fd_passing.rs:298)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 28 | duplicate | `storage_lifecycle_report_clean_is_pass` (packages/d2b/src/doctor.rs:2027)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 29 | duplicate | `policy_runs_before_capacity_and_rejects_the_whole_frame` (packages/d2b-provider-observability-otel/src/ingress_policy.rs:855)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 30 | duplicate | `delete_persistent_tap_checks_both_fences_before_mutation` (packages/d2b-broker/src/ops/network.rs:1334)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 31 | duplicate | `kernel_module_matrix_clean_is_pass` (packages/d2b/src/doctor.rs:1902)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 32 | duplicate | `kernel_module_matrix_required_missing_is_fail` (packages/d2b/src/doctor.rs:1946)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 33 | duplicate | `process_unique_token_and_identity_canaries_never_render` (packages/d2b-provider-credential-entra/src/audit.rs:39)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 34 | duplicate | `autostart_status_failed_is_fail` (packages/d2b/src/doctor.rs:1966)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 35 | duplicate | `process_unique_entra_canary_is_rejected_from_closed_values` (packages/d2b-provider-credential-entra/src/telemetry.rs:34)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 36 | duplicate | `process_unique_canary_is_rejected_as_an_allowed_key_value` (packages/d2b-provider-credential-secret-service/src/telemetry.rs:32)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 37 | duplicate | `delete_persistent_tap_foreign_marker_fails_without_deletion` (packages/d2b-broker/src/ops/network.rs:1365)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 38 | duplicate | `provider_committed_control_shapes_are_admitted` (packages/d2b-provider-endpoint/src/driver.rs:1026)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 39 | duplicate | `forbidden_identity_keys_fail_before_values` (packages/d2b-telemetry/src/metric_label_policy.rs:53)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 40 | duplicate | `delete_persistent_tap_validated_absence_is_idempotent` (packages/d2b-broker/src/ops/network.rs:1354)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 41 | duplicate | `exit_code_is_two_when_fail` (packages/d2b/src/doctor.rs:1812)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 42 | duplicate | `semaphore_refuses_at_cap_then_readmits_after_release` (packages/d2bd/src/composition.rs:24038)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 43 | duplicate | `autostart_status_degraded_is_warn` (packages/d2b/src/doctor.rs:1988)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 44 | duplicate | `if_name_rejects_invalid_names` (packages/d2b-core/src/host.rs:553)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 45 | duplicate | `if_name_accepts_safe_linux_names` (packages/d2b-core/src/host.rs:547)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 46 | duplicate | `ambient_sdk_chain_names_are_rejected_without_reading_values` (packages/d2b-provider-credential-secret-service/src/lib.rs:2148)… | cross-layer (C2) | `lane/_cross-layer-overlap.md` | -20 t / -305 L |
| 47 | duplicate | `qemu_controller_never_calls_target_process_path` (src/audio_dispatch.rs:812)… | d2bd | `lane/d2bd.md` | -13 t / -164 L |
| 48 | duplicate | `qemu_controller_applied_maps_to_host_only` (src/audio_dispatch.rs:801)… | d2bd | `lane/d2bd.md` | -13 t / -164 L |
| 49 | duplicate | `fake_controller_failure_on_level_maps_to_unsupported` (src/audio_dispatch.rs:789)… | d2bd | `lane/d2bd.md` | -13 t / -164 L |
| 50 | duplicate | `qemu_controller_level_is_applied` (src/audio_host_controller.rs:412)… | d2bd | `lane/d2bd.md` | -13 t / -164 L |
| 51 | duplicate | `qemu_controller_on_grant_is_applied` (src/audio_host_controller.rs:422)… | d2bd | `lane/d2bd.md` | -13 t / -164 L |
| 52 | duplicate | `a_context_minted_against_an_older_provider_set_revision_is_refused` (src/forward_rendezvous.rs:3435)… | d2bd | `lane/d2bd.md` | -13 t / -164 L |
| 53 | duplicate | `a_context_minted_against_another_zone_is_refused` (src/forward_rendezvous.rs:3479)… | d2bd | `lane/d2bd.md` | -13 t / -164 L |
| 54 | duplicate | `the_plane_drains_its_providers` (src/resource_plane_v3.rs:5240)… | d2bd | `lane/d2bd.md` | -13 t / -164 L |
| 55 | trivial | `fake_success_returns_applied_for_grant` (src/audio_host_controller.rs:349)… | d2bd | `lane/d2bd.md` | -13 t / -164 L |
| 56 | trivial | `fake_success_returns_applied_for_level` (src/audio_host_controller.rs:358) - mock-echo of configurable fake; nothing lost. | d2bd | `lane/d2bd.md` | -13 t / -164 L |
| 57 | trivial | `fake_failed_returns_failed_for_grant` (src/audio_host_controller.rs:368) - mock-echo of configurable fake; nothing lost. | d2bd | `lane/d2bd.md` | -13 t / -164 L |
| 58 | trivial | `fake_failed_returns_failed_for_level` (src/audio_host_controller.rs:377) - mock-echo of configurable fake; nothing lost. | d2bd | `lane/d2bd.md` | -13 t / -164 L |
| 59 | trivial | `fake_unsupported_returns_unsupported` (src/audio_host_controller.rs:387)… | d2bd | `lane/d2bd.md` | -13 t / -164 L |
| 60 | gap | the full vm-start supervisor DAG drive is `#[ignore]`d as flaky (src/composition.rs:25679 `vm_start_drives_supervisor_dag_in_topo_order`)… | d2bd | `lane/d2bd.md` | -13 t / -164 L |
| 61 | gap | local SIGTERM→SIGKILL stop escalation is `#[ignore]`d as flaky (src/composition.rs:27132 `vm_stop_escalates_to_sigkill_after_term_timeout`)… | d2bd | `lane/d2bd.md` | -13 t / -164 L |
| 62 | gap | audio dispatch error paths are untested (src/audio_dispatch.rs:386-680)… | d2bd | `lane/d2bd.md` | -13 t / -164 L |
| 63 | duplicate | `crash_between_durable_commit_and_effect_reconciles_without_double_grant_or_leak` (src/state_cells.rs:1359)… | d2b-broker | `lane/d2b-broker.md` | -11 t / -188 L |
| 64 | trivial | `umask_validation_bound_is_0o777` (src/sys.rs:4097) - asserts `0o007 <= 0o777` and `0o1000 > 0o777` on literals… | d2b-broker | `lane/d2b-broker.md` | -11 t / -188 L |
| 65 | trivial | `fake_records_ip_route` (src/ops/exec_reconcile.rs:2146) - mock-echo: asserts the `FakeReconcileExecutor` test double records an `IpRoute` op it was h… | d2b-broker | `lane/d2b-broker.md` | -11 t / -188 L |
| 66 | trivial | `fake_records_nft_apply` (src/ops/exec_reconcile.rs:2101) - mock-echo of the fake's `apply_nft_script` recording… | d2b-broker | `lane/d2b-broker.md` | -11 t / -188 L |
| 67 | trivial | `fake_records_ssh_keygen` (src/ops/exec_reconcile.rs:2169) - mock-echo of the fake's `run_ssh_keygen` recording; nothing a consumer observes. | d2b-broker | `lane/d2b-broker.md` | -11 t / -188 L |
| 68 | trivial | `user_namespace_round_trips_some` (src/ops/spawn_runner.rs:504)… | d2b-broker | `lane/d2b-broker.md` | -11 t / -188 L |
| 69 | trivial | `fake_records_atomic_write` (src/ops/exec_reconcile.rs:2132) - mock-echo of the fake's `write_atomic_file` recording… | d2b-broker | `lane/d2b-broker.md` | -11 t / -188 L |
| 70 | trivial | `fake_records_sysctl_write` (src/ops/exec_reconcile.rs:2119) - mock-echo of the fake's `write_sysctl` recording… | d2b-broker | `lane/d2b-broker.md` | -11 t / -188 L |
| 71 | trivial | `isolation_spec_umask_field_accepts_octal_007` (src/sys.rs:3876)… | d2b-broker | `lane/d2b-broker.md` | -11 t / -188 L |
| 72 | trivial | `isolation_spec_umask_field_defaults_to_none` (src/sys.rs:3870)… | d2b-broker | `lane/d2b-broker.md` | -11 t / -188 L |
| 73 | trivial | `user_namespace_round_trips_none` (src/ops/spawn_runner.rs:498) - default-echo: `None` in, `None` out of preflight. Nothing lost. | d2b-broker | `lane/d2b-broker.md` | -11 t / -188 L |
| 74 | gap | per-live-arm typed audit record shapes are pinned only by `dispatch_request_writes_typed_op_audit_records_for_all_live_arms` (src/runtime.rs:16715)… | d2b-broker | `lane/d2b-broker.md` | -11 t / -188 L |
| 75 | duplicate | `parse_init_packet_identifies_cmd_and_cid` (src/relay_service.rs:668)… | d2b-provider-device-security-key | `lane/d2b-provider-device-security-key.md` | -10 t / -120 L |
| 76 | duplicate | `cid_translator_allocs_fresh_host_cid` (src/relay_service.rs:721) - covered by `cid_translation_isolated_and_released` (src/relay.rs:370). | d2b-provider-device-security-key | `lane/d2b-provider-device-security-key.md` | -10 t / -120 L |
| 77 | duplicate | `cid_translator_release_removes_mapping` (src/relay_service.rs:768) - covered by `cid_translation_isolated_and_released` (src/relay.rs:370). | d2b-provider-device-security-key | `lane/d2b-provider-device-security-key.md` | -10 t / -120 L |
| 78 | duplicate | `lease_acquire_succeeds_when_available` (src/relay_service.rs:780) - covered by `lease_busy_and_release_are_owner_bound` (src/relay.rs:382). | d2b-provider-device-security-key | `lane/d2b-provider-device-security-key.md` | -10 t / -120 L |
| 79 | duplicate | `lease_acquire_fails_when_held_by_other_vm` (src/relay_service.rs:788) - covered by `lease_busy_and_release_are_owner_bound` (src/relay.rs:382). | d2b-provider-device-security-key | `lane/d2b-provider-device-security-key.md` | -10 t / -120 L |
| 80 | duplicate | `lease_release_makes_key_available` (src/relay_service.rs:799) - covered by `lease_busy_and_release_are_owner_bound` (src/relay.rs:382). | d2b-provider-device-security-key | `lane/d2b-provider-device-security-key.md` | -10 t / -120 L |
| 81 | duplicate | `lease_release_wrong_vm_does_not_release` (src/relay_service.rs:807) - covered by `lease_busy_and_release_are_owner_bound` (src/relay.rs:382). | d2b-provider-device-security-key | `lane/d2b-provider-device-security-key.md` | -10 t / -120 L |
| 82 | duplicate | `contention_second_vm_cannot_acquire_active_lease` (src/relay_service.rs:843)… | d2b-provider-device-security-key | `lane/d2b-provider-device-security-key.md` | -10 t / -120 L |
| 83 | duplicate | `hidraw_device_write_report_to_socket_succeeds` (src/relay_service.rs:946)… | d2b-provider-device-security-key | `lane/d2b-provider-device-security-key.md` | -10 t / -120 L |
| 84 | trivial | `disabled_vm_is_not_registered_in_enabled_set` (src/relay_service.rs:854) - asserts a fresh `SecurityKeyState` has an empty `enabled_vms` set, i.e. | d2b-provider-device-security-key | `lane/d2b-provider-device-security-key.md` | -10 t / -120 L |
| 85 | duplicate | `rate_limiter_suppresses_after_max` (wayland_proxy/diag.rs:222) - covered by `bind_denied_rate_limits_by_interface` (wayland_proxy/diag.rs:282). | d2b-provider-display-wayland | `lane/d2b-provider-display-wayland.md` | -9 t / -164 L |
| 86 | duplicate | `failed_handoff_closes_local_fd_copy` (wayland_proxy/bridge.rs:519)… | d2b-provider-display-wayland | `lane/d2b-provider-display-wayland.md` | -9 t / -164 L |
| 87 | duplicate | `backpressured_handoff_closes_local_fd_copy` (wayland_proxy/bridge.rs:524)… | d2b-provider-display-wayland | `lane/d2b-provider-display-wayland.md` | -9 t / -164 L |
| 88 | duplicate | `identity_target_is_not_overridden_by_app_id_metadata` (wayland_proxy/policy.rs:592)… | d2b-provider-display-wayland | `lane/d2b-provider-display-wayland.md` | -9 t / -164 L |
| 89 | trivial | `create_dimensions_are_queued_for_multiple_async_creates` (wayland_proxy/dmabuf.rs:1040)… | d2b-provider-display-wayland | `lane/d2b-provider-display-wayland.md` | -9 t / -164 L |
| 90 | trivial | `filtered_globals_preserve_original_global_names` (wayland_proxy/filter.rs:3213)… | d2b-provider-display-wayland | `lane/d2b-provider-display-wayland.md` | -9 t / -164 L |
| 91 | trivial | `standard_clipboard_global_is_advertised_as_synthetic` (wayland_proxy/filter.rs:3264)… | d2b-provider-display-wayland | `lane/d2b-provider-display-wayland.md` | -9 t / -164 L |
| 92 | gap | StopRequest::Active immediate-termination branch (`DisplayController::finalize`, controller.rs:1308-1318)… | d2b-provider-display-wayland | `lane/d2b-provider-display-wayland.md` | -9 t / -164 L |
| 93 | gap | volume-not-deleted branch requesting `delete_runtime_volume` (controller.rs:1327-1335)… | d2b-provider-display-wayland | `lane/d2b-provider-display-wayland.md` | -9 t / -164 L |
| 94 | gap | `enqueue_bridge_handoff` queue-full drop path at `MAX_PENDING_BRIDGE_HANDOFFS` (wayland_proxy/filter.rs:646-658)… | d2b-provider-display-wayland | `lane/d2b-provider-display-wayland.md` | -9 t / -164 L |
| 95 | duplicate | `workload_target_parse_canonical` (src/workload_identity.rs:209) - covered by `canonical_examples_parse_and_render` (src/target.rs:604). | d2b-contracts | `lane/d2b-contracts.md` | -8 t / -38 L |
| 96 | duplicate | `workload_target_parse_nested_realm` (src/workload_identity.rs:216) - covered by `canonical_examples_parse_and_render` (src/target.rs:604). | d2b-contracts | `lane/d2b-contracts.md` | -8 t / -38 L |
| 97 | duplicate | `workload_target_rejects_no_dot` (src/workload_identity.rs:224) - covered by `bare_alias_requires_context_by_default` (src/target.rs:654). | d2b-contracts | `lane/d2b-contracts.md` | -8 t / -38 L |
| 98 | duplicate | `workload_target_rejects_missing_d2b_suffix` (src/workload_identity.rs:229)… | d2b-contracts | `lane/d2b-contracts.md` | -8 t / -38 L |
| 99 | duplicate | `stable_fingerprint_orders_by_capability_code` (src/capability.rs:478)… | d2b-contracts | `lane/d2b-contracts.md` | -8 t / -38 L |
| 100 | duplicate | `frame_too_large_is_rejected` (src/lib.rs:328) - covered by `encode_frame_public_sock_cap_boundary_is_exact` (src/lib.rs:335). | d2b-contracts | `lane/d2b-contracts.md` | -8 t / -38 L |
| 101 | trivial | `workload_identity_new_has_none_optional_fields` (src/workload_identity.rs:234) - constructor stores its arguments and leaves optionals `None`. | d2b-contracts | `lane/d2b-contracts.md` | -8 t / -38 L |
| 102 | trivial | `workload_identity_target_accessor` (src/workload_identity.rs:244) - accessor echoes the stored `canonical_target`. Nothing lost. | d2b-contracts | `lane/d2b-contracts.md` | -8 t / -38 L |
| 103 | gap | configured-argv argc cap (128) and total-byte cap (16 KiB) never exercised (src/configured_argv.rs:31-53)… | d2b-contracts | `lane/d2b-contracts.md` | -8 t / -38 L |
| 104 | gap | `decode_frame` short-frame (<4 bytes → `frame-too-short`) and invalid-JSON body → `wire-malformed-json` untested (src/lib.rs:252-275)… | d2b-contracts | `lane/d2b-contracts.md` | -8 t / -38 L |
| 105 | gap | `SemverRange::new` / `Version::new` invalid-input rejection untested (src/error.rs) - only the valid-range match path is pinned. | d2b-contracts | `lane/d2b-contracts.md` | -8 t / -38 L |
| 106 | duplicate | `broker_request_envelope_round_trips_with_admin` (src/broker_wire.rs:3181)… | d2b-contracts-broker | `lane/d2b-contracts-broker.md` | -6 t / -100 L |
| 107 | duplicate | `set_bridge_port_flags_rejects_raw_bridge_field` (src/broker_wire.rs:3436)… | d2b-contracts-broker | `lane/d2b-contracts-broker.md` | -6 t / -100 L |
| 108 | duplicate | `create_persistent_tap_rejects_raw_ifname_field` (src/broker_wire.rs:3460)… | d2b-contracts-broker | `lane/d2b-contracts-broker.md` | -6 t / -100 L |
| 109 | duplicate | `create_tap_fd_rejects_invalid_ifname` (src/broker_wire.rs:3803)… | d2b-contracts-broker | `lane/d2b-contracts-broker.md` | -6 t / -100 L |
| 110 | duplicate | `usbip_bind_firewall_rule_rejects_raw_bus_id_field` (src/broker_wire.rs:3481)… | d2b-contracts-broker | `lane/d2b-contracts-broker.md` | -6 t / -100 L |
| 111 | duplicate | `broker_caller_role_default_is_not_authorized` (src/broker_wire.rs:3135)… | d2b-contracts-broker | `lane/d2b-contracts-broker.md` | -6 t / -100 L |
| 112 | gap | RunnerRole kebab-case wire tokens for `ProviderController`, `QemuMedia`… | d2b-contracts-broker | `lane/d2b-contracts-broker.md` | -6 t / -100 L |
| 113 | gap | `for_display` audit labels for `RootUid` and `HostShutdownUid` unpinned (`broker_caller_role_display_uses_stable_audit_labels` covers 3 of 5 variants;… | d2b-contracts-broker | `lane/d2b-contracts-broker.md` | -6 t / -100 L |
| 114 | duplicate | `agent_is_planned_only_after_admission_and_dependency_readiness` (src/controller.rs:337)… | d2b-provider-credential-managed-identity | `lane/d2b-provider-credential-managed-identity.md` | -6 t / -100 L |
| 115 | duplicate | `ready_and_unavailable_are_closed_status_observations` (src/controller.rs:311)… | d2b-provider-credential-managed-identity | `lane/d2b-provider-credential-managed-identity.md` | -6 t / -100 L |
| 116 | duplicate | `collector_allowlist_rejects_nonclosed_values_for_allowed_keys` (src/telemetry.rs:33)… | d2b-provider-credential-managed-identity | `lane/d2b-provider-credential-managed-identity.md` | -6 t / -100 L |
| 117 | duplicate | `user_agent_placement_is_rejected` (src/lib.rs:1518) - covered by `machine_placements_are_accepted_and_user_agent_is_rejected` (tests/placement.rs:8).… | d2b-provider-credential-managed-identity | `lane/d2b-provider-credential-managed-identity.md` | -6 t / -100 L |
| 118 | duplicate | `client_id_is_redacted_from_debug` (src/lib.rs:1530) - covered by `process_unique_managed_identity_canaries_are_absent_from_rendered_surfaces` (tests/… | d2b-provider-credential-managed-identity | `lane/d2b-provider-credential-managed-identity.md` | -6 t / -100 L |
| 119 | duplicate | `process_unique_managed_identity_canary_never_renders` (src/audit.rs:38)… | d2b-provider-credential-managed-identity | `lane/d2b-provider-credential-managed-identity.md` | -6 t / -100 L |
| 120 | gap | `poll_client_sync` deadline expiry - Pending future past deadline must return `DeadlineExceeded` (src/lib.rs:1171-1176)… | d2b-provider-credential-managed-identity | `lane/d2b-provider-credential-managed-identity.md` | -6 t / -100 L |
| 121 | gap | `ManagedIdentityClientConfig::new` lease-ceiling rejection - `max_leases` outside `1..=MAX_LOCAL_LEASES` must fail closed (src/lib.rs:520-522)… | d2b-provider-credential-managed-identity | `lane/d2b-provider-credential-managed-identity.md` | -6 t / -100 L |
| 122 | gap | `ManagedIdentityCredentialProviderFactory::new` consumer check - non-`Provider` `consumer_ref` must return `InvalidConsumer` (src/lib.rs:801-804)… | d2b-provider-credential-managed-identity | `lane/d2b-provider-credential-managed-identity.md` | -6 t / -100 L |
| 123 | duplicate | `resize_retry_with_same_op_id_replays_cached_ack` (src/exec_session.rs:3150)… | d2bd-runtime | `lane/d2bd-runtime.md` | -6 t / -60 L |
| 124 | duplicate | `validate_lock_parent_accepts_production_tmpfile_shape`（src/runtime_process.rs:774)… | d2bd-runtime | `lane/d2bd-runtime.md` | -6 t / -60 L |
| 125 | duplicate | `comm_with_paren`（src/readiness.rs:415) - covered by `comm_with_spaces_and_paren`（src/readiness.rs:423). | d2bd-runtime | `lane/d2bd-runtime.md` | -6 t / -60 L |
| 126 | duplicate | `no_paren_at_all`（src/readiness.rs:436) - covered by `empty_input`（src/readiness.rs:443). | d2bd-runtime | `lane/d2bd-runtime.md` | -6 t / -60 L |
| 127 | duplicate | `simple_running`（src/readiness.rs:409) - covered by `simple_zombie`（src/readiness.rs:403). | d2bd-runtime | `lane/d2bd-runtime.md` | -6 t / -60 L |
| 128 | duplicate | `dead_process`（src/readiness.rs:449) - covered by `simple_zombie`（src/readiness.rs:403). Same well-formed→Alive(state char) behavior。 | d2bd-runtime | `lane/d2bd-runtime.md` | -6 t / -60 L |
| 129 | gap | `resource_api` non-list request parsers fail-closed error paths（`parse_resource_names`, `parse_typed_filters`, `typed_filter`, `aliased_cursor`… | d2bd-runtime | `lane/d2bd-runtime.md` | -6 t / -60 L |
| 130 | gap | `broker_transport` mode-bound broker dispatch fail-closed（`validate_instance` → SocketPath/InstanceMismatch;`dispatch` → RequestDenied… | d2bd-runtime | `lane/d2bd-runtime.md` | -6 t / -60 L |
| 131 | gap | `resource_operator_activation` `select_wave6_resources`/`select_authenticated_resource` error paths（ResourceSelection/ProviderRoute… | d2bd-runtime | `lane/d2bd-runtime.md` | -6 t / -60 L |
| 132 | duplicate | `async_recv_drains_then_missed_then_ends` (src/watch.rs:891) - covered by `slow_subscriber_gets_explicit_missed_never_silent_drop` (src/watch.rs:737).… | d2b-resource-runtime | `lane/d2b-resource-runtime.md` | -5 t / -103 L |
| 133 | trivial | `watch_and_manager_shapes_send_and_receive_intact` (src/context.rs:1594)… | d2b-resource-runtime | `lane/d2b-resource-runtime.md` | -5 t / -103 L |
| 134 | trivial | `modules_resolve` (src/lib.rs:69) - echoes each module's `MODULE_NAME` constant against its literal name… | d2b-resource-runtime | `lane/d2b-resource-runtime.md` | -5 t / -103 L |
| 135 | trivial | `regenerate_failure_kind_reference` (src/error.rs:1026, `#[ignore]`d)… | d2b-resource-runtime | `lane/d2b-resource-runtime.md` | -5 t / -103 L |
| 136 | trivial | `wire_budget_bounds_sequence_for_u32_low_word` (src/revision.rs:183)… | d2b-resource-runtime | `lane/d2b-resource-runtime.md` | -5 t / -103 L |
| 137 | duplicate | `api_status_updates_have_no_persistent_write_path` (src/manager_backend/tests.rs:1773)… | d2b-resource-api | `lane/d2b-resource-api.md` | -5 t / -95 L |
| 138 | duplicate | `conflict_revision_survives_typed_wire_mapping` (src/error.rs:228)… | d2b-resource-api | `lane/d2b-resource-api.md` | -5 t / -95 L |
| 139 | duplicate | `conflict_revision_can_be_hidden_without_changing_the_kind` (src/error.rs:282)… | d2b-resource-api | `lane/d2b-resource-api.md` | -5 t / -95 L |
| 140 | trivial | `configuration_revision_is_a_monotonic_ordinal_in_the_snapshot` (src/authz.rs:2965)… | d2b-resource-api | `lane/d2b-resource-api.md` | -5 t / -95 L |
| 141 | trivial | `status_owner_matching_generation_is_representable` (src/service.rs:3377)… | d2b-resource-api | `lane/d2b-resource-api.md` | -5 t / -95 L |
| 142 | duplicate | `cancellation_propagates_delivery_failure_after_local_cleanup` (src/admission.rs:1944)… | d2b-session | `lane/d2b-session.md` | -5 t / -92 L |
| 143 | duplicate | `frame_type_rejects_trailing_bytes_and_oversize_bodies` (src/client.rs:243) - covered by `frame_validation_is_exact_and_bounded` (src/server.rs:564). | d2b-session | `lane/d2b-session.md` | -5 t / -92 L |
| 144 | trivial | `canonical_member_has_one_slash_and_two_identifiers` (src/operation.rs:337)… | d2b-session | `lane/d2b-session.md` | -5 t / -92 L |
| 145 | trivial | `guest_seed_operation_is_exactly_commit_batch` (src/operation.rs:367)… | d2b-session | `lane/d2b-session.md` | -5 t / -92 L |
| 146 | trivial | `expected_handshake_failures_have_specific_closed_reasons` (src/metrics.rs:96)… | d2b-session | `lane/d2b-session.md` | -5 t / -92 L |
| 147 | gap | `SessionError::new` never panics on any `SessionErrorCode` (src/error.rs… | d2b-session | `lane/d2b-session.md` | -5 t / -92 L |
| 148 | gap | `OperationMember::method` rejects identifiers containing `?` (src/operation.rs:337)… | d2b-session | `lane/d2b-session.md` | -5 t / -92 L |
| 149 | duplicate | `modules_disabled_locks_required_absent_module` (src/modules.rs:566)… | d2b-host | `lane/d2b-host.md` | -5 t / -47 L |
| 150 | duplicate | `stat_failed_for_missing_path` (src/ownership_matrix.rs:512) - covered by `required_missing_entry_reports_not_found_stat_failed` (src/ownership_matrix… | d2b-host | `lane/d2b-host.md` | -5 t / -47 L |
| 151 | duplicate | `marker_round_trip_serializable` (src/hardlink_farm.rs:2288) - covered by `marker_round_trip` (src/hardlink_farm.rs:2223). | d2b-host | `lane/d2b-host.md` | -5 t / -47 L |
| 152 | duplicate | `loose_mode_when_group_execute_set` (src/devices.rs:454) - covered by `loose_mode_when_world_bit_set` (src/devices.rs:440). | d2b-host | `lane/d2b-host.md` | -5 t / -47 L |
| 153 | duplicate | `no_raw_mangle_nat_hooks` (src/nftables.rs:975) - covered by `build_inet_d2b_chains_layout_matches_plan` (src/nftables.rs:946). | d2b-host | `lane/d2b-host.md` | -5 t / -47 L |
| 154 | gap | `extract_managed_block` absent-marker path (src/routes.rs:60-67)… | d2b-host | `lane/d2b-host.md` | -5 t / -47 L |
| 155 | gap | `gunzip_inflate` error paths `TooShort` / `UnsupportedMethod` / `Truncated` / `Inflate` (src/modules.rs:306-347)… | d2b-host | `lane/d2b-host.md` | -5 t / -47 L |
| 156 | duplicate | `an_on_disk_provider_omitted_from_workspace_is_rejected` (src/provider_crate_policy.rs:9918)… | xtask | `lane/xtask.md` | -4 t / -65 L |
| 157 | duplicate | `every_failure_class_exits_nonzero` (src/delivery/mod.rs:370)… | xtask | `lane/xtask.md` | -4 t / -65 L |
| 158 | duplicate | `redaction_preserves_dispatch_evidence_while_removing_secret` (src/bazel_evidence.rs:585)… | xtask | `lane/xtask.md` | -4 t / -65 L |
| 159 | trivial | `filtered_lock_omits_unselected_lock_only_packages` (src/production_closure.rs:1774)… | xtask | `lane/xtask.md` | -4 t / -65 L |
| 160 | duplicate | `the_pair_names_the_exact_frozen_resource_types` (semantic_services/audio.rs:125)… | d2b-contracts-provider | `lane/d2b-contracts-provider.md` | -4 t / -44 L |
| 161 | duplicate | `the_pair_names_the_exact_frozen_resource_types` (semantic_services/security_key.rs:95)… | d2b-contracts-provider | `lane/d2b-contracts-provider.md` | -4 t / -44 L |
| 162 | duplicate | `the_pair_names_the_exact_frozen_resource_types` (semantic_services/telemetry.rs:126)… | d2b-contracts-provider | `lane/d2b-contracts-provider.md` | -4 t / -44 L |
| 163 | duplicate | `the_pair_names_the_exact_frozen_resource_types` (semantic_services/usb.rs:122)… | d2b-contracts-provider | `lane/d2b-contracts-provider.md` | -4 t / -44 L |
| 164 | duplicate | `daemon_input_pins_cross_domain_and_wayland_bind` (src/gpu_argv.rs:269) - covered by `daemon_input_snapshot_line` (src/gpu_argv.rs:257). | d2b-provider-device-gpu | `lane/d2b-provider-device-gpu.md` | -4 t / -36 L |
| 165 | duplicate | `context_type_string_round_trip` (src/gpu_argv.rs:397) - covered by `daemon_input_snapshot_line` (src/gpu_argv.rs:257). | d2b-provider-device-gpu | `lane/d2b-provider-device-gpu.md` | -4 t / -36 L |
| 166 | duplicate | `audit_parity_minimal` (src/video_argv.rs:175) - covered by `audit_parity_snapshot_line` (src/video_argv.rs:266). | d2b-provider-device-gpu | `lane/d2b-provider-device-gpu.md` | -4 t / -36 L |
| 167 | duplicate | `backend_string_round_trip` (src/video_argv.rs:238) - covered by `audit_parity_snapshot_line` (src/video_argv.rs:266). | d2b-provider-device-gpu | `lane/d2b-provider-device-gpu.md` | -4 t / -36 L |
| 168 | duplicate | `explicit_plan_preserves_step_stop_and_execution_order` (src/state_machine.rs:803)… | d2b-provider-device-usbip | `lane/d2b-provider-device-usbip.md` | -3 t / -60 L |
| 169 | duplicate | `bind_failure_rollback_preserves_started_backend` (src/state_machine.rs:601)… | d2b-provider-device-usbip | `lane/d2b-provider-device-usbip.md` | -3 t / -60 L |
| 170 | duplicate | `explicit_plan_carries_explicit_claim_source` (src/state_machine.rs:767)… | d2b-provider-device-usbip | `lane/d2b-provider-device-usbip.md` | -3 t / -60 L |
| 171 | gap | `build_usbip_plan` declared-path failure checks (src/state_machine.rs:254-333)… | d2b-provider-device-usbip | `lane/d2b-provider-device-usbip.md` | -3 t / -60 L |
| 172 | gap | `UsbipBindingContext::new` zero-physical-key / empty-ref rejection (src/broker.rs:41-63)… | d2b-provider-device-usbip | `lane/d2b-provider-device-usbip.md` | -3 t / -60 L |
| 173 | duplicate | `garbage_checkpoint_scratch_without_commit_is_discarded` (src/segment.rs:1379)… | d2b-audit | `lane/d2b-audit.md` | -3 t / -40 L |
| 174 | duplicate | `impossible_success_is_integrity_failure_and_one_sided_terminal_is_not_replayed` (src/reconcile.rs:177)… | d2b-audit | `lane/d2b-audit.md` | -3 t / -40 L |
| 175 | duplicate | `the_root_invocation_id_survives_every_hop` (src/evidence_chain.rs:278)… | d2b-audit | `lane/d2b-audit.md` | -3 t / -40 L |
| 176 | gap | `evidence_from_decision_result` valid closed pairs (allowed/success, denied/denied… | d2b-audit | `lane/d2b-audit.md` | -3 t / -40 L |
| 177 | gap | `AuditSink::append` non-privileged outcomes - Standard/BestEffort → `RateLimited` and append-failure → `DroppedUnavailable` are never exercised (all s… | d2b-audit | `lane/d2b-audit.md` | -3 t / -40 L |
| 178 | gap | export bounds - `MAX_EXPORT_RECORDS`/`MAX_EXPORT_BYTES`/`MAX_EXPORT_SCAN_LINES`/`MAX_EXPORT_SCAN_BYTES`/`MAX_EXPORT_DIRECTORY_ENTRIES` limits and `aft… | d2b-audit | `lane/d2b-audit.md` | -3 t / -40 L |
| 179 | duplicate | `an_unregistered_admitted_operation_fails_the_startup_invariant` (src/seam.rs:720)… | d2b-broker-composition | `lane/d2b-broker-composition.md` | -3 t / -36 L |
| 180 | duplicate | `production_registration_accepts_only_the_catalog_row` (src/seam.rs:600)… | d2b-broker-composition | `lane/d2b-broker-composition.md` | -3 t / -36 L |
| 181 | duplicate | `the_pure_fixture_crate_passes_the_source_surface_probe` (src/dependency_surface.rs:426)… | d2b-broker-composition | `lane/d2b-broker-composition.md` | -3 t / -36 L |
| 182 | gap | `register_production_handlers` row-identity `ptr::eq` refusal branch (src/seam.rs:141-150)… | d2b-broker-composition | `lane/d2b-broker-composition.md` | -3 t / -36 L |
| 183 | gap | `verify_startup_routing` admitted-without-handler leg, the "committed operation(s) route to the in-broker leg with no registered handler" error (src/s… | d2b-broker-composition | `lane/d2b-broker-composition.md` | -3 t / -36 L |
| 184 | duplicate | `host_system_placement_is_rejected` (src/lib.rs:1368) - covered by `guest_user_and_system_domains_are_accepted_but_host_system_is_rejected` (tests/pla… | d2b-provider-credential-entra | `lane/d2b-provider-credential-entra.md` | -3 t / -32 L |
| 185 | trivial | `cleanup_metadata_preserves_invalid_grant_fences` (src/service.rs:783)… | d2b-provider-credential-entra | `lane/d2b-provider-credential-entra.md` | -3 t / -32 L |
| 186 | trivial | `exact_consumer_guard_is_independent_of_request_fields` (src/lib.rs:1361)… | d2b-provider-credential-entra | `lane/d2b-provider-credential-entra.md` | -3 t / -32 L |
| 187 | gap | `EntraConfig::new` lease-bound rejection (`max_leases` outside 1..=MAX_LOCAL_LEASES → `InvalidConfig`, src/lib.rs:505)… | d2b-provider-credential-entra | `lane/d2b-provider-credential-entra.md` | -3 t / -32 L |
| 188 | gap | `EntraPlacement::new` `InvalidEndpoint` path (non-Guest identity ref, non-Endpoint login ref, or `endpoint_generation == 0`, src/lib.rs:591)… | d2b-provider-credential-entra | `lane/d2b-provider-credential-entra.md` | -3 t / -32 L |
| 189 | gap | `EntraCredentialProviderFactory::new` `InvalidConsumer` (consumer_ref not a Provider ref, src/lib.rs:854) - no test anywhere in the crate. | d2b-provider-credential-entra | `lane/d2b-provider-credential-entra.md` | -3 t / -32 L |
| 190 | duplicate | `the_envelope_never_carries_attachment_settings` (src/bindings.rs:390)… | d2b-provider-volume-virtiofs | `lane/d2b-provider-volume-virtiofs.md` | -3 t / -25 L |
| 191 | duplicate | `the_thread_pool_falls_back_to_the_guest_vcpu_count` (src/worker.rs:161)… | d2b-provider-volume-virtiofs | `lane/d2b-provider-volume-virtiofs.md` | -3 t / -25 L |
| 192 | duplicate | `a_write_binding_over_a_read_only_view_is_rejected` (src/worker.rs:171)… | d2b-provider-volume-virtiofs | `lane/d2b-provider-volume-virtiofs.md` | -3 t / -25 L |
| 193 | gap | `vcpu_count == 0` → `InvalidBinding` (src/worker.rs:47) - real guard boundary in `VirtiofsdWorkerPlan::for_binding`… | d2b-provider-volume-virtiofs | `lane/d2b-provider-volume-virtiofs.md` | -3 t / -25 L |
| 194 | duplicate | `a_guest_execution_binding_with_a_wrong_digest_refuses_through_the_hosted_service` (src/effects_service.rs:351)… | d2b-provider-process-systemd | `lane/d2b-provider-process-systemd.md` | -2 t / -66 L |
| 195 | trivial | `the_factory_builds_a_new_service_value` (src/effects_service.rs:336) - asserts two `Factory::build()` calls return non-`Arc::ptr_eq` instances. | d2b-provider-process-systemd | `lane/d2b-provider-process-systemd.md` | -2 t / -66 L |
| 196 | trivial | `uncertain_revocation_never_unblocks_cleanup` (src/session.rs:478)… | d2b-provider-credential | `lane/d2b-provider-credential.md` | -2 t / -58 L |
| 197 | trivial | `factory_builds_the_service_over_the_same_facet_set` (src/effects_service.rs:271)… | d2b-provider-credential | `lane/d2b-provider-credential.md` | -2 t / -58 L |
| 198 | gap | `CredentialRevocationRequest::new` rejects `session_generation == 0` (`credential-revoke-operation-id` validation at src/session.rs:190-192)… | d2b-provider-credential | `lane/d2b-provider-credential.md` | -2 t / -58 L |
| 199 | duplicate | `offline_verification_refuses_before_the_handoff_is_dispatched` (src/driver.rs:1501)… | d2b-provider-activation-nixos | `lane/d2b-provider-activation-nixos.md` | -2 t / -54 L |
| 200 | duplicate | `factory_registers_only_the_generation_resource_type` (src/driver.rs:1216)… | d2b-provider-activation-nixos | `lane/d2b-provider-activation-nixos.md` | -2 t / -54 L |
| 201 | gap | `recover`'s Adopted branch (src/driver.rs, `recover()`) - a Guest target with an existing owned runner must return `RecoveryOutcome::Adopted` and proj… | d2b-provider-activation-nixos | `lane/d2b-provider-activation-nixos.md` | -2 t / -54 L |
| 202 | gap | StaleGeneration outcome (src/driver.rs, `host_handoff_outcome` Completed-with-equal-generations branch and `execute_host_runner`'s `source_generation… | d2b-provider-activation-nixos | `lane/d2b-provider-activation-nixos.md` | -2 t / -54 L |
| 203 | gap | `activation_detail` Superseded (Ready phase + non-success outcome) and Adopted branches (src/driver.rs)… | d2b-provider-activation-nixos | `lane/d2b-provider-activation-nixos.md` | -2 t / -54 L |
| 204 | duplicate | `relay_grant_and_forwarded_target_grant_fail_independently` (src/authorization.rs:1521)… | d2b-bus | `lane/d2b-bus.md` | -2 t / -47 L |
| 205 | trivial | `the_session_reports_its_link_epoch_and_generation` (src/session/zone_link.rs:1120)… | d2b-bus | `lane/d2b-bus.md` | -2 t / -47 L |
| 206 | gap | `RegistryError::RouteCapacity` (src/registry.rs:689) - no test anywhere in the crate ever hits route-registration capacity exhaustion;(session_seam te… | d2b-bus | `lane/d2b-bus.md` | -2 t / -47 L |
| 207 | gap | `RegistryError::UnauthenticatedTransport` (src/registry.rs:711-737)… | d2b-bus | `lane/d2b-bus.md` | -2 t / -47 L |
| 208 | gap | `route_outcome` classification mapping (src/metrics.rs:578) - BusError -> Ok/Denied/NotFound/Error feeds route metrics… | d2b-bus | `lane/d2b-bus.md` | -2 t / -47 L |
| 209 | duplicate | `policy_runs_before_capacity_and_rejects_the_whole_frame` (src/ingress_policy.rs:855)… | d2b-provider-observability-otel | `lane/d2b-provider-observability-otel.md` | -2 t / -43 L |
| 210 | duplicate | `raw_unknown_descriptor_is_rejected_before_series_accounting` (src/ingress_policy.rs:1004)… | d2b-provider-observability-otel | `lane/d2b-provider-observability-otel.md` | -2 t / -43 L |
| 211 | gap | EmitterSocket::bind parent validation (`validate_socket_parent`, src/emitter_socket.rs:349-379)… | d2b-provider-observability-otel | `lane/d2b-provider-observability-otel.md` | -2 t / -43 L |
| 212 | gap | bounded-storage eviction (byte-budget eviction loop and MAX_RETAINED_AGE `prune_expired`, src/emitter_socket.rs:216-230, 337-346)… | d2b-provider-observability-otel | `lane/d2b-provider-observability-otel.md` | -2 t / -43 L |
| 213 | gap | `ProviderAgentProcess::process_effect` (src/agent.rs:279-304) and the `AuditBackpressure` error path (src/agent.rs:326-330)… | d2b-provider-observability-otel | `lane/d2b-provider-observability-otel.md` | -2 t / -43 L |
| 214 | duplicate | `inspect_network_report_spells_the_catalog_wire_names` (src/effects_service.rs:340)… | d2b-provider-network-local | `lane/d2b-provider-network-local.md` | -2 t / -41 L |
| 215 | duplicate | `foreign_marker_in_target_slot_fails_closed` (src/nftables.rs:721)… | d2b-provider-network-local | `lane/d2b-provider-network-local.md` | -2 t / -41 L |
| 216 | gap | `evaluate_observation` decision state machine (src/observe.rs:545, consumed at src/controller.rs:1064)… | d2b-provider-network-local | `lane/d2b-provider-network-local.md` | -2 t / -41 L |
| 217 | gap | `check_network_services` `RoutesNotApplied` branch (src/routes.rs:387)… | d2b-provider-network-local | `lane/d2b-provider-network-local.md` | -2 t / -41 L |
| 218 | duplicate | `a_zone_outside_the_table_is_refused_before_any_call_is_prepared` (src/client.rs:166)… | d2b-resource-client | `lane/d2b-resource-client.md` | -2 t / -37 L |
| 219 | duplicate | `a_cancelled_cross_zone_call_is_refused_at_the_client_boundary` (src/client.rs:181)… | d2b-resource-client | `lane/d2b-resource-client.md` | -2 t / -37 L |
| 220 | gap | `record_remote_verdict` classification has no test (dispatch.rs:293; reached via process_attach.rs:795 and zone_client.rs:946)… | d2b-resource-client | `lane/d2b-resource-client.md` | -2 t / -37 L |
| 221 | gap | `GuestControlEndpoint::new` rejection path untested (zone_client.rs:163)… | d2b-resource-client | `lane/d2b-resource-client.md` | -2 t / -37 L |
| 222 | gap | `GuestControlEndpoint::validate_for` mismatch untested (zone_client.rs:242)… | d2b-resource-client | `lane/d2b-resource-client.md` | -2 t / -37 L |
| 223 | duplicate | `placement_is_user_agent_only` (src/lib.rs:1974) - covered by `only_user_agent_on_host_or_guest_is_accepted` (tests/placement.rs:6). | d2b-provider-credential-secret-service | `lane/d2b-provider-credential-secret-service.md` | -2 t / -36 L |
| 224 | duplicate | `runtime_provider_accepts_missing_controller_user_scope_claim` (src/lib.rs:1953)… | d2b-provider-credential-secret-service | `lane/d2b-provider-credential-secret-service.md` | -2 t / -36 L |
| 225 | gap | `SecretServicePlacement::new` InvalidScope path - execution_ref not Host/Guest or user_ref not User (src/lib.rs:610-625)… | d2b-provider-credential-secret-service | `lane/d2b-provider-credential-secret-service.md` | -2 t / -36 L |
| 226 | gap | `runtime_provider` route-rejection paths - missing provider ref, non-Guest execution ref, subject≠provider… | d2b-provider-credential-secret-service | `lane/d2b-provider-credential-secret-service.md` | -2 t / -36 L |
| 227 | gap | `SecretServiceConfig::new` bounds - max_leases outside 1..=MAX_LOCAL_LEASES or alias over MAX_COLLECTION_ALIAS_BYTES → `InvalidConfig` (src/lib.rs:523… | d2b-provider-credential-secret-service | `lane/d2b-provider-credential-secret-service.md` | -2 t / -36 L |
| 228 | duplicate | `resolve_refuses_grants_wider_than_the_group_class` (src/layout.rs:419)… | d2b-provider-volume-local | `lane/d2b-provider-volume-local.md` | -2 t / -30 L |
| 229 | duplicate | `replacement_aware_quota_rejects_overage` (src/atomic.rs:487)… | d2b-provider-volume-local | `lane/d2b-provider-volume-local.md` | -2 t / -30 L |
| 230 | duplicate | `duplicate_json_keys_are_rejected_before_envelope_materialization` (src/v3/resource.rs:1145)… | d2b-contracts-resource | `lane/d2b-contracts-resource.md` | -2 t / -27 L |
| 231 | duplicate | `status_debug_redacts_dynamic_and_message_values` (src/v3/resource_status.rs:940)… | d2b-contracts-resource | `lane/d2b-contracts-resource.md` | -2 t / -27 L |
| 232 | gap | seal identity mismatch for zone and epoch kinds (src/v3/operations/seal.rs:229 `diagnose_identity`)… | d2b-contracts-resource | `lane/d2b-contracts-resource.md` | -2 t / -27 L |
| 233 | duplicate | `factory_registers_only_the_binding_resource_type` (src/driver.rs:1440)… | d2b-provider-volume-binding | `lane/d2b-provider-volume-binding.md` | -2 t / -24 L |
| 234 | duplicate | `child_cannot_silently_change_owner` (src/driver.rs:1952) - covered by `absent_parent_row_defers_retryably_while_owner_mismatch_stays_terminal` (src/d… | d2b-provider-volume-binding | `lane/d2b-provider-volume-binding.md` | -2 t / -24 L |
| 235 | gap | driver spec refusal - `SpecInvalid`/`ProviderUnsupported` terminal paths at `decoded_binding` (src/driver.rs:437-458)… | d2b-provider-volume-binding | `lane/d2b-provider-volume-binding.md` | -2 t / -24 L |
| 236 | gap | `ParentSpecInvalid` (src/driver.rs:540) - a present parent Volume row whose uid/spec does not decode (terminal) is untested… | d2b-provider-volume-binding | `lane/d2b-provider-volume-binding.md` | -2 t / -24 L |
| 237 | gap | retryable effect/mutation failures - `ServingEffect` on socket-removal failure (src/driver.rs:1015-1021) and `ChildMutation` on manager ensure/delete… | d2b-provider-volume-binding | `lane/d2b-provider-volume-binding.md` | -2 t / -24 L |
| 238 | duplicate | `factory_registers_only_the_volume_resource_type` (src/driver.rs:1012)… | d2b-provider-volume | `lane/d2b-provider-volume.md` | -2 t / -23 L |
| 239 | trivial | `the_has_layout_wire_payloads_are_canonical` (.src/effects_service.rs:334)… | d2b-provider-volume | `lane/d2b-provider-volume.md` | -2 t / -23 L |
| 240 | duplicate | `factory_registers_only_the_service_type` (src/driver.rs:698)… | d2b-provider-telemetry-service | `lane/d2b-provider-telemetry-service.md` | -2 t / -20 L |
| 241 | duplicate | `validate_accepts_a_provider_declared_spec` (src/driver.rs:729)… | d2b-provider-telemetry-service | `lane/d2b-provider-telemetry-service.md` | -2 t / -20 L |
| 242 | gap | recover/reconcile on a corrupt stored spec return `InvalidResource` + Retryable (envelope error path, src/driver.rs:241)… | d2b-provider-telemetry-service | `lane/d2b-provider-telemetry-service.md` | -2 t / -20 L |
| 243 | gap | reconcile propagates a manager row-read failure as `Reconcile`-class error ( src/driver.rs:304) -the `RecordingManager` fixture never returns an error… | d2b-provider-telemetry-service | `lane/d2b-provider-telemetry-service.md` | -2 t / -20 L |
| 244 | trivial | `a_child_creation_keeps_every_field` (src/child_creation.rs:51)… | d2b-resource-types | `lane/d2b-resource-types.md` | -2 t / -18 L |
| 245 | trivial | `custody_distinguishes_its_two_values` (src/child_creation.rs:61)… | d2b-resource-types | `lane/d2b-resource-types.md` | -2 t / -18 L |
| 246 | duplicate | `accepts_valid_maxish_open_request_line` (src/clipd_host/framing.rs:120)… | d2b-provider-clipboard-wayland | `lane/d2b-provider-clipboard-wayland.md` | -2 t / -14 L |
| 247 | trivial | `published_selection_echo_is_always_suppressed_once` (src/bin/d2b-clipd.rs:4021)… | d2b-provider-clipboard-wayland | `lane/d2b-provider-clipboard-wayland.md` | -2 t / -14 L |
| 248 | gap | `should_suppress_published_selection_echo` wrapper itself (src/bin/d2b-clipd.rs:3776) is never directly unit-tested… | d2b-provider-clipboard-wayland | `lane/d2b-provider-clipboard-wayland.md` | -2 t / -14 L |
| 249 | duplicate | `seccomp_field_parse_disabled` (src/doctor.rs:2288) - covered by `seccomp_field_parse_bpf` (src/doctor.rs:2279). | d2b | `lane/d2b.md` | -2 t / -13 L |
| 250 | duplicate | `nstgid_single_parse` (src/doctor.rs:2360) - covered by `nstgid_nested_parse` (src/doctor.rs:2352). | d2b | `lane/d2b.md` | -2 t / -13 L |
| 251 | gap | `FdStateGuard::enter` raw-mode setup failure path (src/exec_client.rs:982)… | d2b | `lane/d2b.md` | -2 t / -13 L |
| 252 | gap | `CliSocket::send_frame` deadline path (src/context.rs:514) - a stalled send must surface as bounded `TimedOut` like `recv_frame`… | d2b | `lane/d2b.md` | -2 t / -13 L |
| 253 | duplicate | `durable_provider_effect_launch_failure_still_retries_under_the_budget` (src/driver.rs:4334)… | d2b-provider-process | `lane/d2b-provider-process.md` | -1 t / -31 L |
| 254 | gap | family operation handlers (src/operations.rs) - the 11 declared operations (OpenPidfd, SpawnRunner, CgroupKill… | d2b-provider-process | `lane/d2b-provider-process.md` | -1 t / -31 L |
| 255 | gap | validate `ExecutionUnsupported` (src/driver.rs `validate`) - every driver test runs Host mode with `Host/host-system` refs… | d2b-provider-process | `lane/d2b-provider-process.md` | -1 t / -31 L |
| 256 | gap | `restart_delay` multiplier/cap (src/driver.rs `restart_delay`)… | d2b-provider-process | `lane/d2b-provider-process.md` | -1 t / -31 L |
| 257 | duplicate | `reconcile_refuses_a_qemu_guest_without_its_provider_row` (src/driver.rs:2176)… | d2b-provider-guest | `lane/d2b-provider-guest.md` | -1 t / -25 L |
| 258 | gap | target-control service refusals - foreign-zone source, unregistered resource type… | d2b-provider-guest | `lane/d2b-provider-guest.md` | -1 t / -25 L |
| 259 | gap | `CloudHypervisorShutdown::poll_state` state classification - Created/Shutdown → GuestStopped, Running/Paused → Running… | d2b-provider-guest | `lane/d2b-provider-guest.md` | -1 t / -25 L |
| 260 | gap | ACA kind child graph through the driver - `aca_child_ensures` commits the sandbox-agent Endpoint (src/driver.rs) but no unit test asserts the ACA reco… | d2b-provider-guest | `lane/d2b-provider-guest.md` | -1 t / -25 L |
| 261 | duplicate | `audio_set_volume_rejects_out_of_range_level` (src/public_wire.rs:3711)… | d2b-contracts-control | `lane/d2b-contracts-control.md` | -1 t / -23 L |
| 262 | gap | `cli_output` module wire shapes (src/cli_output.rs, 411 lines)… | d2b-contracts-control | `lane/d2b-contracts-control.md` | -1 t / -23 L |
| 263 | gap | `ProxyReadinessEvent::failed` path (src/proxy_readiness.rs:75)… | d2b-contracts-control | `lane/d2b-contracts-control.md` | -1 t / -23 L |
| 264 | duplicate | `long_lived_argv_has_expected_shape` (src/swtpm_argv.rs:294) - covered by `audit_swtpm_input_parity_golden` (src/swtpm_argv.rs:275). | d2b-provider-device-tpm | `lane/d2b-provider-device-tpm.md` | -1 t / -23 L |
| 265 | gap | `LiveTpmResourceEffectPort` error surface (src/effects_service.rs:341-487)… | d2b-provider-device-tpm | `lane/d2b-provider-device-tpm.md` | -1 t / -23 L |
| 266 | gap | `declared_child` error paths (src/effects_service.rs:168) - missing `type`/`metadata.name`/`spec` → `EffectRejected` and unparseable ref → `InvalidDev… | d2b-provider-device-tpm | `lane/d2b-provider-device-tpm.md` | -1 t / -23 L |
| 267 | trivial | `bound_evidence_exposes_only_exact_bounded_commitments` (src/health.rs:541)… | d2b-provider-guest-cloud-hypervisor | `lane/d2b-provider-guest-cloud-hypervisor.md` | -1 t / -23 L |
| 268 | gap | `GuestSessionEvidence::current` rejection branch - non-Guest resource ref, empty name, reconnect generation 0… | d2b-provider-guest-cloud-hypervisor | `lane/d2b-provider-guest-cloud-hypervisor.md` | -1 t / -23 L |
| 269 | duplicate | `canonical_metric_frames_are_admitted` (src/emitter.rs:714) - covered by `emit_parses_one_shared_frame_for_metric_admission_and_redaction` (src/emitte… | d2b-telemetry | `lane/d2b-telemetry.md` | -1 t / -19 L |
| 270 | duplicate | `factory_registers_exactly_the_host_resource_type` (src/driver.rs:715)… | d2b-provider-host | `lane/d2b-provider-host.md` | -1 t / -18 L |
| 271 | gap | the bounded-read boundary - `read_bounded` refuses reads over `limit` bytes or non-UTF8 content (src/probe.rs:199)… | d2b-provider-host | `lane/d2b-provider-host.md` | -1 t / -18 L |
| 272 | duplicate | `factory_registers_exactly_the_user_resource_type` (src/driver.rs:667)… | d2b-provider-user | `lane/d2b-provider-user.md` | -1 t / -15 L |
| 273 | gap | production NSS probe `UserProbe` (src/probe.rs:20, `discover_local_user`) is never exercised by any test in the crate… | d2b-provider-user | `lane/d2b-provider-user.md` | -1 t / -15 L |
| 274 | trivial | `adoption_degrades_identity_ambiguity_without_stopping_scope` (src/runtime.rs:1565)… | d2b-unsafe-local-helper | `lane/d2b-unsafe-local-helper.md` | -1 t / -15 L |
| 275 | duplicate | `factory_registers_only_the_binding_type` (src/driver.rs:950)… | d2b-provider-telemetry-binding | `lane/d2b-provider-telemetry-binding.md` | -1 t / -12 L |
| 276 | gap | manager route failure → `Reconcile`-class error (src/driver.rs `derived_binding_children`/`reconcile_binding`/`recover` `map_err` sites)… | d2b-provider-telemetry-binding | `lane/d2b-provider-telemetry-binding.md` | -1 t / -12 L |
| 277 | trivial | `projection_status_never_claims_host_readiness` (src/audio_registry.rs:677)… | d2b-provider-wayland-policy | `lane/d2b-provider-wayland-policy.md` | -1 t / -11 L |
| 278 | gap | `AudioResourceRuntime` reconcile/finalize paths (`reconcile_service_resource`, `reconcile_binding_resource`… | d2b-provider-wayland-policy | `lane/d2b-provider-wayland-policy.md` | -1 t / -11 L |
| 279 | gap | product `is_audio_resource`/`decode_spec` (src/audio_registry.rs)… | d2b-provider-wayland-policy | `lane/d2b-provider-wayland-policy.md` | -1 t / -11 L |
| 280 | gap | hosted `audio-binding-statuses` method (`serve_audio_binding_statuses`/`binding_status_value` in src/effects_service.rs)… | d2b-provider-wayland-policy | `lane/d2b-provider-wayland-policy.md` | -1 t / -11 L |
| 281 | duplicate | `matching_owner_proof_adopts_one_cursor` (src/zonelink.rs:426) - covered by `handler_owns_restart_cursor_adoption` (src/zonelink.rs:472). | d2b-provider-zone-link | `lane/d2b-provider-zone-link.md` | -1 t / -9 L |
| 282 | gap | `RoutePolicyCommitted` refusal paths (src/zone_links.rs:1687)… | d2b-provider-zone-link | `lane/d2b-provider-zone-link.md` | -1 t / -9 L |
| 283 | gap | `transport_error_is_quarantine` (src/zonelink.rs:332) - the public StaleCommitProof/ReconcileInFlight → quarantine mapping is exercised nowhere in the… | d2b-provider-zone-link | `lane/d2b-provider-zone-link.md` | -1 t / -9 L |
| 284 | gap | `PskIssued` monotonicity refusal (src/zone_links.rs:1427) - an issuance event at or below the highest recorded issuance fails closed with `BootstrapPs… | d2b-provider-zone-link | `lane/d2b-provider-zone-link.md` | -1 t / -9 L |
| 285 | duplicate | `inverted_template_and_executable_digests_fail_closed` (src/adoption.rs:88)… | d2b-provider-guest-qemu-media | `lane/d2b-provider-guest-qemu-media.md` | -1 t / -6 L |
| 286 | gap | `EmptySlot` rejection of `qemu_media_hotplug_scaffold` (src/hotplug.rs:50)… | d2b-provider-guest-qemu-media | `lane/d2b-provider-guest-qemu-media.md` | -1 t / -6 L |
| 287 | gap | `verify_identity` Quarantined path (src/adoption.rs:61; decision sites src/controller/reconcile.rs:412,615)… | d2b-provider-guest-qemu-media | `lane/d2b-provider-guest-qemu-media.md` | -1 t / -6 L |
| 288 | duplicate | `RecordingManager` (12 src copies, 464 ln) - canonical home `d2b-provider-toolkit::testing::fakes` (add a `RecordingManagerEndpoint` fake… | cross-helper (C1) | `lane/_cross-helper-duplication.md` | -646 L (lines only) |
| 289 | duplicate | `RecordingRequeue` (10 src copies, 148 ln) - canonical home `d2b-provider-toolkit::testing::fakes` (same module as above). | cross-helper (C1) | `lane/_cross-helper-duplication.md` | -646 L (lines only) |
| 290 | duplicate | `block_on` (6 src copies, 66 ln) - canonical home `d2b-provider-toolkit::testing::block_on` (src/testing/mod.rs:78-88, "this is that driver, once"). | cross-helper (C1) | `lane/_cross-helper-duplication.md` | -646 L (lines only) |
| 291 | duplicate | scratch-root resolution `test_scratch_root`/`test_root`/`writable_manifest_dir` (7 src copies, 59 ln, 5 crates)… | cross-helper (C1) | `lane/_cross-helper-duplication.md` | -646 L (lines only) |
| 292 | duplicate | `sample_zone_native_host_json`/`sample_v3_host_contract_json` (2 src copies, 66 ln)… | cross-helper (C1) | `lane/_cross-helper-duplication.md` | -646 L (lines only) |
| 293 | gap | `validate_finalizer` rejects any finalizer other than the drain finalizer (src/v3/zone.rs:423)… | d2b-contracts-zone-session | `lane/d2b-contracts-zone-session.md` | -0 L (lines only) |
| 294 | gap | `BootstrapIdentityBinding::validate` rejects subject refs outside Host/Guest/Provider/Zone/Process (src/v3/component_session.rs:2346)… | d2b-contracts-zone-session | `lane/d2b-contracts-zone-session.md` | -0 L (lines only) |
| 295 | gap | `AuthorityRecoveryCoordinator::resolve_observed_closed` fail-closed rollback (src/authority_persistence.rs:282)… | d2b-core-controller | `lane/d2b-core-controller.md` | -0 L (lines only) |
| 296 | gap | provenance rejection in `validated_recovery_receipt` (src/authority_persistence.rs:321)… | d2b-core-controller | `lane/d2b-core-controller.md` | -0 L (lines only) |
| 297 | gap | `PreparedAuthorityOperation::new` RowInvalid boundary (src/authority_persistence.rs:50)… | d2b-core-controller | `lane/d2b-core-controller.md` | -0 L (lines only) |
| 298 | gap | kernel-seat refusal paths (`kernel_seat.rs` - `run()`/`admit()` return `KernelRefusal::Busy` on saturation and `Unavailable` on dead/absent workers… | d2b-core | `lane/d2b-core.md` | -0 L (lines only) |
| 299 | gap | static-invariants validators (`static_invariants.rs` - `world_readable_field_leaks`, `path_bearing_key_violations`, `is_broad_cap_violation`… | d2b-core | `lane/d2b-core.md` | -0 L (lines only) |
| 300 | gap | CLI validation error paths (`parse_args`/`parse_gid`, src/main.rs:63-105)… | d2b-host-activation-helper | `lane/d2b-host-activation-helper.md` | -0 L (lines only) |
| 301 | gap | NUL-byte path/name rejection (`cstring_path`/`cstring_name`, src/main.rs:107-118)… | d2b-host-activation-helper | `lane/d2b-host-activation-helper.md` | -0 L (lines only) |
| 302 | gap | zero-`assignment_epoch` rejection is untested - `with_assignment_binding` (src/ticket.rs:714) and `GuestExecutionBinding::new` (src/ticket.rs:307) bot… | d2b-process-conformance | `lane/d2b-process-conformance.md` | -0 L (lines only) |
| 303 | gap | `ProcessOutcome::exited` bounds 0..=255 untested (src/terminal.rs:52)… | d2b-process-conformance | `lane/d2b-process-conformance.md` | -0 L (lines only) |
| 304 | gap | `ParentWaitEvidence::verified_by` zero-identity / zero-token rejection untested (src/terminal.rs:135)… | d2b-process-conformance | `lane/d2b-process-conformance.md` | -0 L (lines only) |
| 305 | gap | over-bound argv and length ceilings (src/command.rs:199, lines 40/91)… | d2b-provider-command | `lane/d2b-provider-command.md` | -0 L (lines only) |
| 306 | gap | socket-effect failure → Retryable `ENDPOINT_SOCKET_EFFECT_FAILED` (driver.rs delete `remove_socket` error map… | d2b-provider-endpoint | `lane/d2b-provider-endpoint.md` | -0 L (lines only) |
| 307 | gap | evidence realize-budget timeout (effects_service.rs `ensure_socket`… | d2b-provider-endpoint | `lane/d2b-provider-endpoint.md` | -0 L (lines only) |
| 308 | gap | delete on an undecodable stored spec converges with `Ok` and no effects (driver.rs `let Ok(spec) = self.decoded_spec(...) else { return Ok(()) }`)… | d2b-provider-endpoint | `lane/d2b-provider-endpoint.md` | -0 L (lines only) |
| 309 | gap | `AcaRuntimeConfig::new` plan-ttl and operation-capacity bounds (`plan_ttl_ms` 0/>300_000 → `InvalidPlanTtl`… | d2b-provider-guest-azure-container-apps | `lane/d2b-provider-guest-azure-container-apps.md` | -0 L (lines only) |
| 310 | gap | `AcaSandboxProfile::new` `auto_suspend_secs` (60..=86_400) and `AcaMemoryMib` (512..=16_384 step 256) bounds have no test (effects.rs:118-128… | d2b-provider-guest-azure-container-apps | `lane/d2b-provider-guest-azure-container-apps.md` | -0 L (lines only) |
| 311 | gap | `AcaProviderConfig::new` resource-type checks on `gateway_execution_ref`/`control_credential_ref`/`pull_credential_ref`/`network_ref` have no test (ef… | d2b-provider-guest-azure-container-apps | `lane/d2b-provider-guest-azure-container-apps.md` | -0 L (lines only) |
| 312 | gap | the route-admission authentication surface is untested end to end… | d2b-provider-notification-desktop | `lane/d2b-provider-notification-desktop.md` | -0 L (lines only) |
| 313 | gap | lifecycle identity constructor rejection branches - `NotificationSourceIdentity::new` (src/lifecycle.rs:25) and `NotificationHostSinkIdentity::new` (s… | d2b-provider-notification-desktop | `lane/d2b-provider-notification-desktop.md` | -0 L (lines only) |
| 314 | gap | `NotificationTelemetryFrame::validate_collector_fields` (src/metrics.rs:93) injection branches… | d2b-provider-notification-desktop | `lane/d2b-provider-notification-desktop.md` | -0 L (lines only) |
| 315 | gap | over-limit audit facets rejected (`OperationAudit::new`, src/operation.rs:148)… | d2b-provider-operation | `lane/d2b-provider-operation.md` | -0 L (lines only) |
| 316 | gap | over-limit fd contract lists rejected (`OperationFds::new`, src/operation.rs:356)… | d2b-provider-operation | `lane/d2b-provider-operation.md` | -0 L (lines only) |
| 317 | gap | ownerRef naming a non-Command resource rejected (`OperationSpec::new`, src/operation.rs:490)… | d2b-provider-operation | `lane/d2b-provider-operation.md` | -0 L (lines only) |
| 318 | gap | system-minijail fixed-host readiness gate (`fixed_provider_host_ready`, src/providers.rs:362) - no test anywhere in crate. | d2b-provider-provider | `lane/d2b-provider-provider.md` | -0 L (lines only) |
| 319 | gap | `provider_observation` canonical-JSON decode failure → `CoreReconcileError` (src/providers.rs:406,415,466)… | d2b-provider-provider | `lane/d2b-provider-provider.md` | -0 L (lines only) |
| 320 | gap | expiry boundary of `PositiveDecisionCache` (insert_allow/contains at rbac.rs:76-95)… | d2b-provider-role | `lane/d2b-provider-role.md` | -0 L (lines only) |
| 321 | gap | bounded capacity eviction of `PositiveDecisionCache` (insert_allow at rbac.rs:91-94)… | d2b-provider-role | `lane/d2b-provider-role.md` | -0 L (lines only) |
| 322 | gap | `TooManyDeviceBinds` error path (src/seccomp_profile.rs:186-188, `SeccompProfileSpec::new` rejects `devices.len() > MAX_SECCOMP_DEVICE_BINDS`)… | d2b-provider-seccomp-profile | `lane/d2b-provider-seccomp-profile.md` | -0 L (lines only) |
| 323 | gap | `DeviceNodePath` length bound (src/seccomp_profile.rs:33, `parse` rejects paths over `MAX_DEVICE_NODE_PATH_BYTES` = 255)… | d2b-provider-seccomp-profile | `lane/d2b-provider-seccomp-profile.md` | -0 L (lines only) |
| 324 | gap | wire-level invalid device path (src/seccomp_profile.rs:54-59, `DeviceNodePath::deserialize` maps parse errors to serde errors)… | d2b-provider-seccomp-profile | `lane/d2b-provider-seccomp-profile.md` | -0 L (lines only) |
| 325 | gap | `BrokerSystemdEffectOwner` (systemd.rs:456-707) - the daemon's fixed supervisor owner has zero tests in this crate (unit or tests/): its launch/observ… | d2b-provider-supervisor | `lane/d2b-provider-supervisor.md` | -0 L (lines only) |
| 326 | gap | `wait_pidfd_exit` / `wait_pidfd_observer` (broker.rs:1586-1663)… | d2b-provider-supervisor | `lane/d2b-provider-supervisor.md` | -0 L (lines only) |
| 327 | gap | `map_probe_error` `DeadlineExceeded` branch (adapter.rs:905-911)… | d2b-provider-supervisor | `lane/d2b-provider-supervisor.md` | -0 L (lines only) |
| 328 | gap | `KernelTooOld` error path (`MinijailPlatformGate::validate`, src/host.rs:122-124)… | d2b-provider-system-core | `lane/d2b-provider-system-core.md` | -0 L (lines only) |
| 329 | gap | retry-on-transient-failure loop in `run()` (src/main.rs:52-56)… | d2b-provider-test-controller | `lane/d2b-provider-test-controller.md` | -0 L (lines only) |
| 330 | gap | `ProviderEntrypoint` name validation and builder double-set guards (src/base/runtime.rs:249-252, 321-322, 336-345, 356-358, 370-372)… | d2b-provider-toolkit | `lane/d2b-provider-toolkit.md` | -0 L (lines only) |
| 331 | gap | BadSchemaVersion guard - unsupported sealed-envelope `schema_version` rejected (src/guest_credential.rs:load_sealed_inner)… | d2b-provider-transport-azure-relay | `lane/d2b-provider-transport-azure-relay.md` | -0 L (lines only) |
| 332 | gap | BadFileType guard - credential path must be a regular file (src/guest_credential.rs:read_policy_file) - no test opens a non-regular path (e.g. | d2b-provider-transport-azure-relay | `lane/d2b-provider-transport-azure-relay.md` | -0 L (lines only) |
| 333 | gap | BadSealKey guard - `SealingKey::load` rejects wrong-length key bytes (src/guest_credential.rs:SealingKey::load)… | d2b-provider-transport-azure-relay | `lane/d2b-provider-transport-azure-relay.md` | -0 L (lines only) |
| 334 | gap | `decode_ed25519_spki` rejection paths - bad PEM framing, non-ED25519 algorithm OID, or truncated key returning `None` (src/lib.rs:2311)… | d2b-resource-compiler | `lane/d2b-resource-compiler.md` | -0 L (lines only) |
| 335 | gap | `LinuxAnchoredDir::open_readable` missing-file → `LayoutError::Absent` and non-regular directory target → `LayoutError::NotRegular` (src/linux.rs:333-… | d2b-resource-compiler | `lane/d2b-resource-compiler.md` | -0 L (lines only) |
| 336 | gap | `ZoneBootstrapIdentity::verify` uid checks - InvalidPeerUid (expected_uid==0) and PeerUidMismatch (observed vs expected) untested anywhere in the crat… | d2b-session-unix | `lane/d2b-session-unix.md` | -0 L (lines only) |
| 337 | gap | `CreditPool::new(0)` - ZeroLimit (credit.rs:43-45) untested; zero-limit config boundary must be rejected… | d2b-session-unix | `lane/d2b-session-unix.md` | -0 L (lines only) |
| 338 | gap | `ProcessCreditLimit::derive` - BaselineExceedsLimit (baseline+reserve ≥ soft rlimit; credit.rs:272-273) and Overflow (credit.rs:264-268) untested… | d2b-session-unix | `lane/d2b-session-unix.md` | -0 L (lines only) |
| 339 | gap | `Config::from_env` fail-closed validation (config.rs:69-127) - missing/invalid env vars, `D2B_SK_GUEST_ZONE == D2B_SK_PARENT_ZONE` rejection… | d2b-sk-frontend | `lane/d2b-sk-frontend.md` | -0 L (lines only) |
| 340 | gap | `UhidDevice::read_event` event dispatch (uhid.rs:175-214) - Output/GetReport/Lifecycle/Other type mapping, short-header `UnexpectedEof`… | d2b-sk-frontend | `lane/d2b-sk-frontend.md` | -0 L (lines only) |
| 341 | gap | `build_get_report_reply_error` layout (uhid.rs:337-347) - reply type 10, id echo, err=EPIPE, size=0 fields untested… | d2b-sk-frontend | `lane/d2b-sk-frontend.md` | -0 L (lines only) |

---

## 3. Per-family findings (one row per crate; full rows in each lane)

### 3.1 Shared types / contracts layer

| Crate | Tests | Net | Verdicts | Top finding | Lane |
| --- | ---: | --- | --- | --- | --- |
| d2b-contracts | 118 | -8 t / -38 L | 6 dup · 2 triv · 3 gap | `workload_target_parse_canonical` (src/workload_identity.rs:209)… | `lane/d2b-contracts.md` |
| d2b-contracts-broker | 49 | -6 t / -100 L | 6 dup · 2 gap | `broker_request_envelope_round_trips_with_admin` (src/broker_wire.rs:3181)… | `lane/d2b-contracts-broker.md` |
| d2b-contracts-control | 34 | -1 t / -23 L | 1 dup · 2 gap | `audio_set_volume_rejects_out_of_range_level` (src/public_wire.rs:3711)… | `lane/d2b-contracts-control.md` |
| d2b-contracts-provider | 124 | -4 t / -44 L | 4 dup | `the_pair_names_the_exact_frozen_resource_types` (semantic_services/audio.rs:125)… | `lane/d2b-contracts-provider.md` |
| d2b-contracts-resource | 137 | -2 t / -27 L | 2 dup · 1 gap | `duplicate_json_keys_are_rejected_before_envelope_materialization` (src/v3/resource.rs:1145)… | `lane/d2b-contracts-resource.md` |
| d2b-contracts-zone-session | 71 | 0 | 2 gap | `validate_finalizer` rejects any finalizer other than the drain finalizer (src/v3/zone.rs:423)… | `lane/d2b-contracts-zone-session.md` |
| d2b-resource-api | 89 | -5 t / -95 L | 3 dup · 2 triv | `api_status_updates_have_no_persistent_write_path` (src/manager_backend/tests.rs:1773)… | `lane/d2b-resource-api.md` |
| d2b-resource-client | 40 | -2 t / -37 L | 2 dup · 3 gap | `a_zone_outside_the_table_is_refused_before_any_call_is_prepared` (src/client.rs:166)… | `lane/d2b-resource-client.md` |
| d2b-resource-compiler | 17 | 0 | 2 gap | `decode_ed25519_spki` rejection paths - bad PEM framing, non-ED25519 algorithm OID, or truncated key returning… | `lane/d2b-resource-compiler.md` |
| d2b-resource-runtime | 137 | -5 t / -103 L | 1 dup · 4 triv | `async_recv_drains_then_missed_then_ends` (src/watch.rs:891)… | `lane/d2b-resource-runtime.md` |
| d2b-resource-types | 7 | -2 t / -18 L | 2 triv | `a_child_creation_keeps_every_field` (src/child_creation.rs:51)… | `lane/d2b-resource-types.md` |
| d2b-core | 93 | 0 | 2 gap | kernel-seat refusal paths (`kernel_seat.rs` - `run()`/`admit()` return `KernelRefusal::Busy` on saturation and… | `lane/d2b-core.md` |
| d2b-core-controller | 86 | 0 | 3 gap | `AuthorityRecoveryCoordinator::resolve_observed_closed` fail-closed rollback (src/authority_persistence.rs:282… | `lane/d2b-core-controller.md` |
| d2b-zone-routing | 116 | -24 t / -488 L | 22 dup · 2 triv · 2 gap | `a_second_advertiser_claiming_the_same_descendant_is_refused_as_multi_parent` (src/engine.rs:2879)… | `lane/d2b-zone-routing.md` |
| d2b-session | 28 | -5 t / -92 L | 2 dup · 3 triv · 2 gap | `cancellation_propagates_delivery_failure_after_local_cleanup` (src/admission.rs:1944)… | `lane/d2b-session.md` |
| d2b-session-unix | 25 | 0 | 3 gap | `ZoneBootstrapIdentity::verify` uid checks - InvalidPeerUid (expected_uid==0) and PeerUidMismatch (observed vs… | `lane/d2b-session-unix.md` |
| d2b-bus | 182 | -2 t / -47 L | 1 dup · 1 triv · 3 gap | `relay_grant_and_forwarded_target_grant_fail_independently` (src/authorization.rs:1521)… | `lane/d2b-bus.md` |

### 3.2 Runtime / daemon / broker / tooling

| Crate | Tests | Net | Verdicts | Top finding | Lane |
| --- | ---: | --- | --- | --- | --- |
| d2bd | 474 | -13 t / -164 L | 8 dup · 5 triv · 3 gap | `qemu_controller_never_calls_target_process_path` (src/audio_dispatch.rs:812)… | `lane/d2bd.md` |
| d2bd-runtime | 458 | -6 t / -60 L | 6 dup · 3 gap | `resize_retry_with_same_op_id_replays_cached_ack` (src/exec_session.rs:3150)… | `lane/d2bd-runtime.md` |
| d2b-broker | 663 | -11 t / -188 L | 1 dup · 10 triv · 1 gap | `crash_between_durable_commit_and_effect_reconciles_without_double_grant_or_leak` (src/state_cells.rs:1359)… | `lane/d2b-broker.md` |
| d2b-broker-composition | 31 | -3 t / -36 L | 3 dup · 2 gap | `an_unregistered_admitted_operation_fails_the_startup_invariant` (src/seam.rs:720)… | `lane/d2b-broker-composition.md` |
| d2b-process-conformance | 30 | 0 | 3 gap | zero-`assignment_epoch` rejection is untested - `with_assignment_binding` (src/ticket.rs:714) and `GuestExecut… | `lane/d2b-process-conformance.md` |
| xtask | 455 | -4 t / -65 L | 3 dup · 1 triv | `filtered_lock_omits_unselected_lock_only_packages` (src/production_closure.rs:1774)… | `lane/xtask.md` |
| d2b-unsafe-local-helper | 29 | -1 t / -15 L | 1 triv | `adoption_degrades_identity_ambiguity_without_stopping_scope` (src/runtime.rs:1565)… | `lane/d2b-unsafe-local-helper.md` |
| d2b-sk-frontend | 13 | 0 | 3 gap | `Config::from_env` fail-closed validation (config.rs:69-127)… | `lane/d2b-sk-frontend.md` |

### 3.3 Provider families

#### 3.3.1 Guest / workload

| Crate | Tests | Net | Verdicts | Top finding | Lane |
| --- | ---: | --- | --- | --- | --- |
| d2b-provider-guest | 37 | -1 t / -25 L | 1 dup · 3 gap | `reconcile_refuses_a_qemu_guest_without_its_provider_row` (src/driver.rs:2176)… | `lane/d2b-provider-guest.md` |
| d2b-provider-guest-azure-container-apps | 1 | 0 | 3 gap | `AcaRuntimeConfig::new` plan-ttl and operation-capacity bounds (`plan_ttl_ms` 0/>300_000 → `InvalidPlanTtl`, c… | `lane/d2b-provider-guest-azure-container-apps.md` |
| d2b-provider-guest-cloud-hypervisor | 16 | -1 t / -23 L | 1 triv · 1 gap | `bound_evidence_exposes_only_exact_bounded_commitments` (src/health.rs:541)… | `lane/d2b-provider-guest-cloud-hypervisor.md` |
| d2b-provider-guest-qemu-media | 4 | -1 t / -6 L | 1 dup · 2 gap | `inverted_template_and_executable_digests_fail_closed` (src/adoption.rs:88)… | `lane/d2b-provider-guest-qemu-media.md` |
| d2b-provider-provider | 14 | 0 | 2 gap | system-minijail fixed-host readiness gate (`fixed_provider_host_ready`, src/providers.rs:362)… | `lane/d2b-provider-provider.md` |
| d2b-provider-supervisor | 21 | 0 | 3 gap | `BrokerSystemdEffectOwner` (systemd.rs:456-707) - the daemon's fixed supervisor owner has zero tests in this c… | `lane/d2b-provider-supervisor.md` |

#### 3.3.2 Process / activation / credential / telemetry / observability

| Crate | Tests | Net | Verdicts | Top finding | Lane |
| --- | ---: | --- | --- | --- | --- |
| d2b-provider-process | 56 | -1 t / -31 L | 1 dup · 3 gap | `durable_provider_effect_launch_failure_still_retries_under_the_budget` (src/driver.rs:4334)… | `lane/d2b-provider-process.md` |
| d2b-provider-process-systemd | 12 | -2 t / -66 L | 1 dup · 1 triv | `a_guest_execution_binding_with_a_wrong_digest_refuses_through_the_hosted_service` (src/effects_service.rs:351… | `lane/d2b-provider-process-systemd.md` |
| d2b-provider-activation-nixos | 23 | -2 t / -54 L | 2 dup · 3 gap | `offline_verification_refuses_before_the_handoff_is_dispatched` (src/driver.rs:1501)… | `lane/d2b-provider-activation-nixos.md` |
| d2b-provider-config-nixos | 1 | 0 | - | Nothing to cut - every test pins a distinct behavior. | `lane/d2b-provider-config-nixos.md` |
| d2b-provider-credential | 26 | -2 t / -58 L | 2 triv · 1 gap | `uncertain_revocation_never_unblocks_cleanup` (src/session.rs:478)… | `lane/d2b-provider-credential.md` |
| d2b-provider-credential-entra | 8 | -3 t / -32 L | 1 dup · 2 triv · 3 gap | `host_system_placement_is_rejected` (src/lib.rs:1368)… | `lane/d2b-provider-credential-entra.md` |
| d2b-provider-credential-managed-identity | 8 | -6 t / -100 L | 6 dup · 3 gap | `agent_is_planned_only_after_admission_and_dependency_readiness` (src/controller.rs:337)… | `lane/d2b-provider-credential-managed-identity.md` |
| d2b-provider-credential-secret-service | 14 | -2 t / -36 L | 2 dup · 3 gap | `placement_is_user_agent_only` (src/lib.rs:1974) - covered by `only_user_agent_on_host_or_guest_is_accepted` (… | `lane/d2b-provider-credential-secret-service.md` |
| d2b-provider-telemetry-binding | 10 | -1 t / -12 L | 1 dup · 1 gap | `factory_registers_only_the_binding_type` (src/driver.rs:950)… | `lane/d2b-provider-telemetry-binding.md` |
| d2b-provider-telemetry-service | 9 | -2 t / -20 L | 2 dup · 2 gap | `factory_registers_only_the_service_type` (src/driver.rs:698)… | `lane/d2b-provider-telemetry-service.md` |
| d2b-provider-observability-otel | 28 | -2 t / -43 L | 2 dup · 3 gap | `policy_runs_before_capacity_and_rejects_the_whole_frame` (src/ingress_policy.rs:855)… | `lane/d2b-provider-observability-otel.md` |

#### 3.3.3 Toolkit / provider / test-controller

| Crate | Tests | Net | Verdicts | Top finding | Lane |
| --- | ---: | --- | --- | --- | --- |
| d2b-provider-toolkit | 69 | 0 | 1 gap | `ProviderEntrypoint` name validation and builder double-set guards (src/base/runtime.rs:249-252, 321-322, 336-… | `lane/d2b-provider-toolkit.md` |
| d2b-provider | 4 | 0 | - | Nothing to cut - every test pins a distinct behavior. | `lane/d2b-provider.md` |
| d2b-provider-test-controller | 2 | 0 | 1 gap | retry-on-transient-failure loop in `run()` (src/main.rs:52-56)… | `lane/d2b-provider-test-controller.md` |

#### 3.3.4 Device / display / clipboard / notification

| Crate | Tests | Net | Verdicts | Top finding | Lane |
| --- | ---: | --- | --- | --- | --- |
| d2b-provider-device-gpu | 17 | -4 t / -36 L | 4 dup | `daemon_input_pins_cross_domain_and_wayland_bind` (src/gpu_argv.rs:269)… | `lane/d2b-provider-device-gpu.md` |
| d2b-provider-device-security-key | 39 | -10 t / -120 L | 9 dup · 1 triv | `parse_init_packet_identifies_cmd_and_cid` (src/relay_service.rs:668)… | `lane/d2b-provider-device-security-key.md` |
| d2b-provider-device-tpm | 25 | -1 t / -23 L | 1 dup · 2 gap | `long_lived_argv_has_expected_shape` (src/swtpm_argv.rs:294)… | `lane/d2b-provider-device-tpm.md` |
| d2b-provider-device-usbip | 20 | -3 t / -60 L | 3 dup · 2 gap | `explicit_plan_preserves_step_stop_and_execution_order` (src/state_machine.rs:803)… | `lane/d2b-provider-device-usbip.md` |
| d2b-provider-display-wayland | 148 | -9 t / -164 L | 4 dup · 3 triv · 3 gap | `rate_limiter_suppresses_after_max` (wayland_proxy/diag.rs:222)… | `lane/d2b-provider-display-wayland.md` |
| d2b-provider-clipboard-wayland | 135 | -2 t / -14 L | 1 dup · 1 triv · 1 gap | `accepts_valid_maxish_open_request_line` (src/clipd_host/framing.rs:120)… | `lane/d2b-provider-clipboard-wayland.md` |
| d2b-provider-notification-desktop | 25 | 0 | 3 gap | the route-admission authentication surface is untested end to end… | `lane/d2b-provider-notification-desktop.md` |

#### 3.3.5 Volume / transport / network / endpoint / zone-link / wayland-policy

| Crate | Tests | Net | Verdicts | Top finding | Lane |
| --- | ---: | --- | --- | --- | --- |
| d2b-provider-volume | 14 | -2 t / -23 L | 1 dup · 1 triv | `factory_registers_only_the_volume_resource_type` (src/driver.rs:1012)… | `lane/d2b-provider-volume.md` |
| d2b-provider-volume-binding | 18 | -2 t / -24 L | 2 dup · 3 gap | `factory_registers_only_the_binding_resource_type` (src/driver.rs:1440)… | `lane/d2b-provider-volume-binding.md` |
| d2b-provider-volume-local | 37 | -2 t / -30 L | 2 dup | `resolve_refuses_grants_wider_than_the_group_class` (src/layout.rs:419)… | `lane/d2b-provider-volume-local.md` |
| d2b-provider-volume-virtiofs | 9 | -3 t / -25 L | 3 dup · 1 gap | `the_envelope_never_carries_attachment_settings` (src/bindings.rs:390)… | `lane/d2b-provider-volume-virtiofs.md` |
| d2b-provider-transport-azure-relay | 21 | 0 | 3 gap | BadSchemaVersion guard - unsupported sealed-envelope `schema_version` rejected (src/guest_credential.rs:load_s… | `lane/d2b-provider-transport-azure-relay.md` |
| d2b-provider-transport-unix | 1 | 0 | - | Nothing to cut - every test pins a distinct behavior. | `lane/d2b-provider-transport-unix.md` |
| d2b-provider-transport-vsock | 1 | 0 | - | Nothing to cut - every test pins a distinct behavior. | `lane/d2b-provider-transport-vsock.md` |
| d2b-provider-network-local | 50 | -2 t / -41 L | 2 dup · 2 gap | `inspect_network_report_spells_the_catalog_wire_names` (src/effects_service.rs:340)… | `lane/d2b-provider-network-local.md` |
| d2b-provider-endpoint | 16 | 0 | 3 gap | socket-effect failure → Retryable `ENDPOINT_SOCKET_EFFECT_FAILED` (driver.rs delete `remove_socket` error map;… | `lane/d2b-provider-endpoint.md` |
| d2b-provider-zone-link | 42 | -1 t / -9 L | 1 dup · 3 gap | `matching_owner_proof_adopts_one_cursor` (src/zonelink.rs:426)… | `lane/d2b-provider-zone-link.md` |
| d2b-provider-wayland-policy | 9 | -1 t / -11 L | 1 triv · 3 gap | `projection_status_never_claims_host_readiness` (src/audio_registry.rs:677)… | `lane/d2b-provider-wayland-policy.md` |

#### 3.3.6 Host / user / role / command / operation / seccomp / system-core

| Crate | Tests | Net | Verdicts | Top finding | Lane |
| --- | ---: | --- | --- | --- | --- |
| d2b-provider-host | 19 | -1 t / -18 L | 1 dup · 1 gap | `factory_registers_exactly_the_host_resource_type` (src/driver.rs:715)… | `lane/d2b-provider-host.md` |
| d2b-provider-user | 21 | -1 t / -15 L | 1 dup · 1 gap | `factory_registers_exactly_the_user_resource_type` (src/driver.rs:667)… | `lane/d2b-provider-user.md` |
| d2b-provider-role | 2 | 0 | 2 gap | expiry boundary of `PositiveDecisionCache` (insert_allow/contains at rbac.rs:76-95)… | `lane/d2b-provider-role.md` |
| d2b-provider-command | 4 | 0 | 1 gap | over-bound argv and length ceilings (src/command.rs:199, lines 40/91)… | `lane/d2b-provider-command.md` |
| d2b-provider-operation | 6 | 0 | 3 gap | over-limit audit facets rejected (`OperationAudit::new`, src/operation.rs:148)… | `lane/d2b-provider-operation.md` |
| d2b-provider-seccomp-profile | 3 | 0 | 3 gap | `TooManyDeviceBinds` error path (src/seccomp_profile.rs:186-188, `SeccompProfileSpec::new` rejects `devices.le… | `lane/d2b-provider-seccomp-profile.md` |
| d2b-provider-system-core | 1 | 0 | 1 gap | `KernelTooOld` error path (`MinijailPlatformGate::validate`, src/host.rs:122-124)… | `lane/d2b-provider-system-core.md` |

### 3.4 CLI / host / telemetry

| Crate | Tests | Net | Verdicts | Top finding | Lane |
| --- | ---: | --- | --- | --- | --- |
| d2b | 148 | -2 t / -13 L | 2 dup · 2 gap | `seccomp_field_parse_disabled` (src/doctor.rs:2288)… | `lane/d2b.md` |
| d2b-host | 151 | -5 t / -47 L | 5 dup · 2 gap | `modules_disabled_locks_required_absent_module` (src/modules.rs:566)… | `lane/d2b-host.md` |
| d2b-host-activation-helper | 2 | 0 | 2 gap | CLI validation error paths (`parse_args`/`parse_gid`, src/main.rs:63-105)… | `lane/d2b-host-activation-helper.md` |
| d2b-audit | 53 | -3 t / -40 L | 3 dup · 3 gap | `garbage_checkpoint_scratch_without_commit_is_discarded` (src/segment.rs:1379)… | `lane/d2b-audit.md` |
| d2b-telemetry | 32 | -1 t / -19 L | 1 dup | `canonical_metric_frames_are_admitted` (src/emitter.rs:714)… | `lane/d2b-telemetry.md` |

### 3.5 Cross-cutting (C1, C2)

See §4.

---

## 4. Cross-cutting lanes

### 4.1 C1 - cross-crate test-helper duplication (`lane/_cross-helper-duplication.md`, net -646 lines)

Five duplicated test-helper families, each with member crates and the **one canonical home**:

| Family | Copies | Net | Canonical home |
| --- | ---: | ---: | --- |
| `RecordingManager` (ManagerEndpoint double: call log + owned-row set) | 12 src copies (464 ln) + 1 integration copy (wayland-policy/tests/engine.rs) | -392 | `d2b-provider-toolkit::testing::fakes` (add a `RecordingManagerEndpoint` fake; ADR-046 designates this module as the every-Provider doubles home) |
| `RecordingRequeue` (RequeueScheduler double) | 10 src copies (148 ln) + 1 integration copy (device/tests/device_family.rs) | -119 | `d2b-provider-toolkit::testing::fakes` |
| `block_on` (noop-waker single-thread future driver) | 6 src copies (66 ln) + 5 integration copies | -55 | `d2b-provider-toolkit::testing::block_on` (src/testing/mod.rs:78-88) - note: d2b-core, d2b-process-conformance, d2b-provider-device-tpm, d2b-provider-volume-local, d2b-provider-volume-virtiofs need the toolkit dep |
| scratch-root resolution (`test_scratch_root`/`test_root`/`writable_manifest_dir`) | 7 src copies (59 ln, 5 crates; the d2b-audit triple is intra-crate) | -47 | `d2b-core::test_support` (feature-gated module already consumed cross-crate; d2b-audit needs the dep or its triple stays local) |
| `sample_zone_native_host_json`/`sample_v3_host_contract_json` | 2 src copies (66 ln) | -33 | `d2b-core::test_support` (keeper `d2b-core/src/bundle_resolver.rs:7022-7054`) |

Kept as non-unifiable: per-crate `test_support.rs` `RecordingEffects`/`RecordingRuntime` doubles (each implements a distinct effect/runtime trait), one-line `recording_facets` wrappers, row builders (same name family, crate-specific shapes), `fixture()`/`context()` ResourceContext builders (only the ~6-line channel+`ResourceContext::new` core repeats - revisit once `RecordingManager` lands), per-crate `fixtures` modules, `type Log = Arc<Mutex<Vec<String>>>` aliases, and the already-shared `d2b-core::test_support` builders.

### 4.2 C2 - cross-layer overlap (`lane/_cross-layer-overlap.md`, net -20 tests, -305 lines)

Twenty unit tests whose behavior is already pinned by integration/contract tests or sibling-crate tests, each citing both sides. **Corrections status: 0 corrections this run** - no crate-lane `duplicate:` claim was found wrong; all crate-lane findings stand (the d2b-zone-routing `remote_route_without_runtime_admission_is_refused` deletion was additionally *supported* by integration vectors, and the d2b doctor lane's "exit-code ladder re-pinned end-to-end" claim was narrowed - the warn→exit-1/clean→exit-0 legs are unit-only). Notable C2 rows: 6 d2b doctor status tests covered by `tests/host_doctor_contract.rs`; 5 d2b-broker fd/tap-fence tests covered by `tests/pidfd_handoff_scm_rights.rs` + `tests/persistent_tap_lifecycle.rs`; 3 credential-family canary/telemetry tests covered by d2b-provider-toolkit + d2b-contracts-provider keepers; 2 d2b-core `IfName` tests covered by d2b-contracts `v3/ifname.rs`; 1 d2bd semaphore test covered by d2bd-runtime `concurrency.rs`; 1 d2b-telemetry forbidden-keys test covered by d2b-provider-observability-otel.

C2 also verified the **keep** side: the golden-byte/serde families (d2b-contracts-resource `MINIMAL_*_SPEC`/`GOLDEN_ENVELOPE`, d2b-contracts-control wire-shape pins, d2b-contracts-zone-session canonical vectors, d2b-contracts serde families, d2b-provider-seccomp-profile roundtrip) have no sibling or golden re-pins and stay; d2b-broker realization/protocol/catalog unit tests each keep an extra pin the integration side lacks; d2bd-runtime's three gap claims were confirmed (no integration coverage found); d2b dispatch parser tests (7) each carry a pin the CLI probes lack; xtask bazel_evidence unit vs integration tests are complementary.

---

## 5. Zero-test crates (19)

No `#[cfg(test)]` unit surface in `src/`; integration/contract coverage via `tests/` where noted. These crates appear in the report as mechanical zero-rows - every workspace crate is covered by this audit, whether by a lane or by this table.

| Crate | Unit tests | `tests/` dir | Integration-test status |
| --- | ---: | --- | --- |
| d2b-broker-fixture-handlers | 0 | no | no `tests/` dir - fixture crates consumed by broker-composition tests |
| d2b-broker-fixture-syscall-surface | 0 | no | no `tests/` dir - hostile fixture crate exercised via d2b-broker-composition's dependency-surface tests |
| d2b-controller-toolkit | 0 | no | no `tests/` dir |
| d2b-provider-audio-binding | 0 | yes | 1 integration file |
| d2b-provider-audio-pipewire | 0 | yes | 6 integration files |
| d2b-provider-audio-service | 0 | yes | 1 integration file |
| d2b-provider-device | 0 | yes | 1 integration file (`tests/device_family.rs` - hosts a `RecordingRequeue` copy per C1) |
| d2b-provider-emergency-policy | 0 | yes | 1 integration file |
| d2b-provider-guest-azure-virtual-machine | 0 | yes | 3 integration files |
| d2b-provider-process-minijail | 0 | yes | 3 integration files |
| d2b-provider-quota | 0 | yes | 1 integration file |
| d2b-provider-resource-export | 0 | yes | 1 integration file |
| d2b-provider-resource-import | 0 | yes | 1 integration file |
| d2b-provider-role-binding | 0 | yes | 1 integration file |
| d2b-provider-shell-pool | 0 | yes | 1 integration file |
| d2b-provider-shell-session | 0 | yes | 1 integration file |
| d2b-provider-shell-terminal | 0 | yes | 10 integration files |
| d2b-provider-wayland-session | 0 | yes | 1 integration file |
| d2b-provider-zone | 0 | yes | 2 integration files |

---

## 6. Remediation sequencing

Waves in dependency order; each wave is one commit per crate, and each wave ends with `make test-rust` and `cargo xtask check-async-gate` green before the next starts. No deletion in this audit run - this section is the execution order for the follow-up.

| Wave | Work | Content | Gate |
| --- | --- | --- | --- |
| **1** | `duplicate:` deletes | All 139 crate-lane + 20 C2 duplicate tests (137 bullet rows - 2 rows compress 2-test families), each already covered by its cited covering test (`path::test_name` in the lane) | `make test-rust`; cited covering tests still pass; `cargo xtask check-async-gate` |
| **2** | `trivial:` deletes + orphaned helper removal | All 46 trivial tests; remove test-only helpers/fixtures that die with them (each lane names them, e.g. d2bd `FakeHostController::unsupported()`, d2b-resource-api `status_body`, d2b-provider-volume-local `volume_uid`, d2b-provider-credential `UncertainCredentialSession`); C1 helper consolidation onto `d2b-provider-toolkit::testing::fakes` / `d2b-core::test_support` (add the two missing deps) | `make test-rust`; `cargo xtask check-async-gate` - deleted `#[cfg(test)]` scaffolding can leave stale async-gate allowances in `packages/xtask/data/async-gate-inventory.json`; regenerate the inventory |
| **3** | `gap:` additions | 133 gap findings (max 3 per lane, each naming the product code location) - error paths, fail-closed boundaries, `#[ignore]`d broken promises (d2bd vm-start DAG + SIGKILL escalation, d2b-broker typed-audit arms) | `make test-rust`; `cargo xtask check-async-gate` |

Rollback: one commit per crate per wave; re-add the deleted test on any regression its covering test fails to catch.

---

## 7. Reconciliation

**Verified by summing the lane `net:` lines** (computed from the lane files, not estimated):

- **Headline -205 tests = 185 (sum of 75 crate-lane `net:` test counts) + 20 (C2 cross-layer duplicates).** ✓
- **Headline -3,829 lines = 2,878 (sum of 75 crate-lane `net:` line counts) + 646 (C1 helper families) + 305 (C2).** ✓
- Keep count 4,733 = 4,938 total tests - 205 deletions. ✓
- Coverage: 94 census crates = 75 lane files + 19 zero-test rows. ✓

### Route-out appendix (product-code bugs spotted while reading; not investigated further)

- **d2b-contracts-broker** - `RunnerRole::ActivationNixos` serde kebab-case token is `"activation-nixos"` but `as_str()` returns `"activation-nixos-runner"` (src/broker_wire.rs:2377/2404): the wire token and the role_id string disagree for the same role. `BrokerCallerRole::for_display` returns `"RootUid"` for `RootUid` (src/broker_wire.rs:2922), inconsistent with the stable `d2b-*` audit-label convention of every sibling variant.
- **d2b-session-unix** - `ZoneAdmissionError::ZoneInvalid` variant (zone_admission.rs:44) is declared and Display-ed but never constructed anywhere: dead error variant.
- **d2b-provider-system-core** - `SystemCoreError::BudgetOvercommit` (src/error.rs:34) is never constructed anywhere in the crate: dead variant or an unimplemented budget check.
- **d2b-provider-volume** - `has_layout_response`'s `"has-layout-response-invalid"` declined branch is unreachable by construction (both literals parse cleanly): dead defensive code, or a refactor toward response-from-value.
- **d2bd-runtime** - `metrics_handler_with_ch_stats` re-implements the GET/method/path validation block of `metrics_handler` byte-for-byte instead of delegating (src/metrics.rs:784-806): two copies can drift; the ch_stats 404/405 tests only pin their own copy.

Lanes that explicitly found none: d2b-audit, d2b-contracts-provider, d2b-process-conformance, d2b-provider-credential, d2b-provider-device-usbip, d2b-provider-network-local, d2b-unsafe-local-helper, d2b-zone-routing.

---

Metadata: lanes=77 (75 crate + C1 + C2) · tests audited=4,938 · findings=341 rows (137 duplicate + 46 trivial + 133 gap in crate lanes; 20 C2; 5 C1) · **net -205 tests, -3,829 lines** (verified per-lane sums, §7) · nothing-to-cut lanes=17.