use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    os::fd::{FromRawFd, OwnedFd, RawFd},
};

use rustix::{
    io::{FdFlags, fcntl_getfd},
    net::{
        AddressFamily, SocketFlags, SocketType, accept_with,
        sockopt::{get_socket_acceptconn, get_socket_domain, get_socket_type},
    },
};
use tokio::io::unix::AsyncFd;

use crate::SeqpacketSocket;

const SD_LISTEN_FD_START: RawFd = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemdActivationError {
    InvalidEnvironment,
    InvalidDescriptor,
    Accept,
}

impl std::fmt::Display for SystemdActivationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidEnvironment => "socket-activation-environment-invalid",
            Self::InvalidDescriptor => "socket-activation-descriptor-invalid",
            Self::Accept => "socket-activation-accept-failed",
        })
    }
}

impl std::error::Error for SystemdActivationError {}

pub struct ActivatedSeqpacketListener {
    io: AsyncFd<OwnedFd>,
}

pub struct ActivatedSeqpacketListeners {
    listeners: BTreeMap<String, AsyncFd<OwnedFd>>,
}

impl std::fmt::Debug for ActivatedSeqpacketListeners {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ActivatedSeqpacketListeners(<redacted>)")
    }
}

impl ActivatedSeqpacketListeners {
    pub fn from_systemd(expected_names: &[&str]) -> Result<Self, SystemdActivationError> {
        let names = validate_environments(expected_names)?;
        let mut listeners = BTreeMap::new();
        for (index, name) in names.into_iter().enumerate() {
            let fd = SD_LISTEN_FD_START + index as RawFd;
            validate_listener(fd)?;
            let owned = adopt_raw_fd(fd);
            let io = AsyncFd::new(owned).map_err(|_| SystemdActivationError::InvalidDescriptor)?;
            listeners.insert(name, io);
        }
        Ok(Self { listeners })
    }

    pub async fn accept(&self, name: &str) -> Result<SeqpacketSocket, SystemdActivationError> {
        let listener = self
            .listeners
            .get(name)
            .ok_or(SystemdActivationError::InvalidEnvironment)?;
        accept(listener).await
    }
}

impl std::fmt::Debug for ActivatedSeqpacketListener {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ActivatedSeqpacketListener(<redacted>)")
    }
}

impl ActivatedSeqpacketListener {
    pub fn from_systemd(expected_name: &str) -> Result<Self, SystemdActivationError> {
        validate_environments(&[expected_name])?;
        validate_listener(SD_LISTEN_FD_START)?;
        let owned = adopt_raw_fd(SD_LISTEN_FD_START);
        Ok(Self {
            io: AsyncFd::new(owned).map_err(|_| SystemdActivationError::InvalidDescriptor)?,
        })
    }

    pub async fn accept(&self) -> Result<SeqpacketSocket, SystemdActivationError> {
        accept(&self.io).await
    }
}

async fn accept(listener: &AsyncFd<OwnedFd>) -> Result<SeqpacketSocket, SystemdActivationError> {
    loop {
        let mut ready = listener
            .readable()
            .await
            .map_err(|_| SystemdActivationError::Accept)?;
        match ready.try_io(|inner| {
            accept_with(
                inner.get_ref(),
                SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
            )
            .map_err(std::io::Error::from)
        }) {
            Ok(Ok(fd)) => {
                return SeqpacketSocket::from_owned(fd)
                    .map_err(|_| SystemdActivationError::InvalidDescriptor);
            }
            Ok(Err(_)) => return Err(SystemdActivationError::Accept),
            Err(_) => continue,
        }
    }
}

fn validate_environments(expected_names: &[&str]) -> Result<Vec<String>, SystemdActivationError> {
    let expected = expected_names.iter().copied().collect::<BTreeSet<_>>();
    let names = env::var("LISTEN_FDNAMES")
        .ok()
        .map(|value| value.split(':').map(str::to_owned).collect::<Vec<_>>())
        .ok_or(SystemdActivationError::InvalidEnvironment)?;
    if expected_names.is_empty()
        || expected.len() != expected_names.len()
        || names.iter().map(String::as_str).collect::<BTreeSet<_>>() != expected
        || env::var("LISTEN_PID").ok().as_deref() != Some(std::process::id().to_string().as_str())
        || env::var("LISTEN_FDS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            != Some(names.len())
    {
        return Err(SystemdActivationError::InvalidEnvironment);
    }
    Ok(names)
}

fn validate_listener(fd: RawFd) -> Result<(), SystemdActivationError> {
    let borrowed = borrow_raw_fd(fd);
    if get_socket_domain(borrowed).ok() != Some(AddressFamily::UNIX)
        || get_socket_type(borrowed).ok() != Some(SocketType::SEQPACKET)
        || get_socket_acceptconn(borrowed).ok() != Some(true)
        || !fcntl_getfd(borrowed)
            .map(|flags| flags.contains(FdFlags::CLOEXEC))
            .unwrap_or(false)
    {
        return Err(SystemdActivationError::InvalidDescriptor);
    }
    Ok(())
}

#[allow(unsafe_code)]
fn borrow_raw_fd(fd: RawFd) -> std::os::fd::BorrowedFd<'static> {
    // The descriptor remains owned by systemd activation until `adopt_raw_fd`.
    // Validation never stores this temporary borrow.
    unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) }
}

#[allow(unsafe_code)]
fn adopt_raw_fd(fd: RawFd) -> OwnedFd {
    // `validate_environment` proves there is exactly one systemd descriptor and
    // `validate_listener` proves fd 3 is the expected listener before ownership
    // is transferred exactly once.
    unsafe { OwnedFd::from_raw_fd(fd) }
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;

    #[test]
    fn activation_requires_exact_single_named_descriptor() {
        let expected = "component-session";
        let prior = [
            ("LISTEN_PID", env::var_os("LISTEN_PID")),
            ("LISTEN_FDS", env::var_os("LISTEN_FDS")),
            ("LISTEN_FDNAMES", env::var_os("LISTEN_FDNAMES")),
        ];

        unsafe {
            env::set_var("LISTEN_PID", std::process::id().to_string());
            env::set_var("LISTEN_FDS", "1");
            env::set_var("LISTEN_FDNAMES", expected);
        }
        assert_eq!(
            validate_environments(&[expected]),
            Ok(vec![expected.to_owned()])
        );
        unsafe { env::set_var("LISTEN_FDS", "2") };
        assert_eq!(
            validate_environments(&[expected]),
            Err(SystemdActivationError::InvalidEnvironment)
        );

        for (name, value) in prior {
            unsafe {
                match value {
                    Some(value) => env::set_var(name, value),
                    None => env::remove_var(name),
                }
            }
        }
    }
}
