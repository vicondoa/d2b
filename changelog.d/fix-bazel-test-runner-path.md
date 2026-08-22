### Fixed

- Preserve the pinned development-shell `PATH` for Bazel test-runner actions
  so local Layer-1 tests can resolve their declared shell tools on NixOS.
