### Changed

- Fail Bazel local, remote, trusted-seed, and typed local-fallback runs on
  redacted log lines beginning with `warning:`, deny warnings for first-party
  Rust crates at compilation, and remove the xtask schema, clang
  `--unwindlib=none`, and remote gold-linker warnings.
