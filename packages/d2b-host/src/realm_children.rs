//! Host primitives for paired realm-controller and realm-broker children.
//!
//! The local-root broker receives descriptor metadata separately from the
//! `SCM_RIGHTS` array.  This module keeps the correlation and launch invariants
//! independent of the wire implementation so both the broker and its tests use
//! the same fail-closed rules.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::PathBuf;
use std::time::Duration;

use d2b_contracts::v2_component_session::{AttachmentPolicy, AttachmentPolicyKind, LimitProfile};
use d2b_session_unix::{AncillaryCapacity, CreditPool, CreditScopeSet, SeqpacketSocket};
pub use d2b_session_unix::{
    PidfdEvidence, PidfdIdentityPolicy, PidfdIdentityVerifier, UnixSessionError,
};
use rustix::process::Pid;

use crate::guest_runtime::{
    CONTROLLER_STATIC_IDENTITY_FD_ENV, CONTROLLER_STATIC_IDENTITY_RESOURCE_ID,
};
use crate::realm_broker_bootstrap::{
    REALM_BROKER_AUTHORITY_FD_ENV, REALM_BROKER_AUTHORITY_RESOURCE_ID,
    REALM_BROKER_GUEST_RUNTIME_FD_ENV, REALM_BROKER_GUEST_RUNTIME_RESOURCE_ID,
};
use crate::realm_controller_bootstrap::{
    REALM_CONTROLLER_AUTHORITY_FD_ENV, REALM_CONTROLLER_AUTHORITY_RESOURCE_ID,
};

pub const REALM_CHILD_FD_BASE: i32 = 10;
pub const REALM_CHILD_BOOTSTRAP_MAX_PACKET_BYTES: u32 = 4096;

pub struct RealmChildBootstrapEndpoint {
    socket: SeqpacketSocket,
    child_device: u64,
    child_inode: u64,
}

impl fmt::Debug for RealmChildBootstrapEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RealmChildBootstrapEndpoint(REDACTED)")
    }
}

impl RealmChildBootstrapEndpoint {
    pub fn from_parent_prearmed(
        fd: OwnedFd,
        child_endpoint: impl AsFd,
    ) -> Result<Self, RealmChildCredentialError> {
        let child_stat = rustix::fs::fstat(child_endpoint)
            .map_err(|_| RealmChildCredentialError::ChildEndpointMismatch)?;
        SeqpacketSocket::from_parent_prearmed(fd)
            .map(|socket| Self {
                socket,
                child_device: child_stat.st_dev,
                child_inode: child_stat.st_ino,
            })
            .map_err(RealmChildCredentialError::Session)
    }

    fn matches_child_endpoint(&self, fd: impl AsFd) -> bool {
        rustix::fs::fstat(fd)
            .is_ok_and(|stat| stat.st_dev == self.child_device && stat.st_ino == self.child_inode)
    }

    pub async fn receive_pidfd_evidence(
        &self,
        identity: &RealmChildIdentity,
        pid: u32,
        timeout: Duration,
    ) -> Result<PidfdEvidence, RealmChildCredentialError> {
        let expected = self
            .socket
            .acceptor_peer_credentials()
            .map_err(RealmChildCredentialError::Session)?;
        let expected_pid = i32::try_from(pid)
            .ok()
            .and_then(Pid::from_raw)
            .ok_or(RealmChildCredentialError::InvalidPid)?;
        let policy = AttachmentPolicy {
            kind: AttachmentPolicyKind::PacketAtomic,
            max_per_packet: 1,
            max_per_request: 1,
            max_per_operation: 1,
            max_per_session: 1,
            credentials_allowed: true,
        };
        let capacity =
            AncillaryCapacity::from_policy(policy).map_err(RealmChildCredentialError::Session)?;
        let scopes = bootstrap_credit_scopes()?;
        let mut limits = LimitProfile::local_default();
        limits.protected_ciphertext_bytes = REALM_CHILD_BOOTSTRAP_MAX_PACKET_BYTES;
        let receive = self.socket.recv_burst(limits, capacity, &scopes, 1);
        let mut packets = tokio::time::timeout(timeout, receive)
            .await
            .map_err(|_| RealmChildCredentialError::Timeout)?
            .map_err(RealmChildCredentialError::Session)?
            .packets;
        if packets.len() != 1 {
            return Err(RealmChildCredentialError::MissingFirstPacket);
        }
        let first_packet_credentials = packets
            .pop()
            .expect("packet cardinality checked")
            .verify_first_packet_credentials(expected)
            .map_err(RealmChildCredentialError::Session)?;
        PidfdEvidence::new(
            expected_pid,
            first_packet_credentials,
            identity.executable_digest,
            identity.cgroup_digest,
        )
        .map_err(RealmChildCredentialError::Session)
    }
}

