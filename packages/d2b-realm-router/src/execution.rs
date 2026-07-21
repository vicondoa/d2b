//! Durable execution routing state (ADR 0032).
//!
//! This pure table is the gateway/node-side metadata owner for execution
//! start/join/logs/cancel/reconnect decisions. It deliberately stores only
//! bounded metadata; argv, environment, cwd, stdio, and log bytes remain in
//! operation/stream payloads owned by the provider or guest-control adapter.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use d2b_realm_core::{
    ConstellationError, ErrorKind, ExecAttachRequest, ExecCancelRequest, ExecLogsRequest,
    ExecStartRequest, ExecState, ExecutionId, ExecutionSummary, StreamCursor,
};

/// Default retained execution metadata cap per router scope.
pub const DEFAULT_MAX_EXECUTIONS: usize = 16_384;

/// Pure durable execution metadata table.
#[derive(Debug, Clone)]
pub struct DurableExecTable {
    executions: BTreeMap<ExecutionId, ExecutionSummary>,
    terminal_ids: BTreeSet<ExecutionId>,
    terminal_order: VecDeque<ExecutionId>,
    max_executions: usize,
}

impl Default for DurableExecTable {
    fn default() -> Self {
        Self::new()
    }
}

impl DurableExecTable {
    /// Construct a table with default memory bounds.
    pub fn new() -> Self {
        Self::with_max_executions(DEFAULT_MAX_EXECUTIONS)
    }

    /// Construct a table with an explicit retained execution cap.
    pub fn with_max_executions(max_executions: usize) -> Self {
        Self {
            executions: BTreeMap::new(),
            terminal_ids: BTreeSet::new(),
            terminal_order: VecDeque::new(),
            max_executions: max_executions.max(1),
        }
    }

    /// Start or rediscover a durable execution. A same-id, same-generation
    /// retry returns the existing summary (lost-reply recovery); a same id with
    /// different metadata fails closed as an idempotency conflict.
    pub fn start(&mut self, req: ExecStartRequest) -> Result<ExecutionSummary, ConstellationError> {
        if let Some(existing) = self.executions.get(&req.execution_id) {
            if existing.workload == req.workload
                && existing.generation == req.generation
                && existing.attach_mode == req.attach_mode
                && existing.tty == req.tty
            {
                return Ok(existing.clone());
            }
            return Err(ConstellationError::new(
                ErrorKind::IdempotencyKeyConflict,
                "execution id already exists with different metadata",
            ));
        }
        while self.executions.len() >= self.max_executions {
            if !self.evict_oldest_terminal() {
                break;
            }
        }
        if self.executions.len() >= self.max_executions {
            return Err(ConstellationError::new(
                ErrorKind::Backpressure,
                "durable execution table is full",
            ));
        }
        let summary = ExecutionSummary {
            id: req.execution_id,
            workload: req.workload,
            state: ExecState::Pending,
            exit_code: None,
            tty: req.tty,
            generation: req.generation,
            attach_mode: req.attach_mode,
            stdout_cursor: None,
            stderr_cursor: None,
        };
        self.executions.insert(summary.id.clone(), summary.clone());
        Ok(summary)
    }

    /// Attach/reconnect to an execution after validating generation/boot
    /// identity. Stale generations fail closed before any stream is opened.
    pub fn attach(&self, req: &ExecAttachRequest) -> Result<ExecutionSummary, ConstellationError> {
        let summary = self.get(&req.execution_id)?;
        ensure_generation(summary, &req.generation)?;
        Ok(summary.clone())
    }

    /// Validate and serve retained-log metadata. The request's `max_bytes`
    /// bound is enforced by the DTO decoder; this method only validates that
    /// the execution exists and returns its current metadata.
    pub fn logs(&self, req: &ExecLogsRequest) -> Result<ExecutionSummary, ConstellationError> {
        let summary = self.get(&req.execution_id)?;
        ensure_generation(summary, &req.generation)?;
        Ok(summary.clone())
    }

    /// Idempotently cancel an execution. Returns `true` for the first
    /// transition to cancelled and `false` for terminal/repeated/unknown
    /// cancels, so retries are safe across reconnects.
    pub fn cancel(&mut self, req: &ExecCancelRequest) -> Result<bool, ConstellationError> {
        let Some(summary) = self.executions.get_mut(&req.execution_id) else {
            return Ok(false);
        };
        ensure_generation(summary, &req.generation)?;
        if summary.state.is_terminal() {
            return Ok(false);
        }
        summary.state = ExecState::Cancelled;
        self.remember_terminal(req.execution_id.clone());
        Ok(true)
    }

