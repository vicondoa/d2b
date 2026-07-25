### Changed

- The `test-runtime-ledger` gate now ships as an honest absolute per-test and
  per-crate execution-budget gate: each recorded p95 is judged only against its
  own frozen budget, so a slower run that still fits its budget passes. It holds
  no baseline and makes no historical-regression claim. A genuine cross-machine
  reference baseline and a real multi-crate shard inventory are the deferred
  follow-up `runtime-ledger-full-census-and-real-shards`.
- Removed the shard dimension from the gate entirely, in both the `Makefile`
  recipe and `packages/xtask/src/test_runtime_ledger.rs`. Every shard had been
  assigned the identical per-crate aggregate and no shard target was ever
  executed, so the ledger no longer records, checks, audits, or reports a shard
  scope, and `tests/runtime-ledger-census.json` no longer pins a shard set.
  Real shards land only with the named deferred follow-up.
- Reconciled the remaining ledger prose across `AGENTS.md`, `tests/README.md`,
  the ADR-046 validation-and-delivery, streamline, and feasibility-and-spikes
  specs, and the ADR-046 W0-prep changelog fragments, so no surface advertises
  the removed baseline, historical-regression, shard, or regeneration-workflow
  capabilities (there is no `make runtime-ledger-regen` target and no committed
  baseline file).

### Fixed

- Documented every `xtask` command in the ADR-046 delivery workflow with a
  repository-root-runnable invocation. The previous `cargo xtask ...` form fails
  from the repository root because that alias is defined only in
  `packages/.cargo/config.toml`; the delivery spec now uses
  `cargo run --manifest-path packages/Cargo.toml -p xtask -- ...` throughout and
  notes the `sccache`-wrapper tradeoff plus the `cd packages && cargo xtask ...`
  alternative.
