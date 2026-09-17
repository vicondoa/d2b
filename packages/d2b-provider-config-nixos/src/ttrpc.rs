//! Typed config-nixos service transport.

use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        LazyLock,
        mpsc::{SyncSender, TrySendError, sync_channel},
    },
    thread,
};

use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    ConfigCaller, ConfigError, ConfigOperation, ConfigService, ConfigServiceDescriptor,
    ConfigSyncRequest, GuestConfigDocument, GuestSessionEvidence, SERVICE_NAME, SERVICE_PACKAGE,
};
use d2b_contracts_resource::v3::ResourceRef;

/// Backend for the closed config-nixos service.
///
/// A backend is bound to one authority and, for Guest reads, one authenticated
/// ComponentSession generation before its service map is registered.
pub trait ConfigServiceBackend: Send + Sync {
    /// Dispatch one already decoded operation payload.
    fn dispatch(&self, operation: ConfigOperation, payload: Value) -> Result<Value, ConfigError>;
}

/// Guest-side backend for the single host-declared configuration working copy.
pub struct GuestConfigReader {
    guest_ref: ResourceRef,
    evidence: GuestSessionEvidence,
    path: PathBuf,
}

impl std::fmt::Debug for GuestConfigReader {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuestConfigReader")
            .field("guest_ref", &self.guest_ref)
            .field("path", &"<redacted>")
            .field("evidence", &self.evidence)
            .finish()
    }
}

impl GuestConfigReader {
    /// Bind the reader to one admitted Guest ComponentSession generation.
    pub fn new(
        guest_ref: ResourceRef,
        boot_identity_digest: impl Into<String>,
        reconnect_generation: u64,
        path: impl Into<PathBuf>,
    ) -> Result<Self, ConfigError> {
        let request = ConfigSyncRequest::new(guest_ref.clone())?;
        let path = path.into();
        validate_reader_path(&path)?;
        let evidence = GuestSessionEvidence::new(
            request.guest_ref.clone(),
            boot_identity_digest,
            reconnect_generation,
        )?;
        Ok(Self {
            guest_ref,
            evidence,
            path,
        })
    }

    /// Return the canonical service-only descriptor.
    pub fn descriptor() -> ConfigServiceDescriptor {
        ConfigService::descriptor()
    }
}

impl ConfigServiceBackend for GuestConfigReader {
    fn dispatch(&self, operation: ConfigOperation, payload: Value) -> Result<Value, ConfigError> {
        if operation != ConfigOperation::ReadGuestConfig {
            return Err(ConfigError::Unauthorized);
        }
        let request: ConfigSyncRequest =
            serde_json::from_value(payload).map_err(|error| {
                tracing::debug!(
                    resource = %self.guest_ref.to_canonical_string(),
                    %error,
                    "config-nixos guest read request rejected: payload invalid",
                );
                ConfigError::InvalidRequest
            })?;
        if request.guest_ref != self.guest_ref {
            tracing::debug!(
                resource = %request.guest_ref.to_canonical_string(),
                "config-nixos guest read rejected: session mismatch",
            );
            return Err(ConfigError::SessionMismatch);
        }
        let document = match GuestConfigDocument::new(read_bounded_file(&self.path)?) {
            Ok(document) => document,
            Err(error) => {
                tracing::warn!(
                    resource = %request.guest_ref.to_canonical_string(),
                    %error,
                    "config-nixos guest config read failed",
                );
                return Err(error);
            }
        };
        let response = ConfigService.read_guest_config(
            ConfigCaller::Guest,
            &request,
            &self.evidence,
            document.bytes().to_vec(),
        )?;
        serde_json::to_value(response).map_err(|error| {
            tracing::warn!(
                resource = %request.guest_ref.to_canonical_string(),
                %error,
                "config-nixos guest read response encoding failed",
            );
            ConfigError::EncodingFailed
        })
    }
}

/// Build the only ttrpc service exposed by Provider/config-nixos.
pub fn create_ttrpc_services(
    backend: Arc<dyn ConfigServiceBackend>,
) -> HashMap<String, ttrpc::r#async::Service> {
    let mut methods = HashMap::new();
    for operation in ConfigOperation::ALL {
        let method = operation
            .as_str()
            .strip_prefix("ConfigNixosService/")
            .expect("canonical config operation prefix")
            .to_owned();
        methods.insert(
            method,
            Box::new(ConfigMethod {
                backend: Arc::clone(&backend),
                operation,
            }) as Box<dyn ttrpc::r#async::MethodHandler + Send + Sync>,
        );
    }
    let mut services = HashMap::new();
    services.insert(
        format!("{SERVICE_PACKAGE}.{SERVICE_NAME}"),
        ttrpc::r#async::Service {
            methods,
            streams: HashMap::new(),
        },
    );
    services
}

