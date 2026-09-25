### Changed

- Replaced the d2b-telemetry bucket-constant smoke test with a behavior test: a histogram family built from the controller hint buckets accepts an in-range value and rejects an out-of-range one, so the test now fails on real behavior regressions instead of on refactors.