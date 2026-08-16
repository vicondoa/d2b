//! `shell-terminal.d2bus.org.ShellPool` schema.

use super::{ShellTerminalError, validate_name};

/// Default maximum live or retained sessions in one pool.
pub const DEFAULT_MAX_SESSIONS: u32 = 8;
/// Default maximum concurrently attached sessions in one pool.
pub const DEFAULT_MAX_ATTACHED: u32 = 1;
/// Default in-memory terminal output-ring capacity.
pub const DEFAULT_OUTPUT_RING_CAPACITY: u64 = 256 * 1024;

const MAX_SESSIONS: u32 = 64;
const MAX_ATTACHED: u32 = 8;
const MIN_RING_CAPACITY: u64 = 4 * 1024;
const MAX_RING_CAPACITY: u64 = 1024 * 1024;

/// A target that can host a user-domain session supervisor.
#[derive(Clone, PartialEq, Eq)]
pub enum ExecutionTarget {
    /// A local Host resource.
    Host(String),
    /// A Guest resource with user-domain support.
    Guest(String),
}

impl std::fmt::Debug for ExecutionTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ExecutionTarget(<redacted>)")
    }
}

impl ExecutionTarget {
    /// Build a Host execution target.
    pub fn host(name: impl Into<String>) -> Self {
        Self::Host(name.into())
    }

    /// Build a Guest execution target.
    pub fn guest(name: impl Into<String>) -> Self {
        Self::Guest(name.into())
    }

    /// Return whether the target is a Host.
    pub const fn is_host(&self) -> bool {
        matches!(self, Self::Host(_))
    }

    /// Return the target's resource name.
    pub fn name(&self) -> &str {
        match self {
            Self::Host(name) | Self::Guest(name) => name,
        }
    }

    fn validate(&self) -> Result<(), ShellTerminalError> {
        validate_name(self.name(), 63)
    }
}

/// Immutable policy and capacity configuration for one shell pool.
#[derive(Clone, PartialEq, Eq)]
pub struct PoolSpec {
    execution_target: ExecutionTarget,
    workload_user: String,
    login_shell_ref: String,
    max_sessions: u32,
    max_attached: u32,
    output_ring_capacity: u64,
}

impl std::fmt::Debug for PoolSpec {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PoolSpec")
            .field("max_sessions", &self.max_sessions)
            .field("max_attached", &self.max_attached)
            .field("output_ring_capacity", &self.output_ring_capacity)
            .finish()
    }
}

impl PoolSpec {
    /// Construct a bounded pool policy.
    pub fn new(
        execution_target: ExecutionTarget,
        workload_user: impl Into<String>,
        login_shell_ref: impl Into<String>,
        max_sessions: u32,
        max_attached: u32,
        output_ring_capacity: u64,
    ) -> Result<Self, ShellTerminalError> {
        execution_target.validate()?;
        let workload_user = workload_user.into();
        validate_name(&workload_user, 63)?;
        let login_shell_ref = login_shell_ref.into();
        if !login_shell_ref.starts_with("artifact://")
            || login_shell_ref.len() > 255
            || login_shell_ref.len() == "artifact://".len()
        {
            return Err(ShellTerminalError::InvalidLoginShell);
        }
        if !(1..=MAX_SESSIONS).contains(&max_sessions)
            || !(1..=MAX_ATTACHED).contains(&max_attached)
            || max_attached > max_sessions
            || !(MIN_RING_CAPACITY..=MAX_RING_CAPACITY).contains(&output_ring_capacity)
        {
            return Err(ShellTerminalError::CapacityOutOfRange);
        }
        Ok(Self {
            execution_target,
            workload_user,
            login_shell_ref,
            max_sessions,
            max_attached,
            output_ring_capacity,
        })
    }

    /// Borrow the execution target.
    pub const fn execution_target(&self) -> &ExecutionTarget {
        &self.execution_target
    }

    /// Borrow the exact workload user name.
    pub fn workload_user(&self) -> &str {
        &self.workload_user
    }

    /// Borrow the manifest-fixed login shell artifact reference.
    pub fn login_shell_ref(&self) -> &str {
        &self.login_shell_ref
    }

    /// Return the maximum retained session count.
    pub const fn max_sessions(&self) -> u32 {
        self.max_sessions
    }

    /// Return the maximum concurrent attachment count.
    pub const fn max_attached(&self) -> u32 {
        self.max_attached
    }

    /// Return the inherited output-ring capacity.
    pub const fn output_ring_capacity(&self) -> u64 {
        self.output_ring_capacity
    }
}

/// A qualified `ShellPool` resource.
#[derive(Clone, PartialEq, Eq)]
pub struct ShellPool {
    name: String,
    zone: String,
    spec: PoolSpec,
}

impl std::fmt::Debug for ShellPool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ShellPool")
            .field("spec", &self.spec)
            .finish_non_exhaustive()
    }
}

impl ShellPool {
    /// Construct a Zone-scoped shell pool.
    pub fn new(
        name: impl Into<String>,
        zone: impl Into<String>,
        spec: PoolSpec,
    ) -> Result<Self, ShellTerminalError> {
        let name = name.into();
        let zone = zone.into();
        validate_name(&name, 63)?;
        validate_name(&zone, 63)?;
        Ok(Self { name, zone, spec })
    }

    /// Return the canonical qualified resource type.
    pub const fn resource_type(&self) -> &'static str {
        "shell-terminal.d2bus.org.ShellPool"
    }

    /// Borrow the resource name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Borrow the owning Zone name.
    pub fn zone(&self) -> &str {
        &self.zone
    }

    /// Borrow the immutable pool policy.
    pub const fn spec(&self) -> &PoolSpec {
        &self.spec
    }

    /// Borrow the execution target.
    pub const fn execution_target(&self) -> &ExecutionTarget {
        self.spec.execution_target()
    }

    /// Borrow the pool's workload user.
    pub fn workload_user(&self) -> &str {
        self.spec.workload_user()
    }

    /// Return capacity for active sessions.
    pub const fn active_session_capacity(&self) -> u32 {
        self.spec.max_sessions()
    }
}