fn bootstrap_credit_scopes() -> Result<CreditScopeSet, RealmChildCredentialError> {
    let pool = || CreditPool::new(1).map_err(|_| RealmChildCredentialError::Credit);
    Ok(CreditScopeSet::new(
        pool()?,
        pool()?,
        pool()?,
        pool()?,
        pool()?,
        pool()?,
    ))
}

#[derive(Debug)]
pub struct RealmChildBootstrapEndpoints {
    pub controller: RealmChildBootstrapEndpoint,
    pub broker: RealmChildBootstrapEndpoint,
}

impl RealmChildBootstrapEndpoints {
    pub fn validate_child_descriptors(
        &self,
        controller: &RealmChildDescriptorSet,
        broker: &RealmChildDescriptorSet,
    ) -> Result<(), RealmChildCredentialError> {
        let controller_fd = controller
            .fd(RealmChildFdKind::BootstrapSession)
            .ok_or(RealmChildCredentialError::ChildEndpointMismatch)?;
        let broker_fd = broker
            .fd(RealmChildFdKind::BootstrapSession)
            .ok_or(RealmChildCredentialError::ChildEndpointMismatch)?;
        if !self.controller.matches_child_endpoint(controller_fd)
            || !self.broker.matches_child_endpoint(broker_fd)
        {
            return Err(RealmChildCredentialError::ChildEndpointMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealmChildCredentialError {
    InvalidPid,
    MissingFirstPacket,
    Timeout,
    Credit,
    ChildEndpointMismatch,
    Session(UnixSessionError),
}

impl fmt::Display for RealmChildCredentialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPid => "realm-child-invalid-pid",
            Self::MissingFirstPacket => "realm-child-missing-first-packet",
            Self::Timeout => "realm-child-credential-timeout",
            Self::Credit => "realm-child-credential-credit",
            Self::ChildEndpointMismatch => "realm-child-bootstrap-endpoint-mismatch",
            Self::Session(error) => return error.fmt(formatter),
        })
    }
}

impl std::error::Error for RealmChildCredentialError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RealmChildRole {
    Controller,
    Broker,
}

impl RealmChildRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Controller => "controller",
            Self::Broker => "broker",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RealmChildFdKind {
    PublicListener,
    BrokerListener,
    UserNamespace,
    MountNamespace,
    NetworkNamespace,
    IpcNamespace,
    PidNamespace,
    CgroupNamespace,
    CgroupLeaf,
    StateRoot,
    AuditRoot,
    Resource,
    Lease,
    BootstrapSession,
}

impl RealmChildFdKind {
    pub const fn is_singleton(self) -> bool {
        !matches!(self, Self::Resource | Self::Lease)
    }

