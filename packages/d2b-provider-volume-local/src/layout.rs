//! The layout engine: per-entry policy evaluation.
//!
//! Evaluation is a pure function of the declared entry, what the effect
//! adapter observed, and the Volume provisioning marker. It performs no
//! effect itself, so every policy decision is testable without a
//! filesystem, a broker, or a privileged host.

use std::collections::BTreeSet;

use serde::Serialize;

use d2b_contracts_resource::v3::ResourceUid;
use d2b_contracts_resource::v3::volume::{
    CleanupPolicy, CreatePolicy, EntryAdoptionPolicy, EntryRestartPolicy, EntryType,
    ForeignChildPolicy, Invariant, LayoutEntry, LeaseClass, RepairPolicy,
};

use crate::acl::AclBinding;
use crate::error::VolumeLocalError;
use crate::identity::{EntryDigest, MarkerState, OwnerProof};
use crate::port::{DriftClass, ObservedEntry};

/// One declared layout entry, resolved for the effect port.
///
/// It borrows nothing from the resolved host tree: the anchored relative
/// path it carries is authored spec data, and the effect adapter alone
/// joins it to the private root descriptor.
#[derive(Clone, PartialEq, Eq)]
pub struct EntryRequest {
    digest: EntryDigest,
    declared: LayoutEntry,
    create_policy: CreatePolicy,
    repair_policy: RepairPolicy,
    cleanup_policy: CleanupPolicy,
    adoption_policy: EntryAdoptionPolicy,
    restart_policy: EntryRestartPolicy,
    lease_class: LeaseClass,
    foreign_child_policy: ForeignChildPolicy,
    no_follow: bool,
    recursive: bool,
    has_acl: bool,
    invariants: BTreeSet<Invariant>,
}

impl EntryRequest {
    /// Resolve one declared layout entry of one Volume.
    ///
    /// The lifecycle policy fields are read back from the entry's own
    /// canonical JSON because the contract type exposes no accessor for
    /// them; the canonical rendering is the frozen contract, so this is a
    /// read of the same authority rather than a second vocabulary. The ACL
    /// projection decodes from the same rendering, so a declaration whose
    /// grants exceed the mode's group class fails resolution here.
    pub fn resolve(
        volume_uid: &ResourceUid,
        declared: &LayoutEntry,
    ) -> Result<Self, VolumeLocalError> {
        let rendered = serde_json::to_value(declared).map_err(|_| VolumeLocalError::InvalidSpec)?;
        let decode = |name: &str| -> Result<serde_json::Value, VolumeLocalError> {
            rendered
                .get(name)
                .cloned()
                .ok_or(VolumeLocalError::InvalidSpec)
        };

        let create_policy: CreatePolicy = serde_json::from_value(decode("createPolicy")?)
            .map_err(|_| VolumeLocalError::InvalidSpec)?;
        let repair_policy: RepairPolicy = serde_json::from_value(decode("repairPolicy")?)
            .map_err(|_| VolumeLocalError::InvalidSpec)?;
        let cleanup_policy: CleanupPolicy = serde_json::from_value(decode("cleanupPolicy")?)
            .map_err(|_| VolumeLocalError::InvalidSpec)?;
        let adoption_policy: EntryAdoptionPolicy =
            serde_json::from_value(decode("adoptionPolicy")?)
                .map_err(|_| VolumeLocalError::InvalidSpec)?;
        let restart_policy: EntryRestartPolicy = serde_json::from_value(decode("restartPolicy")?)
            .map_err(|_| VolumeLocalError::InvalidSpec)?;
        let lease_class: LeaseClass = serde_json::from_value(decode("leaseClass")?)
            .map_err(|_| VolumeLocalError::InvalidSpec)?;
        let foreign_child_policy: ForeignChildPolicy =
            serde_json::from_value(decode("foreignChildPolicy")?)
                .map_err(|_| VolumeLocalError::InvalidSpec)?;
        let invariants: Vec<Invariant> = serde_json::from_value(decode("invariants")?)
            .map_err(|_| VolumeLocalError::InvalidSpec)?;
        let no_follow = decode("noFollow")?
            .as_bool()
            .ok_or(VolumeLocalError::InvalidSpec)?;
        let recursive = decode("recursive")?
            .as_bool()
            .ok_or(VolumeLocalError::InvalidSpec)?;
        // Decode the ACL projection with the same canonical rendering, so a
        // declaration whose grants exceed the mode's group class fails
        // resolution instead of reaching an effect that would widen the mode.
        let acl = AclBinding::from_rendered(&rendered)?;
        let has_acl = acl.has_access() || acl.has_default();

        Ok(Self {
            digest: EntryDigest::derive(volume_uid, declared.path()),
            declared: declared.clone(),
            create_policy,
            repair_policy,
            cleanup_policy,
            adoption_policy,
            restart_policy,
            lease_class,
            foreign_child_policy,
            no_follow,
            recursive,
            has_acl,
            invariants: invariants.into_iter().collect(),
        })
    }

