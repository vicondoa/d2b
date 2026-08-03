### Changed

- Recorded the decision to make Bazel the build and test scheduler for the
  Rust gate. The current `make test-rust` path and the existing Rust
  continuous-integration jobs stay authoritative; a new `make test-bazel-rust`
  target and a separate, non-required workflow run the Bazel path beside them
  so the two can be compared. Switching over requires a complete
  surface-by-surface coverage map, evidence that each check still fails when it
  should, an unchanged pinned test inventory, and measured wall-clock ceilings
  of ten minutes for a warm local run and fifteen minutes for a cold local run
  and a cold continuous-integration run. Cargo manifests, lock files, and the
  pinned Rust toolchains remain the authoritative dependency and toolchain
  inputs, and the decision covers Rust only.
- The evidence for switching over is drawn from post-merge pushes to the
  protected integration lineage, so both paths always compare the same commit.
  Ten consecutive matching pushes are required, and the cold
  continuous-integration measurement is taken from the five most recent
  qualifying cold pushes. A run that reaches no verdict while its counterpart
  reaches one resets the count, so cancelling a run cannot inflate it.
  Pull-request runs stay diagnostic and keep their path filter.
- Integration tests will locate their binaries and fixtures through a locator
  with two modes: declared build inputs under Bazel, and the existing Cargo
  environment under Cargo, with the Cargo mode expanding in the test crate that
  owns the binary. The two modes never chain, so a missing declared input fails
  with the path it expected instead of silently finding a stale binary left
  over from an earlier build, and no test resolves a binary by absolute build
  path.
- The offline dependency-policy inputs are produced by a repository-owned rule
  that re-declares each locked crate download by URL and the checksum the lock
  file already records, and handles the single pinned git dependency
  explicitly. Downloads stay pinned and reuse the existing download cache;
  build actions still reach no network. Every lock entry must classify or the
  rule refuses by name, and the crate count is checked against the lock before
  the policy tool runs, so a vendored tree that is short a crate cannot report
  fewer findings and pass.
- If the combined dependency check and its offline decomposition disagree about
  yanked crates, the difference is carried by an added check against a
  committed registry snapshot bounded by the lock files and reported under the
  existing dependency-policy surfaces. Switching over stays blocked until both
  paths produce the same enforcing findings; no advisory or licence outcome may
  be dropped.
- The requirement that no shell appears in the execution path is scoped to the
  build wrapper, test runner, cleanup, and process-control code this repository
  owns. The documentation-test runner that the Rust build rules generate on a
  stable toolchain is recorded as a known difference. The rule that the shipped
  `d2b` command line never invokes a shell is unchanged and is not widened.
- The nightly toolchain the public-API inventory needs is selected for that
  part of the build graph only, so the whole Rust suite stays one invocation
  and no other crate compiles on nightly. The inventory is rendered by a
  repository-owned rule that emits the toolchain version it actually used,
  checked against the committed pin, rather than asserting the pin file alone.
- Coverage-map enforcement is split so each half runs where it can actually
  execute: a mapped label that does not exist fails when the build graph is
  analysed, and completeness across the graph is checked by the existing drift
  tooling instead of from inside a test.
- The repository-owned case runner publishes one JUnit case per Rust test,
  preserves ignored outcomes, and gives each case its own temporary directory,
  so Bazel event data and continuous-integration test results retain the same
  failure attribution contributors have today. The JUnit record is bounded and
  redacted and does not publish environment values, command arguments, local
  paths, identifiers, opaque handles, or raw child output.
- The repository development shell supplies the pinned Bazel tools, the
  `cargo-bazel` generator cannot fall back to an unpinned source bootstrap, and
  the pinned git dependency is fetched with both its revision and integrity
  hash.
- Retiring the Cargo executor does not retire the public `test-rust` or focused
  Rust Make targets. They continue to invoke the Bazel carriers, while the
  fixture-contract lane remains an enforcing Cargo and Nix companion.