/// Typed client for the closed config-nixos service.
#[derive(Clone)]
pub struct ConfigNixosClient {
    client: ttrpc::r#async::Client,
}

impl std::fmt::Debug for ConfigNixosClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ConfigNixosClient(<redacted>)")
    }
}

impl ConfigNixosClient {
    /// Bind a client to an authenticated ComponentSession ttrpc client.
    pub fn new(client: ttrpc::r#async::Client) -> Self {
        Self { client }
    }

    /// Invoke one closed, typed service method.
    pub async fn call<Request, Response>(
        &self,
        context: ttrpc::context::Context,
        operation: ConfigOperation,
        request: &Request,
    ) -> ttrpc::Result<Response>
    where
        Request: Serialize,
        Response: DeserializeOwned,
    {
        let payload = serde_json::to_vec(request).map_err(|error| {
            tracing::warn!(
                operation = operation.as_str(),
                %error,
                "config-nixos client request encoding failed",
            );
            ttrpc::Error::RpcStatus(invalid_status())
        })?;
        let method = operation
            .as_str()
            .strip_prefix("ConfigNixosService/")
            .expect("canonical config operation prefix");
        let response = self
            .client
            .request(ttrpc::Request {
                service: format!("{SERVICE_PACKAGE}.{SERVICE_NAME}"),
                method: method.to_owned(),
                timeout_nano: context.timeout_nano,
                metadata: ttrpc::context::to_pb(context.metadata),
                payload,
                ..Default::default()
            })
            .await?;
        serde_json::from_slice(&response.payload).map_err(|error| {
            tracing::warn!(
                operation = operation.as_str(),
                %error,
                "config-nixos client response decoding failed",
            );
            ttrpc::Error::RpcStatus(invalid_status())
        })
    }
}

/// The bound on admitted-but-unstarted blocking config dispatches, per seat。
///
/// The Guest read walks the working-copy path with `O_NOFOLLOW` and reads the
/// document through `rustix`;that kernel path has no async form, so a
/// dispatch must not run on the runtime worker that polls this service: a
/// blocked worker stalls every other task sharing it. Dispatches run on one
/// dedicated bounded worker (plan R4) instead of the runtime's shared
/// blocking pool: the worker admits at most this many queued jobs, and a
/// full queue refuses the caller (mapped to `Unavailable`) rather than
/// parking an executor worker or growing a thread per call。
const MAX_DISPATCH_QUEUE_DEPTH: usize = 16;

type DispatchJob = Box<dyn FnOnce() + Send + 'static>;

/// One dedicated dispatch worker thread with its own bounded queue。
struct DispatchWorker {
    sender: SyncSender<DispatchJob>,
}

/// Start one named worker with its own bounded queue。
///
/// `None` records a worker that could not start, so every later call refuses
/// rather than retrying a failing spawn。
fn start_dispatch_worker() -> Option<DispatchWorker> {
    let (sender, receiver) = sync_channel::<DispatchJob>(MAX_DISPATCH_QUEUE_DEPTH);
    thread::Builder::new()
        .name("d2b-config-nixos-dispatch".to_owned())
        .spawn(move || {
            // The sanctioned R4 channel boundary: a blocking `sync_channel`
            // recv on the worker's own dedicated thread, with
            // `tokio::sync::oneshot` replies (plan R4 / KTD3).
            #[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
            while let Ok(job) = receiver.recv() {
                job();
            }
        })
        .ok()
        .map(|_| DispatchWorker { sender })
}

/// The blocking config-dispatch seat, started on first use。
static DISPATCH_WORKER: LazyLock<Option<DispatchWorker>> = LazyLock::new(start_dispatch_worker);

