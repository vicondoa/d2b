//! Allocator-issued bootstrap authority for a parent-spawned realm broker.

use std::{fmt, path::Path};

use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

pub const REALM_BROKER_AUTHORITY_RESOURCE_ID: &str = "realm-broker-authority-v1";
pub const REALM_BROKER_AUTHORITY_FD_ENV: &str = "D2B_REALM_BROKER_AUTHORITY_FD";
pub const REALM_BROKER_GUEST_RUNTIME_RESOURCE_ID: &str = "realm-broker-guest-runtime-v1";
pub const REALM_BROKER_GUEST_RUNTIME_FD_ENV: &str = "D2B_REALM_BROKER_GUEST_RUNTIME_FD";

const MAGIC: &[u8; 8] = b"D2BRBA2\0";
const MAX_ID_BYTES: usize = 128;
const MAX_PATH_BYTES: usize = 4096;
const MAX_CONFIGURED_BYTES: usize = 2 * 1024 * 1024;
const MAX_WORKLOADS: usize = 128;
const FIXED_BYTES: usize = 8 + 2 + 2 + 2 + 8 + 4 + 4 + 4 + 4 + 32 + 32;
const GUEST_RUNTIME_MAGIC: &[u8; 8] = b"D2BRGR1\0";

#[derive(Clone, PartialEq, Eq)]
pub struct RealmBrokerChildAuthority {
    pub realm_id: String,
    pub controller_generation: String,
    pub broker_process_id: String,
    pub session_generation: u64,
    pub controller_uid: u32,
    pub controller_gid: u32,
    pub broker_uid: u32,
    pub broker_gid: u32,
    pub cgroup_digest: [u8; 32],
    pub guest_runtime_digest: [u8; 32],
}

