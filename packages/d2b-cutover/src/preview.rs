//! Canonical, non-mutating preview construction.

use std::fmt;

use d2b_contracts::v3::{
    CanonicalJsonError, CanonicalJsonValue, canonical_digest, canonical_json_bytes,
};
use serde::{Deserialize, Serialize};

use crate::{
    inventory::{HostInventory, InventoryError},
    model::{CandidateId, CutoverPhase, Digest, OperationId, OperationKind, RevisionPlanId},
    reset::{EffectAllowlist, ResetInventory},
};

/// Domain separator for cutover preview digests.
pub const PREVIEW_DOMAIN: &str = "d2b:cutover:preview:v1";

/// Inventory carried by a canonical preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind", content = "value")]
pub enum PreviewInventory {
    /// Host-wide all-Zone inventory.
    Host(HostInventory),
    /// One scoped reset inventory.
    Reset(ResetInventory),
}

/// A canonical dry-run preview bound to one operation and inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CutoverPreview {
    operation_id: OperationId,
    operation_kind: OperationKind,
    candidate_id: CandidateId,
    revision_plan_id: RevisionPlanId,
    inventory: PreviewInventory,
    recovery_digest: Option<Digest>,
    rollback_boundary: CutoverPhase,
    effect_allowlist: EffectAllowlist,
}

impl CutoverPreview {
    /// Build a preview from a normalized inventory without mutating the host.
    pub fn new(
        operation_id: OperationId,
        operation_kind: OperationKind,
        candidate_id: CandidateId,
        revision_plan_id: RevisionPlanId,
        inventory: HostInventory,
        recovery_digest: Option<Digest>,
    ) -> Result<Self, PreviewError> {
        if !operation_kind.is_cutover() {
            return Err(PreviewError::Inventory(
                InventoryError::InventoryKindMismatch,
            ));
        }
        if operation_kind.is_cutover() && inventory.zones().is_empty() {
            return Err(PreviewError::Inventory(InventoryError::NoConfiguredZones));
        }
        Ok(Self {
            operation_id,
            operation_kind,
            candidate_id,
            revision_plan_id,
            inventory: PreviewInventory::Host(inventory),
            recovery_digest,
            rollback_boundary: CutoverPhase::Disposition,
            effect_allowlist: EffectAllowlist::for_operation(operation_kind),
        })
    }

    /// Build a preview for a scoped reset without widening its inventory.
    pub fn new_reset(
        operation_id: OperationId,
        operation_kind: OperationKind,
        candidate_id: CandidateId,
        revision_plan_id: RevisionPlanId,
        inventory: ResetInventory,
    ) -> Result<Self, PreviewError> {
        if operation_kind.reset_scope() != Some(inventory.scope()) {
            return Err(PreviewError::Inventory(
                InventoryError::InventoryKindMismatch,
            ));
        }
        Ok(Self {
            operation_id,
            operation_kind,
            candidate_id,
            revision_plan_id,
            inventory: PreviewInventory::Reset(inventory),
            recovery_digest: None,
            rollback_boundary: CutoverPhase::Disposition,
            effect_allowlist: EffectAllowlist::for_operation(operation_kind),
        })
    }

    /// Borrow the operation identity.
    pub fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    /// Return the operation kind.
    pub const fn operation_kind(&self) -> OperationKind {
        self.operation_kind
    }

    /// Borrow the candidate identity.
    pub fn candidate_id(&self) -> &CandidateId {
        &self.candidate_id
    }

    /// Borrow the revision-bound plan identity.
    pub fn revision_plan_id(&self) -> &RevisionPlanId {
        &self.revision_plan_id
    }

    /// Borrow the all-Zone inventory.
    pub fn inventory(&self) -> &PreviewInventory {
        &self.inventory
    }

    /// Borrow the host inventory when this is a cutover preview.
    pub fn host_inventory(&self) -> Option<&HostInventory> {
        match &self.inventory {
            PreviewInventory::Host(inventory) => Some(inventory),
            PreviewInventory::Reset(_) => None,
        }
    }

    /// Borrow the reset inventory when this is a reset preview.
    pub fn reset_inventory(&self) -> Option<&ResetInventory> {
        match &self.inventory {
            PreviewInventory::Host(_) => None,
            PreviewInventory::Reset(inventory) => Some(inventory),
        }
    }

    /// Return the recovery digest bound into this preview.
    pub fn recovery_digest(&self) -> Option<&Digest> {
        self.recovery_digest.as_ref()
    }

    /// Return the explicit native rollback boundary.
    pub const fn rollback_boundary(&self) -> CutoverPhase {
        self.rollback_boundary
    }

    /// Borrow the closed effect allowlist.
    pub fn effect_allowlist(&self) -> &EffectAllowlist {
        &self.effect_allowlist
    }

    /// Render exact canonical preview bytes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PreviewError> {
        canonical_json_bytes(self).map_err(PreviewError::CanonicalJson)
    }

    /// Compute the domain-separated preview digest.
    pub fn digest(&self) -> Result<Digest, PreviewError> {
        let bytes = self.canonical_bytes()?;
        Ok(Digest::parse(canonical_digest(PREVIEW_DOMAIN, &bytes))?)
    }

    /// Decode a preview through the strict canonical JSON path.
    pub fn decode_json(bytes: &[u8]) -> Result<Self, PreviewError> {
        CanonicalJsonValue::parse(bytes).map_err(PreviewError::CanonicalJson)?;
        let preview: Self =
            serde_json::from_slice(bytes).map_err(|_| PreviewError::MalformedJson)?;
        preview.validate()?;
        Ok(preview)
    }

    fn validate(&self) -> Result<(), PreviewError> {
        match (&self.operation_kind, &self.inventory) {
            (OperationKind::Cutover, PreviewInventory::Host(inventory))
                if !inventory.zones().is_empty() =>
            {
                inventory.validate().map_err(PreviewError::Inventory)?;
            }
            (OperationKind::ScopedReset(scope), PreviewInventory::Reset(inventory))
                if Some(*scope) == Some(inventory.scope()) => {}
            (OperationKind::Cutover, _) | (OperationKind::ScopedReset(_), _) => {
                return Err(PreviewError::Inventory(
                    InventoryError::InventoryKindMismatch,
                ));
            }
        }
        if self.rollback_boundary != CutoverPhase::Disposition
            || self.effect_allowlist != EffectAllowlist::for_operation(self.operation_kind)
        {
            return Err(PreviewError::InvalidContract);
        }
        Ok(())
    }
}

/// Preview construction or canonicalization failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewError {
    /// The inventory was incomplete or inconsistent.
    Inventory(InventoryError),
    /// Canonical JSON rejected a value.
    CanonicalJson(CanonicalJsonError),
    /// A digest could not be parsed.
    Digest(crate::model::IdError),
    /// The typed preview JSON shape was invalid.
    MalformedJson,
    /// A decoded preview violated its closed contract.
    InvalidContract,
}

impl fmt::Display for PreviewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Inventory(_) => "preview inventory rejected",
            Self::CanonicalJson(_) => "preview canonicalization failed",
            Self::Digest(_) => "preview digest failed",
            Self::MalformedJson => "preview JSON shape rejected",
            Self::InvalidContract => "preview contract rejected",
        })
    }
}

impl std::error::Error for PreviewError {}

impl From<InventoryError> for PreviewError {
    fn from(error: InventoryError) -> Self {
        Self::Inventory(error)
    }
}

impl From<crate::model::IdError> for PreviewError {
    fn from(error: crate::model::IdError) -> Self {
        Self::Digest(error)
    }
}
