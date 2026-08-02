//! Redacted status projection for the system-core Host reconciler.

#![allow(dead_code)]

use d2b_contracts::v3::host::{HostSpec, IsolationPosture};
use serde::Serialize;
use std::fmt;

/// The fixed status message for a user-only Host.
pub const ISOLATION_POSTURE_MESSAGE: &str = "This host resource runs processes as the authenticated user with no isolation boundary. All child processes share the host user environment.";

/// Status fields owned by the system-core reconciler.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStatusProjection {
    /// Explicit posture, present only for user-only Hosts.
    pub isolation_posture: Option<IsolationPosture>,
    /// Fixed explanatory message paired with the posture.
    pub isolation_posture_message: Option<&'static str>,
}

impl fmt::Debug for HostStatusProjection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HostStatusProjection(<redacted>)")
    }
}

impl HostStatusProjection {
    /// Derive the non-suppressible posture from an admitted Host spec.
    pub fn from_spec(spec: &HostSpec) -> Self {
        let isolation_posture = spec.isolation_posture();
        Self {
            isolation_posture,
            isolation_posture_message: isolation_posture.map(|_| ISOLATION_POSTURE_MESSAGE),
        }
    }

    /// Whether this projection declares the no-isolation posture.
    pub const fn is_no_isolation(&self) -> bool {
        self.isolation_posture.is_some()
    }
}
