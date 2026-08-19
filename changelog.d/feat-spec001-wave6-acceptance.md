### Changed

- Cache source-built host-tool dependencies across acceptance VM checks so
  in-tree Rust changes rebuild only affected binaries instead of the full
  workspace.
