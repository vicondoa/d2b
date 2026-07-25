### Fixed

- Clarified the tier0 gate's message when the shell linter is absent from
  `PATH`. The previous wording read as though shell linting had been skipped
  entirely; the authoritative gate is `make test-lint`, which provisions the
  linter through nix and fails closed when it cannot.
