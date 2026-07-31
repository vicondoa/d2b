//! Authorized bounded audit records for managed identity operations.

const SHA256_PREFIX: &str = "sha256:";

/// Closed managed identity audit operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedIdentityAuditOperation {
    /// A token lease was acquired.
    AcquireToken,
    /// A token lease was refreshed.
    RefreshToken,
    /// A token lease was revoked.
    RevokeToken,
    /// Lease metadata was inspected.
    InspectMetadata,
    /// An agent Process was started.
    AgentStart,
    /// An agent Process was stopped.
    AgentStop,
}

impl ManagedIdentityAuditOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::AcquireToken => "acquire-token",
            Self::RefreshToken => "refresh-token",
            Self::RevokeToken => "revoke-token",
            Self::InspectMetadata => "inspect-metadata",
            Self::AgentStart => "agent-start",
            Self::AgentStop => "agent-stop",
        }
    }
}

/// Closed managed identity audit outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedIdentityAuditOutcome {
    /// The operation completed.
    Success,
    /// The backing client was unavailable.
    ProviderUnavailable,
    /// Policy denied the operation.
    Denied,
    /// A lease was already revoked.
    AlreadyRevoked,
}

impl ManagedIdentityAuditOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::ProviderUnavailable => "provider-unavailable",
            Self::Denied => "denied",
            Self::AlreadyRevoked => "already-revoked",
        }
    }
}

/// Audit construction failure without caller-controlled diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedIdentityAuditError {
    /// The authorized resource-name digest was not a canonical SHA-256 digest.
    InvalidResourceNameDigest,
}

impl core::fmt::Display for ManagedIdentityAuditError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("credential audit record is invalid")
    }
}

impl std::error::Error for ManagedIdentityAuditError {}

/// Authorized audit record containing only the permitted identity digest and
/// closed operation metadata.
pub struct ManagedIdentityAuditRecord {
    resource_name_digest: String,
    operation: ManagedIdentityAuditOperation,
    outcome: ManagedIdentityAuditOutcome,
    rotation_generation: u64,
}

impl ManagedIdentityAuditRecord {
    /// Validate and construct one authorized audit record.
    pub fn new(
        resource_name_digest: impl Into<String>,
        operation: ManagedIdentityAuditOperation,
        outcome: ManagedIdentityAuditOutcome,
        rotation_generation: u64,
    ) -> Result<Self, ManagedIdentityAuditError> {
        let resource_name_digest = resource_name_digest.into();
        if !valid_sha256(&resource_name_digest) {
            return Err(ManagedIdentityAuditError::InvalidResourceNameDigest);
        }
        Ok(Self {
            resource_name_digest,
            operation,
            outcome,
            rotation_generation,
        })
    }

    /// Render the bounded authorized audit payload.
    pub fn to_wire_record(&self) -> String {
        format!(
            "provider=credential-managed-identity resource_name_digest={} operation={} outcome={} rotation_generation={}",
            self.resource_name_digest,
            self.operation.as_str(),
            self.outcome.as_str(),
            self.rotation_generation
        )
    }
}

impl core::fmt::Debug for ManagedIdentityAuditRecord {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ManagedIdentityAuditRecord(<redacted>)")
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with(SHA256_PREFIX)
        && value[SHA256_PREFIX.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_accepts_only_a_canonical_authorized_digest() {
        let digest = format!("sha256:{}", "a".repeat(64));
        let record = ManagedIdentityAuditRecord::new(
            &digest,
            ManagedIdentityAuditOperation::AcquireToken,
            ManagedIdentityAuditOutcome::Success,
            1,
        )
        .unwrap();
        assert!(record.to_wire_record().contains(&digest));
        assert!(!format!("{record:?}").contains(&digest));
        assert!(
            ManagedIdentityAuditRecord::new(
                "Credential/work-token",
                ManagedIdentityAuditOperation::AcquireToken,
                ManagedIdentityAuditOutcome::Success,
                1,
            )
            .is_err()
        );
    }
}
