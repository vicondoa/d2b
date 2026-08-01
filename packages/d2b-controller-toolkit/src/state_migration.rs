//! Cross-component Provider state migration planning.
//!
//! The planner is pure and returns resource mutations or worker intent to the
//! owning controller. It never performs filesystem effects and always chooses
//! roll-forward once any member marker proves the target version.

use std::collections::BTreeSet;

use d2b_contracts::v3::{
    MAX_BATCH_MUTATIONS, MarkerStatus, ResourceRef, ResourceUid, SchemaVersion, StateSchemaPhase,
};

/// Observed phase of one migration EphemeralProcess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationWorkerPhase {
    /// No worker has been created.
    Absent,
    /// The worker is pending or running.
    Running,
    /// The worker completed successfully.
    Succeeded,
    /// The worker failed before commit.
    Failed,
}

/// One Volume member of an N-Volume migration group.
#[derive(Clone, PartialEq, Eq)]
pub struct MigrationMember {
    volume_ref: ResourceRef,
    volume_uid: ResourceUid,
    installed: SchemaVersion,
    target: SchemaVersion,
    state_phase: StateSchemaPhase,
    marker_status: MarkerStatus,
    commit_order: u32,
    prepare_committed: bool,
    writer_ready: bool,
    staging_present: bool,
    worker_phase: MigrationWorkerPhase,
}

impl MigrationMember {
    /// Construct one bounded migration observation.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        volume_ref: ResourceRef,
        volume_uid: ResourceUid,
        installed: SchemaVersion,
        target: SchemaVersion,
        state_phase: StateSchemaPhase,
        marker_status: MarkerStatus,
        commit_order: u32,
        prepare_committed: bool,
        writer_ready: bool,
        staging_present: bool,
        worker_phase: MigrationWorkerPhase,
    ) -> Result<Self, MigrationPlanError> {
        if volume_ref.resource_type().as_str() != "Volume" {
            return Err(MigrationPlanError::InvalidMember);
        }
        Ok(Self {
            volume_ref,
            volume_uid,
            installed,
            target,
            state_phase,
            marker_status,
            commit_order,
            prepare_committed,
            writer_ready,
            staging_present,
            worker_phase,
        })
    }

    /// Borrow the source Volume reference.
    pub const fn volume_ref(&self) -> &ResourceRef {
        &self.volume_ref
    }
}

impl core::fmt::Debug for MigrationMember {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MigrationMember")
            .field("state_phase", &self.state_phase)
            .field("marker_status", &self.marker_status)
            .field("commit_order", &self.commit_order)
            .field("worker_phase", &self.worker_phase)
            .finish_non_exhaustive()
    }
}

/// Pure coordinated action selected for the complete migration group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationPlan {
    /// Every member is current and no staging remains.
    Complete,
    /// Commit `PrepareMigration` on every member in one mutation batch.
    PrepareAll { volumes: Vec<ResourceRef> },
    /// Wait for every component writer to acknowledge prepare.
    AwaitWriterReadiness,
    /// Create staging Volumes for the listed source Volumes.
    CreateStaging { volumes: Vec<ResourceRef> },
    /// Dispatch a signed worker for each listed source/staging pair.
    DispatchWorkers { volumes: Vec<ResourceRef> },
    /// Wait for every running worker to reach a terminal phase.
    AwaitWorkers,
    /// Roll back every precommit staging Volume and preserve active state.
    RollbackAll { volumes: Vec<ResourceRef> },
    /// Atomically cut over each member in stable UID order.
    CommitAll { volumes: Vec<ResourceRef> },
    /// Continue or finish an interrupted commit; rollback is forbidden.
    RollForward {
        remaining: Vec<ResourceRef>,
        cleanup_staging: Vec<ResourceRef>,
    },
}

/// Closed migration planner rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationPlanError {
    /// The group was empty, oversized, duplicated, or not made of Volumes.
    InvalidMember,
    /// A source marker did not verify.
    MarkerNotVerified,
    /// Installed state is newer than its desired target.
    DowngradeForbidden,
    /// Observations conflict in a way that cannot safely choose rollback.
    AmbiguousState,
}

