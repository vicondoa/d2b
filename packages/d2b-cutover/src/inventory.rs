//! Host-wide inventory construction and fail-closed classification.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use d2b_contracts_resource::v3::{CanonicalJsonError, CanonicalJsonValue, canonical_json_bytes};
use serde::{Deserialize, Serialize};

use crate::model::{ArtifactId, Digest, Disposition, FailureCode, ZoneId};

/// Inventory classes admitted by the cutover contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InventoryClass {
    /// A configured Zone identity.
    Zone,
    /// A declared Guest identity.
    Guest,
    /// A declared Provider identity.
    Provider,
    /// TPM marker and NVRAM identity.
    TpmIdentity,
    /// A durable user-data Volume.
    DurableVolume,
    /// A closure-only store-view and its gcroot identity.
    StoreView,
    /// A framework-managed SSH key identity.
    SshKey,
    /// A managed network ownership marker.
    NetworkMarker,
    /// A legacy audit chain that must remain historical evidence.
    AuditChain,
    /// Host runtime metadata.
    HostRuntime,
    /// Regenerable boot-scoped runtime state.
    EphemeralRuntime,
    /// An item not recognized by the current migration matrix.
    Unclassified,
}

impl InventoryClass {
    /// Return whether the class carries an identity that must survive phase 10.
    pub const fn is_identity_bearing(self) -> bool {
        matches!(
            self,
            Self::TpmIdentity
                | Self::DurableVolume
                | Self::StoreView
                | Self::SshKey
                | Self::AuditChain
        )
    }
}

/// A single path-free inventory observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InventoryItem {
    identity: ArtifactId,
    class: InventoryClass,
    identity_bearing: bool,
    disposition: Disposition,
    source_retained_through_phase10: bool,
}

impl InventoryItem {
    /// Construct a classified observation.
    pub fn classified(
        identity: impl Into<String>,
        class: InventoryClass,
        disposition: Disposition,
    ) -> Result<Self, InventoryError> {
        let identity = ArtifactId::new(identity).map_err(|_| InventoryError::InvalidIdentity)?;
        let identity_bearing = class.is_identity_bearing();
        if identity_bearing && disposition == Disposition::Destroy {
            return Err(InventoryError::IdentityArtifactDestroy);
        }
        Ok(Self {
            identity,
            class,
            identity_bearing,
            disposition,
            source_retained_through_phase10: identity_bearing
                || disposition != Disposition::Destroy,
        })
    }

    /// Construct an unclassified item, which always defaults to Preserve.
    pub fn unclassified(identity: impl Into<String>) -> Result<Self, InventoryError> {
        Self::classified(
            identity,
            InventoryClass::Unclassified,
            Disposition::Preserve,
        )
    }

    /// Borrow the opaque identity.
    pub fn identity(&self) -> &ArtifactId {
        &self.identity
    }

    /// Return the inventory class.
    pub const fn class(&self) -> InventoryClass {
        self.class
    }

    /// Return the selected disposition.
    pub const fn disposition(&self) -> Disposition {
        self.disposition
    }

    /// Return whether the source is identity-bearing.
    pub const fn identity_bearing(&self) -> bool {
        self.identity_bearing
    }

    /// Return whether the source must remain through phase 10.
    pub const fn source_retained_through_phase10(&self) -> bool {
        self.source_retained_through_phase10
    }
}

/// Input item that can explicitly demonstrate the gateway custody boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InventoryInputItem {
    /// A normal path-free item.
    Item(InventoryItem),
    /// A forbidden attempt to enumerate gateway credentials or gateway audit.
    RealmGatewayCredentialAudit(ArtifactId),
}

impl From<InventoryItem> for InventoryInputItem {
    fn from(value: InventoryItem) -> Self {
        Self::Item(value)
    }
}

/// Inventory for one configured Zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ZoneInventory {
    zone_id: ZoneId,
    complete: bool,
    items: Vec<InventoryItem>,
}

impl ZoneInventory {
    /// Construct one Zone observation.
    pub fn new(
        zone_id: impl Into<String>,
        complete: bool,
        items: impl IntoIterator<Item = InventoryInputItem>,
    ) -> Result<Self, InventoryError> {
        let zone_id = ZoneId::new(zone_id).map_err(|_| InventoryError::InvalidZone)?;
        let items = normalize_items(items)?;
        Ok(Self {
            zone_id,
            complete,
            items,
        })
    }

    /// Construct a complete empty Zone inventory.
    pub fn empty(zone_id: impl Into<String>) -> Result<Self, InventoryError> {
        Self::new(zone_id, true, [])
    }

