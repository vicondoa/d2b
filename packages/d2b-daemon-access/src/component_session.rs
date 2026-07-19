//! Authenticated ComponentSession binding for local daemon access.

use std::{
    os::fd::{AsRawFd, OwnedFd},
    path::Path,
};

use d2b_client::{
    Client, HostSocketConnector, RouteRecord, RouteTable, ServiceKind, ServiceOwner, TargetInput,
    TransportKind, TransportSelection, local_daemon_endpoint_identity,
};
use d2b_contracts::v2_identity::{RealmId, RealmPath, WorkloadName};
use nix::{
    sys::socket::{AddressFamily, SockFlag, SockType, UnixAddr, connect, socket},
    unistd::{Gid, Uid, User},
};

use crate::LocalUnixDaemonAccess;

pub use d2b_client::{
    CallOptions, CancellationToken, ClientError, DaemonClient, DaemonLifecycleRequest,
    DaemonMethod, DaemonTerminal, GuestCancelCall, GuestClient, GuestInspectCall, GuestOperation,
    GuestRetainedLogCall, RemoteErrorKind, Response, RetryClass, daemon_call_options,
};

/// One authenticated local daemon session. Endpoint construction remains
/// private to `d2b-daemon-access`; callers receive only the typed service.
pub struct LocalDaemonSession {
    daemon: DaemonClient,
}

impl std::fmt::Debug for LocalDaemonSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LocalDaemonSession([authenticated])")
    }
}

impl LocalDaemonSession {
    pub fn daemon(&self) -> &DaemonClient {
        &self.daemon
    }

    /// Bind a workload-scoped guest proxy to this authenticated daemon session.
    pub fn guest(&self, workload: &str) -> Result<GuestClient, ClientError> {
        let workload = WorkloadName::parse(workload).map_err(|_| ClientError::InvalidTarget)?;
        self.daemon.guest_proxy(&workload)
    }
}

impl LocalUnixDaemonAccess {
    /// Establish the fixed local-root daemon ComponentSession after verifying
    /// the connected peer is the configured `d2bd` service identity.
    pub async fn connect_component_session(&self) -> Result<LocalDaemonSession, ClientError> {
        let path = self.socket_path().to_owned();
        let fd = tokio::task::spawn_blocking(move || connect_seqpacket(&path))
            .await
            .map_err(|_| ClientError::ConnectFailed)??;
        let uid = Uid::effective().as_raw();
        let gid = Gid::effective().as_raw();
        let daemon_uid = User::from_name("d2bd")
            .map_err(|_| ClientError::ConnectFailed)?
            .ok_or(ClientError::ConnectFailed)?
            .uid
            .as_raw();
        let identity = local_daemon_endpoint_identity(uid, gid)?;
        let connector = HostSocketConnector::from_seqpacket_fd(
            fd,
            daemon_uid,
            identity,
            d2b_client::HandshakeCredentials::Nn,
        )?;
        let realm_path = RealmPath::parse("local-root").map_err(|_| ClientError::InvalidTarget)?;
        let realm = RealmId::derive(&realm_path);
        let connected = Client::new(
            RouteTable::new(vec![RouteRecord {
                owner: ServiceOwner::LocalRoot(realm.clone()),
                transport: TransportKind::LocalUnix,
            }]),
            connector,
        )
        .connect(
            TargetInput::LocalRoot(realm),
            ServiceKind::Daemon,
            TransportSelection::exact(TransportKind::LocalUnix),
        )
        .await?;
        Ok(LocalDaemonSession {
            daemon: DaemonClient::new(connected)?,
        })
    }
}

fn connect_seqpacket(path: &Path) -> Result<OwnedFd, ClientError> {
    let fd = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
        None,
    )
    .map_err(|_| ClientError::ConnectFailed)?;
    let address = UnixAddr::new(path).map_err(|_| ClientError::ConnectFailed)?;
    connect(fd.as_raw_fd(), &address).map_err(|_| ClientError::ConnectFailed)?;
    Ok(fd)
}
