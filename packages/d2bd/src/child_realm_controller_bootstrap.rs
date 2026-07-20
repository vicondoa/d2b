//! Allocator-owned bootstrap authority for parent-spawned realm controllers.

use std::{
    env,
    ffi::{OsStr, OsString},
    fmt,
    io::IoSlice,
    os::fd::{AsFd, AsRawFd, OwnedFd, RawFd},
};

use d2b_host::realm_children::REALM_CHILD_BOOTSTRAP_MAX_PACKET_BYTES;
use nix::{
    fcntl::{FcntlArg, FdFlag, fcntl},
    sys::socket::{UnixAddr, getpeername, getsockopt, sockopt::AcceptConn},
    unistd,
};
use rustix::{
    fs::{FileType, fstat},
    net::{
        AddressFamily, SendAncillaryBuffer, SendAncillaryMessage, SendFlags, SocketType, UCred,
        sendmsg,
        sockopt::{get_socket_domain, get_socket_type},
    },
    process::{getgid, getpid, getuid},
};
use socket2::Socket;

use crate::{TypedError, duplicate_fd_cloexec};

const CHILD_ROLE_ENV: &str = "D2B_CHILD_ROLE";
const PUBLIC_LISTENER_FD_ENV: &str = "D2B_PUBLIC_LISTENER_FD";
const BOOTSTRAP_SESSION_FD_ENV: &str = "D2B_BOOTSTRAP_SESSION_FD";
const CGROUP_LEAF_FD_ENV: &str = "D2B_CGROUP_LEAF_FD";
const READY_PACKET: &[u8] = b"ready";
const _: () = assert!(READY_PACKET.len() <= REALM_CHILD_BOOTSTRAP_MAX_PACKET_BYTES as usize);

pub(crate) struct ControllerBootstrap {
    child: Option<ChildControllerAuthority>,
}

struct ChildControllerAuthority {
    listener: Option<Socket>,
    bootstrap: OwnedFd,
    _cgroup: OwnedFd,
    ready_sent: bool,
}

impl fmt::Debug for ControllerBootstrap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(if self.child.is_some() {
            "ControllerBootstrap::Child(REDACTED)"
        } else {
            "ControllerBootstrap::LocalRoot"
        })
    }
}

impl ControllerBootstrap {
    pub(crate) fn from_environment() -> Result<Self, ControllerBootstrapError> {
        Self::from_snapshot(EnvironmentSnapshot::capture())
    }