    /// Mark an execution running after the provider/guest accepted it.
    pub fn mark_running(&mut self, id: &ExecutionId) -> Result<(), ConstellationError> {
        let summary = self.get_mut(id)?;
        if summary.state.is_terminal() {
            return Err(ConstellationError::new(
                ErrorKind::Cancelled,
                "terminal execution cannot be marked running",
            ));
        }
        summary.state = ExecState::Running;
        Ok(())
    }

    /// Mark an execution exited and retain its latest cursors.
    pub fn mark_exited(
        &mut self,
        id: &ExecutionId,
        exit_code: i32,
        stdout_cursor: Option<StreamCursor>,
        stderr_cursor: Option<StreamCursor>,
    ) -> Result<(), ConstellationError> {
        let summary = self.get_mut(id)?;
        if summary.state.is_terminal() {
            return Err(ConstellationError::new(
                ErrorKind::Cancelled,
                "terminal execution cannot be marked exited",
            ));
        }
        summary.state = ExecState::Exited;
        summary.exit_code = Some(exit_code);
        if let Some(cursor) = stdout_cursor {
            summary.stdout_cursor = Some(cursor);
        }
        if let Some(cursor) = stderr_cursor {
            summary.stderr_cursor = Some(cursor);
        }
        self.remember_terminal(id.clone());
        Ok(())
    }

    /// Mark an execution failed before an exit code was available.
    pub fn mark_failed(&mut self, id: &ExecutionId) -> Result<(), ConstellationError> {
        let summary = self.get_mut(id)?;
        if summary.state.is_terminal() {
            return Err(ConstellationError::new(
                ErrorKind::Cancelled,
                "terminal execution cannot be marked failed",
            ));
        }
        summary.state = ExecState::Failed;
        self.remember_terminal(id.clone());
        Ok(())
    }

    /// Record retained log cursors while an execution is still running.
    pub fn update_cursors(
        &mut self,
        id: &ExecutionId,
        stdout_cursor: Option<StreamCursor>,
        stderr_cursor: Option<StreamCursor>,
    ) -> Result<(), ConstellationError> {
        let summary = self.get_mut(id)?;
        if let Some(cursor) = stdout_cursor {
            summary.stdout_cursor = Some(cursor);
        }
        if let Some(cursor) = stderr_cursor {
            summary.stderr_cursor = Some(cursor);
        }
        Ok(())
    }

    /// Return a summary by id.
    pub fn summary(&self, id: &ExecutionId) -> Option<&ExecutionSummary> {
        self.executions.get(id)
    }

    /// Remove retained execution metadata. Returns whether a record existed.
    pub fn remove(&mut self, id: &ExecutionId) -> bool {
        self.terminal_ids.remove(id);
        self.terminal_order.retain(|entry| entry != id);
        self.executions.remove(id).is_some()
    }

    /// Number of retained execution summaries.
    pub fn len(&self) -> usize {
        self.executions.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.executions.is_empty()
    }

    fn get(&self, id: &ExecutionId) -> Result<&ExecutionSummary, ConstellationError> {
        self.executions.get(id).ok_or_else(|| {
            ConstellationError::new(ErrorKind::InvalidTarget, "unknown durable execution id")
        })
    }

    fn get_mut(&mut self, id: &ExecutionId) -> Result<&mut ExecutionSummary, ConstellationError> {
        self.executions.get_mut(id).ok_or_else(|| {
            ConstellationError::new(ErrorKind::InvalidTarget, "unknown durable execution id")
        })
    }

    fn remember_terminal(&mut self, id: ExecutionId) {
        if self.terminal_ids.insert(id.clone()) {
            self.terminal_order.push_back(id);
        }
    }

    fn evict_oldest_terminal(&mut self) -> bool {
        while let Some(id) = self.terminal_order.pop_front() {
            if self.terminal_ids.remove(&id) && self.executions.remove(&id).is_some() {
                return true;
            }
        }
        false
    }
}

fn ensure_generation(
    summary: &ExecutionSummary,
    generation: &d2b_realm_core::ExecutionGeneration,
) -> Result<(), ConstellationError> {
    if summary.generation_matches(generation) {
        Ok(())
    } else {
        Err(ConstellationError::new(
            ErrorKind::InvalidTarget,
            "execution generation does not match current boot",
        ))
    }
}