    pub const fn as_env_key(self) -> &'static str {
        match self {
            Self::PublicListener => "D2B_PUBLIC_LISTENER_FD",
            Self::BrokerListener => "D2B_BROKER_LISTENER_FD",
            Self::UserNamespace => "D2B_USER_NAMESPACE_FD",
            Self::MountNamespace => "D2B_MOUNT_NAMESPACE_FD",
            Self::NetworkNamespace => "D2B_NETWORK_NAMESPACE_FD",
            Self::IpcNamespace => "D2B_IPC_NAMESPACE_FD",
            Self::PidNamespace => "D2B_PID_NAMESPACE_FD",
            Self::CgroupNamespace => "D2B_CGROUP_NAMESPACE_FD",
            Self::CgroupLeaf => "D2B_CGROUP_LEAF_FD",
            Self::StateRoot => "D2B_STATE_ROOT_FD",
            Self::AuditRoot => "D2B_AUDIT_ROOT_FD",
            Self::Resource => "D2B_RESOURCE_FD",
            Self::Lease => "D2B_LEASE_FD",
            Self::BootstrapSession => "D2B_BOOTSTRAP_SESSION_FD",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmChildFdBinding {
    pub role: RealmChildRole,
    pub kind: RealmChildFdKind,
    pub attachment_index: u32,
    pub resource_id: Option<String>,
}

#[derive(Debug)]
pub struct CorrelatedRealmChildFd {
    pub binding: RealmChildFdBinding,
    pub fd: OwnedFd,
}

#[derive(Debug)]
pub struct RealmChildFdTable {
    entries: Vec<CorrelatedRealmChildFd>,
}

impl RealmChildFdTable {
    pub fn correlate(
        bindings: Vec<RealmChildFdBinding>,
        attachments: Vec<OwnedFd>,
    ) -> Result<Self, RealmChildPlanError> {
        if bindings.len() != attachments.len() {
            return Err(RealmChildPlanError::AttachmentCount {
                metadata: bindings.len(),
                descriptors: attachments.len(),
            });
        }

        let mut by_index = attachments
            .into_iter()
            .enumerate()
            .map(|(index, fd)| (index as u32, fd))
            .collect::<BTreeMap<_, _>>();
        let mut singleton = BTreeSet::new();
        let mut entries = Vec::with_capacity(bindings.len());

        for binding in bindings {
            if binding.kind.is_singleton() {
                if binding.resource_id.is_some() {
                    return Err(RealmChildPlanError::UnexpectedResourceId { kind: binding.kind });
                }
                if !singleton.insert((binding.role, binding.kind)) {
                    return Err(RealmChildPlanError::DuplicateSingleton {
                        role: binding.role,
                        kind: binding.kind,
                    });
                }
            } else {
                let id = binding.resource_id.as_deref().unwrap_or_default();
                if id.is_empty() || id.len() > 128 {
                    return Err(RealmChildPlanError::InvalidResourceId);
                }
            }
            let fd = by_index.remove(&binding.attachment_index).ok_or(
                RealmChildPlanError::AttachmentIndex {
                    index: binding.attachment_index,
                },
            )?;
            entries.push(CorrelatedRealmChildFd { binding, fd });
        }

        if let Some(index) = by_index.keys().next().copied() {
            return Err(RealmChildPlanError::AttachmentIndex { index });
        }
        entries.sort_by_key(|entry| entry.binding.attachment_index);
        Ok(Self { entries })
    }

    pub fn into_role(
        self,
        role: RealmChildRole,
    ) -> Result<RealmChildDescriptorSet, RealmChildPlanError> {
        let mut selected = self
            .entries
            .into_iter()
            .filter(|entry| entry.binding.role == role)
            .collect::<Vec<_>>();
        validate_required_descriptors(role, &selected)?;
        selected.sort_by_key(|entry| entry.binding.attachment_index);
        Ok(RealmChildDescriptorSet {
            role,
            entries: selected,
        })
    }

    pub fn into_pair(
        self,
    ) -> Result<(RealmChildDescriptorSet, RealmChildDescriptorSet), RealmChildPlanError> {
        let mut controller = Vec::new();
        let mut broker = Vec::new();
        for entry in self.entries {
            match entry.binding.role {
                RealmChildRole::Controller => controller.push(entry),
                RealmChildRole::Broker => broker.push(entry),
            }
        }
        validate_required_descriptors(RealmChildRole::Controller, &controller)?;
        validate_required_descriptors(RealmChildRole::Broker, &broker)?;
        controller.sort_by_key(|entry| entry.binding.attachment_index);
        broker.sort_by_key(|entry| entry.binding.attachment_index);
        Ok((
            RealmChildDescriptorSet {
                role: RealmChildRole::Controller,
                entries: controller,
            },
            RealmChildDescriptorSet {
                role: RealmChildRole::Broker,
                entries: broker,
            },
        ))
    }
}

fn validate_required_descriptors(
    role: RealmChildRole,
    entries: &[CorrelatedRealmChildFd],
) -> Result<(), RealmChildPlanError> {
    let listener = match role {
        RealmChildRole::Controller => RealmChildFdKind::PublicListener,
        RealmChildRole::Broker => RealmChildFdKind::BrokerListener,
    };
    for kind in [
        listener,
        RealmChildFdKind::BootstrapSession,
        RealmChildFdKind::CgroupLeaf,
    ] {
        if !entries.iter().any(|entry| entry.binding.kind == kind) {
            return Err(RealmChildPlanError::MissingRequired { role, kind });
        }
    }
    if role == RealmChildRole::Controller
        && !entries.iter().any(|entry| {
            entry.binding.kind == RealmChildFdKind::Resource
                && entry.binding.resource_id.as_deref()
                    == Some(CONTROLLER_STATIC_IDENTITY_RESOURCE_ID)
        })
    {
        return Err(RealmChildPlanError::MissingControllerStaticIdentity);
    }
    Ok(())
}

#[derive(Debug)]
pub struct RealmChildDescriptorSet {
    role: RealmChildRole,
    entries: Vec<CorrelatedRealmChildFd>,
}

impl RealmChildDescriptorSet {
    pub const fn role(&self) -> RealmChildRole {
        self.role
    }

    pub fn entries(&self) -> &[CorrelatedRealmChildFd] {
        &self.entries
    }

    pub fn fd(&self, kind: RealmChildFdKind) -> Option<BorrowedFd<'_>> {
        self.entries
            .iter()
            .find(|entry| entry.binding.kind == kind)
            .map(|entry| entry.fd.as_fd())
    }

    pub fn resource_fd(&self, resource_id: &str) -> Option<BorrowedFd<'_>> {
        self.entries
            .iter()
            .find(|entry| {
                entry.binding.kind == RealmChildFdKind::Resource
                    && entry.binding.resource_id.as_deref() == Some(resource_id)
            })
            .map(|entry| entry.fd.as_fd())
    }

