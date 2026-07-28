use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    os::fd::OwnedFd,
    sync::atomic::{AtomicBool, Ordering},
};

use listenfd::ListenFd;
use rustix::{
    fs::{OFlags, fcntl_getfl, fcntl_setfl},
    io::{FdFlags, fcntl_getfd, fcntl_setfd},
    net::{
        AddressFamily, SocketFlags, SocketType, accept_with,
        sockopt::{get_socket_acceptconn, get_socket_domain, get_socket_type},
    },
};
use tokio::io::unix::AsyncFd;

use crate::SeqpacketSocket;

static ACTIVATION_CONSUMED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemdActivationError {
    InvalidEnvironment,
    InvalidDescriptor,
    Accept,
    AlreadyConsumed,
}

impl std::fmt::Display for SystemdActivationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidEnvironment => "socket-activation-environment-invalid",
            Self::InvalidDescriptor => "socket-activation-descriptor-invalid",
            Self::Accept => "socket-activation-accept-failed",
            Self::AlreadyConsumed => "socket-activation-already-consumed",
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
        claim_activation()?;
        let (names, mut source) = consume_environment(expected_names)?;
        let mut listeners = BTreeMap::new();
        for (index, name) in names.into_iter().enumerate() {
            let owned = source
                .take_custom::<OwnedFd>(
                    index,
                    libc::AF_UNIX,
                    libc::SOCK_SEQPACKET,
                    "Unix seqpacket listener",
                )
                .map_err(|_| SystemdActivationError::InvalidDescriptor)?
                .ok_or(SystemdActivationError::InvalidDescriptor)?;
            prepare_listener(&owned)?;
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
        claim_activation()?;
        let (_, mut source) = consume_environment(&[expected_name])?;
        let owned = source
            .take_custom::<OwnedFd>(
                0,
                libc::AF_UNIX,
                libc::SOCK_SEQPACKET,
                "Unix seqpacket listener",
            )
            .map_err(|_| SystemdActivationError::InvalidDescriptor)?
            .ok_or(SystemdActivationError::InvalidDescriptor)?;
        prepare_listener(&owned)?;
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
            loop {
                match accept_with(
                    inner.get_ref(),
                    SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
                ) {
                    Err(rustix::io::Errno::INTR) => continue,
                    result => break result.map_err(std::io::Error::from),
                }
            }
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

fn claim_activation() -> Result<(), SystemdActivationError> {
    claim_once(&ACTIVATION_CONSUMED)
}

fn claim_once(consumed: &AtomicBool) -> Result<(), SystemdActivationError> {
    consumed
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ())
        .map_err(|_| SystemdActivationError::AlreadyConsumed)
}

fn consume_environment(
    expected_names: &[&str],
) -> Result<(Vec<String>, ListenFd), SystemdActivationError> {
    let listen_pid = env::var("LISTEN_PID").ok();
    let listen_fds = env::var("LISTEN_FDS").ok();
    let listen_fdnames = env::var("LISTEN_FDNAMES").ok();
    let source = ListenFd::from_env();
    envmnt::remove("LISTEN_PID");
    envmnt::remove("LISTEN_FDS");
    envmnt::remove("LISTEN_FDNAMES");
    let names = validate_environment_values(
        expected_names,
        listen_pid.as_deref(),
        listen_fds.as_deref(),
        listen_fdnames.as_deref(),
        std::process::id(),
    )?;
    if source.len() != names.len() {
        return Err(SystemdActivationError::InvalidEnvironment);
    }
    Ok((names, source))
}

fn validate_environment_values(
    expected_names: &[&str],
    listen_pid: Option<&str>,
    listen_fds: Option<&str>,
    listen_fdnames: Option<&str>,
    current_pid: u32,
) -> Result<Vec<String>, SystemdActivationError> {
    let expected = expected_names.iter().copied().collect::<BTreeSet<_>>();
    let names = listen_fdnames
        .map(|value| value.split(':').map(str::to_owned).collect::<Vec<_>>())
        .ok_or(SystemdActivationError::InvalidEnvironment)?;
    let current_pid = current_pid.to_string();
    if expected_names.is_empty()
        || expected.len() != expected_names.len()
        || names.len() != expected_names.len()
        || names.iter().map(String::as_str).collect::<BTreeSet<_>>() != expected
        || listen_pid != Some(current_pid.as_str())
        || listen_fds.and_then(|value| value.parse::<usize>().ok()) != Some(names.len())
    {
        return Err(SystemdActivationError::InvalidEnvironment);
    }
    Ok(names)
}