    fn from_snapshot(snapshot: EnvironmentSnapshot) -> Result<Self, ControllerBootstrapError> {
        match snapshot.role.as_deref() {
            None if !snapshot.has_descriptor() => return Ok(Self { child: None }),
            None => return Err(ControllerBootstrapError::PartialDescriptors),
            Some(role) if role.to_str() != Some("controller") => {
                return Err(ControllerBootstrapError::InvalidRole);
            }
            Some(_) => {}
        }
        if snapshot.systemd_activation {
            return Err(ControllerBootstrapError::SystemdActivation);
        }

        let listener_raw = parse_fd(snapshot.listener.as_deref())?;
        let bootstrap_raw = parse_fd(snapshot.bootstrap.as_deref())?;
        let cgroup_raw = parse_fd(snapshot.cgroup.as_deref())?;
        let mut distinct = [listener_raw, bootstrap_raw, cgroup_raw];
        distinct.sort_unstable();
        if distinct.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ControllerBootstrapError::DescriptorAlias);
        }

        let inherited = [
            InheritedFd(listener_raw),
            InheritedFd(bootstrap_raw),
            InheritedFd(cgroup_raw),
        ];
        let listener = duplicate_fd_cloexec(
            inherited[0].as_raw_fd(),
            "adopt child controller public listener",
        )
        .map_err(|_| ControllerBootstrapError::DescriptorAdoption)?;
        let bootstrap = duplicate_fd_cloexec(
            inherited[1].as_raw_fd(),
            "adopt child controller bootstrap session",
        )
        .map_err(|_| ControllerBootstrapError::DescriptorAdoption)?;
        let cgroup = duplicate_fd_cloexec(
            inherited[2].as_raw_fd(),
            "adopt child controller cgroup leaf",
        )
        .map_err(|_| ControllerBootstrapError::DescriptorAdoption)?;
        drop(inherited);

        validate_listener(&listener)?;
        validate_bootstrap_session(&bootstrap)?;
        validate_cgroup(&cgroup)?;

        Ok(Self {
            child: Some(ChildControllerAuthority {
                listener: Some(Socket::from(listener)),
                bootstrap,
                _cgroup: cgroup,
                ready_sent: false,
            }),
        })
    }

    pub(crate) fn public_listener_or_else<E>(
        &mut self,
        bind_local_root: impl FnOnce() -> Result<Socket, E>,
    ) -> Result<Socket, E>
    where
        E: From<ControllerBootstrapError>,
    {
        let Some(child) = self.child.as_mut() else {
            return bind_local_root();
        };
        child
            .listener
            .take()
            .ok_or(ControllerBootstrapError::ListenerAlreadyTaken)
            .map_err(E::from)
    }

    pub(crate) fn send_ready(&mut self) -> Result<(), ControllerBootstrapError> {
        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };
        if child.ready_sent {
            return Err(ControllerBootstrapError::ReadyAlreadySent);
        }

        let credentials = UCred {
            pid: getpid(),
            uid: getuid(),
            gid: getgid(),
        };
        let mut control_bytes = vec![0_u8; rustix::cmsg_space!(ScmCredentials(1))];
        let mut control = SendAncillaryBuffer::new(&mut control_bytes);
        if !control.push(SendAncillaryMessage::ScmCredentials(credentials)) {
            return Err(ControllerBootstrapError::ReadySend);
        }
        let sent = sendmsg(
            child.bootstrap.as_fd(),
            &[IoSlice::new(READY_PACKET)],
            &mut control,
            SendFlags::DONTWAIT | SendFlags::NOSIGNAL,
        )
        .map_err(|_| ControllerBootstrapError::ReadySend)?;
        if sent != READY_PACKET.len() {
            return Err(ControllerBootstrapError::ReadySend);
        }
        child.ready_sent = true;
        Ok(())
    }
}

#[derive(Default)]
struct EnvironmentSnapshot {
    role: Option<OsString>,
    listener: Option<OsString>,
    bootstrap: Option<OsString>,
    cgroup: Option<OsString>,
    systemd_activation: bool,
}

impl EnvironmentSnapshot {
    fn capture() -> Self {
        Self {
            role: env::var_os(CHILD_ROLE_ENV),
            listener: env::var_os(PUBLIC_LISTENER_FD_ENV),
            bootstrap: env::var_os(BOOTSTRAP_SESSION_FD_ENV),
            cgroup: env::var_os(CGROUP_LEAF_FD_ENV),
            systemd_activation: ["LISTEN_FDS", "LISTEN_PID", "LISTEN_FDNAMES"]
                .iter()
                .any(|name| env::var_os(name).is_some()),
        }
    }

    fn has_descriptor(&self) -> bool {
        self.listener.is_some() || self.bootstrap.is_some() || self.cgroup.is_some()
    }
}

struct InheritedFd(RawFd);

impl AsRawFd for InheritedFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

impl Drop for InheritedFd {
    fn drop(&mut self) {
        let _ = unistd::close(self.0);
    }
}

fn parse_fd(value: Option<&OsStr>) -> Result<RawFd, ControllerBootstrapError> {
    let value = value.ok_or(ControllerBootstrapError::PartialDescriptors)?;
    let value = value
        .to_str()
        .ok_or(ControllerBootstrapError::InvalidDescriptor)?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ControllerBootstrapError::InvalidDescriptor);
    }
    value
        .parse::<RawFd>()
        .ok()
        .filter(|fd| (3..=4096).contains(fd))
        .ok_or(ControllerBootstrapError::InvalidDescriptor)
}