    pub fn install_allocator_resource(
        &mut self,
        resource_id: &str,
        fd: OwnedFd,
    ) -> Result<(), RealmChildPlanError> {
        let allowed = matches!(
            (self.role, resource_id),
            (RealmChildRole::Broker, REALM_BROKER_AUTHORITY_RESOURCE_ID)
                | (
                    RealmChildRole::Controller,
                    REALM_CONTROLLER_AUTHORITY_RESOURCE_ID
                )
        );
        if !allowed
            || self.entries.iter().any(|entry| {
                entry.binding.kind == RealmChildFdKind::Resource
                    && entry.binding.resource_id.as_deref() == Some(resource_id)
            })
        {
            return Err(RealmChildPlanError::DuplicateAllocatorResource);
        }
        let attachment_index = self
            .entries
            .iter()
            .map(|entry| entry.binding.attachment_index)
            .max()
            .and_then(|index| index.checked_add(1))
            .ok_or(RealmChildPlanError::AttachmentIndex { index: u32::MAX })?;
        self.entries.push(CorrelatedRealmChildFd {
            binding: RealmChildFdBinding {
                role: self.role,
                kind: RealmChildFdKind::Resource,
                attachment_index,
                resource_id: Some(resource_id.to_owned()),
            },
            fd,
        });
        Ok(())
    }

