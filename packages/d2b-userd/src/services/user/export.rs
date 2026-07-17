use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_identity::ProviderId,
    v2_provider::{Generation, SourceVersion},
};
use sha2::{Digest, Sha256};
use tokio::{io::AsyncWriteExt, process::Command};
use zeroize::Zeroizing;

use super::{
    identity::{
        AuthenticatedUser, OwnerBinding, SecretMetricSink, SecretOperation, UserSecretError,
        record_result,
    },
    secret_service::{
        EntropySource, OsEntropy, OwnedSecretMetadata, OwnedSecretSelector, SecretMaterial,
        SecretStore, SecretStoreError, SystemUserdClock, UserdClock,
    },
};

const MAX_SCOPED_EXPORTS: usize = 128;
const MAX_SEALED_CREDENTIAL_BYTES: usize = 256 * 1024;
const OPAQUE_ID_BYTES: usize = 16;
const CREDENTIAL_HASH_BYTES: usize = 12;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExportHandle(String);

impl ExportHandle {
    pub fn parse(value: impl Into<String>) -> Result<Self, UserSecretError> {
        let value = value.into();
        if valid_opaque(&value) {
            Ok(Self(value))
        } else {
            Err(UserSecretError::InvalidRequest)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ExportHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ExportHandle(<redacted>)")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ExportRequest {
    pub authenticated_user: AuthenticatedUser,
    pub credential_provider_id: ProviderId,
    pub target_service: String,
    pub allowed_purpose: String,
    pub host_binding_digest: [u8; 32],
    pub requested_expiry_unix_ms: u64,
    pub operation_id: String,
    pub idempotency_key: String,
}

impl fmt::Debug for ExportRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExportRequest")
            .field("authenticated_user", &"<redacted>")
            .field("credential_provider_id", &"<redacted>")
            .field("target_service", &"<redacted>")
            .field("allowed_purpose", &"<redacted>")
            .field("host_binding_digest", &"<redacted>")
            .field("requested_expiry_unix_ms", &self.requested_expiry_unix_ms)
            .field("operation_id", &"<redacted>")
            .field("idempotency_key", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportState {
    Pending,
    Active,
    Revoked,
    Expired,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ExportInspection {
    pub handle: ExportHandle,
    pub state: ExportState,
    pub source_version: SourceVersion,
    pub export_generation: Generation,
    pub expires_at_unix_ms: u64,
}

impl fmt::Debug for ExportInspection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExportInspection")
            .field("handle", &"<redacted>")
            .field("state", &self.state)
            .field("source_version", &"<redacted>")
            .field("export_generation", &self.export_generation)
            .field("expires_at_unix_ms", &self.expires_at_unix_ms)
            .finish()
    }
}

pub struct SealedExport {
    handle: ExportHandle,
    credential_name: String,
    sealed_credential: Zeroizing<Vec<u8>>,
    source_version: SourceVersion,
    export_generation: Generation,
    expires_at_unix_ms: u64,
}

impl SealedExport {
    pub fn handle(&self) -> &ExportHandle {
        &self.handle
    }

    pub fn credential_name(&self) -> &str {
        &self.credential_name
    }

    pub fn sealed_credential(&self) -> &[u8] {
        self.sealed_credential.as_slice()
    }

    pub fn source_version(&self) -> &SourceVersion {
        &self.source_version
    }

    pub const fn export_generation(&self) -> Generation {
        self.export_generation
    }

    pub const fn expires_at_unix_ms(&self) -> u64 {
        self.expires_at_unix_ms
    }
}

impl fmt::Debug for SealedExport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SealedExport")
            .field("handle", &"<redacted>")
            .field("credential_name", &"<redacted>")
            .field("sealed_credential", &"<redacted>")
            .field("source_version", &"<redacted>")
            .field("export_generation", &self.export_generation)
            .field("expires_at_unix_ms", &self.expires_at_unix_ms)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tpm2SealError {
    InvalidConfiguration,
    Unavailable,
    Denied,
    OutputInvalid,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Tpm2SealContext {
    credential_name: String,
}

impl Tpm2SealContext {
    pub fn credential_name(&self) -> &str {
        &self.credential_name
    }
}

impl fmt::Debug for Tpm2SealContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Tpm2SealContext")
            .field("credential_name", &"<redacted>")
            .finish()
    }
}

#[async_trait]
pub trait Tpm2Sealer: Send + Sync {
    async fn seal(
        &self,
        context: &Tpm2SealContext,
        secret: SecretMaterial,
    ) -> Result<Zeroizing<Vec<u8>>, Tpm2SealError>;
}

pub struct SystemdCredsTpm2Sealer {
    executable: PathBuf,
}

impl fmt::Debug for SystemdCredsTpm2Sealer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SystemdCredsTpm2Sealer")
            .field("executable", &"<redacted>")
            .finish()
    }
}

