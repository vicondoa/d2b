# d2b-provider-test-controller - unit-test audit
tests: 2 · src files: 1
net: -0 tests, -0 lines

## Findings (biggest net first)
- gap: retry-on-transient-failure loop in `run()` (src/main.rs:52-56) - no test pins that a failed `run_session` (handshake_setup failure) is retried after 500ms backoff instead of exiting the process; the only coverage (unit test 2) pins run_session's terminal-once bootstrap send directly, bypassing run().
 Integration test `no_bootstrap_descriptor_fails_closed` (tests/controller.rs) covers only the fail-closed exit path (no fd 10).

Nothing to cut. Ship. Checked both `#[test]` fns against the product code (`should_reconnect`, `run_session` bootstrap/handshake path), the crate's `tests/controller.rs`, and the `run()` retry loop.



## Keep
- `session_loss_reconnects_but_peer_shutdown_is_graceful` - pins `should_reconnect` mapping: RoleMismatch/SessionLost → reconnect, Normal/PeerRequested → graceful shutdown.
- `initial_handshake_failure_sends_one_bootstrap_then_is_terminal` - pins run_session sends exactly one bootstrap packet,then errors terminal on initial handshake failure (no second packet, no hang}。