    pub fn into_launch_parts(
        self,
    ) -> Result<(OwnedFd, Vec<OwnedFd>, Vec<String>), RealmChildPlanError> {
        let mut cgroup_leaf = None;
        let mut inherited = Vec::new();
        let mut env = Vec::new();
        let mut repeated = BTreeMap::<RealmChildFdKind, usize>::new();

        let role = self.role;
        for entry in self.entries {
            let target = REALM_CHILD_FD_BASE + i32::try_from(inherited.len()).unwrap_or(i32::MAX);
            let suffix = repeated.entry(entry.binding.kind).or_default();
            let key = if entry.binding.kind.is_singleton() {
                entry.binding.kind.as_env_key().to_owned()
            } else {
                let key = format!("{}_{}", entry.binding.kind.as_env_key(), *suffix);
                *suffix += 1;
                key
            };
            env.push(format!("{key}={target}"));
            if let Some(resource_id) = entry.binding.resource_id {
                if role == RealmChildRole::Controller
                    && entry.binding.kind == RealmChildFdKind::Resource
                    && resource_id == CONTROLLER_STATIC_IDENTITY_RESOURCE_ID
                {
                    env.push(format!("{CONTROLLER_STATIC_IDENTITY_FD_ENV}={target}"));
                }
                if role == RealmChildRole::Controller
                    && entry.binding.kind == RealmChildFdKind::Resource
                    && resource_id == REALM_CONTROLLER_AUTHORITY_RESOURCE_ID
                {
                    env.push(format!("{REALM_CONTROLLER_AUTHORITY_FD_ENV}={target}"));
                }
                if role == RealmChildRole::Broker
                    && entry.binding.kind == RealmChildFdKind::Resource
                    && resource_id == REALM_BROKER_AUTHORITY_RESOURCE_ID
                {
                    env.push(format!("{REALM_BROKER_AUTHORITY_FD_ENV}={target}"));
                }
                if role == RealmChildRole::Broker
                    && entry.binding.kind == RealmChildFdKind::Resource
                    && resource_id == REALM_BROKER_GUEST_RUNTIME_RESOURCE_ID
                {
                    env.push(format!("{REALM_BROKER_GUEST_RUNTIME_FD_ENV}={target}"));
                }
                env.push(format!("{key}_ID={resource_id}"));
            }
            if entry.binding.kind == RealmChildFdKind::CgroupLeaf {
                cgroup_leaf = Some(entry.fd.try_clone().map_err(RealmChildPlanError::CloneFd)?);
            }
            inherited.push(entry.fd);
        }

        let cgroup_leaf = cgroup_leaf.ok_or(RealmChildPlanError::MissingRequired {
            role: self.role,
            kind: RealmChildFdKind::CgroupLeaf,
        })?;
        Ok((cgroup_leaf, inherited, env))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmChildIdentity {
    pub role: RealmChildRole,
    pub process_id: String,
    pub executable: PathBuf,
    pub executable_digest: [u8; 32],
    pub cgroup_digest: [u8; 32],
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmChildLaunchRecord {
    pub realm_id: String,
    pub controller_generation_id: String,
    pub launch_record_digest: [u8; 32],
    pub controller: RealmChildIdentity,
    pub broker: RealmChildIdentity,
}

impl RealmChildLaunchRecord {
    pub fn system_user(&self, role: RealmChildRole) -> String {
        format!(
            "{}-r-{}",
            match role {
                RealmChildRole::Controller => "d2bd",
                RealmChildRole::Broker => "d2bbr",
            },
            self.realm_id
        )
    }

    pub fn cgroup_group(&self) -> String {
        format!("d2bcg-r-{}", self.realm_id)
    }

    pub fn public_group(&self) -> String {
        format!("d2b-r-{}", self.realm_id)
    }

    pub fn validate_for_request(
        &self,
        realm_id: &str,
        controller_generation_id: &str,
        controller_process_id: &str,
        broker_process_id: &str,
        launch_record_digest: &[u8],
    ) -> Result<(), RealmChildPlanError> {
        validate_realm_id(realm_id)?;
        if self.realm_id != realm_id
            || self.controller_generation_id != controller_generation_id
            || self.controller.process_id != controller_process_id
            || self.broker.process_id != broker_process_id
            || self.launch_record_digest.as_slice() != launch_record_digest
            || self.controller.role != RealmChildRole::Controller
            || self.broker.role != RealmChildRole::Broker
        {
            return Err(RealmChildPlanError::LaunchRecordMismatch);
        }
        if self.controller.uid == 0
            || self.broker.uid == 0
            || self.controller.uid == self.broker.uid
        {
            return Err(RealmChildPlanError::InvalidIdentity);
        }
        if self.controller.executable.as_os_str().is_empty()
            || !self.controller.executable.is_absolute()
            || self.broker.executable.as_os_str().is_empty()
            || !self.broker.executable.is_absolute()
        {
            return Err(RealmChildPlanError::InvalidExecutable);
        }
        if self.controller.executable_digest == [0; 32]
            || self.controller.cgroup_digest == [0; 32]
            || self.broker.executable_digest == [0; 32]
            || self.broker.cgroup_digest == [0; 32]
        {
            return Err(RealmChildPlanError::MissingIdentityDigest);
        }
        Ok(())
    }

    pub fn cgroup_leaf(&self, role: RealmChildRole) -> PathBuf {
        PathBuf::from("/sys/fs/cgroup/d2b.slice")
            .join(format!("r-{}", self.realm_id))
            .join(role.as_str())
    }

    pub fn runtime_socket(&self, role: RealmChildRole) -> PathBuf {
        PathBuf::from("/run/d2b/r")
            .join(&self.realm_id)
            .join(match role {
                RealmChildRole::Controller => "public.sock",
                RealmChildRole::Broker => "broker.sock",
            })
    }
}

pub fn validate_realm_id(realm_id: &str) -> Result<(), RealmChildPlanError> {
    let mut chars = realm_id.chars();
    if realm_id.len() > 20
        || !chars.next().is_some_and(|ch| ch.is_ascii_lowercase())
        || !chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        return Err(RealmChildPlanError::InvalidRealmId);
    }
    Ok(())
}

#[derive(Debug)]
pub enum RealmChildPlanError {
    InvalidRealmId,
    InvalidResourceId,
    InvalidIdentity,
    InvalidExecutable,
    MissingIdentityDigest,
    MissingControllerStaticIdentity,
    DuplicateAllocatorResource,
    LaunchRecordMismatch,
    AttachmentCount {
        metadata: usize,
        descriptors: usize,
    },
    AttachmentIndex {
        index: u32,
    },
    UnexpectedResourceId {
        kind: RealmChildFdKind,
    },
    DuplicateSingleton {
        role: RealmChildRole,
        kind: RealmChildFdKind,
    },
    MissingRequired {
        role: RealmChildRole,
        kind: RealmChildFdKind,
    },
    CloneFd(std::io::Error),
}

impl fmt::Display for RealmChildPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRealmId => formatter.write_str("invalid realm id"),
            Self::InvalidResourceId => formatter.write_str("invalid resource or lease id"),
            Self::InvalidIdentity => {
                formatter.write_str("realm children require distinct non-root identities")
            }
            Self::InvalidExecutable => formatter.write_str("child executable must be absolute"),
            Self::MissingIdentityDigest => {
                formatter.write_str("child executable and cgroup digests must be present")
            }
            Self::MissingControllerStaticIdentity => {
                formatter.write_str("controller static identity descriptor is missing")
            }
            Self::DuplicateAllocatorResource => {
                formatter.write_str("allocator realm-broker authority resource is invalid")
            }
            Self::LaunchRecordMismatch => formatter.write_str("trusted launch record mismatch"),
            Self::AttachmentCount {
                metadata,
                descriptors,
            } => write!(
                formatter,
                "attachment metadata count {metadata} does not equal descriptor count {descriptors}"
            ),
            Self::AttachmentIndex { index } => {
                write!(
                    formatter,
                    "attachment index {index} is not exactly correlated"
                )
            }
            Self::UnexpectedResourceId { kind } => {
                write!(
                    formatter,
                    "singleton descriptor {kind:?} carries a resource id"
                )
            }
            Self::DuplicateSingleton { role, kind } => {
                write!(formatter, "duplicate {role:?} {kind:?} descriptor")
            }
            Self::MissingRequired { role, kind } => {
                write!(formatter, "missing required {role:?} {kind:?} descriptor")
            }
            Self::CloneFd(error) => write!(formatter, "could not duplicate cgroup fd: {error}"),
        }
    }
}

