### Changed

- Locked in the Azure Container Apps guest provider contract that a reconcile retried with a previously completed operation id returns `Converged` without re-running control-plane effects: a regression test reconciles twice with the same id against a running sandbox and asserts the second pass makes no effect or credential-lease calls.
- Dropped a dead `ResourceRef` parse from the `stable_error_codes_are_bounded` test that asserted nothing the test name promises and duplicated parse coverage exercised elsewhere.