impl RealmBrokerChildAuthority {
    pub fn validate(&self) -> Result<(), RealmBrokerAuthorityError> {
        crate::realm_children::validate_realm_id(&self.realm_id)
            .map_err(|_| RealmBrokerAuthorityError::Invalid)?;
        if !valid_id(&self.controller_generation)
            || !valid_id(&self.broker_process_id)
            || self.session_generation == 0
            || self.controller_uid == 0
            || self.controller_gid == 0
            || self.broker_uid == 0
            || self.broker_gid == 0
            || self.cgroup_digest == [0; 32]
            || self.guest_runtime_digest == [0; 32]
        {
            return Err(RealmBrokerAuthorityError::Invalid);
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, RealmBrokerAuthorityError> {
        self.validate()?;
        let mut encoded = Vec::with_capacity(
            FIXED_BYTES
                + self.realm_id.len()
                + self.controller_generation.len()
                + self.broker_process_id.len(),
        );
        encoded.extend_from_slice(MAGIC);
        put_string(&mut encoded, &self.realm_id)?;
        put_string(&mut encoded, &self.controller_generation)?;
        put_string(&mut encoded, &self.broker_process_id)?;
        encoded.extend_from_slice(&self.session_generation.to_be_bytes());
        encoded.extend_from_slice(&self.controller_uid.to_be_bytes());
        encoded.extend_from_slice(&self.controller_gid.to_be_bytes());
        encoded.extend_from_slice(&self.broker_uid.to_be_bytes());
        encoded.extend_from_slice(&self.broker_gid.to_be_bytes());
        encoded.extend_from_slice(&self.cgroup_digest);
        encoded.extend_from_slice(&self.guest_runtime_digest);
        Ok(encoded)
    }

    pub fn decode(encoded: &[u8]) -> Result<Self, RealmBrokerAuthorityError> {
        let mut reader = Reader::new(encoded);
        if reader.take(MAGIC.len())? != MAGIC {
            return Err(RealmBrokerAuthorityError::Invalid);
        }
        let authority = Self {
            realm_id: reader.string()?,
            controller_generation: reader.string()?,
            broker_process_id: reader.string()?,
            session_generation: reader.u64()?,
            controller_uid: reader.u32()?,
            controller_gid: reader.u32()?,
            broker_uid: reader.u32()?,
            broker_gid: reader.u32()?,
            cgroup_digest: reader.array()?,
            guest_runtime_digest: reader.array()?,
        };
        if !reader.done() {
            return Err(RealmBrokerAuthorityError::Invalid);
        }
        authority.validate()?;
        Ok(authority)
    }
}

pub struct RealmBrokerGuestRuntimeBootstrap {
    pub realm_id: String,
    pub session_generation: u64,
    pub replay_ledger_path: String,
    pub audit_log_path: String,
    pub workloads: Vec<RealmBrokerGuestWorkloadBootstrap>,
}

pub struct RealmBrokerGuestWorkloadBootstrap {
    pub workload_id: String,
    pub parent_static_public_key: [u8; 32],
    pub channel_binding: [u8; 32],
    pub bootstrap_operation_id: [u8; 16],
    pub replay_nonce: [u8; 32],
    pub issued_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub bootstrap_psk: [u8; 32],
    pub session_storage_ref: String,
    pub session_path: String,
    pub configured_storage_ref: String,
    pub configured_path: String,
    pub owner_uid: u32,
    pub owner_gid: u32,
    pub mode: u32,
    pub configured_launches: Vec<u8>,
    pub configured_launch_digest: [u8; 32],
}

impl RealmBrokerGuestRuntimeBootstrap {
    pub fn validate(&self) -> Result<(), RealmBrokerAuthorityError> {
        crate::realm_children::validate_realm_id(&self.realm_id)
            .map_err(|_| RealmBrokerAuthorityError::Invalid)?;
        if self.session_generation == 0
            || !valid_absolute_path(&self.replay_ledger_path)
            || !valid_absolute_path(&self.audit_log_path)
            || self.replay_ledger_path == self.audit_log_path
            || self.workloads.is_empty()
            || self.workloads.len() > MAX_WORKLOADS
        {
            return Err(RealmBrokerAuthorityError::Invalid);
        }
        let mut workload_ids = std::collections::BTreeSet::new();
        for workload in &self.workloads {
            workload.validate()?;
            if !workload_ids.insert(workload.workload_id.as_str()) {
                return Err(RealmBrokerAuthorityError::Invalid);
            }
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Zeroizing<Vec<u8>>, RealmBrokerAuthorityError> {
        self.validate()?;
        let mut encoded = Zeroizing::new(Vec::new());
        encoded.extend_from_slice(GUEST_RUNTIME_MAGIC);
        put_string(&mut encoded, &self.realm_id)?;
        encoded.extend_from_slice(&self.session_generation.to_be_bytes());
        put_path(&mut encoded, &self.replay_ledger_path)?;
        put_path(&mut encoded, &self.audit_log_path)?;
        encoded.extend_from_slice(
            &u16::try_from(self.workloads.len())
                .map_err(|_| RealmBrokerAuthorityError::Invalid)?
                .to_be_bytes(),
        );
        for workload in &self.workloads {
            put_string(&mut encoded, &workload.workload_id)?;
            encoded.extend_from_slice(&workload.parent_static_public_key);
            encoded.extend_from_slice(&workload.channel_binding);
            encoded.extend_from_slice(&workload.bootstrap_operation_id);
            encoded.extend_from_slice(&workload.replay_nonce);
            encoded.extend_from_slice(&workload.issued_at_unix_ms.to_be_bytes());
            encoded.extend_from_slice(&workload.expires_at_unix_ms.to_be_bytes());
            encoded.extend_from_slice(&workload.bootstrap_psk);
            put_string(&mut encoded, &workload.session_storage_ref)?;
            put_path(&mut encoded, &workload.session_path)?;
            put_string(&mut encoded, &workload.configured_storage_ref)?;
            put_path(&mut encoded, &workload.configured_path)?;
            encoded.extend_from_slice(&workload.owner_uid.to_be_bytes());
            encoded.extend_from_slice(&workload.owner_gid.to_be_bytes());
            encoded.extend_from_slice(&workload.mode.to_be_bytes());
            encoded.extend_from_slice(
                &u32::try_from(workload.configured_launches.len())
                    .map_err(|_| RealmBrokerAuthorityError::Invalid)?
                    .to_be_bytes(),
            );
            encoded.extend_from_slice(&workload.configured_launches);
            encoded.extend_from_slice(&workload.configured_launch_digest);
        }
        Ok(encoded)
    }

    pub fn decode(encoded: &[u8]) -> Result<Self, RealmBrokerAuthorityError> {
        let mut reader = Reader::new(encoded);
        if reader.take(GUEST_RUNTIME_MAGIC.len())? != GUEST_RUNTIME_MAGIC {
            return Err(RealmBrokerAuthorityError::Invalid);
        }
        let realm_id = reader.string()?;
        let session_generation = reader.u64()?;
        let replay_ledger_path = reader.path()?;
        let audit_log_path = reader.path()?;
        let workload_count = usize::from(reader.u16()?);
        if workload_count == 0 || workload_count > MAX_WORKLOADS {
            return Err(RealmBrokerAuthorityError::Invalid);
        }
        let mut workloads = Vec::with_capacity(workload_count);
        for _ in 0..workload_count {
            let workload_id = reader.string()?;
            let parent_static_public_key = reader.array()?;
            let channel_binding = reader.array()?;
            let bootstrap_operation_id = reader.array()?;
            let replay_nonce = reader.array()?;
            let issued_at_unix_ms = reader.u64()?;
            let expires_at_unix_ms = reader.u64()?;
            let bootstrap_psk = reader.array()?;
            let session_storage_ref = reader.string()?;
            let session_path = reader.path()?;
            let configured_storage_ref = reader.string()?;
            let configured_path = reader.path()?;
            let owner_uid = reader.u32()?;
            let owner_gid = reader.u32()?;
            let mode = reader.u32()?;
            let configured_len =
                usize::try_from(reader.u32()?).map_err(|_| RealmBrokerAuthorityError::Invalid)?;
            if configured_len == 0 || configured_len > MAX_CONFIGURED_BYTES {
                return Err(RealmBrokerAuthorityError::Invalid);
            }
            let configured_launches = reader.take(configured_len)?.to_vec();
            let configured_launch_digest = reader.array()?;
            workloads.push(RealmBrokerGuestWorkloadBootstrap {
                workload_id,
                parent_static_public_key,
                channel_binding,
                bootstrap_operation_id,
                replay_nonce,
                issued_at_unix_ms,
                expires_at_unix_ms,
                bootstrap_psk,
                session_storage_ref,
                session_path,
                configured_storage_ref,
                configured_path,
                owner_uid,
                owner_gid,
                mode,
                configured_launches,
                configured_launch_digest,
            });
        }
        if !reader.done() {
            return Err(RealmBrokerAuthorityError::Invalid);
        }
        let runtime = Self {
            realm_id,
            session_generation,
            replay_ledger_path,
            audit_log_path,
            workloads,
        };
        runtime.validate()?;
        Ok(runtime)
    }
}

impl RealmBrokerGuestWorkloadBootstrap {
    fn validate(&self) -> Result<(), RealmBrokerAuthorityError> {
        let configured_digest: [u8; 32] = Sha256::digest(&self.configured_launches).into();
        if !valid_id(&self.workload_id)
            || self.parent_static_public_key == [0; 32]
            || self.channel_binding == [0; 32]
            || self.bootstrap_operation_id == [0; 16]
            || self.replay_nonce == [0; 32]
            || self.bootstrap_psk == [0; 32]
            || self.issued_at_unix_ms >= self.expires_at_unix_ms
            || self.expires_at_unix_ms - self.issued_at_unix_ms > 5 * 60 * 1000
            || !valid_id(&self.session_storage_ref)
            || !valid_id(&self.configured_storage_ref)
            || !self
                .session_storage_ref
                .starts_with("path:workload-guest-session-credential:")
            || !self
                .configured_storage_ref
                .starts_with("path:workload-configured-launch-credential:")
            || !valid_absolute_path(&self.session_path)
            || !valid_absolute_path(&self.configured_path)
            || Path::new(&self.session_path).parent() != Path::new(&self.configured_path).parent()
            || Path::new(&self.session_path)
                .file_name()
                .and_then(|name| name.to_str())
                != Some("d2b-guest-session-v2")
            || Path::new(&self.configured_path)
                .file_name()
                .and_then(|name| name.to_str())
                != Some("d2b-configured-launch-v2")
            || self.owner_uid != 0
            || self.owner_gid != 0
            || self.mode != 0o440
            || self.configured_launches.is_empty()
            || self.configured_launches.len() > MAX_CONFIGURED_BYTES
            || self.configured_launch_digest != configured_digest
        {
            return Err(RealmBrokerAuthorityError::Invalid);
        }
        Ok(())
    }
}

impl fmt::Debug for RealmBrokerGuestRuntimeBootstrap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RealmBrokerGuestRuntimeBootstrap(REDACTED)")
    }
}

impl fmt::Debug for RealmBrokerGuestWorkloadBootstrap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RealmBrokerGuestWorkloadBootstrap(REDACTED)")
    }
}

