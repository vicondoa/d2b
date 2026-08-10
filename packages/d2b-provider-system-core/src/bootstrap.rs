//! Fixed system-core bootstrap ordering.
//!
//! The bootstrap process is deliberately a small state machine rather than a
//! collection of booleans.  Host readiness must precede every Process
//! Provider launch; initial User rows must exist before user-domain admission;
//! and compiled Role/RoleBinding policy is handed off only after both
//! observations are complete.

use std::fmt;

/// The only bootstrap stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BootstrapStage {
    /// No Host observation has committed.
    Fresh,
    /// The initial Host observation committed.
    HostReady,
    /// Initial User resources have been published.
    UsersPublished,
    /// Initial Role and RoleBinding policy has been published.
    RolesPublished,
    /// Stored RBAC owns subsequent authorization decisions.
    AuthorizationHandedOff,
}

/// Fixed bootstrap authorization capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BootstrapCapability {
    /// Read and write the initial Host row.
    HostBootstrap,
    /// Create the initial User rows.
    UserBootstrap,
    /// Publish the initial Role and RoleBinding rows.
    RbacBootstrap,
}

/// A monotonic bootstrap sequence.
#[derive(Clone, PartialEq, Eq)]
pub struct BootstrapSequence {
    stage: BootstrapStage,
    users_published: u32,
    capabilities: [bool; 3],
}

impl BootstrapSequence {
    /// Create a fresh sequence with no process-launch authority.
    pub const fn new() -> Self {
        Self {
            stage: BootstrapStage::Fresh,
            users_published: 0,
            capabilities: [false; 3],
        }
    }

    /// Return the current stage.
    pub const fn stage(&self) -> BootstrapStage {
        self.stage
    }

    /// Record the initial Host reconcile.
    pub fn host_ready(&mut self) -> Result<(), BootstrapError> {
        if self.stage != BootstrapStage::Fresh {
            return Err(BootstrapError::InvalidTransition);
        }
        self.stage = BootstrapStage::HostReady;
        self.capabilities[BootstrapCapability::HostBootstrap as usize] = true;
        Ok(())
    }

    /// Publish the initial User rows.
    pub fn publish_users(&mut self, count: u32) -> Result<(), BootstrapError> {
        if self.stage != BootstrapStage::HostReady {
            return Err(BootstrapError::InvalidTransition);
        }
        self.users_published = count;
        self.stage = BootstrapStage::UsersPublished;
        self.capabilities[BootstrapCapability::UserBootstrap as usize] = true;
        Ok(())
    }

    /// Publish initial Role and RoleBinding policy.
    pub fn publish_roles(&mut self) -> Result<(), BootstrapError> {
        if self.stage != BootstrapStage::UsersPublished {
            return Err(BootstrapError::InvalidTransition);
        }
        self.stage = BootstrapStage::RolesPublished;
        self.capabilities[BootstrapCapability::RbacBootstrap as usize] = true;
        Ok(())
    }

    /// Hand authorization to the durable RBAC store.
    pub fn handoff_authorization(&mut self) -> Result<(), BootstrapError> {
        if self.stage != BootstrapStage::RolesPublished {
            return Err(BootstrapError::InvalidTransition);
        }
        self.stage = BootstrapStage::AuthorizationHandedOff;
        Ok(())
    }

    /// Whether the fixed bootstrap may launch a Process Provider.
    pub const fn process_launch_allowed(&self) -> bool {
        matches!(self.stage, BootstrapStage::AuthorizationHandedOff)
    }

    /// Return the number of initial User rows published.
    pub const fn users_published(&self) -> u32 {
        self.users_published
    }

    /// Whether one fixed bootstrap capability is still active.
    pub const fn has_capability(&self, capability: BootstrapCapability) -> bool {
        self.capabilities[capability as usize]
    }
}

impl Default for BootstrapSequence {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for BootstrapSequence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BootstrapSequence")
            .field("stage", &self.stage)
            .field("users_published", &self.users_published)
            .finish()
    }
}

/// Closed bootstrap transition errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapError {
    /// The next stage was attempted before its predecessor.
    InvalidTransition,
}

impl fmt::Display for BootstrapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("bootstrap-stage-transition-invalid")
    }
}

impl std::error::Error for BootstrapError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_users_roles_and_handoff_are_strictly_ordered() {
        let mut sequence = BootstrapSequence::new();
        assert!(!sequence.process_launch_allowed());
        assert_eq!(
            sequence.publish_users(1),
            Err(BootstrapError::InvalidTransition)
        );
        sequence.host_ready().unwrap();
        sequence.publish_users(1).unwrap();
        sequence.publish_roles().unwrap();
        assert!(!sequence.process_launch_allowed());
        sequence.handoff_authorization().unwrap();
        assert!(sequence.process_launch_allowed());
        assert_eq!(sequence.users_published(), 1);
    }

    #[test]
    fn stages_cannot_be_replayed_or_skipped() {
        let mut sequence = BootstrapSequence::new();
        sequence.host_ready().unwrap();
        assert_eq!(
            sequence.host_ready(),
            Err(BootstrapError::InvalidTransition)
        );
        assert_eq!(
            sequence.handoff_authorization(),
            Err(BootstrapError::InvalidTransition)
        );
    }
}
