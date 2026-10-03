### Fixed

- Made the resource-runtime requeue cancellation test deterministic under aggregate test contention.
- Woke controller-session reconciliation after transient bootstrap setup failures so retries cannot stall under load, and exposed controller launch diagnostics when Guest VMM readiness fails.
