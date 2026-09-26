### Fixed

- The broker's fake reconcile re-export is compiled only where its users
  are: the fake-backends library build no longer fails on the unused
  import while the unit tests that drive `FakeReconcileExecutor` and
  `ReconcileOp` keep working unchanged.
- `xtask`'s production-closure context no longer passes a `&ContextSpec`
  that is already a reference, so the schema-reproducibility clippy
  target builds clean.
