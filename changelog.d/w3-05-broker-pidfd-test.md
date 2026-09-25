### Fixed

- The broker pidfd handoff test now asserts the typed reconciliation refusal (`ReconciliationStartTimeMismatch`, including the drifted pid and start times) instead of matching the human-readable error string, so the test fails when the error variant or its payload changes.