/// Stable, short observability/audit vocabulary for an [`ExecState`].
///
/// This is a router-side naming contract only: a fixed, lowercase, ASCII
/// code per state, safe to place in metric labels, audit records, or CLI
/// status output without leaking argv/env/paths. The guest-runner side
/// (`d2b-exec-runner`) mirrors the identical string set for its own
/// terminal-outcome vocabulary (see
/// `d2b_exec_runner::service_mode::ExecutionOutcomeCode`) so that a router
/// execution state and a guest runner outcome can be correlated by an
/// external observer without either crate depending on the other. Keep the
/// two vocabularies in lockstep if either changes.
pub fn state_code(state: ExecState) -> &'static str {
    match state {
        ExecState::Pending => "pending",
        ExecState::Running => "running",
        ExecState::Exited => "exited",
        ExecState::Cancelled => "cancelled",
        ExecState::Failed => "failed",
    }
}

// `work_executor` composes this module's `DurableExecTable` (plus
// `target_resolver`/`session_lifecycle`/`remote_node`, all already declared
// in `lib.rs`) into the crate's single typed dispatch surface (ADR 0032).
// `lib.rs` is integrator-owned, so this crate cannot add the production
#[cfg(test)]
mod tests {
    use super::*;
    use d2b_realm_core::{ExecAttachMode, ExecutionGeneration, ProtocolToken, WorkloadId};
    use std::num::NonZeroU32;

    fn generation(n: &str) -> ExecutionGeneration {
        ExecutionGeneration {
            guest_boot_id: ProtocolToken::parse(format!("boot-{n}")).unwrap(),
            workload_generation: ProtocolToken::parse(format!("gen-{n}")).unwrap(),
        }
    }

    fn start(id: &str) -> ExecStartRequest {
        ExecStartRequest {
            execution_id: ExecutionId::parse(id).unwrap(),
            workload: WorkloadId::parse("workload-a").unwrap(),
            generation: generation("1"),
            attach_mode: ExecAttachMode::Detached,
            tty: false,
        }
    }