    /// Borrow the opaque public identity of this entry.
    pub const fn digest(&self) -> EntryDigest {
        self.digest
    }

    /// Borrow the declared entry exactly as the Volume spec froze it.
    pub const fn declared(&self) -> &LayoutEntry {
        &self.declared
    }

    /// Return the declared entry class.
    pub const fn entry_type(&self) -> EntryType {
        self.declared.entry_type()
    }

    /// Return when this entry is created.
    pub const fn create_policy(&self) -> CreatePolicy {
        self.create_policy
    }

    /// Return how drift is reconciled.
    pub const fn repair_policy(&self) -> RepairPolicy {
        self.repair_policy
    }

    /// Return when this entry is removed.
    pub const fn cleanup_policy(&self) -> CleanupPolicy {
        self.cleanup_policy
    }

    /// Return how an existing entry is treated on first bind.
    pub const fn adoption_policy(&self) -> EntryAdoptionPolicy {
        self.adoption_policy
    }

    /// Return the behavior across a controller restart.
    pub const fn restart_policy(&self) -> EntryRestartPolicy {
        self.restart_policy
    }

    /// Return the live-ownership lease class.
    pub const fn lease_class(&self) -> LeaseClass {
        self.lease_class
    }

    /// Return how unlisted child ACL entries are treated.
    pub const fn foreign_child_policy(&self) -> ForeignChildPolicy {
        self.foreign_child_policy
    }

    /// Whether symlink traversal is rejected for this entry.
    pub const fn no_follow(&self) -> bool {
        self.no_follow
    }

    /// Whether repair recurses into children.
    pub const fn recursive(&self) -> bool {
        self.recursive
    }

    /// Whether the entry declares any ACL grant.
    pub const fn has_acl(&self) -> bool {
        self.has_acl
    }

    /// Borrow the declared fail-closed invariants.
    pub const fn invariants(&self) -> &BTreeSet<Invariant> {
        &self.invariants
    }
}

impl core::fmt::Debug for EntryRequest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("EntryRequest(<redacted>)")
    }
}

/// How serious one entry observation is for the Volume phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConditionSeverity {
    /// Recoverable: the Volume is Degraded and reconcile continues.
    Degraded,
    /// Fail-closed: the Volume is Failed and no further mutation of this
    /// entry is attempted.
    Failed,
}

/// One condition about one layout entry.
///
/// The entry is named only by its digest and the reason is one member of
/// the closed error set, so no path, ACL value, or principal name can
/// reach public status through a condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryCondition {
    /// The opaque entry identity.
    pub entry: EntryDigest,
    /// The stable condition code.
    #[serde(serialize_with = "serialize_reason")]
    pub reason: VolumeLocalError,
    /// How the condition affects the Volume phase.
    pub severity: ConditionSeverity,
}

