//! Trusted Core inventory receipts for irreversible state adoption.

use core::fmt;
use d2b_contracts_zone_session::v3::canonical_digest;

/// Opaque identity of one legacy TPM state row.
///
/// The type has no public constructor or deserialization implementation.
/// Only an anchored Core inventory value can mint it.
#[derive(Clone, PartialEq, Eq)]
pub struct LegacyTpmStateId([u8; 32]);

impl LegacyTpmStateId {
    /// Mint a receipt from an anchored, already-validated Core inventory row.
    pub(crate) fn from_anchored_inventory(anchor: &str) -> Self {
        use sha2::{Digest, Sha256};

        let mut digest = Sha256::new();
        digest.update(b"d2b:legacy-tpm-state/v1:");
        digest.update(anchor.as_bytes());
        Self(digest.finalize().into())
    }
}

/// Core-owned decision held by the production TPM effect adapter.
#[derive(Clone, PartialEq, Eq)]
pub struct LegacyTpmMigrationDecision {
    state_id: Option<LegacyTpmStateId>,
    vm_binding: String,
    intent_binding: String,
}

impl LegacyTpmMigrationDecision {
    /// Construct a migration decision from the trusted Core bundle inventory.
    ///
    /// The anchor is supplied only by Core's bundle resolver, never by the
    /// reconcile request. The broker remains authoritative for source-state
    /// inspection and replay.
    pub fn adoption_required(vm_id: &str, intent_ref: &str, inventory_anchor: &str) -> Self {
        Self::from_anchored_inventory(
            Some(LegacyTpmStateId::from_anchored_inventory(inventory_anchor)),
            vm_id,
            intent_ref,
        )
    }

    /// Construct the Core-owned no-migration decision for a Device proven
    /// never to have a legacy state row.
    pub fn not_applicable(vm_id: &str, intent_ref: &str) -> Self {
        Self::from_anchored_inventory(None, vm_id, intent_ref)
    }

    pub const fn requires_migration(&self) -> bool {
        self.state_id.is_some()
    }

    pub fn validates_binding(&self, vm_id: &str, intent_ref: &str) -> bool {
        self.vm_binding == canonical_digest("d2b:tpm-vm-binding/v1", vm_id.as_bytes())
            && self.intent_binding
                == canonical_digest("d2b:tpm-intent-binding/v1", intent_ref.as_bytes())
    }

    #[allow(dead_code)]
    pub(crate) fn from_anchored_inventory(
        state_id: Option<LegacyTpmStateId>,
        vm_id: &str,
        intent_ref: &str,
    ) -> Self {
        Self {
            state_id,
            vm_binding: canonical_digest("d2b:tpm-vm-binding/v1", vm_id.as_bytes()),
            intent_binding: canonical_digest("d2b:tpm-intent-binding/v1", intent_ref.as_bytes()),
        }
    }
}

impl fmt::Debug for LegacyTpmMigrationDecision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyTpmMigrationDecision(<sealed>)")
    }
}

impl fmt::Debug for LegacyTpmStateId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyTpmStateId(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_adapter_mints_only_an_opaque_receipt() {
        let receipt = LegacyTpmStateId::from_anchored_inventory("legacy-swtpm:vm:work");
        assert_eq!(format!("{receipt:?}"), "LegacyTpmStateId(<redacted>)");
    }

    #[test]
    fn migration_decision_binds_vm_and_intent() {
        let decision = LegacyTpmMigrationDecision::from_anchored_inventory(
            Some(LegacyTpmStateId::from_anchored_inventory(
                "legacy-swtpm:vm:work",
            )),
            "work",
            "legacy-swtpm:vm:work",
        );
        assert_eq!(
            format!("{decision:?}"),
            "LegacyTpmMigrationDecision(<sealed>)"
        );
        assert!(decision.validates_binding("work", "legacy-swtpm:vm:work"));
        assert!(!decision.validates_binding("other", "legacy-swtpm:vm:work"));
        assert!(!decision.validates_binding("work", "legacy-swtpm:vm:other"));
    }

    #[test]
    fn adoption_required_keeps_the_broker_migration_path_active() {
        let decision = LegacyTpmMigrationDecision::adoption_required(
            "work",
            "legacy-swtpm:vm:work",
            "legacy-swtpm:vm:work",
        );
        assert!(decision.requires_migration());
        assert!(decision.validates_binding("work", "legacy-swtpm:vm:work"));
    }
}
