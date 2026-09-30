//! Resource identity and ownership-edge types (U4).
//!
//! The durable identity types ([`ResourceKey`], [`ResourceProvenance`],
//! [`StoredDesiredResource`]) live with the spec store (U2) and are
//! re-exported here so the driver contract has one identity surface; types
//! the driver contract itself needs live here.
//!
//! U5 adds the two identities a durable authority change is named by: the
//! publication transaction ([`TransactionId`]) and the exact committed
//! desired state one admitted effect is fenced against
//! ([`row_freshness`]). Both reuse the shared authority contracts rather
//! than restating them, and neither carries a bearer credential: they
//! identify what was committed, never the right to act on it.

pub const MODULE_NAME: &str = "identity";

pub use crate::authority_journal::{DesiredRow, PublicationTransaction};
pub use crate::spec_store::{ResourceKey, ResourceProvenance, StoredDesiredResource};

/// Name of one resource type (for example `Process`, `Volume`).
///
/// The [`crate::provider::ProviderDirectory`] keys driver factories by this
/// name, and it must match the `type_name` component of [`ResourceKey`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResourceTypeName(String);

impl ResourceTypeName {
    /// Build a type name from its string form.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The type name as a plain string.
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

// ---------------------------------------------------------------------------
// Durable authority identity (U5, KTD5)
// ---------------------------------------------------------------------------

/// The stable 16-byte identity of one publication transaction.
///
/// It is derived from durable facts - the store incarnation, the Zone, the
/// reserved desired sequence, and the candidate digest - rather than drawn
/// from a random source, so a replay of the same staged candidate against the
/// same reserved sequence names the same transaction and recovery cannot mint
/// a second identity for one mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TransactionId([u8; 16]);

impl TransactionId {
    /// The raw identity bytes, as persisted.
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// Wrap 16 bytes read back from the store.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Read a persisted identity column back.
    pub fn from_column(bytes: &[u8]) -> Option<Self> {
        <[u8; 16]>::try_from(bytes).ok().map(Self)
    }
}

impl std::fmt::Display for TransactionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The canonical UUID spelling of the durable bytes: logs, refusals,
        // and operator-facing recovery output all read the same identity.
        match d2b_contracts_resource::v3::ResourceUid::from_bytes(&self.0) {
            Ok(uid) => f.write_str(uid.as_str()),
            Err(_) => write!(f, "transaction(<unrepresentable>)"),
        }
    }
}

/// Why one stored desired row cannot name its own freshness.
#[derive(Debug, thiserror::Error)]
pub enum RowIdentityError {
    /// The stored Zone is not a canonical Zone label.
    #[error("stored zone {0:?} is not a canonical Zone identity")]
    Zone(String),
    /// The stored `(type, name)` is not a canonical same-Zone resource
    /// reference.
    #[error("stored resource reference {type_name:?}/{name:?} is not canonical")]
    Reference { type_name: String, name: String },
}

/// The exact committed desired state one admitted effect is fenced against.
///
/// This is what decides whether earlier authority still holds, and it is
/// deliberately not the spec generation: an ownership, view, consumer,
/// provider-assignment, or execution-policy change advances the desired
/// revision even when the rendered spec bytes are identical (KTD5, R35,
/// AE16).
///
/// The typed reference is resolved from the stored `(type, name)` pair, so
/// this is the one place that turns a stored row into a canonical reference;
/// a row whose identity is not representable is refused rather than
/// approximated with a second spelling.
pub fn row_freshness(
    row: &DesiredRow,
    incarnation: &d2b_contracts_resource::v3::authority::StoreIncarnation,
) -> Result<d2b_contracts_resource::v3::authority::FreshnessTuple, RowIdentityError> {
    use d2b_contracts_resource::v3::authority::FreshnessTuple;
    use d2b_contracts_resource::v3::{ResourceRef, ResourceUid, ZoneId};

    let zone = ZoneId::parse(row.row.key.zone.clone()).map_err(|_| RowIdentityError::Zone(row.row.key.zone.clone()))?;
    let resource_ref = ResourceRef::parse(&format!(
        "{}/{}",
        row.row.key.type_name, row.row.key.name
    ))
    .map_err(|_| RowIdentityError::Reference {
        type_name: row.row.key.type_name.clone(),
        name: row.row.key.name.clone(),
    })?;
    let resource_uid = ResourceUid::from_bytes(&row.row.uid)
        .map_err(|_| RowIdentityError::Reference {
            type_name: row.row.key.type_name.clone(),
            name: row.row.key.name.clone(),
        })?;
    Ok(FreshnessTuple::new(
        zone,
        incarnation.clone(),
        resource_ref,
        resource_uid,
        row.revision,
        row.digest.clone(),
    ))
}
