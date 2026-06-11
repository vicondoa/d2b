use std::{
    env, fs,
    fs::File,
    io::{Read, Result as IoResult},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard,
    },
    task::{Context, Poll},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use futures::stream;
use nixling_ipc::{guest_proto as pb, guest_wire::GUEST_CONTROL_PROTOCOL_VERSION};
use protobuf::{EnumOrUnknown, Message, MessageField};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    time::Duration,
};
use tokio_vsock::{VsockAddr, VsockListener, VMADDR_CID_ANY};

use crate::{
    auth::{
        AuthConnectionContext, AuthDirection, AuthPurpose, BootIdSource, CapabilitiesProvider,
        CapabilitiesSnapshot, GuestAuthCore, GuestAuthError, InMemoryChallengeStore, NonceRng,
        SharedSecretToken, AUTH_NONCE_LEN, CONNECTION_INSTANCE_LEN, GUEST_CONTROL_AUTH_PORT,
    },
    generated::guest_control_ttrpc::{create_guest_control, GuestControl},
};

const TOKEN_FILE_NAME: &str = "guest_control_token";
const MAX_TOKEN_BYTES: usize = 4096;

type RuntimeAuthCore = GuestAuthCore<
    SharedSecretToken,
    OsNonceRng,
    ProcBootIdSource,
    MinimalCapabilitiesProvider,
    InMemoryChallengeStore,
    SystemClock,
>;
type SharedAuthCore = Arc<Mutex<RuntimeAuthCore>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestdServiceError {
    MissingCredentialsDirectory,
    UnsafeCredential,
    TokenUnavailable,
    Io,
    TimeUnavailable,
    Ttrpc,
}

impl GuestdServiceError {
    pub fn public_message(self) -> &'static str {
        match self {
            Self::MissingCredentialsDirectory => {
                "guest-control credential directory is unavailable"
            }
            Self::UnsafeCredential => "guest-control credential is unsafe",
            Self::TokenUnavailable => "guest-control token is unavailable",
            Self::Io => "guest-control I/O failed",
            Self::TimeUnavailable => "guest-control clock is unavailable",
            Self::Ttrpc => "guest-control service failed",
        }
    }
}

#[derive(Clone)]
pub struct GuestdServeConfig {
    pub vm_id: String,
    pub token: Vec<u8>,
}

impl GuestdServeConfig {
    pub fn new(vm_id: impl Into<String>, token: Vec<u8>) -> Result<Self, GuestdServiceError> {
        let vm_id = vm_id.into();
        if vm_id.is_empty() || token.is_empty() {
            return Err(GuestdServiceError::TokenUnavailable);
        }
        Ok(Self { vm_id, token })
    }
}

pub fn load_token_from_credentials_env() -> Result<Vec<u8>, GuestdServiceError> {
    let dir = env::var_os("CREDENTIALS_DIRECTORY")
        .map(PathBuf::from)
        .ok_or(GuestdServiceError::MissingCredentialsDirectory)?;
    load_token_from_credentials_dir(&dir)
}

pub fn load_token_from_credentials_dir(dir: &Path) -> Result<Vec<u8>, GuestdServiceError> {
    validate_safe_directory_path(dir)?;
    let path = dir.join(TOKEN_FILE_NAME);
    validate_token_path(dir, &path)?;
    let mut file = File::open(&path).map_err(|_| GuestdServiceError::TokenUnavailable)?;
    let mut data = Vec::new();
    file.by_ref()
        .take((MAX_TOKEN_BYTES + 1) as u64)
        .read_to_end(&mut data)
        .map_err(|_| GuestdServiceError::Io)?;
    if data.is_empty() || data.len() > MAX_TOKEN_BYTES {
        return Err(GuestdServiceError::TokenUnavailable);
    }
    while matches!(data.last(), Some(b'\n' | b'\r')) {
        data.pop();
    }
    if data.is_empty() {
        return Err(GuestdServiceError::TokenUnavailable);
    }
    Ok(data)
}

