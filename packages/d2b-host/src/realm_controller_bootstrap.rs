//! Allocator-issued bootstrap authority for a parent-spawned realm controller.

use std::fmt;

pub const REALM_CONTROLLER_AUTHORITY_RESOURCE_ID: &str = "realm-controller-authority-v1";
pub const REALM_CONTROLLER_AUTHORITY_FD_ENV: &str = "D2B_REALM_CONTROLLER_AUTHORITY_FD";
pub const OVERFLOW_ID: u32 = 65_534;

const MAGIC: &[u8; 8] = b"D2BRCA1\0";
const MAX_ID_BYTES: usize = 128;

#[derive(Clone, PartialEq, Eq)]
pub struct RealmControllerChildAuthority {
    pub realm_id: String,
    pub controller_generation: String,
    pub controller_process_id: String,
    pub session_generation: u64,
    pub controller_host_uid: u32,
    pub controller_host_gid: u32,
    pub broker_host_uid: u32,
    pub broker_host_gid: u32,
    pub broker_namespace_uid: u32,
    pub broker_namespace_gid: u32,
    pub cgroup_digest: [u8; 32],
}

impl RealmControllerChildAuthority {
    pub fn validate(&self) -> Result<(), RealmControllerAuthorityError> {
        crate::realm_children::validate_realm_id(&self.realm_id)
            .map_err(|_| RealmControllerAuthorityError::Invalid)?;
        let expected_uid = u32::from(self.controller_host_uid != self.broker_host_uid);
        let expected_gid = u32::from(self.controller_host_gid != self.broker_host_gid);
        if !valid_id(&self.controller_generation)
            || !valid_id(&self.controller_process_id)
            || self.session_generation == 0
            || self.controller_host_uid == 0
            || self.controller_host_gid == 0
            || self.broker_host_uid == 0
            || self.broker_host_gid == 0
            || self.broker_namespace_uid != expected_uid
            || self.broker_namespace_gid != expected_gid
            || self.broker_namespace_uid == OVERFLOW_ID
            || self.broker_namespace_gid == OVERFLOW_ID
            || self.cgroup_digest == [0; 32]
        {
            return Err(RealmControllerAuthorityError::Invalid);
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, RealmControllerAuthorityError> {
        self.validate()?;
        let mut encoded = Vec::new();
        encoded.extend_from_slice(MAGIC);
        put_string(&mut encoded, &self.realm_id)?;
        put_string(&mut encoded, &self.controller_generation)?;
        put_string(&mut encoded, &self.controller_process_id)?;
        encoded.extend_from_slice(&self.session_generation.to_be_bytes());
        encoded.extend_from_slice(&self.controller_host_uid.to_be_bytes());
        encoded.extend_from_slice(&self.controller_host_gid.to_be_bytes());
        encoded.extend_from_slice(&self.broker_host_uid.to_be_bytes());
        encoded.extend_from_slice(&self.broker_host_gid.to_be_bytes());
        encoded.extend_from_slice(&self.broker_namespace_uid.to_be_bytes());
        encoded.extend_from_slice(&self.broker_namespace_gid.to_be_bytes());
        encoded.extend_from_slice(&self.cgroup_digest);
        Ok(encoded)
    }

    pub fn decode(encoded: &[u8]) -> Result<Self, RealmControllerAuthorityError> {
        let mut reader = Reader::new(encoded);
        if reader.take(MAGIC.len())? != MAGIC {
            return Err(RealmControllerAuthorityError::Invalid);
        }
        let authority = Self {
            realm_id: reader.string()?,
            controller_generation: reader.string()?,
            controller_process_id: reader.string()?,
            session_generation: reader.u64()?,
            controller_host_uid: reader.u32()?,
            controller_host_gid: reader.u32()?,
            broker_host_uid: reader.u32()?,
            broker_host_gid: reader.u32()?,
            broker_namespace_uid: reader.u32()?,
            broker_namespace_gid: reader.u32()?,
            cgroup_digest: reader.array()?,
        };
        if !reader.done() {
            return Err(RealmControllerAuthorityError::Invalid);
        }
        authority.validate()?;
        Ok(authority)
    }
}

impl fmt::Debug for RealmControllerChildAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RealmControllerChildAuthority(REDACTED)")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RealmControllerAuthorityError {
    Invalid,
    Truncated,
}

impl fmt::Display for RealmControllerAuthorityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "realm-controller-authority-invalid",
            Self::Truncated => "realm-controller-authority-truncated",
        })
    }
}

impl std::error::Error for RealmControllerAuthorityError {}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
}

fn put_string(encoded: &mut Vec<u8>, value: &str) -> Result<(), RealmControllerAuthorityError> {
    let length = u16::try_from(value.len()).map_err(|_| RealmControllerAuthorityError::Invalid)?;
    encoded.extend_from_slice(&length.to_be_bytes());
    encoded.extend_from_slice(value.as_bytes());
    Ok(())
}

struct Reader<'a> {
    encoded: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    const fn new(encoded: &'a [u8]) -> Self {
        Self { encoded, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], RealmControllerAuthorityError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(RealmControllerAuthorityError::Truncated)?;
        let bytes = self
            .encoded
            .get(self.offset..end)
            .ok_or(RealmControllerAuthorityError::Truncated)?;
        self.offset = end;
        Ok(bytes)
    }

    fn string(&mut self) -> Result<String, RealmControllerAuthorityError> {
        let length = usize::from(u16::from_be_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| RealmControllerAuthorityError::Truncated)?,
        ));
        if length == 0 || length > MAX_ID_BYTES {
            return Err(RealmControllerAuthorityError::Invalid);
        }
        std::str::from_utf8(self.take(length)?)
            .map(str::to_owned)
            .map_err(|_| RealmControllerAuthorityError::Invalid)
    }

    fn u32(&mut self) -> Result<u32, RealmControllerAuthorityError> {
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| RealmControllerAuthorityError::Truncated)?,
        ))
    }

    fn u64(&mut self) -> Result<u64, RealmControllerAuthorityError> {
        Ok(u64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| RealmControllerAuthorityError::Truncated)?,
        ))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], RealmControllerAuthorityError> {
        self.take(N)?
            .try_into()
            .map_err(|_| RealmControllerAuthorityError::Truncated)
    }

    fn done(&self) -> bool {
        self.offset == self.encoded.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authority() -> RealmControllerChildAuthority {
        RealmControllerChildAuthority {
            realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
            controller_generation: "generation-1".to_owned(),
            controller_process_id: "controller-1".to_owned(),
            session_generation: 7,
            controller_host_uid: 1000,
            controller_host_gid: 1000,
            broker_host_uid: 1001,
            broker_host_gid: 1001,
            broker_namespace_uid: 1,
            broker_namespace_gid: 1,
            cgroup_digest: [9; 32],
        }
    }

    #[test]
    fn controller_authority_round_trips_and_redacts() {
        let authority = authority();
        assert_eq!(
            RealmControllerChildAuthority::decode(&authority.encode().unwrap()).unwrap(),
            authority
        );
        assert_eq!(
            format!("{authority:?}"),
            "RealmControllerChildAuthority(REDACTED)"
        );
    }

    #[test]
    fn controller_authority_rejects_overflow_or_wrong_translation() {
        let mut invalid = authority();
        invalid.broker_namespace_uid = OVERFLOW_ID;
        assert_eq!(
            invalid.validate(),
            Err(RealmControllerAuthorityError::Invalid)
        );
        let mut invalid = authority();
        invalid.broker_namespace_gid = 0;
        assert_eq!(
            invalid.validate(),
            Err(RealmControllerAuthorityError::Invalid)
        );
    }
}