    #[test]
    fn start_retries_return_existing_summary_but_conflicts_fail_closed() {
        let mut table = DurableExecTable::new();
        let first = table.start(start("exec-1")).unwrap();
        let replay = table.start(start("exec-1")).unwrap();
        assert_eq!(first, replay);

        let mut conflicting = start("exec-1");
        conflicting.tty = true;
        assert_eq!(
            table.start(conflicting).unwrap_err().kind(),
            ErrorKind::IdempotencyKeyConflict
        );
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn attach_rejects_stale_generation() {
        let mut table = DurableExecTable::new();
        table.start(start("exec-1")).unwrap();
        let ok = ExecAttachRequest {
            execution_id: ExecutionId::parse("exec-1").unwrap(),
            generation: generation("1"),
            stdout_cursor: None,
            stderr_cursor: None,
        };
        table.attach(&ok).unwrap();
        let stale = ExecAttachRequest {
            generation: generation("2"),
            ..ok
        };
        assert_eq!(
            table.attach(&stale).unwrap_err().kind(),
            ErrorKind::InvalidTarget
        );
    }

    #[test]
    fn logs_validate_retained_cursor_and_bound() {
        let mut table = DurableExecTable::new();
        let id = ExecutionId::parse("exec-1").unwrap();
        table.start(start("exec-1")).unwrap();
        table
            .update_cursors(&id, Some(StreamCursor::parse("cur-out").unwrap()), None)
            .unwrap();
        let ok = ExecLogsRequest {
            execution_id: id.clone(),
            generation: generation("1"),
            cursor: Some(StreamCursor::parse("cur-out").unwrap()),
            max_bytes: NonZeroU32::new(4096).unwrap(),
        };
        table.logs(&ok).unwrap();
        let older_cursor = ExecLogsRequest {
            cursor: Some(StreamCursor::parse("older-client-cursor").unwrap()),
            ..ok
        };
        table
            .logs(&older_cursor)
            .expect("cursor range validation is provider-owned");
    }

    #[test]
    fn logs_and_cancel_reject_stale_generation() {
        let mut table = DurableExecTable::new();
        let id = ExecutionId::parse("exec-1").unwrap();
        table.start(start("exec-1")).unwrap();
        let logs = ExecLogsRequest {
            execution_id: id.clone(),
            generation: generation("2"),
            cursor: None,
            max_bytes: NonZeroU32::new(1024).unwrap(),
        };
        assert_eq!(
            table.logs(&logs).unwrap_err().kind(),
            ErrorKind::InvalidTarget
        );
        let cancel = ExecCancelRequest {
            execution_id: id,
            generation: generation("2"),
        };
        assert_eq!(
            table.cancel(&cancel).unwrap_err().kind(),
            ErrorKind::InvalidTarget
        );
    }

    #[test]
    fn cancel_is_idempotent_and_terminal_safe() {
        let mut table = DurableExecTable::new();
        let id = ExecutionId::parse("exec-1").unwrap();
        table.start(start("exec-1")).unwrap();
        assert_eq!(
            table.cancel(&ExecCancelRequest {
                execution_id: id.clone(),
                generation: generation("1"),
            }),
            Ok(true)
        );
        assert_eq!(table.summary(&id).unwrap().state, ExecState::Cancelled);
        assert_eq!(
            table.cancel(&ExecCancelRequest {
                execution_id: id.clone(),
                generation: generation("1"),
            }),
            Ok(false)
        );
        assert_eq!(
            table.cancel(&ExecCancelRequest {
                execution_id: ExecutionId::parse("unknown").unwrap(),
                generation: generation("1"),
            }),
            Ok(false)
        );
    }

    #[test]
    fn table_capacity_fails_closed_before_accepting_start() {
        let mut table = DurableExecTable::with_max_executions(1);
        table.start(start("exec-1")).unwrap();
        assert_eq!(
            table.start(start("exec-2")).unwrap_err().kind(),
            ErrorKind::Backpressure
        );
    }

    #[test]
    fn terminal_executions_can_be_removed_or_evicted_for_capacity() {
        let mut table = DurableExecTable::with_max_executions(1);
        let id1 = ExecutionId::parse("exec-1").unwrap();
        table.start(start("exec-1")).unwrap();
        table.mark_running(&id1).unwrap();
        table
            .mark_exited(&id1, 0, Some(StreamCursor::parse("cur-out").unwrap()), None)
            .unwrap();
        let second = table.start(start("exec-2")).unwrap();
        assert_eq!(second.id.as_str(), "exec-2");
        assert!(table.summary(&id1).is_none());
        assert_eq!(table.len(), 1);
        assert!(table.remove(&ExecutionId::parse("exec-2").unwrap()));
        assert!(table.is_empty());
    }

    #[test]
    fn remove_cleans_terminal_order_before_id_reuse() {
        let mut table = DurableExecTable::with_max_executions(1);
        let reused = ExecutionId::parse("exec-1").unwrap();
        table.start(start("exec-1")).unwrap();
        table.mark_running(&reused).unwrap();
        table.mark_failed(&reused).unwrap();
        assert!(table.remove(&reused));

        table.start(start("exec-1")).unwrap();
        table.mark_running(&reused).unwrap();
        table.mark_failed(&reused).unwrap();
        let second = table.start(start("exec-2")).unwrap();
        assert_eq!(second.id.as_str(), "exec-2");
        assert!(
            table.summary(&reused).is_none(),
            "current reused terminal record should be evicted exactly once"
        );
    }

    #[test]
    fn terminal_state_is_not_overwritten_and_cursors_are_preserved() {
        let mut table = DurableExecTable::new();
        let id = ExecutionId::parse("exec-1").unwrap();
        table.start(start("exec-1")).unwrap();
        table.mark_running(&id).unwrap();
        table
            .update_cursors(&id, Some(StreamCursor::parse("cur-out").unwrap()), None)
            .unwrap();
        table.mark_exited(&id, 0, None, None).unwrap();
        assert_eq!(
            table
                .summary(&id)
                .unwrap()
                .stdout_cursor
                .as_ref()
                .unwrap()
                .as_str(),
            "cur-out"
        );
        assert_eq!(
            table.mark_exited(&id, 1, None, None).unwrap_err().kind(),
            ErrorKind::Cancelled
        );
    }

    #[test]
    fn mark_failed_records_failed_terminal_state() {
        let mut table = DurableExecTable::new();
        let id = ExecutionId::parse("exec-1").unwrap();
        table.start(start("exec-1")).unwrap();
        table.mark_failed(&id).unwrap();
        assert_eq!(table.summary(&id).unwrap().state, ExecState::Failed);
        assert_eq!(
            table.mark_running(&id).unwrap_err().kind(),
            ErrorKind::Cancelled
        );
    }

    #[test]
    fn state_code_is_stable_lowercase_ascii_and_covers_every_state() {
        let cases = [
            (ExecState::Pending, "pending"),
            (ExecState::Running, "running"),
            (ExecState::Exited, "exited"),
            (ExecState::Cancelled, "cancelled"),
            (ExecState::Failed, "failed"),
        ];
        for (state, expected) in cases {
            let code = state_code(state);
            assert_eq!(code, expected);
            assert!(code.chars().all(|c| c.is_ascii_lowercase()));
        }
    }

    #[test]
    fn state_code_values_are_pairwise_distinct() {
        let all = [
            ExecState::Pending,
            ExecState::Running,
            ExecState::Exited,
            ExecState::Cancelled,
            ExecState::Failed,
        ];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(state_code(*a), state_code(*b));
            }
        }
    }
}
