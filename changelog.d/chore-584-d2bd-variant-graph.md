### Added

- Added a suite-membership guard (`tests/unit/meta/rust-main-packages-suite-guard.sh`)
  that runs under `make check`: every package declaring an `all-tests`
  aggregate must be listed in the Layer-1 `rust-main-packages` suite, and no
  aggregate may carry a positive tag that would silently drop it from the
  parent suite's expansion. `d2b-realm-core` is a documented intentional
  exclusion (retired owner, kept out of the shared aggregate Bazel edges).
  The guard's `sh_test` reads the real per-package BUILD files through
  `//:packages_workspace_sources`, which now lists every package's
  `BUILD.bazel` explicitly instead of relying on a glob.

### Fixed

- The d2bd test-support variant graph is unified: d2bd tests are re-included
  in Layer-1 through `//packages/d2bd:all-tests`, and the clippy debt that
  kept them out is fixed at source (the test-support rlib variants are linked
  in the Bazel tests and the family identifiers ride the shared
  family-knowledge ratchet).
- `d2b-provider-config-nixos` joins `rust-main-packages` with a native
  empty-suite `all-tests` aggregate instead of a hand-written test list, so
  its suite matches the canonical xtask buildbuddy-config rule for Layer-1
  members and can no longer drift from the package's actual tests.