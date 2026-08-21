//! Scoped reset inventories and operation-specific effect capabilities.

use std::collections::BTreeSet;
use std::fmt;

use d2b_contracts_resource::v3::{CanonicalJsonError, canonical_digest, canonical_json_bytes};
use serde::{Deserialize, Serialize};

use crate::model::{
    ArtifactId, Digest, EffectKind, FailureCode, OperationId, OperationKind, OperatorId, ResetScope,
};

/// A scope-specific reset target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResetTarget {
    scope: ResetScope,
    identity: ArtifactId,
}

impl ResetTarget {
    /// Construct a target for one reset scope.
    pub fn new(scope: ResetScope, identity: impl Into<String>) -> Result<Self, ResetError> {
        Ok(Self {
            scope,
            identity: ArtifactId::new(identity).map_err(|_| ResetError::InvalidTarget)?,
        })
    }

    /// Return the target scope.
    pub const fn scope(&self) -> ResetScope {
        self.scope
    }

    /// Borrow the opaque target identity.
    pub fn identity(&self) -> &ArtifactId {
        &self.identity
    }
}

/// A reset inventory that cannot widen into host-wide cutover state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResetInventory {
    scope: ResetScope,
    target: ResetTarget,
    preserve_durable_volumes: bool,
    destroy_durable_consent: bool,
}

impl ResetInventory {
    /// Build the default preserve-durable reset inventory.
    pub fn new(scope: ResetScope, identity: impl Into<String>) -> Result<Self, ResetError> {
        let target = ResetTarget::new(scope, identity)?;
        Ok(Self {
            scope,
            target,
            preserve_durable_volumes: true,
            destroy_durable_consent: false,
        })
    }

    /// Set whether durable Volumes remain detached and preserved.
    pub const fn with_preserve_durable_volumes(mut self, preserve: bool) -> Self {
        self.preserve_durable_volumes = preserve;
        self
    }

    /// Bind the separate consent required to destroy durable Volumes.
    pub const fn with_destroy_durable_consent(mut self, consented: bool) -> Self {
        self.destroy_durable_consent = consented;
        self
    }

    /// Return the reset scope.
    pub const fn scope(&self) -> ResetScope {
        self.scope
    }

    /// Borrow the single reset target.
    pub fn target(&self) -> &ResetTarget {
        &self.target
    }

    /// Return whether durable Volumes are preserved by default.
    pub const fn preserves_durable_volumes(&self) -> bool {
        self.preserve_durable_volumes
    }

    /// Return whether this inventory separately authorizes durable destruction.
    pub const fn allows_destroy_durable_volumes(&self) -> bool {
        !self.preserve_durable_volumes && self.destroy_durable_consent
    }

    /// Render exact canonical reset-inventory bytes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ResetError> {
        canonical_json_bytes(self).map_err(ResetError::CanonicalJson)
    }

    /// Compute the digest bound to a reset request.
    pub fn digest(&self) -> Result<Digest, ResetError> {
        Digest::parse(canonical_digest(
            "d2b:cutover:reset-inventory:v1",
            &self.canonical_bytes()?,
        ))
        .map_err(|_| ResetError::Digest)
    }
}

/// Closed operation-specific effect allowlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectAllowlist {
    operation_kind: OperationKind,
    effects: BTreeSet<EffectKind>,
}

