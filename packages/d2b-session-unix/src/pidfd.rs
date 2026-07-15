use crate::{FirstPacketCredentials, UnixSessionError};
use rustix::{
    fd::{AsFd, BorrowedFd},
    process::Pid,
};
use std::{fmt, fs, os::fd::AsRawFd, sync::Arc};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PidfdEvidence {
    expected_pid: Pid,
    first_packet_credentials: FirstPacketCredentials,
    executable_digest: [u8; 32],
    cgroup_digest: [u8; 32],
}

impl fmt::Debug for PidfdEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PidfdEvidence(REDACTED)")
    }
}

impl PidfdEvidence {
    pub fn new(
        expected_pid: Pid,
        first_packet_credentials: FirstPacketCredentials,
        executable_digest: [u8; 32],
        cgroup_digest: [u8; 32],
    ) -> Result<Self, UnixSessionError> {
        if first_packet_credentials.pid() != expected_pid
            || executable_digest == [0; 32]
            || cgroup_digest == [0; 32]
        {
            return Err(UnixSessionError::PidfdEvidenceUnavailable);
        }
        Ok(Self {
            expected_pid,
            first_packet_credentials,
            executable_digest,
            cgroup_digest,
        })
    }

    pub fn expected_pid(self) -> Pid {
        self.expected_pid
    }
}

pub trait PidfdIdentityVerifier: Send + Sync {
    fn verify(
        &self,
        pidfd: BorrowedFd<'_>,
        evidence: &PidfdEvidence,
    ) -> Result<(), UnixSessionError>;
}

pub trait PidfdInfoSource: Send + Sync {
    fn read_fdinfo(&self, pidfd: BorrowedFd<'_>) -> Result<String, UnixSessionError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProcSelfFdInfoSource;

impl PidfdInfoSource for ProcSelfFdInfoSource {
    fn read_fdinfo(&self, pidfd: BorrowedFd<'_>) -> Result<String, UnixSessionError> {
        let path = format!("/proc/self/fdinfo/{}", pidfd.as_raw_fd());
        fs::read_to_string(path).map_err(|_| UnixSessionError::PidfdEvidenceUnavailable)
    }
}

pub type DigestEvidenceCallback =
    Arc<dyn Fn(Pid) -> Result<[u8; 32], UnixSessionError> + Send + Sync>;

pub struct ProcPidfdIdentityVerifier<S = ProcSelfFdInfoSource> {
    source: S,
    executable_digest: DigestEvidenceCallback,
    cgroup_digest: DigestEvidenceCallback,
}

impl<S> fmt::Debug for ProcPidfdIdentityVerifier<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProcPidfdIdentityVerifier(REDACTED)")
    }
}

impl<S> ProcPidfdIdentityVerifier<S>
where
    S: PidfdInfoSource,
{
    pub fn new(
        source: S,
        executable_digest: DigestEvidenceCallback,
        cgroup_digest: DigestEvidenceCallback,
    ) -> Self {
        Self {
            source,
            executable_digest,
            cgroup_digest,
        }
    }
}

impl<S> PidfdIdentityVerifier for ProcPidfdIdentityVerifier<S>
where
    S: PidfdInfoSource,
{
    fn verify(
        &self,
        pidfd: BorrowedFd<'_>,
        evidence: &PidfdEvidence,
    ) -> Result<(), UnixSessionError> {
        if evidence.first_packet_credentials.pid() != evidence.expected_pid {
            return Err(UnixSessionError::PidfdIdentityMismatch);
        }
        let contents = self.source.read_fdinfo(pidfd)?;
        let referenced_pid = parse_pidfd_fdinfo(&contents)?;
        if referenced_pid != evidence.expected_pid {
            return Err(UnixSessionError::PidfdIdentityMismatch);
        }
        let executable = (self.executable_digest)(referenced_pid)
            .map_err(|_| UnixSessionError::PidfdEvidenceUnavailable)?;
        let cgroup = (self.cgroup_digest)(referenced_pid)
            .map_err(|_| UnixSessionError::PidfdEvidenceUnavailable)?;
        if executable != evidence.executable_digest || cgroup != evidence.cgroup_digest {
            return Err(UnixSessionError::PidfdIdentityMismatch);
        }
        Ok(())
    }
}

pub fn parse_pidfd_fdinfo(contents: &str) -> Result<Pid, UnixSessionError> {
    let mut parsed = None;
    for line in contents.lines() {
        let Some((name, value)) = line.split_once(':') else {
            return Err(UnixSessionError::PidfdEvidenceUnavailable);
        };
        if name != "Pid" {
            continue;
        }
        if parsed.is_some() {
            return Err(UnixSessionError::PidfdEvidenceUnavailable);
        }
        let value = value.trim();
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(UnixSessionError::PidfdEvidenceUnavailable);
        }
        let raw = value
            .parse::<i32>()
            .map_err(|_| UnixSessionError::PidfdEvidenceUnavailable)?;
        parsed = Pid::from_raw(raw);
        if parsed.is_none() {
            return Err(UnixSessionError::PidfdEvidenceUnavailable);
        }
    }
    parsed.ok_or(UnixSessionError::PidfdEvidenceUnavailable)
}

pub(crate) fn verify_pidfd(
    fd: impl AsFd,
    evidence: &PidfdEvidence,
    verifier: &dyn PidfdIdentityVerifier,
) -> Result<(), UnixSessionError> {
    verifier.verify(fd.as_fd(), evidence)
}