fn serialize_reason<S: serde::Serializer>(
    reason: &VolumeLocalError,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(reason.code())
}

/// The effects one reconcile pass will request for one entry.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EntryPlan {
    /// Remove the entry before recreating it.
    pub recreate: bool,
    /// Create the entry.
    pub provision: bool,
    /// Reconcile exactly these drift classes.
    pub repair: BTreeSet<DriftClass>,
    /// Re-apply the declared ACLs.
    pub apply_acl: bool,
    /// The condition this observation raises, if any.
    pub condition: Option<EntryCondition>,
}

impl EntryPlan {
    /// Whether this plan requests no effect at all.
    pub fn is_noop(&self) -> bool {
        !self.recreate && !self.provision && self.repair.is_empty() && !self.apply_acl
    }
}

/// Evaluate one declared entry against one observation.
///
/// Ambiguity never deletes, never reuses, and never silently
/// re-provisions: it quarantines and reports.
pub fn plan_entry(
    entry: &EntryRequest,
    observed: &ObservedEntry,
    marker: MarkerState,
) -> EntryPlan {
    let failed = |reason: VolumeLocalError| EntryPlan {
        condition: Some(EntryCondition {
            entry: entry.digest(),
            reason,
            severity: ConditionSeverity::Failed,
        }),
        ..EntryPlan::default()
    };
    let degraded = |reason: VolumeLocalError, plan: EntryPlan| EntryPlan {
        condition: Some(EntryCondition {
            entry: entry.digest(),
            reason,
            severity: ConditionSeverity::Degraded,
        }),
        ..plan
    };

    if observed.symlink_encountered && entry.no_follow() {
        return failed(VolumeLocalError::SymlinkTraversalRejected);
    }
    if observed.drift.contains(&DriftClass::SameFilesystem)
        || observed.drift.contains(&DriftClass::EntryType)
    {
        return failed(VolumeLocalError::InvariantViolated);
    }

    if !observed.present {
        return match entry.create_policy() {
            CreatePolicy::ObserveOnly => {
                degraded(VolumeLocalError::EntryMissing, EntryPlan::default())
            }
            CreatePolicy::CreateIfNeverProvisioned if marker == MarkerState::Provisioned => {
                failed(VolumeLocalError::PreviouslyProvisionedStateMissing)
            }
            _ => EntryPlan {
                provision: true,
                apply_acl: entry.has_acl(),
                ..EntryPlan::default()
            },
        };
    }

    if entry.create_policy() == CreatePolicy::AlwaysRecreate {
        if observed.owner_proof == OwnerProof::Unknown
            || (entry.lease_class() != LeaseClass::None && observed.owner_proof != OwnerProof::Dead)
        {
            return degraded(VolumeLocalError::EntryQuarantined, EntryPlan::default());
        }
        return EntryPlan {
            recreate: true,
            provision: true,
            apply_acl: entry.has_acl(),
            ..EntryPlan::default()
        };
    }

    let adoptable = match entry.adoption_policy() {
        EntryAdoptionPolicy::NeverAdopt | EntryAdoptionPolicy::NotAdoptable => false,
        EntryAdoptionPolicy::AdoptWithLiveOwnerProof
        | EntryAdoptionPolicy::QuarantineOnAmbiguity => match entry.lease_class() {
            LeaseClass::None => observed.owner_proof != OwnerProof::Unknown,
            _ => observed.owner_proof == OwnerProof::Live,
        },
        EntryAdoptionPolicy::RecreateFromPersistent => observed.owner_proof != OwnerProof::Unknown,
        EntryAdoptionPolicy::DeleteIfOwnerDead => {
            if observed.owner_proof == OwnerProof::Dead {
                return EntryPlan {
                    recreate: true,
                    provision: true,
                    apply_acl: entry.has_acl(),
                    ..EntryPlan::default()
                };
            }
            observed.owner_proof != OwnerProof::Unknown
        }
    };
    if !adoptable {
        return degraded(VolumeLocalError::EntryQuarantined, EntryPlan::default());
    }
    if entry.adoption_policy() == EntryAdoptionPolicy::RecreateFromPersistent {
        return EntryPlan {
            recreate: true,
            provision: true,
            apply_acl: entry.has_acl(),
            ..EntryPlan::default()
        };
    }

    let repair: BTreeSet<DriftClass> = match entry.repair_policy() {
        RepairPolicy::None | RepairPolicy::NixActivation | RepairPolicy::OperatorOnly => {
            BTreeSet::new()
        }
        RepairPolicy::ExactOwner => [DriftClass::Owner, DriftClass::Mode].into_iter().collect(),
        RepairPolicy::FailClosed => {
            if !observed.drift.is_empty() {
                return failed(VolumeLocalError::InvariantViolated);
            }
            BTreeSet::new()
        }
        RepairPolicy::ExactMode => [DriftClass::Mode].into_iter().collect(),
        // The ACL half of this policy is delivered by the separate `apply_acl`
        // pass, which runs for every entry that declares grants; the layout
        // effect's repair pass cannot recover ACLs.
        RepairPolicy::ExactOwnerAndAcl => {
            [DriftClass::Owner, DriftClass::Mode].into_iter().collect()
        }
    };
    let repair: BTreeSet<DriftClass> = repair.intersection(&observed.drift).copied().collect();
    // ACL drift is never *unrepaired* drift: the `apply_acl` pass re-applies
    // the declared grants on every plan, so counting the class here would
    // report `EntryDrift` on every cycle of a healthy ACL-declaring entry.
    let acl_covered = entry.has_acl();
    let unrepaired: BTreeSet<DriftClass> = observed
        .drift
        .difference(&repair)
        .copied()
        .filter(|class| !(acl_covered && *class == DriftClass::Acl))
        .collect();

    let plan = EntryPlan {
        repair,
        apply_acl: entry.has_acl(),
        ..EntryPlan::default()
    };

    if observed.foreign_children && entry.foreign_child_policy() == ForeignChildPolicy::Fail {
        return degraded(VolumeLocalError::ForeignAclViolation, plan);
    }
    if !unrepaired.is_empty() {
        return degraded(VolumeLocalError::EntryDrift, plan);
    }
    plan
}

