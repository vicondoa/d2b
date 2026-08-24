//! Typed config-nixos service transport.

use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::Arc,
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
    fn dispatch(
        &self,
        operation: ConfigOperation,
        payload: Value,
    ) -> Result<Value, ConfigError>;
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
        let evidence =
            GuestSessionEvidence::new(request.guest_ref.clone(), boot_identity_digest, reconnect_generation)?;
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
    fn dispatch(
        &self,
        operation: ConfigOperation,
        payload: Value,
    ) -> Result<Value, ConfigError> {
        if operation != ConfigOperation::ReadGuestConfig {
            return Err(ConfigError::Unauthorized);
        }
        let request: ConfigSyncRequest =
            serde_json::from_value(payload).map_err(|_| ConfigError::InvalidRequest)?;
        if request.guest_ref != self.guest_ref {
            return Err(ConfigError::SessionMismatch);
        }
        let document = GuestConfigDocument::new(read_bounded_file(&self.path)?)?;
        let response = ConfigService.read_guest_config(
            ConfigCaller::Guest,
            &request,
            &self.evidence,
            document.bytes().to_vec(),
        )?;
        serde_json::to_value(response).map_err(|_| ConfigError::EncodingFailed)
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
        let payload = serde_json::to_vec(request)
            .map_err(|_| ttrpc::Error::RpcStatus(invalid_status()))?;
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
        serde_json::from_slice(&response.payload)
            .map_err(|_| ttrpc::Error::RpcStatus(invalid_status()))
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
            .map_err(|_| rpc_error(ConfigError::InvalidRequest))?;
        ConfigService
            .validate_operation(self.operation, &payload)
            .map_err(rpc_error)?;
        let value = self
            .backend
            .dispatch(self.operation, payload)
            .map_err(rpc_error)?;
        let mut response = ttrpc::Response::new();
        response.set_status(ttrpc::get_status(ttrpc::Code::OK, ""));
        response.payload =
            serde_json::to_vec(&value).map_err(|_| rpc_error(ConfigError::EncodingFailed))?;
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
        if matches!(component, Component::CurDir | Component::ParentDir | Component::Prefix(_)) {
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
        directory = openat(&directory, *parent, directory_flags, Mode::empty())
            .map_err(map_open_error)?;
    }
    let file = openat(&directory, *leaf, file_flags, Mode::empty()).map_err(map_open_error)?;
    let metadata = fstat(&file).map_err(|_| ConfigError::Unavailable)?;
    if FileType::from_raw_mode(metadata.st_mode) != FileType::RegularFile
        || metadata.st_nlink != 1
    {
        return Err(ConfigError::InvalidRequest);
    }
    let size = usize::try_from(metadata.st_size).unwrap_or(crate::MAX_CONFIG_BYTES + 1);
    if size > crate::MAX_CONFIG_BYTES {
        return Err(ConfigError::DocumentTooLarge);
    }
    let mut bytes = Vec::with_capacity(size);
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        let count = read(&file, &mut chunk).map_err(|_| ConfigError::Unavailable)?;
        if count == 0 {
            break;
        }
        if bytes.len() + count > crate::MAX_CONFIG_BYTES {
            return Err(ConfigError::DocumentTooLarge);
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    Ok(bytes)
}
