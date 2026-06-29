//! Internal bridge helpers between `d2b-wayland-proxy` and `d2b-clipd`.
//!
//! The bridge socket is d2b-internal and per user/per VM. It is not the picker
//! protocol, does not depend on `NIRI_SOCKET`, and may carry transfer FDs only
//! between d2b components once the protocol is implemented.

use std::{
    io::IoSlice,
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::{ffi::OsStrExt, net::UnixStream},
    },
    path::{Path, PathBuf},
    time::Duration,
};

const LINUX_SUN_PATH_BYTES: usize = 108;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeConfig {
    pub socket_path: Option<PathBuf>,
    pub reconnect: BridgeReconnectPolicy,
}

impl BridgeConfig {
    pub fn disabled() -> Self {
        Self {
            socket_path: None,
            reconnect: BridgeReconnectPolicy::default(),
        }
    }

    pub fn from_parts(
        explicit_socket: Option<PathBuf>,
        root: &Path,
        user_uid: Option<u32>,
        vm_name: &str,
        reconnect: BridgeReconnectPolicy,
    ) -> Result<Self, BridgeConfigError> {
        if reconnect.initial_delay > reconnect.max_delay {
            return Err(BridgeConfigError::InvalidReconnectPolicy);
        }

        let socket_path = match explicit_socket {
            Some(path) => Some(validate_socket_path(path)?),
            None => match user_uid {
                Some(uid) => Some(validate_socket_path(path_for_user_vm(root, uid, vm_name)?)?),
                None => None,
            },
        };

        Ok(Self {
            socket_path,
            reconnect,
        })
    }

    pub fn is_enabled(&self) -> bool {
        self.socket_path.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeReconnectPolicy {
    pub initial_delay: Duration,
    pub max_delay: Duration,
}

impl Default for BridgeReconnectPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(5),
        }
    }
}

pub fn path_for_user_vm(
    root: &Path,
    user_uid: u32,
    vm_name: &str,
) -> Result<PathBuf, BridgeConfigError> {
    validate_vm_path_component(vm_name)?;
    Ok(root
        .join(user_uid.to_string())
        .join("bridge")
        .join(vm_name)
        .join("clip.sock"))
}

fn validate_vm_path_component(vm_name: &str) -> Result<(), BridgeConfigError> {
    if vm_name.is_empty()
        || vm_name == "."
        || vm_name == ".."
        || vm_name.contains('/')
        || vm_name.contains('\0')
    {
        return Err(BridgeConfigError::InvalidVmName);
    }
    Ok(())
}

