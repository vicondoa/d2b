#![doc = "Canonical data transfer objects for nixling bundle artifacts."]

pub mod base64_codec;
pub mod bundle;
pub mod bundle_resolver;
pub mod closures;
pub mod error;
pub mod host;
pub mod host_check;
pub mod host_w3;
pub mod manifest;
pub mod manifest_v04;
pub mod minijail_profile;
pub mod privileges;
pub mod privileges_w3;
pub mod processes;
pub mod runtime;
pub mod static_invariants;

// `test_support` is needed both by external crates (which opt in via the
// `test-support` feature) and by nixling-core's OWN tests. Gating on
// `any(test, feature = "test-support")` makes it available automatically when
// compiling this crate's tests, so `cargo test -p nixling-core` works without
// anyone having to remember `--features test-support`.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