/// Whether one entry is removed during Volume cleanup.
///
/// `never` always preserves. A process-scoped entry is removed only with
/// proof that its owner is gone; ambiguity preserves.
pub fn plan_cleanup(entry: &EntryRequest, observed: &ObservedEntry) -> bool {
    match entry.cleanup_policy() {
        CleanupPolicy::Never => false,
        CleanupPolicy::Boot
        | CleanupPolicy::VmStopWithProof
        | CleanupPolicy::External
        | CleanupPolicy::OwnerControlled => observed.present,
        CleanupPolicy::ProcessExitWithProof | CleanupPolicy::ProcessExit => {
            observed.present && observed.owner_proof == OwnerProof::Dead
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume_uid() -> ResourceUid {
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("volume UID")
    }

    /// Regression (security review): the daemon's entry decode refuses a
    /// declaration whose grants exceed the declared mode's group class, so no
    /// effect is ever planned for a declaration that would widen the mode.
    #[test]
    fn resolve_refuses_grants_wider_than_the_group_class() {
        let widened: LayoutEntry = serde_json::from_value(serde_json::json!({
            "path": "",
            "type": "directory",
            "ownerRef": "User/d2bd",
            "groupRef": "User/d2bd",
            "mode": "0700",
            "accessAcl": [{ "principal": { "ref": "User/alice" }, "permissions": "rwx" }],
            "defaultAcl": [],
            "foreignChildPolicy": "preserve"
        }))
        .expect("valid entry");
        assert_eq!(
            EntryRequest::resolve(&volume_uid(), &widened),
            Err(VolumeLocalError::InvalidSpec)
        );
    }
}
