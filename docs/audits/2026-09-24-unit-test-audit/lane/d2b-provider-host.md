# d2b-provider-host — unit-test audit
tests: 19 · src files: 6
net: -1 tests, -18 lines

## Findings (biggest net first)
- duplicate: `factory_registers_exactly_the_host_resource_type` (src/driver.rs:715) — covered by `descriptor_declares_and_registers_the_host_type` (tests/registration.rs:27). Both pin the Host driver factory serves exactly one resource type, named `Host`, and `create` succeeds on the standard Host row key; the integration test pins strictly more (the whole descriptor/registration surface, same factory type, same assertions, same key.

- gap: the bounded-read boundary — `read_bounded` refuses reads over `limit` bytes or non-UTF8 content (src/probe.rs:199) — no test anywhere in the crate walks a file over the limit or with invalid UTF-8; a regression dropping the size bound or UTF-8 check would pass the suite (the only production-probe test reads small real files下.

## Keep
- `the_registry_serves_the_declared_factory_for_a_host_row` — pins the ProviderDirectory-registered declaration serving a Host row through reconcile → Satisfied + Ready (registry→driver→effects wiring端到端诊。
- `validate_accepts_the_bootstrap_host_row` — pins validate's accept path for the canonical bootstrap row.

- remaining 3 validate tests (`validate_rejects_a_host_with_a_foreign_provider`, `validate_rejects_a_host_without_a_provider_ref`, `validate_rejects_a_malformed_host_spec`) — pin three distinct Terminal spec-rejection fences: foreign providerRef, missing providerRef, undecodable payload. 
- `recover_adopts_without_touching_the_target` — recover → Adopted with no effects and no status published. 
- `reconcile_observes_once_per_desired_generation` — reconcile → Satisfied + Ready, exactly one observe per desired generation, second reconcile short-circuits, manager untouched. 
- `reconcile_publishes_a_degraded_host_observation` — degraded report → RetryScheduled + one requeue, Degraded phase published with capabilities/minijail fields intact. 
- `reconcile_maps_a_probe_failure_to_a_retryable_failure` — probe refusal → Retryable failure, no status published. 
- `finalize_finalizes_owned_children_before_the_delete_noop` — live owned child → Retryable + manager delete nudged; same pass converges once child retired. 
- `delete_converges_without_effects_or_child_mutation` — delete no-ops,no effects/manager calls. 
- `driver_operations_stay_off_every_spawn_surface` — all four driver ops leave manager/requeue untouched (no spawn surface (KTD13).
- `observe_host_publishes_the_probe_observation` — effects observe_host publishes the full bounded report (capabilities, kernel/os, user-manager, process count, minijail)+ one probe per capability class, Ready. 
- `observe_host_publishes_degraded_when_the_probe_cannot_complete` — probe failure → degraded fallback report with unknown/empty fields, Degraded (spec decision survivesфail. 
- `inspect_host_answers_the_bounded_host_observations` — hosted inspect-host serves the full canonical payload (capabilities, metadata, minijail gate fields. 
- `inspect_host_refuses_when_the_probe_cannot_complete` — failing probe → Declined closed code "inspect-host-probe-failed", never a half-built report. 
- `the_factory_builds_the_service_over_the_facets` — composition-root factory rebuilds a serving service over the production probe + recording gate (the only production-probe-through-service path in the crate. 
- `production_host_probe_returns_bounded_host_observations` — production probe over recording gate: bounded non-degenerate metadata, positive kernel, pidfd capability agrees with the supplied gate.