fn validate_token_path(dir: &Path, path: &Path) -> Result<(), GuestdServiceError> {
    if path.parent() != Some(dir) {
        return Err(GuestdServiceError::UnsafeCredential);
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| GuestdServiceError::TokenUnavailable)?;
    if metadata.file_type().is_symlink()
        || !metadata.file_type().is_file()
        || metadata.mode() & 0o077 != 0
        || !owner_is_safe(metadata.uid())
    {
        return Err(GuestdServiceError::UnsafeCredential);
    }
    Ok(())
}

fn validate_safe_directory_path(dir: &Path) -> Result<(), GuestdServiceError> {
    if !dir.is_absolute() {
        return Err(GuestdServiceError::MissingCredentialsDirectory);
    }
    if dir == Path::new("/nix/store") || dir.starts_with("/nix/store/") {
        return Err(GuestdServiceError::UnsafeCredential);
    }
    let mut current = PathBuf::new();
    for component in dir.components() {
        current.push(component);
        let metadata =
            fs::symlink_metadata(&current).map_err(|_| GuestdServiceError::UnsafeCredential)?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
            return Err(GuestdServiceError::UnsafeCredential);
        }
        let mode = metadata.mode();
        if !owner_is_safe(metadata.uid()) {
            return Err(GuestdServiceError::UnsafeCredential);
        }
        if mode & 0o002 != 0 && mode & 0o1000 == 0 {
            return Err(GuestdServiceError::UnsafeCredential);
        }
    }
    Ok(())
}

fn owner_is_safe(uid: u32) -> bool {
    uid == 0 || cfg!(test)
}

pub fn build_runtime_auth_core(token: Vec<u8>) -> Result<RuntimeAuthCore, GuestdServiceError> {
    let token = SharedSecretToken::new(token).map_err(|_| GuestdServiceError::TokenUnavailable)?;
    Ok(GuestAuthCore::new(
        token,
        OsNonceRng,
        ProcBootIdSource,
        MinimalCapabilitiesProvider::new(),
        InMemoryChallengeStore::default(),
        SystemClock,
    ))
}

pub async fn serve_vsock(config: GuestdServeConfig) -> Result<(), GuestdServiceError> {
    let auth = Arc::new(Mutex::new(build_runtime_auth_core(config.token)?));
    let listener = VsockListener::bind(VsockAddr::new(VMADDR_CID_ANY, GUEST_CONTROL_AUTH_PORT))
        .map_err(|_| GuestdServiceError::Ttrpc)?;

    loop {
        let Ok((stream, peer_addr)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        };
        let auth = Arc::clone(&auth);
        let vm_id = config.vm_id.clone();
        tokio::spawn(async move {
            if let Ok(context) = connection_context(vm_id, peer_addr.cid()) {
                let _ = run_single_connection(stream, auth, context).await;
            }
        });
    }
}

fn connection_context(
    vm_id: String,
    peer_cid: u32,
) -> Result<AuthConnectionContext, GuestdServiceError> {
    Ok(AuthConnectionContext {
        vm_id,
        protocol_version: GUEST_CONTROL_PROTOCOL_VERSION,
        guest_control_port: GUEST_CONTROL_AUTH_PORT,
        peer_cid,
        direction: AuthDirection::HostToGuest,
        purpose: AuthPurpose::GuestControlAuthV1,
        connection_instance: new_connection_instance()?,
    })
}

fn new_connection_instance() -> Result<[u8; CONNECTION_INSTANCE_LEN], GuestdServiceError> {
    let mut instance = [0_u8; CONNECTION_INSTANCE_LEN];
    let mut rng = OsNonceRng;
    rng.fill_bytes(&mut instance)
        .map_err(|_| GuestdServiceError::TokenUnavailable)?;
    Ok(instance)
}