fn prepare_listener(fd: &OwnedFd) -> Result<(), SystemdActivationError> {
    let descriptor_flags =
        fcntl_getfd(fd).map_err(|_| SystemdActivationError::InvalidDescriptor)?;
    fcntl_setfd(fd, descriptor_flags | FdFlags::CLOEXEC)
        .map_err(|_| SystemdActivationError::InvalidDescriptor)?;
    let flags = fcntl_getfl(fd).map_err(|_| SystemdActivationError::InvalidDescriptor)?;
    fcntl_setfl(fd, flags | OFlags::NONBLOCK)
        .map_err(|_| SystemdActivationError::InvalidDescriptor)?;
    if get_socket_domain(fd).ok() != Some(AddressFamily::UNIX)
        || get_socket_type(fd).ok() != Some(SocketType::SEQPACKET)
        || get_socket_acceptconn(fd).ok() != Some(true)
        || !fcntl_getfd(fd)
            .map(|flags| flags.contains(FdFlags::CLOEXEC))
            .unwrap_or(false)
        || !fcntl_getfl(fd)
            .map(|flags| flags.contains(OFlags::NONBLOCK))
            .unwrap_or(false)
    {
        return Err(SystemdActivationError::InvalidDescriptor);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    use rustix::{
        io::fcntl_setfd,
        net::{SocketAddrUnix, bind_unix, listen, socket_with},
    };

    static LISTENER_ID: AtomicU64 = AtomicU64::new(1);

    fn unix_listener(kind: SocketType) -> OwnedFd {
        let listener = socket_with(AddressFamily::UNIX, kind, SocketFlags::CLOEXEC, None)
            .expect("create Unix listener");
        let name = format!(
            "d2b-session-unix-systemd-{}-{}",
            std::process::id(),
            LISTENER_ID.fetch_add(1, Ordering::Relaxed)
        );
        let address =
            SocketAddrUnix::new_abstract_name(name.as_bytes()).expect("create abstract address");
        bind_unix(&listener, &address).expect("bind Unix listener");
        listen(&listener, 1).expect("listen on Unix socket");
        listener
    }

    #[test]
    fn activation_requires_exact_single_named_descriptor() {
        let expected = "component-session";
        let pid = 42;
        assert_eq!(
            validate_environment_values(&[expected], Some("42"), Some("1"), Some(expected), pid),
            Ok(vec![expected.to_owned()])
        );
        assert_eq!(
            validate_environment_values(&[expected], Some("42"), Some("2"), Some(expected), pid),
            Err(SystemdActivationError::InvalidEnvironment)
        );
        assert_eq!(
            validate_environment_values(
                &[expected],
                Some("42"),
                Some("2"),
                Some("component-session:component-session"),
                pid,
            ),
            Err(SystemdActivationError::InvalidEnvironment)
        );
    }

    #[test]
    fn prepare_listener_sets_real_inherited_flags() {
        let listener = unix_listener(SocketType::SEQPACKET);
        fcntl_setfd(&listener, FdFlags::empty()).expect("clear close-on-exec");
        let flags = fcntl_getfl(&listener).unwrap();
        fcntl_setfl(&listener, flags & !OFlags::NONBLOCK).expect("make listener blocking");
        assert_eq!(prepare_listener(&listener), Ok(()));
        assert!(fcntl_getfd(&listener).unwrap().contains(FdFlags::CLOEXEC));
        assert!(fcntl_getfl(&listener).unwrap().contains(OFlags::NONBLOCK));
    }

    #[test]
    fn validate_listener_rejects_non_unix_domain() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind TCP listener");
        assert_ne!(
            get_socket_domain(&listener).expect("read socket domain"),
            AddressFamily::UNIX
        );
        let listener: OwnedFd = listener.into();
        assert_eq!(
            prepare_listener(&listener),
            Err(SystemdActivationError::InvalidDescriptor)
        );
    }

    #[test]
    fn validate_listener_rejects_non_seqpacket_type() {
        let listener = unix_listener(SocketType::STREAM);
        assert_eq!(
            prepare_listener(&listener),
            Err(SystemdActivationError::InvalidDescriptor)
        );
    }

    #[test]
    fn validate_listener_rejects_socket_without_acceptconn() {
        let socket = socket_with(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )
        .expect("create Unix seqpacket socket");
        assert_eq!(
            prepare_listener(&socket),
            Err(SystemdActivationError::InvalidDescriptor)
        );
    }

    #[test]
    fn activation_can_only_be_claimed_once() {
        let consumed = AtomicBool::new(false);
        assert_eq!(claim_once(&consumed), Ok(()));
        assert_eq!(
            claim_once(&consumed),
            Err(SystemdActivationError::AlreadyConsumed)
        );
    }
}