impl SystemdCredsTpm2Sealer {
    pub fn new(executable: impl Into<PathBuf>) -> Result<Self, Tpm2SealError> {
        let executable = executable.into();
        if !executable.is_absolute()
            || executable.file_name().and_then(|name| name.to_str()) != Some("systemd-creds")
        {
            return Err(Tpm2SealError::InvalidConfiguration);
        }
        Ok(Self { executable })
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }
}

#[async_trait]
impl Tpm2Sealer for SystemdCredsTpm2Sealer {
    async fn seal(
        &self,
        context: &Tpm2SealContext,
        secret: SecretMaterial,
    ) -> Result<Zeroizing<Vec<u8>>, Tpm2SealError> {
        if !valid_credential_name(context.credential_name()) {
            return Err(Tpm2SealError::InvalidConfiguration);
        }
        let mut command = Command::new(&self.executable);
        command
            .arg("encrypt")
            .arg("--with-key=tpm2")
            .arg("--tpm2-device=auto")
            .arg("--tpm2-pcrs=")
            .arg(format!("--name={}", context.credential_name()))
            .arg("-")
            .arg("-")
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|_| Tpm2SealError::Unavailable)?;
        let mut stdin = child.stdin.take().ok_or(Tpm2SealError::Unavailable)?;
        let plaintext = secret.into_bytes();
        stdin
            .write_all(plaintext.as_slice())
            .await
            .map_err(|_| Tpm2SealError::Unavailable)?;
        stdin
            .shutdown()
            .await
            .map_err(|_| Tpm2SealError::Unavailable)?;
        drop(stdin);
        drop(plaintext);

        let output = child
            .wait_with_output()
            .await
            .map_err(|_| Tpm2SealError::Unavailable)?;
        if !output.status.success() {
            return Err(Tpm2SealError::Denied);
        }
        if output.stdout.is_empty() || output.stdout.len() > MAX_SEALED_CREDENTIAL_BYTES {
            return Err(Tpm2SealError::OutputInvalid);
        }
        Ok(Zeroizing::new(output.stdout))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportCommitError {
    NotFound,
    Denied,
    Unavailable,
    CompletionUnknown,
    InvariantViolation,
}

#[async_trait]
pub trait ExportCommitPort: Send + Sync {
    async fn commit(&self, export: SealedExport) -> Result<(), ExportCommitError>;

    async fn inspect(&self, handle: &ExportHandle) -> Result<ExportState, ExportCommitError>;

    async fn revoke(&self, handle: &ExportHandle) -> Result<bool, ExportCommitError>;
}

#[derive(Default)]
pub struct InMemoryExportCommitPort {
    exports: Mutex<BTreeMap<ExportHandle, ExportState>>,
}

impl fmt::Debug for InMemoryExportCommitPort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InMemoryExportCommitPort")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ExportCommitPort for InMemoryExportCommitPort {
    async fn commit(&self, export: SealedExport) -> Result<(), ExportCommitError> {
        let mut exports = self
            .exports
            .lock()
            .map_err(|_| ExportCommitError::InvariantViolation)?;
        if exports.contains_key(export.handle()) {
            return Err(ExportCommitError::InvariantViolation);
        }
        exports.insert(export.handle().clone(), ExportState::Active);
        Ok(())
    }

    async fn inspect(&self, handle: &ExportHandle) -> Result<ExportState, ExportCommitError> {
        self.exports
            .lock()
            .map_err(|_| ExportCommitError::InvariantViolation)?
            .get(handle)
            .copied()
            .ok_or(ExportCommitError::NotFound)
    }

