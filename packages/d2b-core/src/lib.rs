#![doc = "Canonical data transfer objects for d2b bundle artifacts."]

pub mod audio_policy;
pub mod base64_codec;
pub mod bundle;
pub mod bundle_resolver;
pub mod closures;
pub mod contract_id;
pub mod error;
pub mod host;
pub mod host_check;
pub mod host_w3;
pub mod manifest;
pub mod manifest_v04;
pub mod minijail_profile;
pub mod privileges;
pub mod privileges_w3;
pub mod process_builder;
pub mod processes;
pub mod runtime;
pub mod static_invariants;
pub mod storage;
pub mod storage_lifecycle;
pub mod sync;

// `test_support` is needed both by external crates (which opt in via the
// `test-support` feature) and by d2b-core's OWN tests. Gating on
// `any(test, feature = "test-support")` makes it available automatically when
// compiling this crate's tests, so `cargo test -p d2b-core` works without
// anyone having to remember `--features test-support`.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
