use d2b_contracts::v2_state::{
    AuthorityRef, FdTransferPolicy, Generation, LeaseRecord, LeaseRevocation,
    MAX_SAFE_JSON_INTEGER, ResourceId,
};

use crate::{Error, ErrorCode, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseStatus {
    Active,
    Revoked,
    Expired,
}

pub fn grant_lease(
    lease_id: ResourceId,
    resource_id: ResourceId,
    owner: AuthorityRef,
    generation: Generation,
    expires_at_unix_ms: u64,
    fd_transfer: FdTransferPolicy,
) -> Result<LeaseRecord> {
    if expires_at_unix_ms == 0
        || expires_at_unix_ms > MAX_SAFE_JSON_INTEGER
        || fd_transfer == FdTransferPolicy::Never
    {
        return Err(Error::Code(ErrorCode::InvalidSchema));
    }
    Ok(LeaseRecord {
        lease_id,
        resource_id,
        owner,
        generation,
        expires_at_unix_ms,
        revocation: LeaseRevocation::Active,
        fd_transfer,
    })
}

pub fn validate_lease(
    lease: &LeaseRecord,
    expected_generation: Generation,
    now_unix_ms: u64,
) -> Result<LeaseStatus> {
    lease.validate_use(expected_generation, now_unix_ms)?;
    Ok(LeaseStatus::Active)
}

pub fn revoke_lease(
    lease: &mut LeaseRecord,
    expected_generation: Generation,
    reason: LeaseRevocation,
) -> Result<LeaseStatus> {
    if lease.generation != expected_generation || reason == LeaseRevocation::Active {
        return Err(Error::Code(ErrorCode::GenerationRollback));
    }
    lease.revocation = reason;
    Ok(match reason {
        LeaseRevocation::Expired => LeaseStatus::Expired,
        LeaseRevocation::RevokedByOwner | LeaseRevocation::RevokedByGenerationChange => {
            LeaseStatus::Revoked
        }
        LeaseRevocation::Active => unreachable!("active was rejected above"),
    })
}