fn validate_socket_path(path: PathBuf) -> Result<PathBuf, BridgeConfigError> {
    if path.as_os_str().as_bytes().len() >= LINUX_SUN_PATH_BYTES {
        return Err(BridgeConfigError::SocketPathTooLong);
    }
    Ok(path)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BridgeConfigError {
    #[error("invalid VM name for bridge path")]
    InvalidVmName,
    #[error("bridge socket path exceeds Linux sockaddr_un sun_path limit")]
    SocketPathTooLong,
    #[error("bridge reconnect initial delay must not exceed max delay")]
    InvalidReconnectPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeConnectionState {
    Disabled,
    Disconnected,
    Connecting { attempt: u32 },
    Connected,
    Backoff { attempt: u32, delay: Duration },
}

#[derive(Debug, Clone)]
pub struct BridgeReconnectMachine {
    policy: BridgeReconnectPolicy,
    state: BridgeConnectionState,
}

impl BridgeReconnectMachine {
    pub fn new(config: &BridgeConfig) -> Self {
        Self {
            policy: config.reconnect,
            state: if config.is_enabled() {
                BridgeConnectionState::Disconnected
            } else {
                BridgeConnectionState::Disabled
            },
        }
    }

    pub fn state(&self) -> BridgeConnectionState {
        self.state
    }

    pub fn start_connect(&mut self) {
        if matches!(
            self.state,
            BridgeConnectionState::Disconnected | BridgeConnectionState::Backoff { .. }
        ) {
            let attempt = match self.state {
                BridgeConnectionState::Backoff { attempt, .. } => attempt,
                _ => 1,
            };
            self.state = BridgeConnectionState::Connecting { attempt };
        }
    }

    pub fn connect_succeeded(&mut self) {
        if matches!(self.state, BridgeConnectionState::Connecting { .. }) {
            self.state = BridgeConnectionState::Connected;
        }
    }

    pub fn connect_failed(&mut self) {
        if let BridgeConnectionState::Connecting { attempt } = self.state {
            self.state = BridgeConnectionState::Backoff {
                attempt: attempt.saturating_add(1),
                delay: self.delay_for_attempt(attempt),
            };
        }
    }

    pub fn disconnected(&mut self) {
        if matches!(self.state, BridgeConnectionState::Connected) {
            self.state = BridgeConnectionState::Backoff {
                attempt: 1,
                delay: self.policy.initial_delay,
            };
        }
    }

    pub fn retry_due(&mut self) {
        if let BridgeConnectionState::Backoff { attempt, .. } = self.state {
            self.state = BridgeConnectionState::Connecting { attempt };
        }
    }

    fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let factor = 1_u32
            .checked_shl(attempt.saturating_sub(1))
            .unwrap_or(u32::MAX);
        self.policy
            .initial_delay
            .saturating_mul(factor)
            .min(self.policy.max_delay)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffStatus {
    Delivered,
    Failed,
}

/// Owns a local transfer FD until the bridge handoff attempt reaches a terminal
/// result. Dropping the wrapper closes the proxy's local copy on every path.
pub struct LocalTransferFd {
    fd: Option<OwnedFd>,
}

impl LocalTransferFd {
    pub fn new(fd: OwnedFd) -> Self {
        Self { fd: Some(fd) }
    }

    pub fn close_after_handoff(mut self, status: HandoffStatus) -> HandoffStatus {
        drop(self.fd.take());
        status
    }
}

pub trait BridgeHandoff {
    fn handoff_transfer_fd(&mut self, fd: &OwnedFd) -> HandoffStatus;
}

impl BridgeHandoff for UnixStream {
    fn handoff_transfer_fd(&mut self, local_fd: &OwnedFd) -> HandoffStatus {
        let raw_fd = local_fd.as_raw_fd();
        let iov = [IoSlice::new(b"F")];
        let fds = [raw_fd];
        let cmsg = [nix::sys::socket::ControlMessage::ScmRights(&fds)];
        match nix::sys::socket::sendmsg::<()>(
            self.as_raw_fd(),
            &iov,
            &cmsg,
            nix::sys::socket::MsgFlags::MSG_NOSIGNAL,
            None,
        ) {
            Ok(_) => HandoffStatus::Delivered,
            Err(error) => {
                log::debug!("d2b-wayland-proxy: bridge fd handoff failed: {error}");
                HandoffStatus::Failed
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{IoSliceMut, Read};

    fn assert_peer_observes_local_close(status: HandoffStatus) {
        let (local, mut peer) = UnixStream::pair().expect("socket pair");
        let local = LocalTransferFd::new(local.into());

        assert_eq!(local.close_after_handoff(status), status);

        let mut buf = [0_u8; 1];
        assert_eq!(peer.read(&mut buf).expect("peer reads EOF"), 0);
    }

    #[test]
    fn bridge_path_uses_per_user_per_vm_layout() {
        let path = path_for_user_vm(Path::new("/run/d2b/clipd"), 1000, "work").expect("valid path");

        assert_eq!(
            path,
            PathBuf::from("/run/d2b/clipd/1000/bridge/work/clip.sock")
        );
    }

    #[test]
    fn explicit_bridge_socket_path_wins() {
        let config = BridgeConfig::from_parts(
            Some(PathBuf::from("/run/d2b/clipd/1000/bridge/work/custom.sock")),
            Path::new("/run/d2b/clipd"),
            Some(1001),
            "other",
            BridgeReconnectPolicy::default(),
        )
        .expect("valid config");

        assert_eq!(
            config.socket_path.as_deref(),
            Some(Path::new("/run/d2b/clipd/1000/bridge/work/custom.sock"))
        );
    }

    #[test]
    fn bridge_config_can_be_disabled_until_nix_renders_socket() {
        let config = BridgeConfig::from_parts(
            None,
            Path::new("/run/d2b/clipd"),
            None,
            "work",
            BridgeReconnectPolicy::default(),
        )
        .expect("disabled config");

        assert!(!config.is_enabled());
    }

    #[test]
    fn bridge_path_rejects_invalid_vm_component() {
        assert!(matches!(
            path_for_user_vm(Path::new("/run/d2b/clipd"), 1000, "bad/vm"),
            Err(BridgeConfigError::InvalidVmName)
        ));
        assert!(matches!(
            path_for_user_vm(Path::new("/run/d2b/clipd"), 1000, "."),
            Err(BridgeConfigError::InvalidVmName)
        ));
        assert!(matches!(
            path_for_user_vm(Path::new("/run/d2b/clipd"), 1000, ".."),
            Err(BridgeConfigError::InvalidVmName)
        ));
    }

    #[test]
    fn reconnect_state_machine_recovers_after_failure() {
        let config = BridgeConfig::from_parts(
            Some(PathBuf::from("/run/d2b/clipd/1000/bridge/work/clip.sock")),
            Path::new("/run/d2b/clipd"),
            None,
            "work",
            BridgeReconnectPolicy::default(),
        )
        .expect("enabled config");
        let mut machine = BridgeReconnectMachine::new(&config);

        assert_eq!(machine.state(), BridgeConnectionState::Disconnected);
        machine.start_connect();
        assert_eq!(
            machine.state(),
            BridgeConnectionState::Connecting { attempt: 1 }
        );
        machine.connect_failed();
        assert_eq!(
            machine.state(),
            BridgeConnectionState::Backoff {
                attempt: 2,
                delay: Duration::from_millis(250)
            }
        );
        machine.retry_due();
        assert_eq!(
            machine.state(),
            BridgeConnectionState::Connecting { attempt: 2 }
        );
        machine.connect_succeeded();
        assert_eq!(machine.state(), BridgeConnectionState::Connected);
        machine.disconnected();
        assert_eq!(
            machine.state(),
            BridgeConnectionState::Backoff {
                attempt: 1,
                delay: Duration::from_millis(250)
            }
        );
    }

    #[test]
    fn successful_handoff_closes_local_fd_copy() {
        assert_peer_observes_local_close(HandoffStatus::Delivered);
    }

    #[test]
    fn failed_handoff_closes_local_fd_copy() {
        assert_peer_observes_local_close(HandoffStatus::Failed);
    }

    #[test]
    fn bridge_handoff_sends_fd_with_scm_rights() {
        let (mut bridge, peer) = UnixStream::pair().expect("bridge socket pair");
        let (local, mut local_peer) = UnixStream::pair().expect("transfer socket pair");
        let local: OwnedFd = local.into();

        assert_eq!(bridge.handoff_transfer_fd(&local), HandoffStatus::Delivered);
        drop(local);

        let mut byte = [0_u8; 1];
        let mut iov = [IoSliceMut::new(&mut byte)];
        let mut cmsg_space = nix::cmsg_space!([i32; 1]);
        let msg = nix::sys::socket::recvmsg::<()>(
            peer.as_raw_fd(),
            &mut iov,
            Some(&mut cmsg_space),
            nix::sys::socket::MsgFlags::empty(),
        )
        .expect("recvmsg");
        assert_eq!(msg.bytes, 1);
        let mut saw_fd = false;
        for cmsg in msg.cmsgs().expect("cmsgs") {
            if let nix::sys::socket::ControlMessageOwned::ScmRights(fds) = cmsg {
                saw_fd = !fds.is_empty();
                for fd in fds {
                    let _ = nix::unistd::close(fd);
                }
            }
        }
        assert!(saw_fd, "bridge handoff must carry one SCM_RIGHTS fd");
        let mut buf = [0_u8; 1];
        assert_eq!(local_peer.read(&mut buf).expect("local peer EOF"), 0);
    }
}
