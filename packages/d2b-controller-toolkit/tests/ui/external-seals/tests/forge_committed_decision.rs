use std::future::{Ready, ready};

use d2b_contracts::v3::{ResourceGeneration, ResourceUid, ZoneId, ZoneRevision};
use d2b_controller_toolkit::{
    CommitDecision, CommitOutcome, CommittedRevisionProof, ControllerDescriptor, ControllerSource,
    FreshSnapshot, InitialList, ReconcileContext, ReconcileProjection, ReconcileResult, ResourceKey,
    SourceError, StatusPersistence, WatchEvent, WatchFailure,
};

struct ForeignSource;

impl ControllerSource for ForeignSource {
    fn register(
        &self,
        _descriptor: &ControllerDescriptor,
    ) -> Ready<Result<(), SourceError>> {
        ready(Err(SourceError::Unavailable))
    }

    fn list_initial(
        &self,
        _descriptor: &ControllerDescriptor,
    ) -> Ready<Result<InitialList, SourceError>> {
        ready(Err(SourceError::Unavailable))
    }

    fn open_watch(
        &self,
        _descriptor: &ControllerDescriptor,
        _after_revision: ZoneRevision,
    ) -> Ready<Result<(), SourceError>> {
        ready(Err(SourceError::Unavailable))
    }

    fn receive_watch(&self) -> Ready<Result<WatchEvent, WatchFailure>> {
        ready(Err(WatchFailure::Fatal))
    }

    fn read_fresh(&self, _key: &ResourceKey) -> Ready<Result<FreshSnapshot, SourceError>> {
        ready(Err(SourceError::Unavailable))
    }

    fn write_starting(
        &self,
        _context: &ReconcileContext,
    ) -> Ready<Result<(), SourceError>> {
        ready(Err(SourceError::Unavailable))
    }

    fn await_expedited_commit(
        &self,
        _context: &ReconcileContext,
    ) -> Ready<Result<CommitDecision, SourceError>> {
        let proof = CommittedRevisionProof {
            zone: ZoneId::parse("work").unwrap(),
            resource_uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            generation: ResourceGeneration::new(1).unwrap(),
            revision: ZoneRevision::new(1),
            operation_id: "operation".to_owned(),
        };
        ready(Ok(CommitDecision::Committed(proof)))
    }

    fn commit_result(
        &self,
        _context: &ReconcileContext,
        _result: &ReconcileResult,
    ) -> Ready<Result<CommitOutcome, SourceError>> {
        ready(Err(SourceError::Unavailable))
    }

    fn complete_expedited(
        &self,
        _context: &ReconcileContext,
        _projection: &ReconcileProjection,
        _status_persistence: StatusPersistence,
    ) -> Ready<Result<(), SourceError>> {
        ready(Err(SourceError::Unavailable))
    }

    fn persist_outcome(
        &self,
        _projection: &ReconcileProjection,
    ) -> Ready<Result<(), SourceError>> {
        ready(Err(SourceError::Unavailable))
    }

    fn checkpoint(
        &self,
        _context: &ReconcileContext,
        _revision: ZoneRevision,
    ) -> Ready<Result<(), SourceError>> {
        ready(Err(SourceError::Unavailable))
    }

    fn schedule_requeue(
        &self,
        _key: &ResourceKey,
        _at_tick: u64,
    ) -> Ready<Result<(), SourceError>> {
        ready(Err(SourceError::Unavailable))
    }
}

fn probe() {
    let _ = ForeignSource;
}
