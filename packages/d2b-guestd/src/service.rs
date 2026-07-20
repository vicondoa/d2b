//! Production guest-service listener and bootstrap lifecycle.

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use tokio_vsock::{VMADDR_CID_ANY, VMADDR_CID_HOST, VsockAddr, VsockListener, VsockStream};

use crate::configured_launches::ConfiguredLaunchInventory;
use crate::guest_service::{GuestOperationHandler, serve_guest_session};
use crate::production_guest::{ProductionGuestConfig, ProductionGuestOperations};
use crate::service_v2::{
    FramedGuestTransport, GuestSessionAuthority, GuestSessionError, GuestSessionMaterial,
    GuestStaticIdentity, SealedIdentityStore, SystemdGuestCredentialSource, TpmSealedIdentityStore,
};

pub const DEFAULT_SEALED_IDENTITY_PATH: &str = "/var/lib/d2b/guest-identity.sealed";
pub const DEFAULT_SYSTEMD_CREDS_PATH: &str = "/run/current-system/sw/bin/systemd-creds";
pub const DEFAULT_LOGIN_SHELL_PATH: &str = "/run/current-system/sw/bin/bash";
pub const GUEST_SERVICE_VSOCK_PORT: u32 = 14_318;
#[doc(hidden)]
pub const TYPED_ACTIVATION_POLICY_MARKERS: [&str; 4] = [
    "d2b.activation.v2.ActivationService",
    "GUEST_CAPABILITY_SYSTEM_ACTIVATION",
    "KillMode=control-group",
    "RuntimeMaxSec=",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestdServiceError {
    InvalidConfiguration,
    Session,
    Clock,
    Transport,
}

impl GuestdServiceError {
    pub const fn public_message(self) -> &'static str {
        match self {
            Self::InvalidConfiguration => "guest-service configuration is invalid",
            Self::Session => "guest-service session failed",
            Self::Clock => "guest-service clock is unavailable",
            Self::Transport => "guest-service transport failed",
        }
    }
}

impl From<GuestSessionError> for GuestdServiceError {
    fn from(_: GuestSessionError) -> Self {
        Self::Session
    }
}

#[derive(Clone)]
pub struct GuestdServeConfig {
    pub vm_id: String,
    pub sealed_identity_path: PathBuf,
    pub systemd_creds_path: PathBuf,
    pub production: ProductionGuestConfig,
    pub configured_launches_sha256: Option<[u8; 32]>,
}

impl GuestdServeConfig {
    pub fn new(
        vm_id: impl Into<String>,
        sealed_identity_path: impl Into<PathBuf>,
        systemd_creds_path: impl Into<PathBuf>,
    ) -> Result<Self, GuestdServiceError> {
        let vm_id = vm_id.into();
        let sealed_identity_path = sealed_identity_path.into();
        let systemd_creds_path = systemd_creds_path.into();
        if !valid_vm_id(&vm_id)
            || !sealed_identity_path.is_absolute()
            || !systemd_creds_path.is_absolute()
        {
            return Err(GuestdServiceError::InvalidConfiguration);
        }
        Ok(Self {
            production: ProductionGuestConfig::disabled(vm_id.clone()),
            configured_launches_sha256: None,
            vm_id,
            sealed_identity_path,
            systemd_creds_path,
        })
    }

    pub fn with_configured_launches_sha256(mut self, digest: Option<[u8; 32]>) -> Self {
        self.configured_launches_sha256 = digest;
        self
    }

    pub fn with_production(
        mut self,
        production: ProductionGuestConfig,
    ) -> Result<Self, GuestdServiceError> {
        if production.workload_id.is_empty() {
            return Err(GuestdServiceError::InvalidConfiguration);
        }
        self.production = production;
        Ok(self)
    }
}

fn valid_vm_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes.next().is_some_and(|first| first.is_ascii_lowercase())
        && value.len() <= 128
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

pub async fn serve_vsock(mut config: GuestdServeConfig) -> Result<(), GuestdServiceError> {
    let credential_source = SystemdGuestCredentialSource::from_environment()?;
    let material = GuestSessionMaterial::from_runtime_credentials(&credential_source)?;
    if let Some(expected_digest) = config.configured_launches_sha256 {
        let inventory = ConfiguredLaunchInventory::load(
            &credential_source,
            expected_digest,
            &config.production.workload_id,
        )?;
        config.production.configured_launch_realm_id = Some(inventory.realm_id().to_owned());
        config.production.configured_launch_workload_digest = Some(*inventory.workload_digest());
        debug_assert_eq!(inventory.workload_id(), config.production.workload_id);
        config.production.configured_launches = inventory.into_entries();
    }
    let production = Arc::new(ProductionGuestOperations::new(config.production.clone()).await?);
    let activation = production.activation_runtime();
    let operations: Arc<dyn GuestOperationHandler> = production;
    let identity_store: Arc<dyn SealedIdentityStore> = Arc::new(TpmSealedIdentityStore::new(
        config.sealed_identity_path,
        config.systemd_creds_path,
    )?);
    let authority = Arc::new(GuestSessionAuthority::new(
        material,
        Arc::clone(&identity_store),
    ));

    if identity_store.load()?.is_none() {
        let stream =
            VsockStream::connect(VsockAddr::new(VMADDR_CID_HOST, GUEST_SERVICE_VSOCK_PORT))
                .await
                .map_err(|_| GuestdServiceError::Transport)?;
        let mut transport = FramedGuestTransport::new(stream);
        let evidence = transport.receive_bootstrap_evidence().await?;
        let session = authority
            .establish_bootstrap_initiator(transport, evidence, Instant::now(), unix_time_ms()?)
            .await?;
        serve_guest_session(session, Arc::clone(&operations), Arc::clone(&activation)).await?;
        if identity_store.load()?.is_none() {
            return Err(GuestdServiceError::Session);
        }
    }

    let listener = VsockListener::bind(VsockAddr::new(VMADDR_CID_ANY, GUEST_SERVICE_VSOCK_PORT))
        .map_err(|_| GuestdServiceError::Transport)?;
    loop {
        let (stream, peer) = listener
            .accept()
            .await
            .map_err(|_| GuestdServiceError::Transport)?;
        if peer.cid() != VMADDR_CID_HOST {
            continue;
        }
        let authority = Arc::clone(&authority);
        let operations = Arc::clone(&operations);
        let activation = Arc::clone(&activation);
        tokio::spawn(async move {
            let Ok(now_unix_ms) = unix_time_ms() else {
                return;
            };
            let Ok(session) = authority
                .establish_responder(
                    FramedGuestTransport::new(stream),
                    Instant::now(),
                    now_unix_ms,
                )
                .await
            else {
                return;
            };
            let _ = serve_guest_session(session, operations, activation).await;
        });
    }
}

fn unix_time_ms() -> Result<u64, GuestdServiceError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| GuestdServiceError::Clock)?;
    u64::try_from(elapsed.as_millis()).map_err(|_| GuestdServiceError::Clock)
}

pub fn static_identity_public_key(private: [u8; 32]) -> Result<[u8; 32], GuestdServiceError> {
    GuestStaticIdentity::from_private_key(private)
        .and_then(|identity| identity.public_key())
        .map_err(GuestdServiceError::from)
}
