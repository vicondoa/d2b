### Changed

- Fail Bazel local, remote, trusted-seed, and typed local-fallback runs on
  redacted log lines beginning with `warning:`, deny warnings for first-party
  Rust crates at compilation, isolate concurrent facade evidence, and remove
  the xtask schema, clang `--unwindlib=none`, and remote gold-linker warnings.
- Fail closed when a BEP references a non-local `test.log` URI that cannot be
  scanned for warnings.
