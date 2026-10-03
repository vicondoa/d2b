### Fixed

- Made the resource-runtime requeue cancellation test deterministic under aggregate test contention.
- Woke controller-session reconciliation after transient bootstrap setup failures and prevented lock contention from dropping reconcile scheduling or erasing committed Provider and runner executable identities during liveness and restart recovery.
- Exposed controller and virtiofsd worker diagnostics when Guest readiness fails.
