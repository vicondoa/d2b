# d2b-provider-config-nixos - unit-test audit
tests: 1 · src files: 4
net: -0 tests, -0 lines

## Findings (biggest net first)
Nothing to cut. Ship. Checked the crate's single unit test against the rest of the crate and its integration tests (`tests/config_lifecycle.rs`, `tests/service_contract.rs`, `tests/redaction.rs`): it pins a concurrency invariant no other test anywhere in the crate exercises.

## Keep
- `a_blocking_backend_dispatch_does_not_occupy_the_polling_worker` (src/ttrpc.rs:511) - pins that the registered ttrpc service handler (`ConfigMethod::handler` → `dispatch_on_blocking_worker`) runs backend dispatch off the polling worker: with a backend whose `dispatch` parks, the handler on a single-threaded runtime keeps its worker free and answers the parked call with the backend's result. Would fail on any regression to inline dispatch. Integration tests only call `dispatch` directly or through a non-blocking `TestBackend`, so they do not pin this.