    /// Borrow the Zone identity.
    pub fn zone_id(&self) -> &ZoneId {
        &self.zone_id
    }

    /// Return whether the Zone observation is internally complete.
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    /// Borrow Zone-local items in canonical order.
    pub fn items(&self) -> &[InventoryItem] {
        &self.items
    }
}

/// The complete host-wide inventory used by a cutover preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostInventory {
    zones: Vec<ZoneInventory>,
    shared_items: Vec<InventoryItem>,
}

impl HostInventory {
    /// Build an all-Zone inventory and reject partial or inconsistent input.
    pub fn build(
        configured_zones: impl IntoIterator<Item = ZoneId>,
        observed_zones: impl IntoIterator<Item = ZoneInventory>,
        shared_items: impl IntoIterator<Item = InventoryInputItem>,
    ) -> Result<Self, InventoryError> {
        let configured = configured_zones.into_iter().collect::<BTreeSet<_>>();
        if configured.is_empty() {
            return Err(InventoryError::NoConfiguredZones);
        }

        let mut zones_by_id = BTreeMap::new();
        for zone in observed_zones {
            if !configured.contains(zone.zone_id()) {
                return Err(InventoryError::UnexpectedZone);
            }
            if !zone.is_complete() {
                return Err(InventoryError::IncompleteZone(zone.zone_id().clone()));
            }
            if zones_by_id.insert(zone.zone_id().clone(), zone).is_some() {
                return Err(InventoryError::DuplicateZone);
            }
        }
        if zones_by_id.len() != configured.len()
            || configured
                .iter()
                .any(|zone_id| !zones_by_id.contains_key(zone_id))
        {
            return Err(InventoryError::PartialZoneInventory);
        }

        let shared_items = normalize_items(shared_items)?;
        let zones = zones_by_id.into_values().collect::<Vec<_>>();
        let mut identities = BTreeSet::new();
        for item in shared_items
            .iter()
            .chain(zones.iter().flat_map(|zone| zone.items.iter()))
        {
            if !identities.insert(item.identity().clone()) {
                return Err(InventoryError::DuplicateIdentity);
            }
        }

        Ok(Self {
            zones,
            shared_items,
        })
    }

    /// Return the canonical all-Zone inventory digest.
    pub fn digest(&self) -> Result<Digest, InventoryError> {
        let bytes = canonical_json_bytes(self).map_err(InventoryError::CanonicalJson)?;
        Ok(Digest::derive("d2b:cutover:inventory:v1", &bytes))
    }

