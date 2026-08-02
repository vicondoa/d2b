//! Host posture decisions owned by the system-core reconciler.

use d2b_contracts::v3::host::{HostSpec, IsolationPosture};

use crate::host_status::HostStatusProjection;

/// Names of status fields that only the reconciler may write.
pub const RECONCILER_OWNED_STATUS_FIELDS: [&str; 2] =
    ["isolationPosture", "isolationPostureMessage"];

/// A stable refusal for an operator-supplied reconciler-owned field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostStatusInputError {
    /// The submitted value was not an object.
    NotAnObject,
    /// A reconciler-owned field was present, including explicit null.
    OperatorSuppliedField,
}

impl core::fmt::Display for HostStatusInputError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::NotAnObject => "host-status-not-an-object",
            Self::OperatorSuppliedField => "host-status-field-reconciler-owned",
        })
    }
}

impl std::error::Error for HostStatusInputError {}

/// Derive the status that system-core must publish for a Host.
pub fn reconcile_status(spec: &HostSpec) -> HostStatusProjection {
    HostStatusProjection::from_spec(spec)
}

/// Return the only posture a user-only Host may carry.
pub fn isolation_posture(spec: &HostSpec) -> Option<IsolationPosture> {
    spec.isolation_posture()
}

/// Reject an operator status patch that tries to author the posture.
pub fn reject_operator_status_fields(
    value: &serde_json::Value,
) -> Result<(), HostStatusInputError> {
    let object = value.as_object().ok_or(HostStatusInputError::NotAnObject)?;
    if object
        .keys()
        .any(|key| RECONCILER_OWNED_STATUS_FIELDS.contains(&key.as_str()))
    {
        return Err(HostStatusInputError::OperatorSuppliedField);
    }
    Ok(())
}
