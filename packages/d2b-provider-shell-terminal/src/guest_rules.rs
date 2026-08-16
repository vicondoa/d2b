//! Guest pool placement rules.

use crate::{ExecutionTarget, ShellTerminalError};

/// Verified Guest facts supplied by the Guest resource resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestPlacement {
    /// Whether the Guest declares the user execution domain.
    pub user_domain_allowed: bool,
    /// Whether its default workload user matches the pool user.
    pub default_user_matches: bool,
}

/// Validate user-domain Guest placement before a supervisor is created.
pub fn validate_guest_placement(
    target: &ExecutionTarget,
    placement: &GuestPlacement,
) -> Result<(), ShellTerminalError> {
    if target.is_host() || !placement.user_domain_allowed || !placement.default_user_matches {
        return Err(ShellTerminalError::GuestUserDomainUnsupported);
    }
    Ok(())
}
