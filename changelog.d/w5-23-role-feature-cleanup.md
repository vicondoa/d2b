### Removed

- Removed `d2b-provider-role`'s empty `test-support` feature: it gated no code and no manifest enabled it, and `d2b_provider_role::rbac` stays public product surface.

### Fixed

- `d2b-provider-role`'s `registration` suite no longer requires that dead feature, so `cargo test -p d2b-provider-role` runs it instead of silently skipping it.
