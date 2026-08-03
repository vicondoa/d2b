### Added

- Add a bounded GNU Make Rust test DAG with grouped keep-going output,
  dependency-ordered leaves, serial broker feature passes, and explicit
  companion coverage for doctests and harness-free binaries while retaining
  the default fixture-dependent contract and CLI surfaces when Nix is
  available.
- Add opt-in version 1 execution manifests through
  `D2B_EXECUTION_MANIFEST`, including deterministic sub-surface fragments and
  atomic partial evidence for failed and handled-interruption runs.

### Changed

- Use `D2B_RUST_BUDGET` as the supported local Rust budget control. Top-level
  Make `-j` does not cap inner Cargo concurrency; the Rust target derives
  Cargo and nextest quotas from the effective CPU and memory budget.
- Run the complete Nix-unit corpus through one aggregate
  `nix-eval-jobs --no-instantiate` attr per current case file (45 file jobs),
  with focused toolchain self-provisioning, bounded worker control, and
  complete multi-failure reporting. Reuse the same aggregate constructor for
  the seven topical flake checks, and expose one locked inventory containing
  sorted full case names and file-job names.
- Use the operator-intent `D2B_NIX_UNIT_WORKERS` and
  `D2B_NIX_UNIT_MEMORY_MB` controls for Nix-unit resource requests, and retire
  `D2B_NIX_UNIT_JOBS` with an actionable migration error.
- Keep successful full runs concise while retaining one sanitized stderr
  attribution per real `FAIL <case>: <detail>` line, with one fallback for an
  aggregate that emits no such line. Report exact evaluated-vs-pinned case-name
  drift with the `run make nix-unit-pin` remedy; use a fixed path-free `d2b`
  flake label for command progress.
- Record Nix-unit execution evidence as the seven stable baseline leaves while
  keeping evaluation-only runs free of installables and realized checks; use
  one aggregate eval-jobs attribute per case file plus shard/pin integrity.
- Reject the seven-aggregate candidate after its 543s local four-worker
  observation and the per-case candidate after hosted memory exhaustion.
- Keep four requested local workers with a 4096 MiB default, use 3072 MiB on
  GitHub Actions so the existing envelope admits two workers on a 16 GiB
  runner, and retain exact case and file-job inventory checks.
- Keep the separate enforcing fixture lane from duplicating the aggregate by
  honoring `D2B_SKIP_FIXTURE_BUILD=1` in the Layer-1 orchestration.
- Keep the measured parallel profile for warm local runs, retain its API cache
  while using a bounded prebuild plus fixture/inventory/schema chain for cold
  runs, and run each Rust leaf as a separate full-budget CI job behind the
  stable rollup.