fn validate_cloexec(fd: &OwnedFd) -> Result<(), ControllerBootstrapError> {
    let flags = fcntl(fd.as_raw_fd(), FcntlArg::F_GETFD)
        .map_err(|_| ControllerBootstrapError::DescriptorAdoption)?;
    if FdFlag::from_bits_truncate(flags).contains(FdFlag::FD_CLOEXEC) {
        Ok(())
    } else {
        Err(ControllerBootstrapError::DescriptorAdoption)
    }
}

fn validate_listener(fd: &OwnedFd) -> Result<(), ControllerBootstrapError> {
    validate_cloexec(fd)?;
    let valid = get_socket_domain(fd).ok() == Some(AddressFamily::UNIX)
        && get_socket_type(fd).ok() == Some(SocketType::SEQPACKET)
        && getsockopt(fd, AcceptConn).ok() == Some(true);
    if valid {
        Ok(())
    } else {
        Err(ControllerBootstrapError::InvalidListener)
    }
}

fn validate_bootstrap_session(fd: &OwnedFd) -> Result<(), ControllerBootstrapError> {
    validate_cloexec(fd)?;
    let valid = get_socket_domain(fd).ok() == Some(AddressFamily::UNIX)
        && get_socket_type(fd).ok() == Some(SocketType::SEQPACKET)
        && getsockopt(fd, AcceptConn).ok() == Some(false)
        && getpeername::<UnixAddr>(fd.as_raw_fd()).is_ok();
    if valid {
        Ok(())
    } else {
        Err(ControllerBootstrapError::InvalidBootstrapSession)
    }
}

fn validate_cgroup(fd: &OwnedFd) -> Result<(), ControllerBootstrapError> {
    validate_cloexec(fd)?;
    if fstat(fd).is_ok_and(|stat| FileType::from_raw_mode(stat.st_mode) == FileType::Directory) {
        Ok(())
    } else {
        Err(ControllerBootstrapError::InvalidCgroup)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControllerBootstrapError {
    InvalidRole,
    PartialDescriptors,
    InvalidDescriptor,
    DescriptorAlias,
    DescriptorAdoption,
    SystemdActivation,
    InvalidListener,
    InvalidBootstrapSession,
    InvalidCgroup,
    ListenerAlreadyTaken,
    ReadyAlreadySent,
    ReadySend,
}

impl fmt::Display for ControllerBootstrapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRole => "child-controller-role-invalid",
            Self::PartialDescriptors => "child-controller-descriptors-partial",
            Self::InvalidDescriptor => "child-controller-descriptor-invalid",
            Self::DescriptorAlias => "child-controller-descriptor-alias",
            Self::DescriptorAdoption => "child-controller-descriptor-adoption-failed",
            Self::SystemdActivation => "child-controller-systemd-activation-rejected",
            Self::InvalidListener => "child-controller-listener-invalid",
            Self::InvalidBootstrapSession => "child-controller-bootstrap-session-invalid",
            Self::InvalidCgroup => "child-controller-cgroup-invalid",
            Self::ListenerAlreadyTaken => "child-controller-listener-already-taken",
            Self::ReadyAlreadySent => "child-controller-ready-already-sent",
            Self::ReadySend => "child-controller-ready-send-failed",
        })
    }
}

impl std::error::Error for ControllerBootstrapError {}

