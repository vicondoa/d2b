### Fixed

- The quota, resource-export, resource-import, and role provider crates now gate their registration integration tests behind the `test-support` feature, which previously was declared empty and gated nothing; a plain `cargo test -p <crate>` now skips them consistently with the rest of the provider family.