impl Drop for RealmBrokerGuestWorkloadBootstrap {
    fn drop(&mut self) {
        self.bootstrap_psk.zeroize();
        self.configured_launches.zeroize();
    }
}

impl fmt::Debug for RealmBrokerChildAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RealmBrokerChildAuthority(REDACTED)")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RealmBrokerAuthorityError {
    Invalid,
    Truncated,
}

impl fmt::Display for RealmBrokerAuthorityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "realm-broker-authority-invalid",
            Self::Truncated => "realm-broker-authority-truncated",
        })
    }
}

impl std::error::Error for RealmBrokerAuthorityError {}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
}

fn valid_absolute_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_PATH_BYTES
        && !value.as_bytes().contains(&0)
        && Path::new(value).is_absolute()
        && Path::new(value).components().all(|component| {
            !matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
}

fn put_string(encoded: &mut Vec<u8>, value: &str) -> Result<(), RealmBrokerAuthorityError> {
    let length = u16::try_from(value.len()).map_err(|_| RealmBrokerAuthorityError::Invalid)?;
    encoded.extend_from_slice(&length.to_be_bytes());
    encoded.extend_from_slice(value.as_bytes());
    Ok(())
}

fn put_path(encoded: &mut Vec<u8>, value: &str) -> Result<(), RealmBrokerAuthorityError> {
    if !valid_absolute_path(value) {
        return Err(RealmBrokerAuthorityError::Invalid);
    }
    let length = u16::try_from(value.len()).map_err(|_| RealmBrokerAuthorityError::Invalid)?;
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

    fn take(&mut self, length: usize) -> Result<&'a [u8], RealmBrokerAuthorityError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(RealmBrokerAuthorityError::Truncated)?;
        let bytes = self
            .encoded
            .get(self.offset..end)
            .ok_or(RealmBrokerAuthorityError::Truncated)?;
        self.offset = end;
        Ok(bytes)
    }

    fn string(&mut self) -> Result<String, RealmBrokerAuthorityError> {
        let length = usize::from(self.u16()?);
        if length == 0 || length > MAX_ID_BYTES {
            return Err(RealmBrokerAuthorityError::Invalid);
        }
        std::str::from_utf8(self.take(length)?)
            .map(str::to_owned)
            .map_err(|_| RealmBrokerAuthorityError::Invalid)
    }

    fn path(&mut self) -> Result<String, RealmBrokerAuthorityError> {
        let length = usize::from(self.u16()?);
        if length == 0 || length > MAX_PATH_BYTES {
            return Err(RealmBrokerAuthorityError::Invalid);
        }
        let path = std::str::from_utf8(self.take(length)?)
            .map(str::to_owned)
            .map_err(|_| RealmBrokerAuthorityError::Invalid)?;
        if !valid_absolute_path(&path) {
            return Err(RealmBrokerAuthorityError::Invalid);
        }
        Ok(path)
    }

    fn u16(&mut self) -> Result<u16, RealmBrokerAuthorityError> {
        Ok(u16::from_be_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| RealmBrokerAuthorityError::Truncated)?,
        ))
    }

    fn u32(&mut self) -> Result<u32, RealmBrokerAuthorityError> {
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| RealmBrokerAuthorityError::Truncated)?,
        ))
    }

    fn u64(&mut self) -> Result<u64, RealmBrokerAuthorityError> {
        Ok(u64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| RealmBrokerAuthorityError::Truncated)?,
        ))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], RealmBrokerAuthorityError> {
        self.take(N)?
            .try_into()
            .map_err(|_| RealmBrokerAuthorityError::Truncated)
    }

    fn done(&self) -> bool {
        self.offset == self.encoded.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authority() -> RealmBrokerChildAuthority {
        RealmBrokerChildAuthority {
            realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
            controller_generation: "generation-1".to_owned(),
            broker_process_id: "broker-1".to_owned(),
            session_generation: 7,
            controller_uid: 1000,
            controller_gid: 1000,
            broker_uid: 1001,
            broker_gid: 1001,
            cgroup_digest: [9; 32],
            guest_runtime_digest: [10; 32],
        }
    }

    fn guest_runtime() -> RealmBrokerGuestRuntimeBootstrap {
        let configured_launches = b"configured-launches".to_vec();
        RealmBrokerGuestRuntimeBootstrap {
            realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
            session_generation: 7,
            replay_ledger_path: "/run/d2b/realm/replay.ledger".to_owned(),
            audit_log_path: "/run/d2b/realm/material.audit".to_owned(),
            workloads: vec![RealmBrokerGuestWorkloadBootstrap {
                workload_id: "bbbbbbbbbbbbbbbbbbba".to_owned(),
                parent_static_public_key: [1; 32],
                channel_binding: [2; 32],
                bootstrap_operation_id: [3; 16],
                replay_nonce: [4; 32],
                issued_at_unix_ms: 10_000,
                expires_at_unix_ms: 20_000,
                bootstrap_psk: [5; 32],
                session_storage_ref: "path:workload-guest-session-credential:bbbbbbbbbbbbbbbbbbba"
                    .to_owned(),
                session_path: "/run/d2b/r/work/w/editor/guest-session/d2b-guest-session-v2"
                    .to_owned(),
                configured_storage_ref:
                    "path:workload-configured-launch-credential:bbbbbbbbbbbbbbbbbbba".to_owned(),
                configured_path: "/run/d2b/r/work/w/editor/guest-session/d2b-configured-launch-v2"
                    .to_owned(),
                owner_uid: 0,
                owner_gid: 0,
                mode: 0o440,
                configured_launch_digest: Sha256::digest(&configured_launches).into(),
                configured_launches,
            }],
        }
    }

    #[test]
    fn authority_round_trips_and_debug_is_redacted() {
        let authority = authority();
        assert_eq!(
            RealmBrokerChildAuthority::decode(&authority.encode().unwrap()).unwrap(),
            authority
        );
        assert_eq!(
            format!("{authority:?}"),
            "RealmBrokerChildAuthority(REDACTED)"
        );
    }

    #[test]
    fn authority_rejects_truncation_and_identity_collapse() {
        let encoded = authority().encode().unwrap();
        assert_eq!(
            RealmBrokerChildAuthority::decode(&encoded[..encoded.len() - 1]),
            Err(RealmBrokerAuthorityError::Truncated)
        );
        let mut invalid = authority();
        invalid.broker_uid = 0;
        assert_eq!(invalid.validate(), Err(RealmBrokerAuthorityError::Invalid));
    }

    #[test]
    fn guest_runtime_round_trips_and_rejects_tampering() {
        let runtime = guest_runtime();
        let encoded = runtime.encode().unwrap();
        let decoded = RealmBrokerGuestRuntimeBootstrap::decode(&encoded).unwrap();
        assert_eq!(decoded.realm_id, runtime.realm_id);
        assert_eq!(decoded.session_generation, runtime.session_generation);
        assert_eq!(decoded.workloads.len(), 1);
        assert_eq!(decoded.workloads[0].bootstrap_psk, [5; 32]);
        assert_eq!(
            format!("{decoded:?}"),
            "RealmBrokerGuestRuntimeBootstrap(REDACTED)"
        );

        let mut tampered = encoded;
        let last = tampered.last_mut().unwrap();
        *last ^= 1;
        assert_eq!(
            RealmBrokerGuestRuntimeBootstrap::decode(&tampered).unwrap_err(),
            RealmBrokerAuthorityError::Invalid
        );
    }
}
