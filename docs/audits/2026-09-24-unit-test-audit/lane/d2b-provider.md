# d2b-provider — unit-test audit
tests: 4 · src files: 10
net: -0 tests, -0 lines

## Findings (biggest net first)
Nothing to cut. Ship. All 4 unit tests pin distinct behaviors (agent dispatch service gate, request-timeout zero boundary, serve-loop clean termination, and the drain lost-wakeup race); integration tests in tests/runtime.rs cover registry drain only at the manager level and do not overlap.

## Keep
- `unsupported_service_returns_typed_error` (src/agent.rs:359) — pins dispatch of a non-`d2b.provider.v3` service name returning `ProviderAgentError::UnsupportedService` (service gate at agent.rs:203).
- `negative_timeout_is_rejected_before_dispatch` (src/agent.rs:374) — pins `ProviderAgentRequest::new` rejecting `timeout_ms == 0` with `InvalidTimeout` before any dispatch.
- `session_close_terminates_serve_loop` (src/agent.rs:389) — pins `serve` returning `Ok` cleanly on `SessionClosed`/channel drop without emitting a response (clean termination, not retry).
- `dropping_the_final_permit_between_check_and_await_notifies_drain_waiters` (src/registry.rs:732) — pins the lost-wakeup race: final in-flight permit dropped between `wait_until_drained`'s load-check and `notified().await` still wakes the drain waiter.

## Gaps
- `ProviderAgentRequest::new` rejects `timeout_ms > MAX_AGENT_TIMEOUT_MS` (src/agent.rs:46) — only the zero boundary is tested; the protocol-maximum boundary is untested.
- `ProviderAgent::new` rejects `ProviderBindingAxis::Unknown` (src/agent.rs:223) — the constructor's only error path is untested.
- `serve` returns `Err(SessionClosed)` when the responses channel closes mid-dispatch (src/agent.rs:322) — the send-failure path is untested.