impl From<ControllerBootstrapError> for TypedError {
    fn from(error: ControllerBootstrapError) -> Self {
        match error {
            ControllerBootstrapError::DescriptorAdoption | ControllerBootstrapError::ReadySend => {
                Self::InternalIo {
                    context: "child realm controller bootstrap".to_owned(),
                    detail: error.to_string(),
                }
            }
            _ => Self::InternalConfig {
                detail: error.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        fs::File,
        io::IoSliceMut,
        os::fd::{AsRawFd, IntoRawFd, OwnedFd, RawFd},
        os::unix::ffi::OsStringExt,
        path::PathBuf,
        sync::Mutex,
        sync::atomic::{AtomicU64, Ordering},
    };

    use nix::{
        fcntl::{FcntlArg, FdFlag, fcntl},
        sys::socket::{
            AddressFamily as NixAddressFamily, Backlog, SockFlag, SockType, UnixAddr, bind,
            connect, listen, socket,
        },
    };
    use rustix::{
        fs::{FileType, fstat},
        net::{
            RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, recvmsg,
            sockopt::set_socket_passcred,
        },
        process::{getgid, getpid, getuid},
    };
    use socket2::Socket;

    use super::{ControllerBootstrap, ControllerBootstrapError, EnvironmentSnapshot, READY_PACKET};

    static LISTENER_ID: AtomicU64 = AtomicU64::new(1);
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn seqpacket_listener() -> (OwnedFd, UnixAddr) {
        let listener = socket(
            NixAddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC,
            None,
        )
        .unwrap();
        let name = format!(
            "d2bd-child-bootstrap-{}-{}",
            std::process::id(),
            LISTENER_ID.fetch_add(1, Ordering::Relaxed)
        );
        let address = UnixAddr::new_abstract(name.as_bytes()).unwrap();
        bind(listener.as_raw_fd(), &address).unwrap();
        listen(&listener, Backlog::new(4).unwrap()).unwrap();
        (listener, address)
    }

    fn child_snapshot(listener: RawFd, bootstrap: RawFd, cgroup: RawFd) -> EnvironmentSnapshot {
        EnvironmentSnapshot {
            role: Some("controller".into()),
            listener: Some(listener.to_string().into()),
            bootstrap: Some(bootstrap.to_string().into()),
            cgroup: Some(cgroup.to_string().into()),
            systemd_activation: false,
        }
    }

    fn adopted_child() -> (
        ControllerBootstrap,
        OwnedFd,
        UnixAddr,
        [(RawFd, PathBuf); 3],
    ) {
        let (listener, address) = seqpacket_listener();
        let (bootstrap_parent, bootstrap_child) =
            d2b_session_unix::prearmed_seqpacket_pair().unwrap();
        set_socket_passcred(&bootstrap_parent, true).unwrap();
        let cgroup_dir = tempfile::tempdir().unwrap();
        let cgroup = File::open(cgroup_dir.path()).unwrap();
        let raw_fds = [
            listener.as_raw_fd(),
            bootstrap_child.as_raw_fd(),
            cgroup.as_raw_fd(),
        ];
        let original_targets =
            raw_fds.map(|raw| std::fs::read_link(format!("/proc/self/fd/{raw}")).unwrap());
        let originals = [
            (listener.into_raw_fd(), original_targets[0].clone()),
            (bootstrap_child.into_raw_fd(), original_targets[1].clone()),
            (cgroup.into_raw_fd(), original_targets[2].clone()),
        ];
        let adopted = ControllerBootstrap::from_snapshot(child_snapshot(
            originals[0].0,
            originals[1].0,
            originals[2].0,
        ))
        .unwrap();
        (adopted, bootstrap_parent, address, originals)
    }

    #[test]
    fn exact_child_environment_adopts_cloexec_authority_and_reuses_listener() {
        let _guard = TEST_LOCK.lock().unwrap();
        let (mut bootstrap, _bootstrap_parent, address, originals) = adopted_child();
        for (raw, original_target) in originals {
            match std::fs::read_link(format!("/proc/self/fd/{raw}")) {
                Ok(current_target) => assert_ne!(current_target, original_target),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("inspect inherited fd {raw}: {error}"),
            }
        }
        let child = bootstrap.child.as_ref().unwrap();
        for fd in [&child.bootstrap, &child._cgroup] {
            let flags = fcntl(fd.as_raw_fd(), FcntlArg::F_GETFD).unwrap();
            assert!(FdFlag::from_bits_truncate(flags).contains(FdFlag::FD_CLOEXEC));
        }
        assert!(
            fstat(&child._cgroup)
                .is_ok_and(|stat| { FileType::from_raw_mode(stat.st_mode) == FileType::Directory })
        );

        let bind_called = Cell::new(false);
        let selected: Result<Socket, ControllerBootstrapError> =
            bootstrap.public_listener_or_else(|| {
                bind_called.set(true);
                Err(ControllerBootstrapError::InvalidListener)
            });
        let selected = selected.unwrap();
        assert!(!bind_called.get());

        let client = socket(
            NixAddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC,
            None,
        )
        .unwrap();
        connect(client.as_raw_fd(), &address).unwrap();
        selected.accept().unwrap();
    }

    #[test]
    fn ready_is_one_bounded_credentialed_packet() {
        let _guard = TEST_LOCK.lock().unwrap();
        let (mut bootstrap, parent, _address, _originals) = adopted_child();
        bootstrap.send_ready().unwrap();

        let mut payload = [0_u8; 16];
        let mut iov = [IoSliceMut::new(&mut payload)];
        let mut control_bytes = vec![0_u8; rustix::cmsg_space!(ScmCredentials(1))];
        let mut control = RecvAncillaryBuffer::new(&mut control_bytes);
        let received = recvmsg(&parent, &mut iov, &mut control, RecvFlags::empty()).unwrap();
        assert_eq!(&payload[..received.bytes], READY_PACKET);
        let credentials = control.drain().find_map(|message| match message {
            RecvAncillaryMessage::ScmCredentials(credentials) => Some(credentials),
            _ => None,
        });
        assert_eq!(
            credentials,
            Some(rustix::net::UCred {
                pid: getpid(),
                uid: getuid(),
                gid: getgid(),
            })
        );
        assert_eq!(
            bootstrap.send_ready(),
            Err(ControllerBootstrapError::ReadyAlreadySent)
        );
    }

    #[test]
    fn rejects_role_partial_alias_invalid_and_systemd_activation() {
        let _guard = TEST_LOCK.lock().unwrap();
        for role in ["Controller", "controller ", "broker", ""] {
            let snapshot = EnvironmentSnapshot {
                role: Some(role.into()),
                ..EnvironmentSnapshot::default()
            };
            assert_eq!(
                ControllerBootstrap::from_snapshot(snapshot).unwrap_err(),
                ControllerBootstrapError::InvalidRole
            );
        }
        let non_unicode_role = EnvironmentSnapshot {
            role: Some(std::ffi::OsString::from_vec(vec![0xff])),
            ..EnvironmentSnapshot::default()
        };
        assert_eq!(
            ControllerBootstrap::from_snapshot(non_unicode_role).unwrap_err(),
            ControllerBootstrapError::InvalidRole
        );

        let partial = EnvironmentSnapshot {
            role: Some("controller".into()),
            listener: Some("10".into()),
            bootstrap: Some("11".into()),
            ..EnvironmentSnapshot::default()
        };
        assert_eq!(
            ControllerBootstrap::from_snapshot(partial).unwrap_err(),
            ControllerBootstrapError::PartialDescriptors
        );
        let descriptor_without_role = EnvironmentSnapshot {
            listener: Some("10".into()),
            ..EnvironmentSnapshot::default()
        };
        assert_eq!(
            ControllerBootstrap::from_snapshot(descriptor_without_role).unwrap_err(),
            ControllerBootstrapError::PartialDescriptors
        );
        assert_eq!(
            ControllerBootstrap::from_snapshot(child_snapshot(10, 10, 12)).unwrap_err(),
            ControllerBootstrapError::DescriptorAlias
        );

        for invalid in ["", "2", "4097", "+10", "ten"] {
            let snapshot = EnvironmentSnapshot {
                role: Some("controller".into()),
                listener: Some(invalid.into()),
                bootstrap: Some("11".into()),
                cgroup: Some("12".into()),
                systemd_activation: false,
            };
            assert_eq!(
                ControllerBootstrap::from_snapshot(snapshot).unwrap_err(),
                ControllerBootstrapError::InvalidDescriptor
            );
        }

        let systemd = EnvironmentSnapshot {
            systemd_activation: true,
            ..child_snapshot(10, 11, 12)
        };
        assert_eq!(
            ControllerBootstrap::from_snapshot(systemd).unwrap_err(),
            ControllerBootstrapError::SystemdActivation
        );
    }

    #[test]
    fn rejects_non_listening_public_socket() {
        let _guard = TEST_LOCK.lock().unwrap();
        let (listener, listener_peer) = d2b_session_unix::prearmed_seqpacket_pair().unwrap();
        let (_bootstrap_parent, bootstrap_child) =
            d2b_session_unix::prearmed_seqpacket_pair().unwrap();
        let cgroup = File::open(".").unwrap();
        let snapshot = child_snapshot(
            listener.into_raw_fd(),
            bootstrap_child.into_raw_fd(),
            cgroup.into_raw_fd(),
        );
        assert_eq!(
            ControllerBootstrap::from_snapshot(snapshot).unwrap_err(),
            ControllerBootstrapError::InvalidListener
        );
        drop(listener_peer);
    }

    #[test]
    fn rejects_non_session_bootstrap_and_non_directory_cgroup() {
        let _guard = TEST_LOCK.lock().unwrap();
        let (public_listener, _public_address) = seqpacket_listener();
        let (bootstrap_listener, _bootstrap_address) = seqpacket_listener();
        let cgroup = File::open(".").unwrap();
        let snapshot = child_snapshot(
            public_listener.into_raw_fd(),
            bootstrap_listener.into_raw_fd(),
            cgroup.into_raw_fd(),
        );
        assert_eq!(
            ControllerBootstrap::from_snapshot(snapshot).unwrap_err(),
            ControllerBootstrapError::InvalidBootstrapSession
        );

        let (public_listener, _public_address) = seqpacket_listener();
        let (_bootstrap_parent, bootstrap_child) =
            d2b_session_unix::prearmed_seqpacket_pair().unwrap();
        let regular_file = File::open("Cargo.toml").unwrap();
        let snapshot = child_snapshot(
            public_listener.into_raw_fd(),
            bootstrap_child.into_raw_fd(),
            regular_file.into_raw_fd(),
        );
        assert_eq!(
            ControllerBootstrap::from_snapshot(snapshot).unwrap_err(),
            ControllerBootstrapError::InvalidCgroup
        );
    }

    #[test]
    fn local_root_is_noop_and_uses_existing_bind_path() {
        let _guard = TEST_LOCK.lock().unwrap();
        let mut bootstrap = ControllerBootstrap::from_snapshot(EnvironmentSnapshot {
            systemd_activation: true,
            ..EnvironmentSnapshot::default()
        })
        .unwrap();
        assert!(bootstrap.child.is_none());

        let (listener, _address) = seqpacket_listener();
        let expected = fstat(&listener).unwrap();
        let bind_called = Cell::new(false);
        let selected: Result<Socket, ControllerBootstrapError> =
            bootstrap.public_listener_or_else(|| {
                bind_called.set(true);
                Ok(Socket::from(listener))
            });
        let selected = selected.unwrap();
        assert!(bind_called.get());
        let observed = fstat(&selected).unwrap();
        assert_eq!(
            (observed.st_dev, observed.st_ino),
            (expected.st_dev, expected.st_ino)
        );
        bootstrap.send_ready().unwrap();
    }
}
