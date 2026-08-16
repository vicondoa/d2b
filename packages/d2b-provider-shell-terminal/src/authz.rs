//! Authorization bound to the current authenticated request.

use std::collections::BTreeSet;

use crate::{ExecutionTarget, ShellPool, ShellTerminalError};

/// Origin of an authenticated ComponentSession request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerOrigin {
    /// The request was authenticated locally.
    Local,
    /// The request was authenticated by a relay and is never local Host authority.
    Relay,
}

/// Closed roles accepted by shell-terminal service methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// The Zone-scoped shell administration role.
    ShellAdmin,
    /// A documented Zone administrator superset.
    ZoneAdmin,
    /// A non-administrative role.
    Viewer,
}

/// The request-bound authority supplied by the authenticated session layer.
#[derive(Clone, PartialEq, Eq)]
pub struct Subject {
    zone: String,
    origin: CallerOrigin,
    roles: BTreeSet<Role>,
}

impl std::fmt::Debug for Subject {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Subject(<redacted>)")
    }
}

impl Subject {
    /// Construct a request-bound subject from authenticated Zone and role data.
    pub fn new(
        zone: impl Into<String>,
        origin: CallerOrigin,
        roles: impl IntoIterator<Item = Role>,
    ) -> Self {
        Self {
            zone: zone.into(),
            origin,
            roles: roles.into_iter().collect(),
        }
    }

    /// Borrow the authenticated Zone.
    pub fn zone(&self) -> &str {
        &self.zone
    }

    /// Return the immutable authentication origin.
    pub const fn origin(&self) -> CallerOrigin {
        self.origin
    }

    fn is_admin(&self) -> bool {
        self.roles.contains(&Role::ShellAdmin) || self.roles.contains(&Role::ZoneAdmin)
    }
}

/// Validates service authorization before capacity or route lookup.
#[derive(Debug, Default, Clone, Copy)]
pub struct Authorizer;

impl Authorizer {
    /// Authorize the role bound to the current request before resource lookup.
    pub fn authorize_request(subject: &Subject) -> Result<(), ShellTerminalError> {
        if subject.is_admin() {
            Ok(())
        } else {
            Err(ShellTerminalError::NotAuthorized)
        }
    }

    /// Authorize a shell verb against one pool.
    pub fn authorize(subject: &Subject, pool: &ShellPool) -> Result<(), ShellTerminalError> {
        Self::authorize_target(subject, pool.zone(), pool.execution_target())
    }

    /// Authorize an exact Zone and execution target for a session verb.
    pub fn authorize_target(
        subject: &Subject,
        zone: &str,
        target: &ExecutionTarget,
    ) -> Result<(), ShellTerminalError> {
        Self::authorize_request(subject)?;
        if subject.zone() != zone {
            return Err(ShellTerminalError::WrongZone);
        }
        if subject.origin() == CallerOrigin::Relay && target.is_host() {
            return Err(ShellTerminalError::RelayHostUserDomainDenied);
        }
        Ok(())
    }
}