impl MigrationPlanError {
    /// Return a stable redacted error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidMember => "state-migration-member-invalid",
            Self::MarkerNotVerified => "state-migration-marker-not-verified",
            Self::DowngradeForbidden => "state-migration-downgrade-forbidden",
            Self::AmbiguousState => "state-migration-state-ambiguous",
        }
    }
}

impl core::fmt::Display for MigrationPlanError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for MigrationPlanError {}

/// Plan one complete N-Volume migration observation.
pub fn plan(members: &[MigrationMember]) -> Result<MigrationPlan, MigrationPlanError> {
    if members.is_empty() || members.len() > MAX_BATCH_MUTATIONS {
        return Err(MigrationPlanError::InvalidMember);
    }
    let unique: BTreeSet<&ResourceUid> = members.iter().map(|member| &member.volume_uid).collect();
    if unique.len() != members.len() {
        return Err(MigrationPlanError::InvalidMember);
    }
    let order: BTreeSet<u32> = members.iter().map(|member| member.commit_order).collect();
    if order.len() != members.len() {
        return Err(MigrationPlanError::InvalidMember);
    }
    if members
        .iter()
        .any(|member| member.marker_status != MarkerStatus::Verified)
    {
        return Err(MigrationPlanError::MarkerNotVerified);
    }
    if members
        .iter()
        .any(|member| member.installed > member.target)
    {
        return Err(MigrationPlanError::DowngradeForbidden);
    }

    let mut ordered: Vec<&MigrationMember> = members.iter().collect();
    ordered.sort_by_key(|member| member.commit_order);
    let references = |items: &[&MigrationMember]| {
        items
            .iter()
            .map(|member| member.volume_ref.clone())
            .collect::<Vec<_>>()
    };

    let committed: Vec<&MigrationMember> = ordered
        .iter()
        .copied()
        .filter(|member| member.installed == member.target)
        .collect();
    if !committed.is_empty() {
        let remaining: Vec<&MigrationMember> = ordered
            .iter()
            .copied()
            .filter(|member| member.installed < member.target)
            .collect();
        let cleanup: Vec<&MigrationMember> = ordered
            .iter()
            .copied()
            .filter(|member| member.staging_present)
            .collect();
        if remaining.is_empty() && cleanup.is_empty() {
            return Ok(MigrationPlan::Complete);
        }
        return Ok(MigrationPlan::RollForward {
            remaining: references(&remaining),
            cleanup_staging: references(&cleanup),
        });
    }

    if members.iter().any(|member| !member.prepare_committed) {
        return Ok(MigrationPlan::PrepareAll {
            volumes: references(&ordered),
        });
    }
    if members.iter().any(|member| !member.writer_ready) {
        return Ok(MigrationPlan::AwaitWriterReadiness);
    }

    let missing_staging: Vec<&MigrationMember> = ordered
        .iter()
        .copied()
        .filter(|member| !member.staging_present)
        .collect();
    if !missing_staging.is_empty() {
        return Ok(MigrationPlan::CreateStaging {
            volumes: references(&missing_staging),
        });
    }

    if members
        .iter()
        .any(|member| member.worker_phase == MigrationWorkerPhase::Failed)
    {
        return Ok(MigrationPlan::RollbackAll {
            volumes: references(&ordered),
        });
    }
    let absent_workers: Vec<&MigrationMember> = ordered
        .iter()
        .copied()
        .filter(|member| member.worker_phase == MigrationWorkerPhase::Absent)
        .collect();
    if !absent_workers.is_empty() {
        return Ok(MigrationPlan::DispatchWorkers {
            volumes: references(&absent_workers),
        });
    }
    if members
        .iter()
        .any(|member| member.worker_phase == MigrationWorkerPhase::Running)
    {
        return Ok(MigrationPlan::AwaitWorkers);
    }
    if members.iter().all(|member| {
        member.worker_phase == MigrationWorkerPhase::Succeeded
            && matches!(
                member.state_phase,
                StateSchemaPhase::Migrating | StateSchemaPhase::MigrationCommitted
            )
    }) {
        return Ok(MigrationPlan::CommitAll {
            volumes: references(&ordered),
        });
    }
    Err(MigrationPlanError::AmbiguousState)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(uid: &str, name: &str, commit_order: u32) -> MigrationMember {
        MigrationMember::new(
            ResourceRef::parse(&format!("Volume/{name}")).unwrap(),
            ResourceUid::parse(uid).unwrap(),
            SchemaVersion::new(1, 0).unwrap(),
            SchemaVersion::new(2, 0).unwrap(),
            StateSchemaPhase::Migrating,
            MarkerStatus::Verified,
            commit_order,
            true,
            true,
            true,
            MigrationWorkerPhase::Succeeded,
        )
        .unwrap()
    }

    #[test]
    fn commit_order_is_stable_across_n_volume_input_order() {
        let later = member("6f9619ff-8b86-4d01-b42d-00cf4fc964ff", "later", 2);
        let earlier = member("123e4567-e89b-42d3-a456-426614174000", "earlier", 1);
        let plan = plan(&[later, earlier]).unwrap();
        let MigrationPlan::CommitAll { volumes } = plan else {
            panic!("expected commit plan");
        };
        assert_eq!(volumes[0], ResourceRef::parse("Volume/earlier").unwrap());
        assert_eq!(volumes[1], ResourceRef::parse("Volume/later").unwrap());
    }

    #[test]
    fn partial_marker_commit_is_always_roll_forward() {
        let mut committed = member("123e4567-e89b-42d3-a456-426614174000", "committed", 1);
        committed.installed = committed.target;
        let pending = member("6f9619ff-8b86-4d01-b42d-00cf4fc964ff", "pending", 2);
        let result = plan(&[pending, committed]).unwrap();
        assert!(matches!(result, MigrationPlan::RollForward { .. }));
    }

    #[test]
    fn missing_marker_fails_before_any_prepare_or_worker_plan() {
        let mut volume = member("123e4567-e89b-42d3-a456-426614174000", "state", 1);
        volume.marker_status = MarkerStatus::Missing;
        assert_eq!(plan(&[volume]), Err(MigrationPlanError::MarkerNotVerified));
    }

    #[test]
    fn n_volume_group_moves_together_through_prepare_staging_dispatch_and_rollback() {
        let mut first = member("123e4567-e89b-42d3-a456-426614174000", "first", 1);
        let mut second = member("6f9619ff-8b86-4d01-b42d-00cf4fc964ff", "second", 2);
        for member in [&mut first, &mut second] {
            member.prepare_committed = false;
            member.writer_ready = false;
            member.staging_present = false;
            member.worker_phase = MigrationWorkerPhase::Absent;
        }

        assert!(matches!(
            plan(&[first.clone(), second.clone()]).unwrap(),
            MigrationPlan::PrepareAll { volumes } if volumes.len() == 2
        ));
        first.prepare_committed = true;
        second.prepare_committed = true;
        assert_eq!(
            plan(&[first.clone(), second.clone()]).unwrap(),
            MigrationPlan::AwaitWriterReadiness
        );
        first.writer_ready = true;
        second.writer_ready = true;
        assert!(matches!(
            plan(&[first.clone(), second.clone()]).unwrap(),
            MigrationPlan::CreateStaging { volumes } if volumes.len() == 2
        ));
        first.staging_present = true;
        second.staging_present = true;
        assert!(matches!(
            plan(&[first.clone(), second.clone()]).unwrap(),
            MigrationPlan::DispatchWorkers { volumes } if volumes.len() == 2
        ));
        first.worker_phase = MigrationWorkerPhase::Succeeded;
        second.worker_phase = MigrationWorkerPhase::Failed;
        assert!(matches!(
            plan(&[first, second]).unwrap(),
            MigrationPlan::RollbackAll { volumes } if volumes.len() == 2
        ));
    }
}
