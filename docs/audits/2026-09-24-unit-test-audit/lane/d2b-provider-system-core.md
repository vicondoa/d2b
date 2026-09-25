# d2b-provider-system-core - unit-test audit
tests: 1 · src files: 6
net: -0 tests, -0 lines

## Findings (biggest net first)
- gap: `KernelTooOld` error path (`MinijailPlatformGate::validate`, src/host.rs:122-124) - the kernel-floor branch of the minijail gate is never exercised; the integration suite covers the sibling `CgroupKillUnavailable` branch (tests/host_reconciliation.rs:325) and the passing gate, but no test anywhere in the crate builds a gate below the kernel floor. A floor regression would pass silently.
- Nothing to cut. Ship. Checked the sole unit test against the crate's 22 integration tests (tests/ownership.rs, host_reconciliation.rs, user_discovery.rs) and found no duplicate or trivial unit test; the one unit test pins the allowlist decision surface directly, which the end-to-end suite only reaches transitively.

## Keep
- `an_unclaimed_resource_type_is_refused_too` - pins `owns()` as a closed allowlist: a ResourceType in neither OWNED nor DISOWNED is refused, including the future-type drift case (`SomeFutureSemanticType`) that no integration test covers (`a_guest_is_not_a_host` covers only Guest, end-to-end).

route-out: `SystemCoreError::BudgetOvercommit` (src/error.rs:34) is never constructed anywhere in the crate - dead variant or an unimplemented budget check.
