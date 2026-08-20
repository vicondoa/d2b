### Changed

- Move Rust doctest and sealed API coverage into owning Bazel crate targets.
- Remove nested Cargo test compatibility and replace runtime tool builds with declared Bazel artifacts.
- Delete migration, successor-pin, and timing evidence shell scripts instead of translating them.
- Drop the wave-evidence hardware smoke script; physical-device validation is manual.