    async fn revoke(&self, handle: &ExportHandle) -> Result<bool, ExportCommitError> {
        let mut exports = self
            .exports
            .lock()
            .map_err(|_| ExportCommitError::InvariantViolation)?;
        let state = exports.get_mut(handle).ok_or(ExportCommitError::NotFound)?;
        if *state == ExportState::Revoked {
            return Ok(false);
        }
        *state = ExportState::Revoked;
        Ok(true)
    }
}

#[derive(Clone)]
struct ManagedExport {
    request_digest: [u8; 32],
    operation_id: String,
    idempotency_key: String,
    inspection: ExportInspection,
}

pub struct ScopedExportManager {
    owner: OwnerBinding,
    store: Arc<dyn SecretStore>,
    sealer: Arc<dyn Tpm2Sealer>,
    commit: Arc<dyn ExportCommitPort>,
    entropy: Arc<dyn EntropySource>,
    clock: Arc<dyn UserdClock>,
    metrics: Arc<dyn SecretMetricSink>,
    exports: Mutex<BTreeMap<ExportHandle, ManagedExport>>,
    mutation_gate: tokio::sync::Mutex<()>,
}

impl fmt::Debug for ScopedExportManager {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScopedExportManager")
            .field("owner", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl ScopedExportManager {
    pub fn new(
        owner: OwnerBinding,
        store: Arc<dyn SecretStore>,
        sealer: Arc<dyn Tpm2Sealer>,
        commit: Arc<dyn ExportCommitPort>,
        metrics: Arc<dyn SecretMetricSink>,
    ) -> Self {
        Self::new_with_runtime(
            owner,
            store,
            sealer,
            commit,
            Arc::new(OsEntropy),
            Arc::new(SystemUserdClock),
            metrics,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_runtime(
        owner: OwnerBinding,
        store: Arc<dyn SecretStore>,
        sealer: Arc<dyn Tpm2Sealer>,
        commit: Arc<dyn ExportCommitPort>,
        entropy: Arc<dyn EntropySource>,
        clock: Arc<dyn UserdClock>,
        metrics: Arc<dyn SecretMetricSink>,
    ) -> Self {
        Self {
            owner,
            store,
            sealer,
            commit,
            entropy,
            clock,
            metrics,
            exports: Mutex::new(BTreeMap::new()),
            mutation_gate: tokio::sync::Mutex::new(()),
        }
    }

    pub fn owner(&self) -> &OwnerBinding {
        &self.owner
    }

    fn now(&self) -> u64 {
        self.clock.now_unix_ms()
    }

    fn lock_exports(
        &self,
    ) -> Result<MutexGuard<'_, BTreeMap<ExportHandle, ManagedExport>>, UserSecretError> {
        self.exports
            .lock()
            .map_err(|_| UserSecretError::InvariantViolation)
    }

    fn validate_request(&self, request: &ExportRequest) -> Result<(), UserSecretError> {
        self.owner.authorize(&request.authenticated_user)?;
        if !valid_scope_name(&request.target_service)
            || !valid_scope_name(&request.allowed_purpose)
            || !valid_opaque(&request.operation_id)
            || !valid_opaque(&request.idempotency_key)
            || request.requested_expiry_unix_ms <= self.now()
            || request.host_binding_digest.iter().all(|byte| *byte == 0)
        {
            return Err(UserSecretError::InvalidRequest);
        }
        Ok(())
    }

    fn request_digest(request: &ExportRequest, metadata: &OwnedSecretMetadata) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(b"d2b-user-export-v2\0");
        digest.update(request.authenticated_user.peer_uid().to_be_bytes());
        digest.update(request.authenticated_user.realm_id().as_str().as_bytes());
        digest.update([0]);
        digest.update(request.credential_provider_id.as_str().as_bytes());
        digest.update([0]);
        digest.update(request.target_service.as_bytes());
        digest.update([0]);
        digest.update(request.allowed_purpose.as_bytes());
        digest.update([0]);
        digest.update(request.host_binding_digest);
        digest.update(request.requested_expiry_unix_ms.to_be_bytes());
        digest.update(metadata.source_version.as_str().as_bytes());
        digest.update(metadata.rotation_generation.get().to_be_bytes());
        digest.finalize().into()
    }

    fn credential_name(request_digest: &[u8; 32]) -> String {
        format!("d2b-{}", base32(&request_digest[..CREDENTIAL_HASH_BYTES]))
    }

    fn next_handle(
        &self,
        exports: &BTreeMap<ExportHandle, ManagedExport>,
    ) -> Result<ExportHandle, UserSecretError> {
        for _ in 0..4 {
            let mut entropy = [0_u8; OPAQUE_ID_BYTES];
            self.entropy
                .fill(&mut entropy)
                .map_err(|_| UserSecretError::Unavailable)?;
            let mut encoded = String::with_capacity(1 + OPAQUE_ID_BYTES * 2);
            encoded.push('e');
            append_hex(&mut encoded, &entropy);
            let handle = ExportHandle::parse(encoded)?;
            if !exports.contains_key(&handle) {
                return Ok(handle);
            }
        }
        Err(UserSecretError::ResourceExhausted)
    }

    fn map_store(error: SecretStoreError) -> UserSecretError {
        match error {
            SecretStoreError::Locked => UserSecretError::Locked,
            SecretStoreError::NotFound => UserSecretError::NotFound,
            SecretStoreError::Denied => UserSecretError::Unauthorized,
            SecretStoreError::Duplicate | SecretStoreError::InvalidData => {
                UserSecretError::InvariantViolation
            }
            SecretStoreError::Unavailable => UserSecretError::Unavailable,
        }
    }

    fn map_seal(error: Tpm2SealError) -> UserSecretError {
        match error {
            Tpm2SealError::Denied => UserSecretError::Unauthorized,
            Tpm2SealError::InvalidConfiguration | Tpm2SealError::OutputInvalid => {
                UserSecretError::InvariantViolation
            }
            Tpm2SealError::Unavailable => UserSecretError::Unavailable,
        }
    }

    fn map_commit(error: ExportCommitError) -> UserSecretError {
        match error {
            ExportCommitError::NotFound => UserSecretError::NotFound,
            ExportCommitError::Denied => UserSecretError::Unauthorized,
            ExportCommitError::Unavailable => UserSecretError::Unavailable,
            ExportCommitError::CompletionUnknown => UserSecretError::AmbiguousMutation,
            ExportCommitError::InvariantViolation => UserSecretError::InvariantViolation,
        }
    }

    pub async fn export(
        &self,
        request: &ExportRequest,
    ) -> Result<ExportInspection, UserSecretError> {
        let result = self.export_inner(request).await;
        record_result(&self.metrics, SecretOperation::ExportCredential, &result);
        result
    }

    async fn export_inner(
        &self,
        request: &ExportRequest,
    ) -> Result<ExportInspection, UserSecretError> {
        self.validate_request(request)?;
        let _mutation = self
            .mutation_gate
            .try_lock()
            .map_err(|_| UserSecretError::ResourceExhausted)?;
        let selector =
            OwnedSecretSelector::new(self.owner.clone(), request.credential_provider_id.clone());
        let (metadata, secret) = self
            .store
            .read_owned(&selector)
            .await
            .map_err(Self::map_store)?;
        let now = self.now();
        if metadata.expires_at_unix_ms <= now
            || request.requested_expiry_unix_ms > metadata.expires_at_unix_ms
        {
            return Err(UserSecretError::DeadlineExpired);
        }
        let request_digest = Self::request_digest(request, &metadata);

        {
            let exports = self.lock_exports()?;
            if let Some(existing) = exports.values().find(|existing| {
                existing.operation_id == request.operation_id
                    || existing.idempotency_key == request.idempotency_key
            }) {
                if existing.operation_id == request.operation_id
                    && existing.idempotency_key == request.idempotency_key
                    && existing.request_digest == request_digest
                {
                    return Ok(existing.inspection.clone());
                }
                return Err(UserSecretError::Conflict);
            }
            if exports.len() >= MAX_SCOPED_EXPORTS {
                return Err(UserSecretError::ResourceExhausted);
            }
        }

        let (handle, inspection) = {
            let exports = self.lock_exports()?;
            let handle = self.next_handle(&exports)?;
            let inspection = ExportInspection {
                handle: handle.clone(),
                state: ExportState::Pending,
                source_version: metadata.source_version.clone(),
                export_generation: metadata.rotation_generation,
                expires_at_unix_ms: request.requested_expiry_unix_ms,
            };
            (handle, inspection)
        };
        self.lock_exports()?.insert(
            handle.clone(),
            ManagedExport {
                request_digest,
                operation_id: request.operation_id.clone(),
                idempotency_key: request.idempotency_key.clone(),
                inspection: inspection.clone(),
            },
        );

        let credential_name = Self::credential_name(&request_digest);
        let sealed = match self
            .sealer
            .seal(
                &Tpm2SealContext {
                    credential_name: credential_name.clone(),
                },
                secret,
            )
            .await
        {
            Ok(sealed) => sealed,
            Err(error) => {
                self.lock_exports()?.remove(&handle);
                return Err(Self::map_seal(error));
            }
        };
        let export = SealedExport {
            handle: handle.clone(),
            credential_name,
            sealed_credential: sealed,
            source_version: metadata.source_version,
            export_generation: metadata.rotation_generation,
            expires_at_unix_ms: request.requested_expiry_unix_ms,
        };
        match self.commit.commit(export).await {
            Ok(()) => {
                let mut exports = self.lock_exports()?;
                let managed = exports
                    .get_mut(&handle)
                    .ok_or(UserSecretError::InvariantViolation)?;
                managed.inspection.state = ExportState::Active;
                Ok(managed.inspection.clone())
            }
            Err(ExportCommitError::CompletionUnknown) => Err(UserSecretError::AmbiguousMutation),
            Err(error) => {
                self.lock_exports()?.remove(&handle);
                Err(Self::map_commit(error))
            }
        }
    }

    pub async fn inspect(
        &self,
        authenticated_user: &AuthenticatedUser,
        handle: &ExportHandle,
    ) -> Result<ExportInspection, UserSecretError> {
        let result = self.inspect_inner(authenticated_user, handle).await;
        record_result(&self.metrics, SecretOperation::InspectExport, &result);
        result
    }

    async fn inspect_inner(
        &self,
        authenticated_user: &AuthenticatedUser,
        handle: &ExportHandle,
    ) -> Result<ExportInspection, UserSecretError> {
        self.owner.authorize(authenticated_user)?;
        let state = self
            .commit
            .inspect(handle)
            .await
            .map_err(Self::map_commit)?;
        let mut exports = self.lock_exports()?;
        let managed = exports.get_mut(handle).ok_or(UserSecretError::NotFound)?;
        managed.inspection.state = if state == ExportState::Active
            && self.now() >= managed.inspection.expires_at_unix_ms
        {
            ExportState::Expired
        } else {
            state
        };
        Ok(managed.inspection.clone())
    }

    pub async fn revoke(
        &self,
        authenticated_user: &AuthenticatedUser,
        handle: &ExportHandle,
    ) -> Result<bool, UserSecretError> {
        let result = self.revoke_inner(authenticated_user, handle).await;
        record_result(&self.metrics, SecretOperation::RevokeExport, &result);
        result
    }

    async fn revoke_inner(
        &self,
        authenticated_user: &AuthenticatedUser,
        handle: &ExportHandle,
    ) -> Result<bool, UserSecretError> {
        self.owner.authorize(authenticated_user)?;
        let _mutation = self
            .mutation_gate
            .try_lock()
            .map_err(|_| UserSecretError::ResourceExhausted)?;
        {
            let exports = self.lock_exports()?;
            let managed = exports.get(handle).ok_or(UserSecretError::NotFound)?;
            if managed.inspection.state == ExportState::Revoked {
                return Ok(false);
            }
        }
        let applied = self.commit.revoke(handle).await.map_err(Self::map_commit)?;
        let mut exports = self.lock_exports()?;
        let managed = exports
            .get_mut(handle)
            .ok_or(UserSecretError::InvariantViolation)?;
        managed.inspection.state = ExportState::Revoked;
        Ok(applied)
    }
}

fn valid_opaque(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_scope_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'.' | b'_' | b'@')
        })
}