pub async fn run_single_connection<S>(
    stream: S,
    auth: SharedAuthCore,
    context: AuthConnectionContext,
) -> Result<(), GuestdServiceError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
{
    let cleanup = ConnectionCleanup::new(Arc::clone(&auth), context.clone());
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let wrapped = CleanupStream::new(stream, cleanup.clone(), done_tx);
    let listener = ttrpc::r#async::transport::Listener::new(stream::once(async move {
        Ok::<_, std::io::Error>(wrapped)
    }));
    let service = Arc::new(GuestControlService::new(auth, context));
    let mut server = ttrpc::r#async::Server::new()
        .add_listener(listener)
        .register_service(create_guest_control(service));
    server
        .start()
        .await
        .map_err(|_| GuestdServiceError::Ttrpc)?;
    let _ = done_rx.await;
    cleanup.close();
    server.disconnect().await;
    Ok(())
}

#[derive(Clone)]
pub struct GuestControlService {
    auth: SharedAuthCore,
    context: AuthConnectionContext,
}

impl GuestControlService {
    pub fn new(auth: SharedAuthCore, context: AuthConnectionContext) -> Self {
        Self { auth, context }
    }

    fn lock_auth(&self) -> Result<MutexGuard<'_, RuntimeAuthCore>, ttrpc::Error> {
        self.auth
            .lock()
            .map_err(|_| rpc_status(ttrpc::Code::INTERNAL, "guest-control-internal-error"))
    }

    fn require_authenticated(&self) -> Result<(), ttrpc::Error> {
        if self.lock_auth()?.is_authenticated(&self.context) {
            Ok(())
        } else {
            Err(rpc_status(
                ttrpc::Code::UNAUTHENTICATED,
                "guest-control-unauthenticated",
            ))
        }
    }

    fn validate_metadata(
        &self,
        metadata: Option<&pb::RequestMetadata>,
    ) -> Result<(), ttrpc::Error> {
        let metadata = metadata.ok_or_else(|| {
            rpc_status(
                ttrpc::Code::INVALID_ARGUMENT,
                "guest-control-metadata-invalid",
            )
        })?;
        if metadata.vm_id != self.context.vm_id
            || metadata.protocol_version != self.context.protocol_version
        {
            return Err(rpc_status(
                ttrpc::Code::INVALID_ARGUMENT,
                "guest-control-metadata-invalid",
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl GuestControl for GuestControlService {
    async fn hello(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::HelloRequest,
    ) -> ttrpc::Result<pb::HelloResponse> {
        self.lock_auth()?
            .hello(&self.context, &request)
            .map_err(map_auth_rpc_error)
    }

    async fn authenticate(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::AuthenticateRequest,
    ) -> ttrpc::Result<pb::AuthenticateResponse> {
        match self.lock_auth()?.authenticate(&self.context, &request) {
            Ok(response) => Ok(response),
            Err(error) => {
                let mut response = pb::AuthenticateResponse::new();
                response.error = MessageField::some(guest_error(error_kind_for_auth(error)));
                Ok(response)
            }
        }
    }

    async fn capabilities(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::CapabilitiesRequest,
    ) -> ttrpc::Result<pb::CapabilitiesResponse> {
        self.validate_metadata(request.metadata.as_ref())?;
        self.lock_auth()?
            .capabilities(&self.context)
            .map_err(map_auth_rpc_error)
    }

    async fn health(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::HealthRequest,
    ) -> ttrpc::Result<pb::HealthResponse> {
        self.validate_metadata(request.metadata.as_ref())?;
        self.lock_auth()?
            .health(&self.context)
            .map_err(map_auth_rpc_error)
    }

    async fn exec_create(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        _request: pb::ExecCreateRequest,
    ) -> ttrpc::Result<pb::ExecCreateResponse> {
        self.require_authenticated()?;
        let mut response = pb::ExecCreateResponse::new();
        response.state = EnumOrUnknown::new(pb::ExecState::EXEC_STATE_PROTOCOL_ERROR);
        response.error = MessageField::some(exec_disabled_error());
        Ok(response)
    }

    async fn exec_inspect(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        _request: pb::ExecInspectRequest,
    ) -> ttrpc::Result<pb::ExecInspectResponse> {
        self.require_authenticated()?;
        let mut response = pb::ExecInspectResponse::new();
        response.state = EnumOrUnknown::new(pb::ExecState::EXEC_STATE_PROTOCOL_ERROR);
        response.error = MessageField::some(exec_disabled_error());
        Ok(response)
    }

    async fn exec_wait(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        _request: pb::ExecWaitRequest,
    ) -> ttrpc::Result<pb::ExecWaitResponse> {
        self.require_authenticated()?;
        let mut response = pb::ExecWaitResponse::new();
        response.state = EnumOrUnknown::new(pb::ExecState::EXEC_STATE_PROTOCOL_ERROR);
        response.error = MessageField::some(exec_disabled_error());
        Ok(response)
    }

    async fn exec_logs(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        _request: pb::ExecLogsRequest,
    ) -> ttrpc::Result<pb::ExecLogsResponse> {
        self.require_authenticated()?;
        let mut response = pb::ExecLogsResponse::new();
        response.error = MessageField::some(exec_disabled_error());
        Ok(response)
    }

    async fn write_stdin(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        _request: pb::WriteStdinRequest,
    ) -> ttrpc::Result<pb::WriteStdinResponse> {
        self.require_authenticated()?;
        let mut response = pb::WriteStdinResponse::new();
        response.error = MessageField::some(exec_disabled_error());
        Ok(response)
    }

    async fn read_output(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        _request: pb::ReadOutputRequest,
    ) -> ttrpc::Result<pb::ReadOutputResponse> {
        self.require_authenticated()?;
        let mut response = pb::ReadOutputResponse::new();
        response.error = MessageField::some(exec_disabled_error());
        Ok(response)
    }

    async fn close_stdin(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        _request: pb::CloseStdinRequest,
    ) -> ttrpc::Result<pb::CloseStdinResponse> {
        self.require_authenticated()?;
        let mut response = pb::CloseStdinResponse::new();
        response.error = MessageField::some(exec_disabled_error());
        Ok(response)
    }

    async fn tty_win_resize(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        _request: pb::TtyWinResizeRequest,
    ) -> ttrpc::Result<pb::ControlAck> {
        self.require_authenticated()?;
        Ok(control_ack_disabled())
    }

    async fn exec_signal(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        _request: pb::ExecSignalRequest,
    ) -> ttrpc::Result<pb::ControlAck> {
        self.require_authenticated()?;
        Ok(control_ack_disabled())
    }

    async fn exec_cancel(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        _request: pb::ExecCancelRequest,
    ) -> ttrpc::Result<pb::ControlAck> {
        self.require_authenticated()?;
        Ok(control_ack_disabled())
    }
}

fn map_auth_rpc_error(error: GuestAuthError) -> ttrpc::Error {
    match error {
        GuestAuthError::Unauthenticated | GuestAuthError::MacRejected => rpc_status(
            ttrpc::Code::UNAUTHENTICATED,
            "guest-control-unauthenticated",
        ),
        GuestAuthError::ChallengeCapacityExceeded => rpc_status(
            ttrpc::Code::RESOURCE_EXHAUSTED,
            "guest-control-auth-capacity",
        ),
        GuestAuthError::MetadataMissing
        | GuestAuthError::MetadataMismatch
        | GuestAuthError::ProtocolVersionMismatch
        | GuestAuthError::TranscriptVersionMismatch
        | GuestAuthError::NonceLengthInvalid
        | GuestAuthError::TagLengthInvalid
        | GuestAuthError::BootIdMismatch
        | GuestAuthError::ChallengeNotFound
        | GuestAuthError::ChallengeExpired
        | GuestAuthError::ChallengeMismatch => {
            rpc_status(ttrpc::Code::INVALID_ARGUMENT, "guest-control-auth-invalid")
        }
        _ => rpc_status(ttrpc::Code::INTERNAL, "guest-control-internal-error"),
    }
}

fn rpc_status(code: ttrpc::Code, message: &'static str) -> ttrpc::Error {
    ttrpc::Error::RpcStatus(ttrpc::get_status(code, message.to_owned()))
}

fn guest_error(kind: pb::GuestControlErrorKind) -> pb::GuestControlError {
    let mut error = pb::GuestControlError::new();
    error.kind = EnumOrUnknown::new(kind);
    error.remediation = EnumOrUnknown::new(pb::HealthRemediation::HEALTH_REMEDIATION_RETRY);
    error
}

fn error_kind_for_auth(error: GuestAuthError) -> pb::GuestControlErrorKind {
    match error {
        GuestAuthError::Unauthenticated
        | GuestAuthError::MacRejected
        | GuestAuthError::TokenUnavailable => {
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_AUTH_FAILED
        }
        GuestAuthError::ProtocolVersionMismatch => {
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR
        }
        _ => pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
    }
}

fn exec_disabled_error() -> pb::GuestControlError {
    guest_error(pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_GUEST_EXEC_DISABLED)
}

fn control_ack_disabled() -> pb::ControlAck {
    let mut ack = pb::ControlAck::new();
    ack.error = MessageField::some(exec_disabled_error());
    ack
}

#[derive(Clone)]
struct ConnectionCleanup {
    auth: SharedAuthCore,
    context: AuthConnectionContext,
    closed: Arc<AtomicBool>,
}

impl ConnectionCleanup {
    fn new(auth: SharedAuthCore, context: AuthConnectionContext) -> Self {
        Self {
            auth,
            context,
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    fn close(&self) {
        if !self.closed.swap(true, Ordering::SeqCst) {
            if let Ok(mut auth) = self.auth.lock() {
                auth.close_connection(&self.context);
            }
        }
    }
}

struct CleanupStream<S> {
    inner: S,
    cleanup: ConnectionCleanup,
    done: Option<tokio::sync::oneshot::Sender<()>>,
}

impl<S> CleanupStream<S> {
    fn new(inner: S, cleanup: ConnectionCleanup, done: tokio::sync::oneshot::Sender<()>) -> Self {
        Self {
            inner,
            cleanup,
            done: Some(done),
        }
    }
}

impl<S> Drop for CleanupStream<S> {
    fn drop(&mut self) {
        self.cleanup.close();
        if let Some(done) = self.done.take() {
            let _ = done.send(());
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for CleanupStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<IoResult<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for CleanupStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<IoResult<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<IoResult<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<IoResult<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

pub struct OsNonceRng;

impl OsNonceRng {
    fn fill_bytes(&mut self, out: &mut [u8]) -> Result<(), OsNonceError> {
        let mut file = File::open("/dev/urandom").map_err(|_| OsNonceError)?;
        file.read_exact(out).map_err(|_| OsNonceError)
    }
}

impl NonceRng for OsNonceRng {
    fn fill_nonce(&mut self, out: &mut [u8; AUTH_NONCE_LEN]) -> Result<(), GuestAuthError> {
        self.fill_bytes(out)
            .map_err(|_| GuestAuthError::TokenUnavailable)
    }
}

pub struct ProcBootIdSource;

impl BootIdSource for ProcBootIdSource {
    fn guest_boot_id(&self) -> Result<String, GuestAuthError> {
        let boot_id = fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map_err(|_| GuestAuthError::CapabilitiesUnavailable)?;
        let boot_id = boot_id.trim().to_owned();
        if boot_id.is_empty() || boot_id.len() > 128 {
            return Err(GuestAuthError::CapabilitiesUnavailable);
        }
        Ok(boot_id)
    }
}

pub struct SystemClock;

impl crate::auth::Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }
}

pub struct MinimalCapabilitiesProvider {
    snapshot: CapabilitiesSnapshot,
}

impl MinimalCapabilitiesProvider {
    pub fn new() -> Self {
        let mut limits = pb::GuestEffectiveLimits::new();
        limits.max_chunk_bytes = 64 * 1024;
        limits.max_recv_message_bytes = 4 * 1024 * 1024;
        limits.decoded_write_stdin_bytes_per_connection = 16 * 1024 * 1024;
        limits.write_stdin_handlers_per_connection = 4;
        limits.stdin_queue_chunks_per_exec = 1;
        limits.stdout_live_buffer_bytes = 1024 * 1024;
        limits.stderr_live_buffer_bytes = 1024 * 1024;
        limits.detached_stdout_log_bytes = 16 * 1024 * 1024;
        limits.detached_stderr_log_bytes = 16 * 1024 * 1024;
        limits.long_poll_timeout_ms = 100;
        limits.slow_consumer_grace_ms = 30_000;
        limits.exec_sessions_per_vm = 32;
        limits.attached_sessions_per_vm = 8;
        limits.pending_read_output_waits_per_stream = 64;
        limits.pending_exec_waits_per_vm = 64;
        limits.rpc_rate_per_connection_per_second = 200;
        limits.rpc_rate_per_vm_burst = 1_000;

        let mut capabilities = pb::CapabilitiesResponse::new();
        capabilities.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;
        capabilities.capabilities.push(EnumOrUnknown::new(
            pb::GuestCapability::GUEST_CAPABILITY_HEALTH,
        ));
        capabilities.capabilities.push(EnumOrUnknown::new(
            pb::GuestCapability::GUEST_CAPABILITY_CAPABILITIES,
        ));
        capabilities.limits = MessageField::some(limits);

        let mut health = pb::HealthResponse::new();
        health.origin = EnumOrUnknown::new(pb::HealthOrigin::HEALTH_ORIGIN_GUEST_REPORTED);
        health.state = EnumOrUnknown::new(pb::HealthState::HEALTH_STATE_HEALTHY);
        health.reason = EnumOrUnknown::new(pb::HealthReason::HEALTH_REASON_NONE);
        health.remediation = EnumOrUnknown::new(pb::HealthRemediation::HEALTH_REMEDIATION_NONE);
        health.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;
        health.capabilities = capabilities.capabilities.clone();

        let capabilities_hash = sha256_hex(&capabilities.write_to_bytes().unwrap_or_default());
        Self {
            snapshot: CapabilitiesSnapshot {
                capabilities_hash,
                health,
                capabilities,
            },
        }
    }
}

impl Default for MinimalCapabilitiesProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CapabilitiesProvider for MinimalCapabilitiesProvider {
    fn snapshot(&self) -> Result<CapabilitiesSnapshot, GuestAuthError> {
        Ok(self.snapshot.clone())
    }
}

fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[derive(Debug, Clone, Copy)]
pub struct OsNonceError;

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, os::unix::fs::PermissionsExt};

    use crate::auth::{encode_transcript, ProofRole};
    use crate::TokenSource;

    use super::*;

    const TEST_TOKEN: &[u8] = b"service-test-token";
    const HOST_NONCE: [u8; AUTH_NONCE_LEN] = [0x44; AUTH_NONCE_LEN];

    fn test_context(instance: u8) -> AuthConnectionContext {
        AuthConnectionContext {
            vm_id: "corp-vm".to_owned(),
            protocol_version: GUEST_CONTROL_PROTOCOL_VERSION,
            guest_control_port: GUEST_CONTROL_AUTH_PORT,
            peer_cid: 2,
            direction: AuthDirection::HostToGuest,
            purpose: AuthPurpose::GuestControlAuthV1,
            connection_instance: [instance; CONNECTION_INSTANCE_LEN],
        }
    }

    fn test_auth() -> SharedAuthCore {
        Arc::new(Mutex::new(
            build_runtime_auth_core(TEST_TOKEN.to_vec()).unwrap(),
        ))
    }

    fn test_service(instance: u8) -> GuestControlService {
        GuestControlService::new(test_auth(), test_context(instance))
    }

    fn ttrpc_context() -> ttrpc::r#async::TtrpcContext {
        ttrpc::r#async::TtrpcContext {
            mh: ttrpc::proto::MessageHeader::new_request(1, 0),
            metadata: HashMap::new(),
            timeout_nano: 0,
        }
    }

    fn metadata() -> MessageField<pb::RequestMetadata> {
        let mut metadata = pb::RequestMetadata::new();
        metadata.vm_id = "corp-vm".to_owned();
        metadata.request_id = "req-1".to_owned();
        metadata.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;
        MessageField::some(metadata)
    }

    fn health_request() -> pb::HealthRequest {
        let mut request = pb::HealthRequest::new();
        request.metadata = metadata();
        request
    }

    fn capabilities_request() -> pb::CapabilitiesRequest {
        let mut request = pb::CapabilitiesRequest::new();
        request.metadata = metadata();
        request
    }

    async fn authenticate(service: &GuestControlService) {
        let ctx = ttrpc_context();
        let mut hello = pb::HelloRequest::new();
        hello.metadata = metadata();
        hello.host_nonce = HOST_NONCE.to_vec();
        hello.transcript_version = crate::auth::AUTH_TRANSCRIPT_VERSION;
        let hello_response = service.hello(&ctx, hello).await.unwrap();
        let guest_nonce: [u8; AUTH_NONCE_LEN] = hello_response
            .guest_nonce
            .as_slice()
            .try_into()
            .expect("fixed guest nonce");
        let transcript = encode_transcript(
            ProofRole::Host,
            &service.context,
            &HOST_NONCE,
            &guest_nonce,
            &hello_response.guest_boot_id,
            None,
        );
        let host_tag = SharedSecretToken::new(TEST_TOKEN.to_vec())
            .unwrap()
            .sign_tag(&transcript)
            .unwrap();

        let mut auth = pb::AuthenticateRequest::new();
        auth.metadata = metadata();
        auth.host_nonce = HOST_NONCE.to_vec();
        auth.guest_nonce = hello_response.guest_nonce;
        auth.guest_boot_id = hello_response.guest_boot_id;
        auth.transcript_version = crate::auth::AUTH_TRANSCRIPT_VERSION;
        auth.host_auth_tag = host_tag.to_vec();
        let response = service.authenticate(&ctx, auth).await.unwrap();
        assert!(response.error.is_none());
        assert!(response.health.is_some());
        assert!(response.capabilities.is_some());
    }

    fn assert_unauthenticated<T: std::fmt::Debug>(result: ttrpc::Result<T>) {
        match result {
            Err(ttrpc::Error::RpcStatus(status)) => {
                assert_eq!(
                    status.code.enum_value().unwrap(),
                    ttrpc::Code::UNAUTHENTICATED
                );
                assert!(!status.message.contains("token"));
            }
            other => panic!("expected unauthenticated status, got {other:?}"),
        }
    }

    fn assert_disabled(error: &pb::GuestControlError) {
        assert_eq!(
            error.kind.enum_value().unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_GUEST_EXEC_DISABLED
        );
    }

    #[tokio::test]
    async fn health_and_capabilities_are_same_connection_authenticated() {
        let ctx = ttrpc_context();
        let service = test_service(1);
        assert_unauthenticated(service.health(&ctx, health_request()).await);
        assert_unauthenticated(service.capabilities(&ctx, capabilities_request()).await);

        authenticate(&service).await;
        assert!(service.health(&ctx, health_request()).await.is_ok());
        assert!(service.capabilities(&ctx, capabilities_request()).await.is_ok());

        let other = GuestControlService::new(Arc::clone(&service.auth), test_context(2));
        assert_unauthenticated(other.health(&ctx, health_request()).await);
    }

    #[tokio::test]
    async fn health_and_capabilities_validate_request_metadata() {
        let ctx = ttrpc_context();
        let service = test_service(7);
        authenticate(&service).await;

        let mut wrong = health_request();
        wrong.metadata.as_mut().unwrap().vm_id = "other-vm".to_owned();
        match service.health(&ctx, wrong).await {
            Err(ttrpc::Error::RpcStatus(status)) => {
                assert_eq!(
                    status.code.enum_value().unwrap(),
                    ttrpc::Code::INVALID_ARGUMENT
                );
                assert!(!status.message.contains("other-vm"));
            }
            other => panic!("expected invalid metadata status, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn close_connection_drops_authenticated_state() {
        let ctx = ttrpc_context();
        let service = test_service(3);
        authenticate(&service).await;
        assert!(service.health(&ctx, health_request()).await.is_ok());
        service
            .auth
            .lock()
            .unwrap()
            .close_connection(&service.context);
        assert_unauthenticated(service.health(&ctx, health_request()).await);
    }

    #[tokio::test]
    async fn exec_methods_are_preauth_gated_then_disabled() {
        let ctx = ttrpc_context();
        let service = test_service(4);
        assert_unauthenticated(
            service
                .exec_create(&ctx, pb::ExecCreateRequest::new())
                .await,
        );
        assert_unauthenticated(
            service
                .read_output(&ctx, pb::ReadOutputRequest::new())
                .await,
        );
        assert_unauthenticated(
            service
                .exec_signal(&ctx, pb::ExecSignalRequest::new())
                .await,
        );

        authenticate(&service).await;
        assert_disabled(
            service
                .exec_create(&ctx, pb::ExecCreateRequest::new())
                .await
                .unwrap()
                .error
                .as_ref()
                .unwrap(),
        );
        assert_disabled(
            service
                .exec_inspect(&ctx, pb::ExecInspectRequest::new())
                .await
                .unwrap()
                .error
                .as_ref()
                .unwrap(),
        );
        assert_disabled(
            service
                .exec_wait(&ctx, pb::ExecWaitRequest::new())
                .await
                .unwrap()
                .error
                .as_ref()
                .unwrap(),
        );
        assert_disabled(
            service
                .exec_logs(&ctx, pb::ExecLogsRequest::new())
                .await
                .unwrap()
                .error
                .as_ref()
                .unwrap(),
        );
        assert_disabled(
            service
                .write_stdin(&ctx, pb::WriteStdinRequest::new())
                .await
                .unwrap()
                .error
                .as_ref()
                .unwrap(),
        );
        assert_disabled(
            service
                .read_output(&ctx, pb::ReadOutputRequest::new())
                .await
                .unwrap()
                .error
                .as_ref()
                .unwrap(),
        );
        assert_disabled(
            service
                .close_stdin(&ctx, pb::CloseStdinRequest::new())
                .await
                .unwrap()
                .error
                .as_ref()
                .unwrap(),
        );
        assert_disabled(
            service
                .tty_win_resize(&ctx, pb::TtyWinResizeRequest::new())
                .await
                .unwrap()
                .error
                .as_ref()
                .unwrap(),
        );
        assert_disabled(
            service
                .exec_signal(&ctx, pb::ExecSignalRequest::new())
                .await
                .unwrap()
                .error
                .as_ref()
                .unwrap(),
        );
        assert_disabled(
            service
                .exec_cancel(&ctx, pb::ExecCancelRequest::new())
                .await
                .unwrap()
                .error
                .as_ref()
                .unwrap(),
        );
    }

    #[test]
    fn credential_loader_rejects_unsafe_sources_without_leaking_path() {
        let root =
            std::env::temp_dir().join(format!("nixling-guestd-cred-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let token = root.join(TOKEN_FILE_NAME);
        fs::write(&token, b"secret-token\n").unwrap();
        fs::set_permissions(&token, fs::Permissions::from_mode(0o600)).unwrap();

        assert_eq!(
            load_token_from_credentials_dir(&root).unwrap(),
            b"secret-token"
        );

        fs::set_permissions(&token, fs::Permissions::from_mode(0o666)).unwrap();
        let error = load_token_from_credentials_dir(&root).unwrap_err();
        assert_eq!(error, GuestdServiceError::UnsafeCredential);
        assert!(!error.public_message().contains("nixling-guestd-cred-test"));
        let _ = fs::remove_dir_all(&root);
    }
}
