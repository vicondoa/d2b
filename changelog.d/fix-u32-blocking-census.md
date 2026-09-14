# `fix-u32-blocking-census.md`

### Added

- `cargo xtask blocking-census`: counts the workspace's uses of every API on
  the `clippy.toml` `disallowed-methods` deny list, separated into production
  and test contexts, so the U32 async-purity conversion's remaining backlog is
  measurable instead of remembered. First committed measurement: 935
  production / 2,057 test-context call sites.