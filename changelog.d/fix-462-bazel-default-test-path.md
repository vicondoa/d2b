### Changed

- Make public check and test aliases select the pinned Bazel development
  environment automatically, while keeping BuildBuddy locality and CI trust
  boundaries explicit. Bazel now owns the complete Layer-1 composition through
  nested public and package-level test suites instead of a duplicated Make
  label graph. Remove the redundant `make test` and `make check-all`
  convenience aggregates in favor of explicit conditional lanes.
