#![doc = "Canonical data transfer objects for nixling bundle artifacts."]

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

#[cfg(feature = "test-support")]
pub mod test_support;