fn valid_credential_name(value: &str) -> bool {
    value.starts_with("d2b-") && valid_opaque(value)
}

fn append_hex(destination: &mut String, value: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in value {
        destination.push(char::from(HEX[usize::from(byte >> 4)]));
        destination.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
}

fn base32(value: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut output = String::with_capacity((value.len() * 8).div_ceil(5));
    let mut accumulator = 0_u32;
    let mut bits = 0_u8;
    for byte in value {
        accumulator = (accumulator << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            output.push(char::from(
                ALPHABET[((accumulator >> bits) & 0x1f) as usize],
            ));
        }
    }
    if bits > 0 {
        output.push(char::from(
            ALPHABET[((accumulator << (5 - bits)) & 0x1f) as usize],
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::user::{
        NoopSecretMetrics,
        secret_service::tests::{FakeClock, FixedEntropy, MemoryStore, owner},
    };
    use d2b_contracts::{
        v2_identity::{ConfiguredProviderId, ProviderType},
        v2_provider::SourceVersion,
    };

    struct FakeSealer;

    #[async_trait]
    impl Tpm2Sealer for FakeSealer {
        async fn seal(
            &self,
            context: &Tpm2SealContext,
            secret: SecretMaterial,
        ) -> Result<Zeroizing<Vec<u8>>, Tpm2SealError> {
            assert!(context.credential_name().starts_with("d2b-"));
            assert_eq!(secret.len(), 6);
            Ok(Zeroizing::new(b"sealed".to_vec()))
        }
    }

    #[tokio::test]
    async fn export_is_exact_owner_scoped_and_returns_only_opaque_state() {
        let owner = owner();
        let provider = ProviderId::derive(
            owner.realm_id(),
            ProviderType::Credential,
            &ConfiguredProviderId::parse("login").expect("provider"),
        );
        let store = Arc::new(MemoryStore::default());
        store
            .put_owned(
                &OwnedSecretSelector::new(owner.clone(), provider.clone()),
                &OwnedSecretMetadata {
                    source_version: SourceVersion::parse("version-one").expect("version"),
                    rotation_generation: Generation::new(1).expect("generation"),
                    expires_at_unix_ms: 10_000,
                },
                SecretMaterial::new(b"secret".to_vec()).expect("secret"),
            )
            .await
            .expect("put");
        let manager = ScopedExportManager::new_with_runtime(
            owner.clone(),
            store,
            Arc::new(FakeSealer),
            Arc::new(InMemoryExportCommitPort::default()),
            Arc::new(FixedEntropy::default()),
            Arc::new(FakeClock(Mutex::new(100))),
            Arc::new(NoopSecretMetrics),
        );
        let request = ExportRequest {
            authenticated_user: AuthenticatedUser::command_client(
                owner.uid(),
                owner.realm_id().clone(),
                owner.agent_generation(),
            ),
            credential_provider_id: provider,
            target_service: "example.service".to_owned(),
            allowed_purpose: "authenticate".to_owned(),
            host_binding_digest: [7; 32],
            requested_expiry_unix_ms: 5_000,
            operation_id: "export-one".to_owned(),
            idempotency_key: "export-key-one".to_owned(),
        };
        let exported = manager.export(&request).await.expect("export");
        assert_eq!(exported.state, ExportState::Active);
        assert!(exported.handle.as_str().starts_with('e'));
        assert!(!format!("{exported:?}").contains("version-one"));

        let wrong_user = AuthenticatedUser::command_client(
            owner.uid() + 1,
            owner.realm_id().clone(),
            owner.agent_generation(),
        );
        assert_eq!(
            manager.inspect(&wrong_user, &exported.handle).await,
            Err(UserSecretError::Unauthorized)
        );
    }

    #[test]
    fn production_sealer_rejects_non_absolute_or_renamed_binary() {
        assert_eq!(
            SystemdCredsTpm2Sealer::new("systemd-creds").unwrap_err(),
            Tpm2SealError::InvalidConfiguration
        );
        assert_eq!(
            SystemdCredsTpm2Sealer::new("/usr/bin/sh").unwrap_err(),
            Tpm2SealError::InvalidConfiguration
        );
    }
}
