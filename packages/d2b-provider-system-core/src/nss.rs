//! Bounded NSS-backed User observation.
//!
//! The effect port owns the actual `getpwnam`/group-manager calls.  This
//! module only carries the bounded observation returned by that port and
//! turns it into a ResourceType status.  No credential or authentication
//! operation is performed.

use std::{fmt, future::Future};

use d2b_contracts::v3::{
    ResourceRef,
    resource_status::ResourcePhase,
    user::{MAX_USER_GROUPS, OsGroupName, UserSpec},
};
use serde::Serialize;

use crate::{SystemCoreError, ownership};

/// A bounded NSS record returned by the fixed host effect adapter.
#[derive(Clone, PartialEq, Eq)]
pub struct NssUserRecord {
    /// Numeric UID observed by NSS.
    pub uid: u32,
    /// Primary numeric GID observed by NSS.
    pub gid: u32,
    /// Whether the NSS home path exists and is accessible.
    pub home_exists: bool,
    /// Whether the NSS login shell exists and is executable.
    pub shell_valid: bool,
    /// Observed additional group names.
    pub groups: Vec<OsGroupName>,
    /// Whether the fixed user manager responds.
    pub session_manager_available: bool,
}

impl fmt::Debug for NssUserRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NssUserRecord(<redacted>)")
    }
}

impl NssUserRecord {
    /// Construct a bounded observation.
    pub fn new(
        uid: u32,
        gid: u32,
        home_exists: bool,
        shell_valid: bool,
        groups: Vec<OsGroupName>,
        session_manager_available: bool,
    ) -> Result<Self, SystemCoreError> {
        if groups.len() > MAX_USER_GROUPS {
            return Err(SystemCoreError::HostProbeFailed);
        }
        Ok(Self {
            uid,
            gid,
            home_exists,
            shell_valid,
            groups,
            session_manager_available,
        })
    }
}

/// The fixed core effect port for NSS observation.
pub trait NssUserEffectPort: Send + Sync {
    /// Resolve one User with the configured bounded lookup timeout.
    fn lookup(
        &self,
        user_ref: &ResourceRef,
        spec: &UserSpec,
    ) -> impl Future<Output = Result<Option<NssUserRecord>, SystemCoreError>> + Send;
}

/// ResourceType-specific User status produced from an NSS record.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NssUserStatus {
    /// The User resource identity.
    pub user_ref: ResourceRef,
    /// Universal lifecycle phase.
    pub phase: ResourcePhase,
    /// Observed numeric UID.
    pub uid: Option<u32>,
    /// Observed primary numeric GID.
    pub gid: Option<u32>,
    /// Home-directory observation.
    pub home_exists: bool,
    /// Login-shell observation.
    pub shell_valid: bool,
    /// Session-manager observation.
    pub session_manager_available: bool,
    /// Whether every declared group was observed.
    pub group_membership_verified: bool,
    /// Observed group names.
    pub observed_groups: Vec<OsGroupName>,
}

impl fmt::Debug for NssUserStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NssUserStatus(<redacted>)")
    }
}

/// User reconciler using an injected NSS effect port.
#[derive(Debug)]
pub struct NssUserReconciler<P> {
    port: P,
}

impl<P> NssUserReconciler<P>
where
    P: NssUserEffectPort,
{
    /// Build the reconciler.
    pub const fn new(port: P) -> Self {
        Self { port }
    }

    /// Perform one bounded NSS observation.
    pub async fn reconcile(
        &self,
        user_ref: &ResourceRef,
        spec: &UserSpec,
    ) -> Result<NssUserStatus, SystemCoreError> {
        ownership::require_resource_type(user_ref, "User")?;
        let Some(record) = self.port.lookup(user_ref, spec).await? else {
            return Ok(NssUserStatus {
                user_ref: user_ref.clone(),
                phase: ResourcePhase::Degraded,
                uid: None,
                gid: None,
                home_exists: false,
                shell_valid: false,
                session_manager_available: false,
                group_membership_verified: false,
                observed_groups: Vec::new(),
            });
        };
        let group_membership_verified = spec
            .groups()
            .iter()
            .all(|group| record.groups.iter().any(|observed| observed == group));
        let ready = record.home_exists && (spec.groups().is_empty() || group_membership_verified);
        let phase = if ready {
            if record.session_manager_available {
                ResourcePhase::Ready
            } else {
                ResourcePhase::Degraded
            }
        } else {
            ResourcePhase::Degraded
        };
        Ok(NssUserStatus {
            user_ref: user_ref.clone(),
            phase,
            uid: Some(record.uid),
            gid: Some(record.gid),
            home_exists: record.home_exists,
            shell_valid: record.shell_valid,
            session_manager_available: record.session_manager_available,
            group_membership_verified,
            observed_groups: record.groups,
        })
    }
}
