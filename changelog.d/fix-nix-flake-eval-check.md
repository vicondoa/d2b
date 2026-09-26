### Fixed

- The Bazel flake-evaluation checks (`flake-eval-x86`, `flake-eval-aarch64`,
  `flake-eval-x86-outputs`, and the `realized-*` variants) now resolve the
  flake runfile to the real workspace path before handing it to nix, so the
  checks pass when run directly with `bazel test` instead of only through the
  Makefile wrappers. Previously the runfiles-tree layout placed the flake
  under `bazel-out/`, where nix's git-tracking check rejected it as untracked.