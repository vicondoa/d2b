//! `shell-terminal.d2bus.org.ShellSession` schema.

use super::{ShellPool, ShellTerminalError, validate_name};
use crate::resources::ExecutionTarget;
use d2b_contracts_resource::v3::ResourceRef;

/// Common resource phases used by shell pools and sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    /// The resource is awaiting reconcile.
    Pending,
    /// The session is verified and available.
    Ready,
    /// The login shell exited successfully.
    Succeeded,
    /// Identity-safe operation cannot be guaranteed.
    Degraded,
    /// The login shell or supervisor failed.
    Failed,
    /// Finalization removed the resource.
    Deleted,
    /// The state cannot be classified safely.
    Unknown,
}

/// A qualified `ShellSession` with pool-inherited placement fields.
#[derive(Clone, PartialEq, Eq)]
pub struct ShellSession {
    name: String,
    zone: String,
    pool_name: String,
    execution_target: ExecutionTarget,
    workload_user: String,
    login_shell_ref: String,
    session_name: String,
    output_ring_capacity: u64,
    phase: SessionPhase,
    supervisor_process_ref: ResourceRef,
    supervisor_execution_ref: ResourceRef,
    supervisor_user_ref: ResourceRef,
}

impl std::fmt::Debug for ShellSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ShellSession")
            .field("output_ring_capacity", &self.output_ring_capacity)
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

impl ShellSession {
    /// Create a session by freezing placement and shell fields from its pool.
    pub fn from_pool(
        pool: &ShellPool,
        name: impl Into<String>,
        session_name: impl Into<String>,
        output_ring_capacity: Option<u64>,
    ) -> Result<Self, ShellTerminalError> {
        let session_name = session_name.into();
        let name = name.into();
        Self::from_pool_with_resource_name(pool, name, session_name, output_ring_capacity)
    }

    /// Create a session while choosing its stable resource name separately
    /// from its operator-facing session name.
    pub fn from_pool_with_resource_name(
        pool: &ShellPool,
        name: impl Into<String>,
        session_name: impl Into<String>,
        output_ring_capacity: Option<u64>,
    ) -> Result<Self, ShellTerminalError> {
        let name = name.into();
        let session_name = session_name.into();
        validate_name(&name, 63)?;
        validate_name(&session_name, 32)?;
        let output_ring_capacity =
            output_ring_capacity.unwrap_or(pool.spec().output_ring_capacity());
        if !(4 * 1024..=pool.spec().output_ring_capacity()).contains(&output_ring_capacity) {
            return Err(ShellTerminalError::CapacityOutOfRange);
        }
        let target_type = if pool.execution_target().is_host() {
            "Host"
        } else {
            "Guest"
        };
        let supervisor_execution_ref =
            ResourceRef::parse(&format!("{target_type}/{}", pool.execution_target().name()))
                .map_err(|_| ShellTerminalError::InvalidName)?;
        let supervisor_process_ref = ResourceRef::parse(&format!("Process/{name}"))
            .map_err(|_| ShellTerminalError::InvalidName)?;
        let supervisor_user_ref = ResourceRef::parse(&format!("User/{}", pool.workload_user()))
            .map_err(|_| ShellTerminalError::InvalidName)?;
        Ok(Self {
            name,
            zone: pool.zone().to_owned(),
            pool_name: pool.name().to_owned(),
            execution_target: pool.execution_target().clone(),
            workload_user: pool.workload_user().to_owned(),
            login_shell_ref: pool.spec().login_shell_ref().to_owned(),
            session_name,
            output_ring_capacity,
            phase: SessionPhase::Pending,
            supervisor_process_ref,
            supervisor_execution_ref,
            supervisor_user_ref,
        })
    }

    /// Return the canonical qualified resource type.
    pub const fn resource_type(&self) -> &'static str {
        "shell-terminal.d2bus.org.ShellSession"
    }

    /// Borrow the stable resource name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Borrow the owning Zone.
    pub fn zone(&self) -> &str {
        &self.zone
    }

    /// Borrow the parent pool name.
    pub fn pool_name(&self) -> &str {
        &self.pool_name
    }

    /// Borrow the frozen execution target.
    pub const fn execution_target(&self) -> &ExecutionTarget {
        &self.execution_target
    }

    /// Borrow the frozen workload user.
    pub fn workload_user(&self) -> &str {
        &self.workload_user
    }

    /// Borrow the frozen manifest-fixed login shell artifact reference.
    pub fn login_shell_ref(&self) -> &str {
        &self.login_shell_ref
    }

    /// Borrow the operator-friendly session display name.
    pub fn session_name(&self) -> &str {
        &self.session_name
    }

    /// Return the bounded ring capacity.
    pub const fn output_ring_capacity(&self) -> u64 {
        self.output_ring_capacity
    }

    /// Return the current common resource phase.
    pub const fn phase(&self) -> SessionPhase {
        self.phase
    }

    /// Borrow the Process child resource owned by this shell session.
    pub const fn supervisor_process_ref(&self) -> &ResourceRef {
        &self.supervisor_process_ref
    }

    /// Borrow the Host or Guest execution parent for the supervisor Process.
    pub const fn supervisor_execution_ref(&self) -> &ResourceRef {
        &self.supervisor_execution_ref
    }

    /// Borrow the exact workload User resource for the supervisor Process.
    pub const fn supervisor_user_ref(&self) -> &ResourceRef {
        &self.supervisor_user_ref
    }
}
