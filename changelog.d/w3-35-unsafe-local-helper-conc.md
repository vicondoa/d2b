### Fixed

- The d2b-unsafe-local-helper admission counter now uses relaxed atomics: it only bounds the worker pool, and responses carry their own synchronization, so the stronger ordering was pure overhead.
- The helper snapshot test now asserts the extracted identity-mismatch decision instead of re-deriving it, so a regression that makes mismatched scopes report a live state instead of Degraded is caught.