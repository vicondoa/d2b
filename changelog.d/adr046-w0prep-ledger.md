### Changed

- The `test-runtime-ledger` gate now records per-test wall-clock p95s as
  advisory diagnostics and enforces aggregate per-crate process-CPU p95
  budgets. Process CPU excludes time descheduled behind unrelated machine
  load. It holds no baseline and makes no historical-regression claim. A
  genuine cross-machine reference baseline and a real multi-crate shard
  inventory are the deferred follow-up
  `runtime-ledger-full-census-and-real-shards`.
- Removed the shard dimension from the gate entirely, in both the `Makefile`
  recipe and `packages/xtask/src/test_runtime_ledger.rs`. Every shard had been
  assigned the identical per-crate aggregate and no shard target was ever
  executed, so the ledger no longer records, checks, audits, or reports a shard
  scope, and `tests/runtime-ledger-census.json` no longer pins a shard set.
  Real shards land only with the named deferred follow-up.
- Reconciled the remaining ledger prose across `AGENTS.md`, `tests/README.md`,
  the ADR-046 validation-and-delivery, streamline, and feasibility-and-spikes
  specs, and the preparatory changelog fragments, so no surface advertises the
  removed per-test enforcement, baseline, historical-regression, or shard
  capabilities (there is no committed baseline file).
  Regenerating the census pin after a legitimate test change is a separate,
  supported step: `make runtime-ledger-pin`.
- Documented the envelope policy lint's D116 negative-example marker in
  `AGENTS.md` beside the existing lint guidance: the `policy_adr046_envelopes`
  lint exempts an intentional teaching block that demonstrates the D116
  eval-time failure only in the pinned documenting file and only when it carries
  the exact `d2b-lint: expect-d116-eval-error` marker. The guidance frames it as
  a narrowly scoped intentional-rejection signal rather than a general
  suppression switch.

### Fixed

- Documented every `xtask` command in the ADR-046 delivery workflow with a
  repository-root-runnable invocation. The previous `cargo xtask ...` form fails
  from the repository root because that alias is defined only in
  `packages/.cargo/config.toml`; the delivery spec now uses
  `cargo run --manifest-path packages/Cargo.toml -p xtask -- ...` throughout and
  notes the `sccache`-wrapper tradeoff plus the `cd packages && cargo xtask ...`
  alternative.