/// Dispatch one operation on the dedicated bounded dispatch worker。
///
/// The backend read is a synchronous kernel path, so it must not run on the
/// runtime worker that polls this service: a blocked worker stalls every other
/// task sharing it. Admission is a non-blocking `try_send`, so the caller's
/// executor is never parked;the outcome is awaited from the worker。
async fn dispatch_on_blocking_worker(
    backend: Arc<dyn ConfigServiceBackend>,
    operation: ConfigOperation,
    payload: Value,
) -> Result<Value, ttrpc::Error> {
    let (reply, outcome) = tokio::sync::oneshot::channel();
    let admitted = DISPATCH_WORKER
        .as_ref()
        .ok_or_else(|| rpc_error(ConfigError::Unavailable))?
        .sender
        .try_send(Box::new({
            let backend = Arc::clone(&backend);
            move || {
                // A panicking job drops the reply sender, so the waiter sees
                // `Unavailable` instead of hanging on a dead worker。

                let _ = reply.send(backend.dispatch(operation, payload));
            }
        }));
    match admitted {
        Ok(()) => {
            let dispatched = outcome.await.map_err(|_| {
                tracing::warn!(operation = operation.as_str(), "config-nixos service dispatch worker died");
                rpc_error(ConfigError::Unavailable)
            })?;
            dispatched.map_err(|error| {
                tracing::warn!(
                    operation = operation.as_str(),
                    %error,
                    "config-nixos service dispatch failed",
                );
                rpc_error(error)
            })
        }
        // A saturated queue refuses instead of growing threads or parking the
        // caller;the RPC surface maps that refusal to `Unavailable`, matching
        // the previous semaphore ceiling's error.

        Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
            tracing::warn!(
                operation = operation.as_str(),
                "config-nixos service dispatch refused: worker busy or gone",
            );
            Err(rpc_error(ConfigError::Unavailable))
        }
    }
}

struct ConfigMethod {
    backend: Arc<dyn ConfigServiceBackend>,
    operation: ConfigOperation,
}

#[async_trait]
impl ttrpc::r#async::MethodHandler for ConfigMethod {
    async fn handler(
        &self,
        _context: ttrpc::r#async::TtrpcContext,
        request: ttrpc::Request,
    ) -> ttrpc::Result<ttrpc::Response> {
        let payload: Value = serde_json::from_slice(&request.payload)
            .map_err(|error| {
                tracing::debug!(
                    operation = self.operation.as_str(),
                    %error,
                    "config-nixos service request rejected: payload invalid",
                );
                rpc_error(ConfigError::InvalidRequest)
            })?;
        if let Err(error) = ConfigService.validate_operation(self.operation, &payload) {
            tracing::debug!(
                operation = self.operation.as_str(),
                %error,
                "config-nixos service request rejected: validation failed",
            );
            return Err(rpc_error(error));
        }
        let value =
            dispatch_on_blocking_worker(Arc::clone(&self.backend), self.operation, payload).await?;
        let mut response = ttrpc::Response::new();
        response.set_status(ttrpc::get_status(ttrpc::Code::OK, ""));
        response.payload = serde_json::to_vec(&value).map_err(|error| {
            tracing::warn!(
                operation = self.operation.as_str(),
                %error,
                "config-nixos service response encoding failed",
            );
            rpc_error(ConfigError::EncodingFailed)
        })?;
        Ok(response)
    }
}

fn rpc_error(error: ConfigError) -> ttrpc::Error {
    let code = match error {
        ConfigError::Unauthorized => ttrpc::Code::PERMISSION_DENIED,
        ConfigError::SessionMismatch => ttrpc::Code::UNAUTHENTICATED,
        ConfigError::Unavailable => ttrpc::Code::UNAVAILABLE,
        ConfigError::DocumentTooLarge => ttrpc::Code::RESOURCE_EXHAUSTED,
        ConfigError::StagingMissing => ttrpc::Code::NOT_FOUND,
        ConfigError::EncodingFailed => ttrpc::Code::INTERNAL,
        _ => ttrpc::Code::INVALID_ARGUMENT,
    };
    ttrpc::Error::RpcStatus(ttrpc::get_status(code, error.code()))
}

fn invalid_status() -> ttrpc::Status {
    ttrpc::get_status(
        ttrpc::Code::INVALID_ARGUMENT,
        ConfigError::EncodingFailed.code(),
    )
}

fn validate_reader_path(path: &Path) -> Result<(), ConfigError> {
    if !path.is_absolute() {
        return Err(ConfigError::InvalidRequest);
    }
    for component in path.components() {
        if matches!(
            component,
            Component::CurDir | Component::ParentDir | Component::Prefix(_)
        ) {
            return Err(ConfigError::InvalidRequest);
        }
    }
    Ok(())
}

