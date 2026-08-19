### Fixed

- Host-integration Rust tools now use a fallback-safe sccache wrapper with a
  persistent host cache bind mount, while Crane keeps cargo artifacts and
  per-package source isolation.