impl std::error::Error for RealmChildPlanError {}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use super::*;

    fn descriptor(
        kind: RealmChildFdKind,
        index: u32,
        resource_id: Option<&str>,
    ) -> RealmChildFdBinding {
        RealmChildFdBinding {
            role: RealmChildRole::Controller,
            kind,
            attachment_index: index,
            resource_id: resource_id.map(str::to_owned),
        }
    }

    fn fd() -> OwnedFd {
        File::open("/dev/null").unwrap().into()
    }

    #[test]
    fn controller_launch_exports_fixed_static_identity_fd_binding() {
        let table = RealmChildFdTable::correlate(
            vec![
                descriptor(RealmChildFdKind::PublicListener, 0, None),
                descriptor(RealmChildFdKind::BootstrapSession, 1, None),
                descriptor(RealmChildFdKind::CgroupLeaf, 2, None),
                descriptor(
                    RealmChildFdKind::Resource,
                    3,
                    Some(CONTROLLER_STATIC_IDENTITY_RESOURCE_ID),
                ),
            ],
            vec![fd(), fd(), fd(), fd()],
        )
        .unwrap();
        let mut controller = table.into_role(RealmChildRole::Controller).unwrap();
        controller
            .install_allocator_resource(REALM_CONTROLLER_AUTHORITY_RESOURCE_ID, fd())
            .unwrap();
        let (_, inherited, env) = controller.into_launch_parts().unwrap();
        assert_eq!(inherited.len(), 5);
        assert!(
            env.iter()
                .any(|entry| entry == "D2B_CONTROLLER_STATIC_IDENTITY_FD=13")
        );
        assert!(
            env.iter()
                .any(|entry| { entry == "D2B_RESOURCE_FD_0_ID=controller-static-identity-v2" })
        );
        assert!(
            env.iter()
                .any(|entry| entry == "D2B_REALM_CONTROLLER_AUTHORITY_FD=14")
        );
        assert!(
            env.iter()
                .any(|entry| { entry == "D2B_RESOURCE_FD_1_ID=realm-controller-authority-v1" })
        );
    }

    #[test]
    fn controller_launch_without_static_identity_fails_closed() {
        let error = RealmChildFdTable::correlate(
            vec![
                descriptor(RealmChildFdKind::PublicListener, 0, None),
                descriptor(RealmChildFdKind::BootstrapSession, 1, None),
                descriptor(RealmChildFdKind::CgroupLeaf, 2, None),
            ],
            vec![fd(), fd(), fd()],
        )
        .unwrap()
        .into_role(RealmChildRole::Controller)
        .unwrap_err();
        assert!(matches!(
            error,
            RealmChildPlanError::MissingControllerStaticIdentity
        ));
    }

    #[test]
    fn broker_launch_exports_allocator_issued_authority_fd() {
        let mut bindings = [
            RealmChildFdKind::BrokerListener,
            RealmChildFdKind::BootstrapSession,
            RealmChildFdKind::CgroupLeaf,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, kind)| RealmChildFdBinding {
            role: RealmChildRole::Broker,
            kind,
            attachment_index: index as u32,
            resource_id: None,
        })
        .collect::<Vec<_>>();
        bindings.push(RealmChildFdBinding {
            role: RealmChildRole::Broker,
            kind: RealmChildFdKind::Resource,
            attachment_index: 3,
            resource_id: Some(REALM_BROKER_GUEST_RUNTIME_RESOURCE_ID.to_owned()),
        });
        let mut broker = RealmChildFdTable::correlate(bindings, vec![fd(), fd(), fd(), fd()])
            .unwrap()
            .into_role(RealmChildRole::Broker)
            .unwrap();
        broker
            .install_allocator_resource(REALM_BROKER_AUTHORITY_RESOURCE_ID, fd())
            .unwrap();
        let (_, inherited, env) = broker.into_launch_parts().unwrap();
        assert_eq!(inherited.len(), 5);
        assert!(
            env.iter()
                .any(|entry| entry == "D2B_REALM_BROKER_GUEST_RUNTIME_FD=13")
        );
        assert!(
            env.iter()
                .any(|entry| entry == "D2B_REALM_BROKER_AUTHORITY_FD=14")
        );
        assert!(
            env.iter()
                .any(|entry| { entry == "D2B_RESOURCE_FD_0_ID=realm-broker-guest-runtime-v1" })
        );
        assert!(
            env.iter()
                .any(|entry| { entry == "D2B_RESOURCE_FD_1_ID=realm-broker-authority-v1" })
        );
    }
}