impl EffectAllowlist {
    /// Construct the closed allowlist for an operation kind.
    pub fn for_operation(operation_kind: OperationKind) -> Self {
        let effects = match operation_kind {
            OperationKind::Cutover => BTreeSet::from([
                EffectKind::HostDrain,
                EffectKind::CutoverDisposition,
                EffectKind::ResourceStoreCreate,
                EffectKind::ProviderInstall,
                EffectKind::ZoneActivation,
                EffectKind::GuestActivation,
                EffectKind::Verification,
                EffectKind::CutoverFinalization,
                EffectKind::PreserveSource,
                EffectKind::QuarantineDestination,
                EffectKind::CutoverBroker,
                EffectKind::ClosureActivation,
            ]),
            OperationKind::ScopedReset(scope) => {
                let reset_effect = match scope {
                    ResetScope::Zone => EffectKind::ScopedZoneReset,
                    ResetScope::Provider => EffectKind::ScopedProviderReset,
                    ResetScope::Guest => EffectKind::ScopedGuestReset,
                };
                BTreeSet::from([
                    reset_effect,
                    EffectKind::DestroyDurableVolume,
                    EffectKind::PreserveSource,
                    EffectKind::QuarantineDestination,
                    EffectKind::Verification,
                ])
            }
        };
        Self {
            operation_kind,
            effects,
        }
    }

    /// Return the operation kind bound to this allowlist.
    pub const fn operation_kind(&self) -> OperationKind {
        self.operation_kind
    }

    /// Return whether one effect is authorized.
    pub fn permits(&self, effect: EffectKind) -> bool {
        self.effects.contains(&effect)
    }

    /// Return all effects in canonical order.
    pub fn effects(&self) -> impl Iterator<Item = &EffectKind> {
        self.effects.iter()
    }

    /// Reject effects that belong to a different authority.
    pub fn require(&self, effect: EffectKind) -> Result<(), ResetError> {
        if self.permits(effect) {
            Ok(())
        } else {
            Err(ResetError::EffectNotAllowed(effect))
        }
    }
}

impl OperationKind {
    /// Return the closed effect allowlist for this operation authority.
    pub fn allowlist(self) -> EffectAllowlist {
        EffectAllowlist::for_operation(self)
    }
}

/// A pure capability passed to one operation-scoped effect adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectCapability {
    operation_id: OperationId,
    operator_id: OperatorId,
    allowlist: EffectAllowlist,
}

impl EffectCapability {
    /// Bind one capability to an operation and operator.
    pub fn new(
        operation_id: OperationId,
        operator_id: OperatorId,
        operation_kind: OperationKind,
    ) -> Self {
        Self {
            operation_id,
            operator_id,
            allowlist: EffectAllowlist::for_operation(operation_kind),
        }
    }

    /// Borrow the operation identity.
    pub fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    /// Borrow the operator identity.
    pub fn operator_id(&self) -> &OperatorId {
        &self.operator_id
    }

    /// Return the operation kind.
    pub const fn operation_kind(&self) -> OperationKind {
        self.allowlist.operation_kind()
    }

    /// Validate one effect without performing it.
    pub fn authorize(&self, effect: EffectKind) -> Result<(), ResetError> {
        self.allowlist.require(effect)
    }
}

/// Scoped reset failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResetError {
    /// The target identity was invalid.
    InvalidTarget,
    /// An effect belongs to another operation authority.
    EffectNotAllowed(EffectKind),
    /// A reset inventory was used for a different scope.
    ScopeMismatch,
    /// Canonical reset inventory encoding failed.
    CanonicalJson(CanonicalJsonError),
    /// Reset inventory digest encoding failed.
    Digest,
}

impl ResetError {
    /// Return the stable failure class.
    pub const fn code(&self) -> FailureCode {
        match self {
            Self::InvalidTarget => FailureCode::InventoryInconsistent,
            Self::EffectNotAllowed(_) | Self::ScopeMismatch => FailureCode::EffectNotAllowed,
            Self::CanonicalJson(_) | Self::Digest => FailureCode::InventoryInconsistent,
        }
    }
}

impl fmt::Display for ResetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidTarget => "invalid reset target",
            Self::EffectNotAllowed(_) => "effect is not allowed for reset scope",
            Self::ScopeMismatch => "reset scope mismatch",
            Self::CanonicalJson(_) => "reset inventory canonicalization failed",
            Self::Digest => "reset inventory digest failed",
        })
    }
}

impl std::error::Error for ResetError {}
