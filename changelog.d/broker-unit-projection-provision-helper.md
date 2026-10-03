### Fixed

- The broker's unit-test Zone-provisioning helper now publishes the
  reconciliation a reconciling Zone accepts, instead of an install the
  projection refuses. The helper drove `BeginSnapshot` to `EndSnapshot` with
  the cursor at `accepted.try_next()`, which only ever describes an install. A
  transfer that opens from `Reconciling` is by definition a
  resynchronization rather than an install, and a resynchronization is
  checked by equality against the projection the broker holds: a document that
  moves the cursor forward describes revisions no fence this broker
  validated, so it was refused `publication-projection-unproven` /
  `StaleAuthority`, the Zone stayed fenced, and every dispatch that followed
  read it as `authority-fenced` instead of reaching the boundary under test.
  The helper now restates the accepted cursor, which is the one document that
  recovers every posture it can meet - `Unprovisioned`, where accepted is
  initial, and `Reconciling`, where accepted is whatever survived the boot -
  and which is what the fence contract requires of a reconciliation. The
  production refusal, the `PrepareChange`/`CommitChange` path that is the real
  way to advance a cursor, and every assertion under test are unchanged; only
  the document the helper sent was wrong.
- That refusal was invisible to the Bazel gate and permanent under Cargo,
  which is worth stating because the two lanes compile the same 764 tests from
  the same sources with the same (empty) feature set - there was no coverage
  gap. The helper's durable state lives in
  `d2b_core::test_support::scratch_root`, which reads `TEST_TMPDIR`; Bazel's
  `rust_test` hands every invocation a fresh sandbox, while `cargo test` does
  not set it and falls back to the stable
  `packages/d2b-broker/target/d2b-test-scratch/`, so the second and later runs
  of the same binary started from a projection the previous run had left
  `Reconciling`. The first run after a clean target directory passed, every run
  after it failed 21 tests, and Bazel never saw the condition. The helper is
  now correct under the restart semantics it was always subject to rather than
  correct only against a clean directory.
