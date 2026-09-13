//! Resource identity and ownership-edge types (U4).
//!
//! The durable identity types ([`ResourceKey`], [`ResourceProvenance`],
//! [`StoredDesiredResource`]) live with the spec store (U2) and are
//! re-exported here so the driver contract has one identity surface; types
//! the driver contract itself needs live here.

pub const MODULE_NAME: &str = "identity";

pub use crate::spec_store::{ResourceKey, ResourceProvenance, StoredDesiredResource};

/// Name of one resource type (for example `Process`, `Volume`).
///
/// The [`crate::provider::ProviderDirectory`] keys driver factories by this
/// name, and it must match the `type_name` component of [`ResourceKey`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResourceTypeName(String);

impl ResourceTypeName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ResourceTypeName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::fmt::Display for ResourceKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}/{}", self.zone, self.type_name, self.name)
    }
}