fn read_bounded_file(path: &Path) -> Result<Vec<u8>, ConfigError> {
    use rustix::{
        fs::{FileType, Mode, OFlags, fstat, open, openat},
        io::{Errno, read},
    };

    fn map_open_error(error: Errno) -> ConfigError {
        tracing::warn!(
            %error,
            "config-nixos guest config open failed",
        );
        match error {
            Errno::LOOP | Errno::NOTDIR => ConfigError::InvalidRequest,
            _ => ConfigError::Unavailable,
        }
    }

    if !path.is_absolute() {
        return Err(ConfigError::InvalidRequest);
    }
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(value) => components.push(value),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(ConfigError::InvalidRequest);
            }
        }
    }
    let Some((leaf, parents)) = components.split_last() else {
        return Err(ConfigError::InvalidRequest);
    };
    let directory_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let file_flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut directory = open("/", directory_flags, Mode::empty()).map_err(map_open_error)?;
    for parent in parents {
        directory =
            openat(&directory, *parent, directory_flags, Mode::empty()).map_err(map_open_error)?;
    }
    let file = openat(&directory, *leaf, file_flags, Mode::empty()).map_err(map_open_error)?;
    let metadata = fstat(&file).map_err(|error| {
        tracing::warn!(
            %error,
            "config-nixos guest config stat failed",
        );
        ConfigError::Unavailable
    })?;
    if FileType::from_raw_mode(metadata.st_mode) != FileType::RegularFile || metadata.st_nlink != 1
    {
        tracing::debug!("config-nixos guest config rejected: not a singly-linked regular file");
        return Err(ConfigError::InvalidRequest);
    }
    let size = usize::try_from(metadata.st_size).unwrap_or(crate::MAX_CONFIG_BYTES + 1);
    if size > crate::MAX_CONFIG_BYTES {
        tracing::debug!("config-nixos guest config rejected: document too large");
        return Err(ConfigError::DocumentTooLarge);
    }
    let mut bytes = Vec::with_capacity(size);
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        let count = read(&file, &mut chunk).map_err(|error| {
            tracing::warn!(
                %error,
                "config-nixos guest config read failed",
            );
            ConfigError::Unavailable
        })?;
        if count == 0 {
            break;
        }
        if bytes.len() + count > crate::MAX_CONFIG_BYTES {
            tracing::debug!("config-nixos guest config rejected: document too large");
            return Err(ConfigError::DocumentTooLarge);
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
    use std::time::Duration;

    /// A backend whose dispatch parks until the test releases it.
    struct ParkingBackend {
        started: tokio::sync::mpsc::UnboundedSender<()>,
        release: std::sync::Mutex<Option<Receiver<()>>>,
    }

    impl ConfigServiceBackend for ParkingBackend {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn dispatch(
            &self,
            _operation: ConfigOperation,
            _payload: Value,
        ) -> Result<Value, ConfigError> {
            let _ = self.started.send(());
            let release = self
                .release
                .lock()
                .expect("release lock")
                .take()
                .expect("release receiver");
            match release.recv_timeout(Duration::from_secs(5)) {
                Ok(()) => Ok(Value::Null),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                    Err(ConfigError::Unavailable)
                }
            }
        }
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "current_thread")]
    async fn a_blocking_backend_dispatch_does_not_occupy_the_polling_worker() {
        // Drive the registered service handler, not the helper behind it. On
        // the single-threaded runtime below the release can only be delivered
        // while the backend is parked if the handler left its polling worker
        // free: an inline `backend.dispatch` would stall the only worker and
        // the handler would answer the parked call with an error.
        let (started, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
        let (release, release_rx) = channel();
        let services = create_ttrpc_services(Arc::new(ParkingBackend {
            started,
            release: std::sync::Mutex::new(Some(release_rx)),
        }));
        let handler = services
            .get(&format!("{SERVICE_PACKAGE}.{SERVICE_NAME}"))
            .expect("config-nixos service is registered")
            .methods
            .get("ReadGuestConfig")
            .expect("guest read handler is registered");
        let payload = ConfigSyncRequest::new(
            ResourceRef::parse("Guest/parked-dispatch").expect("guest reference"),
        )
        .expect("config sync request");
        let request = ttrpc::Request {
            service: format!("{SERVICE_PACKAGE}.{SERVICE_NAME}"),
            method: "ReadGuestConfig".to_owned(),
            payload: serde_json::to_vec(&payload).expect("request encoding"),
            ..Default::default()
        };
        let context = ttrpc::r#async::TtrpcContext {
            mh: ttrpc::proto::MessageHeader::new_request(1, 0),
            metadata: HashMap::new(),
            timeout_nano: 0,
        };
        let handled = ttrpc::r#async::MethodHandler::handler(handler.as_ref(), context, request);
        tokio::pin!(handled);
        tokio::select! {
            completed = &mut handled => {
                panic!("handler completed before the backend parked: {completed:?}");
            }
            Some(()) = started_rx.recv() => {}
        }
        release.send(()).expect("release parked dispatch");
        let response = handled.await.expect("handler answer");
        assert_eq!(
            serde_json::from_slice::<Value>(&response.payload).expect("response payload"),
            Value::Null,
        );
    }
}
