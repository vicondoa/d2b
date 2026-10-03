### Fixed

- Made the resource-runtime requeue cancellation test deterministic under aggregate test contention.
- Woke controller-session reconciliation after transient bootstrap setup failures, prevented lock contention from dropping reconcile scheduling or identity evidence, and kept nested broker clients listening through their declared execution budgets.
- Exposed controller and virtiofsd worker diagnostics when Guest readiness fails.
