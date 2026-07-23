//! Internal bridge helpers between `d2b-wayland-proxy` and `d2b-clipd`.
//!
//! The bridge socket is d2b-internal and per user/per workload endpoint. It is not the picker
//! protocol, does not depend on `NIRI_SOCKET`, and may carry transfer FDs only
//! between d2b components once the protocol is implemented.

use std::{
    io::IoSlice,
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::{ffi::OsStrExt, net::UnixStream},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use d2b_core::workload_identity::WorkloadTarget;
use d2b_realm_core::WorkloadProviderKind;
use serde::Serialize;

use crate::identity::ProxyIdentity;

const LINUX_SUN_PATH_BYTES: usize = 108;
pub const SCM_RIGHTS_MIN_FDS: usize = 28;
pub const SCM_RIGHTS_MIN_CONTROL_BYTES: usize = 256;
pub const SCM_RIGHTS_CONTROL_FD_SLOTS: usize = 64;

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
        let target = WorkloadTarget::parse(&format!("{vm_name}.local.d2b"))
            .map_err(|_| BridgeConfigError::InvalidEndpointComponent)?;
        let identity = ProxyIdentity::legacy_vm(vm_name, target, WorkloadProviderKind::LocalVm)
            .map_err(|_| BridgeConfigError::InvalidEndpointComponent)?;
        Self::from_identity_parts(explicit_socket, root, user_uid, &identity, reconnect)
    }

    pub fn from_identity_parts(
        explicit_socket: Option<PathBuf>,
        root: &Path,
        user_uid: Option<u32>,
        identity: &ProxyIdentity,
        reconnect: BridgeReconnectPolicy,
    ) -> Result<Self, BridgeConfigError> {
        if reconnect.initial_delay > reconnect.max_delay {
            return Err(BridgeConfigError::InvalidReconnectPolicy);
        }

        let socket_path = match explicit_socket {
            Some(path) => Some(validate_socket_path(path)?),
            None => match user_uid {
                Some(uid) => Some(validate_socket_path(path_for_user_identity(
                    root, uid, identity,
                )?)?),
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

pub fn path_for_user_identity(
    root: &Path,
    user_uid: u32,
    identity: &ProxyIdentity,
) -> Result<PathBuf, BridgeConfigError> {
    let component = identity.bridge_component();
    validate_bridge_path_component(&component)?;
    Ok(root
        .join(user_uid.to_string())
        .join("bridge")
        .join(component)
        .join("clip.sock"))
}

fn validate_vm_path_component(vm_name: &str) -> Result<(), BridgeConfigError> {
    validate_bridge_path_component(vm_name)
}

fn validate_bridge_path_component(value: &str) -> Result<(), BridgeConfigError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\0')
    {
        return Err(BridgeConfigError::InvalidEndpointComponent);
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
    #[error("invalid workload endpoint component for bridge path")]
    InvalidEndpointComponent,
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
    Connected { attempt: u32 },
    Backoff { attempt: u32, delay: Duration },
}

const STABLE_CONNECTION_RESET_AFTER: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct BridgeReconnectMachine {
    policy: BridgeReconnectPolicy,
    state: BridgeConnectionState,
    connected_at: Option<Instant>,
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
            connected_at: None,
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
        if let BridgeConnectionState::Connecting { attempt } = self.state {
            self.state = BridgeConnectionState::Connected { attempt };
            self.connected_at = Some(Instant::now());
        }
    }

    pub fn connect_failed(&mut self) {
        if let BridgeConnectionState::Connecting { attempt } = self.state {
            self.state = BridgeConnectionState::Backoff {
                attempt: attempt.saturating_add(1),
                delay: self.delay_for_attempt(attempt),
            };
            self.connected_at = None;
        }
    }

    pub fn disconnected(&mut self) {
        if let BridgeConnectionState::Connected { attempt } = self.state {
            let stable = self.connected_at.is_some_and(|connected_at| {
                connected_at.elapsed() >= STABLE_CONNECTION_RESET_AFTER
            });
            self.connected_at = None;
            if stable {
                self.state = BridgeConnectionState::Disconnected;
            } else {
                let backoff_attempt = attempt.saturating_add(1);
                self.state = BridgeConnectionState::Backoff {
                    attempt: backoff_attempt,
                    delay: self.delay_for_attempt(attempt),
                };
            }
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
    Backpressure,
    Failed(Option<nix::errno::Errno>),
}

/// Owns a local transfer FD until the bridge handoff attempt reaches a terminal
/// result. Dropping the wrapper closes the proxy's local copy on every path,
/// acting as the CloseAttach barrier for clipboard transfers: callers may only
/// release their local copy after the bridge reports delivered/deferred/failed,
/// never while payload bytes and ancillary FDs can still be separated by
/// backpressure.
#[derive(Debug)]
pub struct LocalTransferFd {
    fd: Option<OwnedFd>,
}

impl From<UnixStream> for LocalTransferFd {
    fn from(value: UnixStream) -> Self {
        Self::new(value.into())
    }
}

impl LocalTransferFd {
    pub fn new(fd: OwnedFd) -> Self {
        Self { fd: Some(fd) }
    }

    pub fn close_after_handoff(mut self, status: HandoffStatus) -> HandoffStatus {
        drop(self.fd.take());
        status
    }

    fn as_owned_fd(&self) -> &OwnedFd {
        self.fd
            .as_ref()
            .expect("fd present until close_after_handoff")
    }
}

pub trait BridgeHandoff {
    fn handoff_transfer_fd(
        &mut self,
        fd: &LocalTransferFd,
        metadata: &BridgeTransferMetadata,
    ) -> HandoffStatus;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTransferMetadata {
    pub identity: ProxyIdentity,
    pub mime_type: String,
    pub source_id: u64,
    pub kind: BridgeTransferKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTransferKind {
    PasteRequest,
    CopySelection,
}

impl BridgeHandoff for UnixStream {
    fn handoff_transfer_fd(
        &mut self,
        local_fd: &LocalTransferFd,
        metadata: &BridgeTransferMetadata,
    ) -> HandoffStatus {
        let raw_fd = local_fd.as_owned_fd().as_raw_fd();
        let frame = bridge_frame(metadata);
        let iov = [IoSlice::new(frame.as_bytes())];
        let fds = [raw_fd];
        let cmsg = [nix::sys::socket::ControlMessage::ScmRights(&fds)];
        handoff_status_from_sendmsg_result(
            nix::sys::socket::sendmsg::<()>(
                self.as_raw_fd(),
                &iov,
                &cmsg,
                nix::sys::socket::MsgFlags::MSG_NOSIGNAL | nix::sys::socket::MsgFlags::MSG_DONTWAIT,
                None,
            ),
            frame.len(),
        )
    }
}

fn handoff_status_from_sendmsg_result(
    result: Result<usize, nix::errno::Errno>,
    frame_len: usize,
) -> HandoffStatus {
    match result {
        Ok(n) if n == frame_len => HandoffStatus::Delivered,
        Err(error) if is_would_block_errno(error) => HandoffStatus::Backpressure,
        Ok(_) => HandoffStatus::Failed(None),
        Err(error) => HandoffStatus::Failed(Some(error)),
    }
}

fn is_would_block_errno(error: nix::errno::Errno) -> bool {
    matches!(
        error,
        nix::errno::Errno::EAGAIN | nix::errno::Errno::ENOTCONN
    )
}

pub fn recv_flags_are_fail_closed(flags: nix::sys::socket::MsgFlags) -> bool {
    !flags.contains(nix::sys::socket::MsgFlags::MSG_CTRUNC)
}

fn bridge_frame(metadata: &BridgeTransferMetadata) -> String {
    #[derive(Serialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum Frame<'a> {
        WorkloadPasteRequest {
            canonical_target: &'a WorkloadTarget,
            provider_kind: WorkloadProviderKind,
            #[serde(skip_serializing_if = "Option::is_none")]
            legacy_vm_name: Option<&'a str>,
            mime_type: &'a str,
            source_id: u64,
            source_attribution: &'static str,
        },
        WorkloadCopySelection {
            canonical_target: &'a WorkloadTarget,
            provider_kind: WorkloadProviderKind,
            #[serde(skip_serializing_if = "Option::is_none")]
            legacy_vm_name: Option<&'a str>,
            mime_type: &'a str,
            source_id: u64,
            source_attribution: &'static str,
        },
    }

    let common = (
        metadata.identity.target(),
        metadata.identity.provider_kind(),
        metadata.identity.legacy_vm_name(),
        metadata.mime_type.as_str(),
        metadata.source_id,
    );
    let frame = match metadata.kind {
        BridgeTransferKind::PasteRequest => Frame::WorkloadPasteRequest {
            canonical_target: common.0,
            provider_kind: common.1,
            legacy_vm_name: common.2,
            mime_type: common.3,
            source_id: common.4,
            source_attribution: "exact_client",
        },
        BridgeTransferKind::CopySelection => Frame::WorkloadCopySelection {
            canonical_target: common.0,
            provider_kind: common.1,
            legacy_vm_name: common.2,
            mime_type: common.3,
            source_id: common.4,
            source_attribution: "exact_client",
        },
    };
    let mut encoded = serde_json::to_string(&frame).expect("typed bridge frame serializes");
    encoded.push('\n');
    encoded
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
    fn canonical_endpoint_bridge_path_is_stably_hash_shortened() {
        let identity = ProxyIdentity::canonical(
            WorkloadTarget::parse("tools.host.d2b").unwrap(),
            WorkloadProviderKind::UnsafeLocal,
        );
        let path = path_for_user_identity(Path::new("/run/d2b/clipd"), 1000, &identity)
            .expect("valid path");

        assert_eq!(
            path,
            PathBuf::from("/run/d2b/clipd/1000/bridge/endpoint-fc002cd9909aab17c2232e85/clip.sock")
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
            Err(BridgeConfigError::InvalidEndpointComponent)
        ));
        assert!(matches!(
            path_for_user_vm(Path::new("/run/d2b/clipd"), 1000, "."),
            Err(BridgeConfigError::InvalidEndpointComponent)
        ));
        assert!(matches!(
            path_for_user_vm(Path::new("/run/d2b/clipd"), 1000, ".."),
            Err(BridgeConfigError::InvalidEndpointComponent)
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
        assert_eq!(
            machine.state(),
            BridgeConnectionState::Connected { attempt: 2 }
        );
        machine.disconnected();
        assert_eq!(
            machine.state(),
            BridgeConnectionState::Backoff {
                attempt: 3,
                delay: Duration::from_millis(500)
            }
        );
        machine.retry_due();
        assert_eq!(
            machine.state(),
            BridgeConnectionState::Connecting { attempt: 3 }
        );
        machine.connect_succeeded();
        machine.connected_at = Some(Instant::now() - Duration::from_secs(2));
        machine.disconnected();
        assert_eq!(machine.state(), BridgeConnectionState::Disconnected);
    }

    #[test]
    fn successful_handoff_closes_local_fd_copy() {
        assert_peer_observes_local_close(HandoffStatus::Delivered);
    }

    #[test]
    fn failed_handoff_closes_local_fd_copy() {
        assert_peer_observes_local_close(HandoffStatus::Failed(Some(nix::errno::Errno::EPIPE)));
    }

    #[test]
    fn backpressured_handoff_closes_local_fd_copy() {
        assert_peer_observes_local_close(HandoffStatus::Backpressure);
    }

    #[test]
    fn sendmsg_backpressure_is_not_fatal_handoff_failure() {
        assert_eq!(
            handoff_status_from_sendmsg_result(Err(nix::errno::Errno::EAGAIN), 128),
            HandoffStatus::Backpressure
        );
        assert_eq!(
            handoff_status_from_sendmsg_result(Err(nix::errno::Errno::ENOTCONN), 128),
            HandoffStatus::Backpressure
        );
        assert_eq!(
            handoff_status_from_sendmsg_result(Ok(64), 128),
            HandoffStatus::Failed(None)
        );
        assert_eq!(
            handoff_status_from_sendmsg_result(Err(nix::errno::Errno::EPIPE), 128),
            HandoffStatus::Failed(Some(nix::errno::Errno::EPIPE))
        );
    }

    #[test]
    fn bridge_handoff_sends_fd_with_scm_rights() {
        let (mut bridge, peer) = UnixStream::pair().expect("bridge socket pair");
        let (local, mut local_peer) = UnixStream::pair().expect("transfer socket pair");
        let local = LocalTransferFd::new(local.into());
        let metadata = BridgeTransferMetadata {
            identity: ProxyIdentity::from("work"),
            mime_type: "text/plain".to_owned(),
            source_id: 7,
            kind: BridgeTransferKind::PasteRequest,
        };

        assert_eq!(
            bridge.handoff_transfer_fd(&local, &metadata),
            HandoffStatus::Delivered
        );
        let _ = local.close_after_handoff(HandoffStatus::Delivered);

        let mut frame = [0_u8; 256];
        let mut iov = [IoSliceMut::new(&mut frame)];
        let mut cmsg_space = vec![0_u8; SCM_RIGHTS_MIN_CONTROL_BYTES];
        const { assert!(SCM_RIGHTS_CONTROL_FD_SLOTS >= SCM_RIGHTS_MIN_FDS) };
        assert!(cmsg_space.len() >= SCM_RIGHTS_MIN_CONTROL_BYTES);
        let msg = nix::sys::socket::recvmsg::<()>(
            peer.as_raw_fd(),
            &mut iov,
            Some(&mut cmsg_space),
            nix::sys::socket::MsgFlags::MSG_CMSG_CLOEXEC,
        )
        .expect("recvmsg");
        assert!(recv_flags_are_fail_closed(msg.flags));
        let bytes = msg.bytes;
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
        let frame = std::str::from_utf8(&frame[..bytes]).expect("utf8 frame");
        assert!(frame.contains("\"type\":\"workload_paste_request\""));
        assert!(frame.contains("\"source_attribution\":\"exact_client\""));
        assert!(frame.contains("\"canonical_target\":\"work.local.d2b\""));
        assert!(frame.contains("\"provider_kind\":\"local-vm\""));
        assert!(frame.contains("\"legacy_vm_name\":\"work\""));
        let mut buf = [0_u8; 1];
        assert_eq!(local_peer.read(&mut buf).expect("local peer EOF"), 0);
    }

    #[test]
    fn ctruncated_scm_rights_receive_is_fail_closed() {
        assert!(!recv_flags_are_fail_closed(
            nix::sys::socket::MsgFlags::MSG_CTRUNC
        ));
        assert!(recv_flags_are_fail_closed(
            nix::sys::socket::MsgFlags::empty()
        ));
    }

    #[test]
    fn bridge_handoff_encodes_provider_neutral_copy_selection() {
        let metadata = BridgeTransferMetadata {
            identity: ProxyIdentity::canonical(
                WorkloadTarget::parse("tools.host.d2b").unwrap(),
                WorkloadProviderKind::UnsafeLocal,
            ),
            mime_type: "text/html".to_owned(),
            source_id: 9,
            kind: BridgeTransferKind::CopySelection,
        };
        let frame = bridge_frame(&metadata);
        assert!(frame.contains("\"type\":\"workload_copy_selection\""));
        assert!(frame.contains("\"canonical_target\":\"tools.host.d2b\""));
        assert!(frame.contains("\"provider_kind\":\"unsafe-local\""));
        assert!(!frame.contains("legacy_vm_name"));
        assert!(frame.contains("\"mime_type\":\"text/html\""));
    }
}
