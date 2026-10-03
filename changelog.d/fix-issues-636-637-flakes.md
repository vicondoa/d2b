### Fixed

- Made the resource-runtime requeue cancellation test deterministic under aggregate test contention.
- Woke controller-session reconciliation after transient bootstrap setup failures and isolated committed Provider identity reads from unrelated registry contention so restart recovery cannot misclassify a present Provider as missing.
- Exposed controller and virtiofsd worker diagnostics when Guest readiness fails.