    /// Render exact canonical inventory bytes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, InventoryError> {
        canonical_json_bytes(self).map_err(InventoryError::CanonicalJson)
    }

    /// Decode an inventory through the duplicate-rejecting canonical JSON path.
    pub fn decode_json(bytes: &[u8]) -> Result<Self, InventoryError> {
        CanonicalJsonValue::parse(bytes).map_err(InventoryError::CanonicalJson)?;
        let wire: HostInventory =
            serde_json::from_slice(bytes).map_err(|_| InventoryError::MalformedJson)?;
        let zones = wire
            .zones
            .into_iter()
            .map(|zone| {
                ZoneInventory::new(
                    zone.zone_id.as_str(),
                    zone.complete,
                    zone.items.into_iter().map(InventoryInputItem::Item),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let configured = zones
            .iter()
            .map(|zone| zone.zone_id.clone())
            .collect::<Vec<_>>();
        Self::build(
            configured,
            zones,
            wire.shared_items.into_iter().map(InventoryInputItem::Item),
        )
    }

    /// Revalidate a deserialized inventory against the all-Zone contract.
    pub fn validate(&self) -> Result<(), InventoryError> {
        let configured = self
            .zones
            .iter()
            .map(|zone| zone.zone_id.clone())
            .collect::<Vec<_>>();
        let zones = self.zones.clone();
        let shared = self
            .shared_items
            .iter()
            .cloned()
            .map(InventoryInputItem::Item)
            .collect::<Vec<_>>();
        Self::build(configured, zones, shared).map(|_| ())
    }

    /// Borrow all configured Zones in canonical order.
    pub fn zones(&self) -> &[ZoneInventory] {
        &self.zones
    }

    /// Borrow shared host items in canonical order.
    pub fn shared_items(&self) -> &[InventoryItem] {
        &self.shared_items
    }

    /// Return every Zone identity.
    pub fn zone_ids(&self) -> impl Iterator<Item = &ZoneId> {
        self.zones.iter().map(ZoneInventory::zone_id)
    }

    /// Return whether every source artifact remains safe through phase 10.
    pub fn sources_retained(&self) -> bool {
        self.shared_items
            .iter()
            .chain(self.zones.iter().flat_map(|zone| zone.items.iter()))
            .all(InventoryItem::source_retained_through_phase10)
    }

    /// Return all identity-bearing source artifacts.
    pub fn identity_bearing_sources(&self) -> Vec<&InventoryItem> {
        self.shared_items
            .iter()
            .chain(self.zones.iter().flat_map(|zone| zone.items.iter()))
            .filter(|item| item.identity_bearing())
            .collect()
    }
}

/// Inventory construction failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InventoryError {
    /// No configured Zone was supplied.
    NoConfiguredZones,
    /// A Zone was present more than once.
    DuplicateZone,
    /// The observed set did not cover every configured Zone.
    PartialZoneInventory,
    /// An observed Zone was not configured.
    UnexpectedZone,
    /// A Zone observation was incomplete.
    IncompleteZone(ZoneId),
    /// An item identity was duplicated.
    DuplicateIdentity,
    /// An item identity was invalid.
    InvalidIdentity,
    /// A Zone identity was invalid.
    InvalidZone,
    /// An identity-bearing source may not be destroyed.
    IdentityArtifactDestroy,
    /// Gateway credentials and audit stay inside their gateway Guest.
    RealmGatewayCredentialAuditForbidden,
    /// Canonical encoding failed.
    CanonicalJson(CanonicalJsonError),
    /// The typed inventory JSON shape was invalid.
    MalformedJson,
    /// The operation kind and inventory kind did not match.
    InventoryKindMismatch,
}

impl InventoryError {
    /// Return the stable fail-closed error class.
    pub const fn code(&self) -> FailureCode {
        match self {
            Self::NoConfiguredZones
            | Self::DuplicateZone
            | Self::PartialZoneInventory
            | Self::UnexpectedZone
            | Self::IncompleteZone(_) => FailureCode::InventoryIncomplete,
            Self::DuplicateIdentity
            | Self::InvalidIdentity
            | Self::InvalidZone
            | Self::IdentityArtifactDestroy
            | Self::InventoryKindMismatch
            | Self::MalformedJson
            | Self::CanonicalJson(_) => FailureCode::InventoryInconsistent,
            Self::RealmGatewayCredentialAuditForbidden => {
                FailureCode::GatewayCredentialAuditEnumerationForbidden
            }
        }
    }
}

impl fmt::Display for InventoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoConfiguredZones => "no configured Zones",
            Self::DuplicateZone => "duplicate Zone",
            Self::PartialZoneInventory => "partial Zone inventory",
            Self::UnexpectedZone => "unexpected Zone",
            Self::IncompleteZone(_) => "incomplete Zone inventory",
            Self::DuplicateIdentity => "duplicate inventory identity",
            Self::InvalidIdentity => "invalid inventory identity",
            Self::InvalidZone => "invalid Zone identity",
            Self::IdentityArtifactDestroy => "identity-bearing source cannot be destroyed",
            Self::InventoryKindMismatch => "inventory kind mismatch",
            Self::MalformedJson => "inventory JSON shape rejected",
            Self::RealmGatewayCredentialAuditForbidden => {
                "gateway credential and audit enumeration is forbidden"
            }
            Self::CanonicalJson(_) => "inventory canonicalization failed",
        })
    }
}

impl std::error::Error for InventoryError {}

fn normalize_items(
    items: impl IntoIterator<Item = InventoryInputItem>,
) -> Result<Vec<InventoryItem>, InventoryError> {
    let mut normalized = Vec::new();
    for item in items {
        match item {
            InventoryInputItem::Item(mut item) => {
                item.identity_bearing |= item.class.is_identity_bearing();
                if item.class == InventoryClass::Unclassified {
                    item.disposition = Disposition::Preserve;
                    item.source_retained_through_phase10 = true;
                }
                if item.identity_bearing && item.disposition == Disposition::Destroy {
                    return Err(InventoryError::IdentityArtifactDestroy);
                }
                item.source_retained_through_phase10 |=
                    item.identity_bearing || item.disposition != Disposition::Destroy;
                normalized.push(item);
            }
            InventoryInputItem::RealmGatewayCredentialAudit(_) => {
                return Err(InventoryError::RealmGatewayCredentialAuditForbidden);
            }
        }
    }
    normalized.sort_by(|left, right| left.identity().cmp(right.identity()));
    let mut identities = BTreeSet::new();
    if normalized
        .iter()
        .any(|item| !identities.insert(item.identity().clone()))
    {
        return Err(InventoryError::DuplicateIdentity);
    }
    Ok(normalized)
}
