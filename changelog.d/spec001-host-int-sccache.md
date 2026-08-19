### Changed

- Host-integration Rust tools keep Crane cargo artifacts and per-package
  source isolation. sccache is opt-in via `D2B_HOST_SCCACHE=1`, uses a
  constant sandbox path, and never world-writes the host cache.
