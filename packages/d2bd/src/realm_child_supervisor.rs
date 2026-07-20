//! Pidfd supervision and adoption for paired realm children.

use std::collections::BTreeMap;
use std::fmt;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};

use d2b_host::realm_children::{RealmChildRole, validate_realm_id};

#[derive(Debug)]
pub struct RealmChildHandle {
    pub role: RealmChildRole,
    pub process_id: String,
    pub pid: u32,
    pub pidfd: OwnedFd,
    pub pidfd_identity: PidfdKernelIdentity,
    pub executable: PathBuf,
    pub executable_digest: [u8; 32],
    pub controller_generation_id: String,
    pub cgroup_leaf: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PidfdKernelIdentity {
    pub device: u64,
    pub inode: u64,
}

#[derive(Debug)]
pub struct RealmChildPair {
    pub realm_id: String,
    pub controller: RealmChildHandle,
    pub broker: RealmChildHandle,
}

impl RealmChildPair {
    pub fn validate(&self) -> Result<(), RealmChildSupervisorError> {
        validate_realm_id(&self.realm_id).map_err(|_| RealmChildSupervisorError::InvalidPair)?;
        if self.controller.role != RealmChildRole::Controller
            || self.broker.role != RealmChildRole::Broker
            || self.controller.pid == 0
            || self.broker.pid == 0
            || self.controller.pid == self.broker.pid
            || self.controller.process_id == self.broker.process_id
            || self.controller.controller_generation_id != self.broker.controller_generation_id
            || self.controller.executable_digest == [0; 32]
            || self.broker.executable_digest == [0; 32]
            || self.controller.pidfd_identity.device == 0
            || self.controller.pidfd_identity.inode == 0
            || self.broker.pidfd_identity.device == 0
            || self.broker.pidfd_identity.inode == 0
        {
            return Err(RealmChildSupervisorError::InvalidPair);
        }
        let root = PathBuf::from("/sys/fs/cgroup/d2b.slice").join(format!("r-{}", self.realm_id));
        if self.controller.cgroup_leaf != root.join("controller")
            || self.broker.cgroup_leaf != root.join("broker")
        {
            return Err(RealmChildSupervisorError::InvalidPair);
        }
        validate_pidfd(self.controller.pidfd.as_fd(), self.controller.pid)?;
        validate_pidfd(self.broker.pidfd.as_fd(), self.broker.pid)?;
        if pidfd_kernel_identity(self.controller.pidfd.as_fd())? != self.controller.pidfd_identity
            || pidfd_kernel_identity(self.broker.pidfd.as_fd())? != self.broker.pidfd_identity
        {
            return Err(RealmChildSupervisorError::PidfdIdentityMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct RealmChildSupervisor {
    pairs: BTreeMap<String, RealmChildPair>,
}

impl RealmChildSupervisor {
    pub fn register_pair(&mut self, pair: RealmChildPair) -> Result<(), RealmChildSupervisorError> {
        pair.validate()?;
        if self.pairs.contains_key(&pair.realm_id) {
            return Err(RealmChildSupervisorError::DuplicateRealm);
        }
        self.pairs.insert(pair.realm_id.clone(), pair);
        Ok(())
    }

    pub fn get(&self, realm_id: &str) -> Option<&RealmChildPair> {
        self.pairs.get(realm_id)
    }

    pub fn remove_pair(&mut self, realm_id: &str) -> Option<RealmChildPair> {
        self.pairs.remove(realm_id)
    }

    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    pub fn adopt_pair<V: RealmChildAdoptionVerifier>(
        &mut self,
        candidate: RealmChildAdoptionPair,
        verifier: &V,
    ) -> Result<(), RealmChildSupervisorError> {
        candidate.validate_shape()?;
        let controller_pidfd = open_pidfd(candidate.controller.pid)?;
        let broker_pidfd = open_pidfd(candidate.broker.pid)?;
        verifier.verify(&candidate.controller, controller_pidfd.as_fd())?;
        verifier.verify(&candidate.broker, broker_pidfd.as_fd())?;
        let pair = RealmChildPair {
            realm_id: candidate.realm_id,
            controller: candidate.controller.into_handle(controller_pidfd),
            broker: candidate.broker.into_handle(broker_pidfd),
        };
        self.register_pair(pair)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmChildAdoptionCandidate {
    pub role: RealmChildRole,
    pub process_id: String,
    pub pid: u32,
    pub pidfd_identity: PidfdKernelIdentity,
    pub executable: PathBuf,
    pub executable_digest: [u8; 32],
    pub controller_generation_id: String,
    pub cgroup_leaf: PathBuf,
}

impl RealmChildAdoptionCandidate {
    fn into_handle(self, pidfd: OwnedFd) -> RealmChildHandle {
        RealmChildHandle {
            role: self.role,
            process_id: self.process_id,
            pid: self.pid,
            pidfd,
            pidfd_identity: self.pidfd_identity,
            executable: self.executable,
            executable_digest: self.executable_digest,
            controller_generation_id: self.controller_generation_id,
            cgroup_leaf: self.cgroup_leaf,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmChildAdoptionPair {
    pub realm_id: String,
    pub controller: RealmChildAdoptionCandidate,
    pub broker: RealmChildAdoptionCandidate,
}

impl RealmChildAdoptionPair {
    fn validate_shape(&self) -> Result<(), RealmChildSupervisorError> {
        if self.controller.role != RealmChildRole::Controller
            || self.broker.role != RealmChildRole::Broker
            || self.controller.pid == self.broker.pid
            || self.controller.pid == 0
            || self.broker.pid == 0
            || self.controller.process_id.is_empty()
            || self.broker.process_id.is_empty()
            || self.controller.process_id == self.broker.process_id
            || self.controller.controller_generation_id != self.broker.controller_generation_id
            || self.controller.controller_generation_id.is_empty()
            || !self.controller.executable.is_absolute()
            || !self.broker.executable.is_absolute()
            || self.controller.executable_digest == [0; 32]
            || self.broker.executable_digest == [0; 32]
            || self.controller.pidfd_identity.device == 0
            || self.controller.pidfd_identity.inode == 0
            || self.broker.pidfd_identity.device == 0
            || self.broker.pidfd_identity.inode == 0
        {
            return Err(RealmChildSupervisorError::InvalidPair);
        }
        Ok(())
    }
}

pub trait RealmChildAdoptionVerifier {
    fn verify(
        &self,
        candidate: &RealmChildAdoptionCandidate,
        pidfd: BorrowedFd<'_>,
    ) -> Result<(), RealmChildSupervisorError>;
}

#[derive(Debug, Default)]
pub struct ProcRealmChildAdoptionVerifier;

impl RealmChildAdoptionVerifier for ProcRealmChildAdoptionVerifier {
    fn verify(
        &self,
        candidate: &RealmChildAdoptionCandidate,
        pidfd: BorrowedFd<'_>,
    ) -> Result<(), RealmChildSupervisorError> {
        validate_pidfd(pidfd, candidate.pid)?;
        if pidfd_kernel_identity(pidfd)? != candidate.pidfd_identity {
            return Err(RealmChildSupervisorError::PidfdIdentityMismatch);
        }
        let proc_root = PathBuf::from("/proc").join(candidate.pid.to_string());
        let cgroups = std::fs::read_to_string(proc_root.join("cgroup"))
            .map_err(|_| RealmChildSupervisorError::ProcessMissing)?;
        let expected_cgroup = candidate
            .cgroup_leaf
            .strip_prefix("/sys/fs/cgroup")
            .unwrap_or(&candidate.cgroup_leaf);
        let expected_cgroup = path_text(expected_cgroup).trim_start_matches('/');
        if !cgroups
            .lines()
            .any(|line| line.strip_prefix("0::/") == Some(expected_cgroup))
        {
            return Err(RealmChildSupervisorError::CgroupMismatch);
        }
        validate_pidfd(pidfd, candidate.pid)?;
        Ok(())
    }
}

fn path_text(path: &Path) -> &str {
    path.to_str().unwrap_or("")
}

pub fn pidfd_kernel_identity(
    pidfd: BorrowedFd<'_>,
) -> Result<PidfdKernelIdentity, RealmChildSupervisorError> {
    let stat = rustix::fs::fstat(pidfd).map_err(|_| RealmChildSupervisorError::ProcessMissing)?;
    let identity = PidfdKernelIdentity {
        device: stat.st_dev,
        inode: stat.st_ino,
    };
    if identity.device == 0 || identity.inode == 0 {
        return Err(RealmChildSupervisorError::PidfdIdentityMismatch);
    }
    Ok(identity)
}

fn open_pidfd(pid: u32) -> Result<OwnedFd, RealmChildSupervisorError> {
    let pid = rustix::process::Pid::from_raw(pid as i32)
        .ok_or(RealmChildSupervisorError::ProcessMissing)?;
    rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty())
        .map_err(|_| RealmChildSupervisorError::ProcessMissing)
}

fn validate_pidfd(
    pidfd: std::os::fd::BorrowedFd<'_>,
    expected_pid: u32,
) -> Result<(), RealmChildSupervisorError> {
    let fdinfo = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", pidfd.as_raw_fd()))
        .map_err(|_| RealmChildSupervisorError::ProcessMissing)?;
    let pid = fdinfo
        .lines()
        .find_map(|line| line.strip_prefix("Pid:"))
        .and_then(|value| value.trim().parse::<i64>().ok())
        .ok_or(RealmChildSupervisorError::InvalidPair)?;
    if pid < 0 {
        Err(RealmChildSupervisorError::ProcessMissing)
    } else if pid == i64::from(expected_pid) {
        Ok(())
    } else {
        Err(RealmChildSupervisorError::InvalidPair)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealmChildSupervisorError {
    InvalidPair,
    DuplicateRealm,
    ProcessMissing,
    PidfdIdentityMismatch,
    CgroupMismatch,
}

impl fmt::Display for RealmChildSupervisorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPair => "invalid realm-child pair",
            Self::DuplicateRealm => "realm-child pair already supervised",
            Self::ProcessMissing => "realm-child process is not live",
            Self::PidfdIdentityMismatch => "realm-child pidfd identity mismatch",
            Self::CgroupMismatch => "realm-child cgroup mismatch",
        })
    }
}

impl std::error::Error for RealmChildSupervisorError {}
