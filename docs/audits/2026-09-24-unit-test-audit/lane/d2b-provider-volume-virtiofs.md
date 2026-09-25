# d2b-provider-volume-virtiofs — unit-test audit
tests: 9 · src files: 8
net: -3 tests, -25 lines

(census 13 = 9 real `#[test]` fns + 4 doc-comment mentions of `#[test]` in src/testing.rs:121,139,151,163)

## Findings (biggest net first)
- duplicate: `the_envelope_never_carries_attachment_settings` (src/bindings.rs:390) — covered by `resource_binding_spec_keeps_one_strict_owner` (tests/lifecycle.rs:422). Both pin: an envelope whose spec carries attachment tuning (`threadPoolSize`) is rejected by `StoredBinding::from_resource_spec`.
- duplicate: `the_thread_pool_falls_back_to_the_guest_vcpu_count` (src/worker.rs:161) — covered by `a_read_only_binding_launches_a_read_only_worker` (tests/lifecycle.rs:158). Both pin: the worker plan's `thread_pool_size` equals the guest vcpu count and a read-only binding yields a `readonly` plan; integration asserts it through the real launch path.
- duplicate: `a_write_binding_over_a_read_only_view_is_rejected` (src/worker.rs:171) — covered by `a_write_binding_over_a_read_only_view_reports_failed_with_a_reason` (tests/lifecycle.rs:168). Both pin: write access over a read-only view fails with `ViewRightsInsufficient`; integration additionally pins the Failed phase and status code.
- gap: `vcpu_count == 0` → `InvalidBinding` (src/worker.rs:47) — real guard boundary in `VirtiofsdWorkerPlan::for_binding`; no test in src or tests/ passes a zero vcpu count.

## Keep
- `a_declared_host_capability_or_root_start_is_rejected` — pins `WorkerSandbox::assert_conformant` accepting the frozen posture and rejecting all four violations (host capability, root start, namespace sandbox mode, non-read-only root).
- `the_frozen_default_posture_survives_the_neutral_envelope` — pins the plan's frozen defaults: `WORKER_TEMPLATE`, no posix_acl/xattr, `cache: Auto`, mapping class, conformant sandbox (integration checks only readonly + thread_pool_size).
- `stored_binding_parses_a_strictly_neutral_envelope` — pins neutral-envelope parse incl. mount_path, generation, revision, fence uid (integration pins only volume_ref plus rejection paths).
- `any_provider_extension_rejects_the_envelope` — pins rejection of both provider-extension schema ids (Export and VolumeBinding) and a foreign owner (integration pins only the Export id and foreign type/owner).
- `worker_and_endpoint_children_keep_distinct_stable_refs` — pins distinct AND stable (deterministic) worker/endpoint child refs; integration pins only distinctness.
- `every_code_is_unique_and_matches_the_frozen_grammar` — pins all 11 error codes unique and matching the frozen lowercase/digit/hyphen grammar.