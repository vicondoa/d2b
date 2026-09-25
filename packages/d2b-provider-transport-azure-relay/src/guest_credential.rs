//! Gateway runtime credential loading.
//!
//! Credentials are runtime state inside the gateway guest, not Nix data. This
//! module refuses `/nix/store` paths, enforces `0600`, optionally enforces the
//! gateway principal uid, and redacts all key debug output. The Guest only
//! opens the sealed envelope the gateway bootstrap delivers; it never authors
//! one.

use std::collections::HashMap;
use std::fmt;
use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::{
    MAX_ACTIVE_RELAY_LEASES, MAX_RELAY_LEASE_TTL_MS, RelayCredentialBinding, RelayCredentialError,
    RelayCredentialLease, RelayCredentialMaterial, RelayCredentialPort, RelayCredentialRole,
    RelaySecret, ScopedCredentialClient,
};
use async_trait::async_trait;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

/// Required file mode for gateway credential envelopes.
pub const GATEWAY_CREDENTIAL_MODE: u32 = 0o600;
/// Required file mode for the in-guest sealing key.
pub const GATEWAY_SEAL_KEY_MODE: u32 = 0o600;
/// Current sealed credential envelope schema.
pub const GATEWAY_CREDENTIAL_SCHEMA_VERSION: u32 = 1;
/// ChaCha20-Poly1305 key length.
pub const GATEWAY_SEAL_KEY_LEN: usize = 32;
const GATEWAY_CREDENTIAL_NONCE_LEN: usize = 12;
const SEALING_AAD_PREFIX: &[u8] = b"d2b-gateway-credential-v1";

/// Runtime credential file policy.
#[derive(Debug, Clone, Default)]
pub struct CredentialFilePolicy {
    /// Optional required owner uid (the gateway principal).
    pub required_uid: Option<u32>,
}

/// The in-guest sealing key used to encrypt the gateway credential envelope.
#[derive(Clone, PartialEq, Eq)]
pub struct SealingKey([u8; GATEWAY_SEAL_KEY_LEN]);

impl SealingKey {
    /// Wrap raw key bytes.
    pub fn from_bytes(bytes: [u8; GATEWAY_SEAL_KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// Load an existing sealing key, enforcing runtime-file policy.
    pub fn load(
        path: impl AsRef<Path>,
        policy: &CredentialFilePolicy,
    ) -> Result<Self, CredentialError> {
        let bytes = read_policy_file(path.as_ref(), GATEWAY_SEAL_KEY_MODE, policy)?;
        let key: [u8; GATEWAY_SEAL_KEY_LEN] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| CredentialError::BadSealKey)?;
        Ok(Self(key))
    }
}

impl core::fmt::Debug for SealingKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SealingKey(<redacted>)")
    }
}

impl Drop for SealingKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Plaintext credential material accepted by the in-guest enrollment flow.
#[derive(Clone, PartialEq, Eq)]
pub struct GatewayCredentialMaterial {
    /// Relay Listen rule name.
    pub listen_key_name: String,
    /// Relay Listen rule key.
    pub listen_key: String,
    /// Relay Send rule name.
    pub send_key_name: String,
    /// Relay Send rule key.
    pub send_key: String,
}

impl core::fmt::Debug for GatewayCredentialMaterial {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GatewayCredentialMaterial")
            .field("listen_key_name", &self.listen_key_name)
            .field("listen_key", &"<redacted>")
            .field("send_key_name", &self.send_key_name)
            .field("send_key", &"<redacted>")
            .finish()
    }
}

impl Drop for GatewayCredentialMaterial {
    fn drop(&mut self) {
        self.listen_key_name.zeroize();
        self.listen_key.zeroize();
        self.send_key_name.zeroize();
        self.send_key.zeroize();
    }
}

/// Metadata attached to a sealed gateway credential envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialEnvelopeMeta {
    /// Gateway credential generation. Rotation must increase it.
    pub generation: u64,
    /// Optional Unix-seconds expiry for the envelope.
    pub not_after: Option<u64>,
}

/// A loaded gateway credential envelope. `Debug` redacts all secret material.
#[derive(Clone)]
pub struct GatewayCredential {
    listen_key_name: String,
    listen_key: String,
    send_key_name: String,
    send_key: String,
    generation: u64,
    not_after: Option<u64>,
}

impl core::fmt::Debug for GatewayCredential {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GatewayCredential")
            .field("listen_key_name", &self.listen_key_name)
            .field("listen_key", &"<redacted>")
            .field("send_key_name", &self.send_key_name)
            .field("send_key", &"<redacted>")
            .field("generation", &self.generation)
            .field("not_after", &self.not_after)
            .finish()
    }
}

impl Drop for GatewayCredential {
    fn drop(&mut self) {
        self.listen_key_name.zeroize();
        self.listen_key.zeroize();
        self.send_key_name.zeroize();
        self.send_key.zeroize();
    }
}

impl GatewayCredential {
    /// Load and validate the legacy plaintext credential envelope at `path`.
    ///
    /// New gateway-owned credential state should use
    /// [`GatewayCredential::load_sealed`]. This loader exists only for parsing
    /// transition fixtures and for explicitly-guarded development paths.
    pub fn load(
        path: impl AsRef<Path>,
        policy: &CredentialFilePolicy,
    ) -> Result<Self, CredentialError> {
        let path = path.as_ref();
        let raw = read_policy_file(path, GATEWAY_CREDENTIAL_MODE, policy)?;
        let raw = std::str::from_utf8(&raw).map_err(|_| CredentialError::Malformed)?;
        Self::from_material(
            Self::parse_material_json(raw)?,
            CredentialEnvelopeMeta {
                generation: 0,
                not_after: None,
            },
        )
    }

    /// Load and unseal the gateway-owned credential envelope.
    pub fn load_sealed(
        path: impl AsRef<Path>,
        sealing_key: &SealingKey,
        policy: &CredentialFilePolicy,
        now_unix: u64,
    ) -> Result<Self, CredentialError> {
        Self::load_sealed_inner(path.as_ref(), sealing_key, policy, Some(now_unix))
    }

    fn load_sealed_inner(
        path: &Path,
        sealing_key: &SealingKey,
        policy: &CredentialFilePolicy,
        now_unix: Option<u64>,
    ) -> Result<Self, CredentialError> {
        let raw = read_policy_file(path, GATEWAY_CREDENTIAL_MODE, policy)?;
        let envelope: SealedCredentialFile =
            serde_json::from_slice(&raw).map_err(|_| CredentialError::Malformed)?;
        if envelope.schema_version != GATEWAY_CREDENTIAL_SCHEMA_VERSION {
            return Err(CredentialError::BadSchemaVersion(envelope.schema_version));
        }
        if let (Some(now_unix), Some(not_after)) = (now_unix, envelope.not_after)
            && now_unix >= not_after
        {
            return Err(CredentialError::Expired);
        }
        let nonce = decode_fixed::<GATEWAY_CREDENTIAL_NONCE_LEN>(&envelope.nonce)?;
        let ciphertext = base64::engine::general_purpose::STANDARD
            .decode(envelope.ciphertext.as_bytes())
            .map_err(|_| CredentialError::Malformed)?;
        let aad = credential_aad(envelope.generation, envelope.not_after);
        let plaintext = cipher(sealing_key)
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| CredentialError::Crypto)?;
        let plaintext = Zeroizing::new(plaintext);
        let plaintext = std::str::from_utf8(&plaintext).map_err(|_| CredentialError::Malformed)?;
        Self::from_material(
            Self::parse_material_json(plaintext)?,
            CredentialEnvelopeMeta {
                generation: envelope.generation,
                not_after: envelope.not_after,
            },
        )
    }

    /// Credential generation.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Optional Unix-seconds expiry for the sealed envelope.
    pub fn not_after(&self) -> Option<u64> {
        self.not_after
    }

    fn parse_material_json(raw: &str) -> Result<GatewayCredentialMaterial, CredentialError> {
        let v: Value = serde_json::from_str(raw).map_err(|_| CredentialError::Malformed)?;
        let material = GatewayCredentialMaterial {
            listen_key_name: required_str(&v, &["relayListen", "keyName"])?,
            listen_key: required_str(&v, &["relayListen", "key"])?,
            send_key_name: required_str(&v, &["relaySend", "keyName"])?,
            send_key: required_str(&v, &["relaySend", "key"])?,
        };
        if [
            &material.listen_key_name,
            &material.listen_key,
            &material.send_key_name,
            &material.send_key,
        ]
        .iter()
        .any(|value| !valid_material_text(value))
        {
            return Err(CredentialError::Malformed);
        }
        Ok(material)
    }

    fn from_material(
        material: GatewayCredentialMaterial,
        meta: CredentialEnvelopeMeta,
    ) -> Result<Self, CredentialError> {
        Ok(Self {
            listen_key_name: material.listen_key_name.clone(),
            listen_key: material.listen_key.clone(),
            send_key_name: material.send_key_name.clone(),
            send_key: material.send_key.clone(),
            generation: meta.generation,
            not_after: meta.not_after,
        })
    }
}

#[derive(Clone)]
struct ActiveRelayLease {
    role: RelayCredentialRole,
    binding: RelayCredentialBinding,
}

/// Gateway Guest-local implementation of [`RelayCredentialPort`].
///
/// The port owns the sealed credential after it has been opened by the Guest.
/// Only a binding-aware acquisition can mint a lease, and the active lease
/// table keeps revocation exact without exposing token bytes to callers.
#[derive(Clone)]
pub struct GatewayGuestCredentialPort {
    credential: Arc<GatewayCredential>,
    active: Arc<Mutex<HashMap<u64, ActiveRelayLease>>>,
    now_unix_ms: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl GatewayGuestCredentialPort {
    /// Build a Guest-local port using the system Unix-millisecond clock.
    pub fn new(credential: Arc<GatewayCredential>) -> Self {
        Self::with_clock(credential, Arc::new(system_now_unix_ms))
    }

    /// Build a Guest-local port with an injected clock for deterministic tests.
    pub fn with_clock(
        credential: Arc<GatewayCredential>,
        now_unix_ms: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Self {
        Self {
            credential,
            active: Arc::new(Mutex::new(HashMap::new())),
            now_unix_ms,
        }
    }

    /// Load a sealed credential directly inside the Guest and own its port.
    pub fn from_sealed(
        path: impl AsRef<Path>,
        sealing_key: &SealingKey,
        policy: &CredentialFilePolicy,
        now_unix: u64,
    ) -> Result<Self, CredentialError> {
        Ok(Self::new(Arc::new(GatewayCredential::load_sealed(
            path,
            sealing_key,
            policy,
            now_unix,
        )?)))
    }

    /// Return the generation of the sealed credential envelope.
    pub fn credential_generation(&self) -> u64 {
        self.credential.generation()
    }

    /// Return a non-secret digest for Guest-local observation.
    ///
    /// The digest is emitted only as a test/diagnostic marker after the
    /// sealed envelope has been opened; credential bytes never leave this
    /// port.
    pub fn safe_observation_digest(&self) -> [u8; 32] {
        Sha256::digest(self.credential.send_key.as_bytes()).into()
    }

    /// Return the number of currently revocable leases.
    ///
    /// Diagnostic read reachable from `Debug` (a sync surface); a
    /// fail-closed `try_lock` keeps this from parking an executor worker while
    /// an async acquire/revoke holds the lock, reporting 0 when contended.
    pub fn active_lease_count(&self) -> usize {
        self.active.try_lock().map(|leases| leases.len()).unwrap_or(0)
    }

    fn material_for(
        &self,
        role: RelayCredentialRole,
    ) -> Result<RelayCredentialMaterial, RelayCredentialError> {
        let (key_name, key) = match role {
            RelayCredentialRole::Listen => (
                &self.credential.listen_key_name,
                &self.credential.listen_key,
            ),
            RelayCredentialRole::Send => {
                (&self.credential.send_key_name, &self.credential.send_key)
            }
        };
        Ok(RelayCredentialMaterial::SasRule {
            key_name: RelaySecret::new(key_name.as_bytes().to_vec())?,
            key: RelaySecret::new(key.as_bytes().to_vec())?,
        })
    }
}

impl fmt::Debug for GatewayGuestCredentialPort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GatewayGuestCredentialPort")
            .field("credential_generation", &self.credential.generation())
            .field("active_lease_count", &self.active_lease_count())
            .finish()
    }
}

#[async_trait]
impl RelayCredentialPort for GatewayGuestCredentialPort {
    async fn acquire(
        &self,
        _: RelayCredentialRole,
        _: u32,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        Err(RelayCredentialError::BindingRequired)
    }

    async fn acquire_bound(
        &self,
        role: RelayCredentialRole,
        binding: &RelayCredentialBinding,
        deadline_ms: u32,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        if deadline_ms == 0 {
            return Err(RelayCredentialError::Expired);
        }
        if self.credential.generation() == 0 {
            tracing::warn!(
                provider = "transport-azure-relay",
                "credential lease acquire rejected: no valid credential generation loaded"
            );
            return Err(RelayCredentialError::Unavailable);
        }
        let now = (self.now_unix_ms)();
        let requested_ttl = u64::from(deadline_ms).min(MAX_RELAY_LEASE_TTL_MS);
        let mut expires_at = now.saturating_add(requested_ttl).saturating_add(1_000);
        if let Some(not_after) = self
            .credential
            .not_after()
            .map(|seconds| seconds.saturating_mul(1_000))
        {
            expires_at = expires_at.min(not_after.saturating_sub(1));
        }
        if expires_at <= now {
            tracing::warn!(
                provider = "transport-azure-relay",
                "credential lease acquire rejected: credential already expired"
            );
            return Err(RelayCredentialError::Expired);
        }
        let mut lease = RelayCredentialLease::new_bound(
            self.material_for(role)?,
            role,
            expires_at,
            binding.clone(),
        )?;
        let lease_id = lease.lease_id();
        let mut active = self.active.lock().await;
        if active.len() >= MAX_ACTIVE_RELAY_LEASES {
            tracing::warn!(
                provider = "transport-azure-relay",
                active_leases = active.len(),
                "credential lease acquire rejected: active lease budget exhausted"
            );
            return Err(RelayCredentialError::Unavailable);
        }
        active.insert(
            lease_id,
            ActiveRelayLease {
                role,
                binding: binding.clone(),
            },
        );
        let active_for_drop = Arc::clone(&self.active);
        lease.set_drop_hook(Arc::new(move |lease_id| {
            // Drop hooks run on a sync surface: a blocking std lock would park an
            // executor worker, so best-effort `try_lock` fail-closed removes the row
            // when uncontended. Revocation and the `MAX_ACTIVE_RELAY_LEASES` budget still
            // bound the table when a contended drop misses.
            if let Ok(mut active) = active_for_drop.try_lock() {
                active.remove(&lease_id);
            }
        }));
        Ok(lease)
    }

    async fn revoke(&self, lease: RelayCredentialLease) -> Result<(), RelayCredentialError> {
        let binding = lease
            .binding()
            .ok_or(RelayCredentialError::BindingRequired)?;
        let mut active = self.active.lock().await;
        let result = match active.get(&lease.lease_id()) {
            Some(record) if record.role == lease.role() && record.binding == *binding => {
                active.remove(&lease.lease_id());
                Ok(())
            }
            Some(_) => {
                tracing::warn!(
                    provider = "transport-azure-relay",
                    "credential lease revoke rejected: binding or role mismatch"
                );
                Err(RelayCredentialError::BindingMismatch)
            }
            None => {
                tracing::warn!(
                    provider = "transport-azure-relay",
                    "credential lease revoke rejected: unknown lease"
                );
                Err(RelayCredentialError::UnknownLease)
            }
        };
        drop(active);
        result
    }
}

#[async_trait]
impl ScopedCredentialClient for GatewayGuestCredentialPort {
    async fn read_credential(
        &self,
        request: &crate::ScopedCredentialRequest,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        self.acquire_bound(request.role(), request.binding(), request.deadline_ms())
            .await
    }

    async fn revoke_credential(
        &self,
        lease: RelayCredentialLease,
    ) -> Result<(), RelayCredentialError> {
        RelayCredentialPort::revoke(self, lease).await
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SealedCredentialFile {
    schema_version: u32,
    generation: u64,
    #[serde(default)]
    not_after: Option<u64>,
    nonce: String,
    ciphertext: String,
}

/// Credential load/validation failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialError {
    /// Credential was configured under `/nix/store`.
    NixStorePath,
    /// File could not be read/stat'd.
    Unreadable,
    /// File mode was not `0600`.
    BadMode(u32),
    /// File owner did not match the configured gateway principal.
    BadOwner(u32),
    /// JSON shape was malformed or missing a required field.
    Malformed,
    /// Runtime credential path was not a regular file.
    BadFileType,
    /// Sealed envelope schema is unsupported.
    BadSchemaVersion(u32),
    /// Sealing key was malformed.
    BadSealKey,
    /// Sealed envelope could not be decrypted or random bytes could not be generated.
    Crypto,
    /// Sealed envelope expired.
    Expired,
}

impl core::fmt::Display for CredentialError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CredentialError::NixStorePath => {
                f.write_str("gateway credential must not live in /nix/store")
            }
            CredentialError::Unreadable => f.write_str("gateway credential cannot be read"),
            CredentialError::BadMode(mode) => {
                write!(f, "gateway credential mode must be 0600, got {mode:o}")
            }
            CredentialError::BadOwner(uid) => {
                write!(f, "gateway credential owner uid mismatch: {uid}")
            }
            CredentialError::Malformed => f.write_str("gateway credential JSON is malformed"),
            CredentialError::BadFileType => {
                f.write_str("gateway credential must be a regular file")
            }
            CredentialError::BadSchemaVersion(version) => {
                write!(
                    f,
                    "gateway credential schema version {version} is unsupported"
                )
            }
            CredentialError::BadSealKey => f.write_str("gateway sealing key is malformed"),
            CredentialError::Crypto => {
                f.write_str("gateway credential envelope cannot be unsealed")
            }
            CredentialError::Expired => f.write_str("gateway credential envelope expired"),
        }
    }
}

impl std::error::Error for CredentialError {}

// Synchronous path: the Gateway opens its single sealed credential envelope
// during bootstrap composition via the public sync loaders, which are consumed
// by `d2bd`'s composition flow before executor workers exist, so an async form
// is unreachable on that path; keep this leaf read sync.

#[allow(
    clippy::disallowed_methods,
    reason = "synchronous path"
)]
fn read_policy_file(
    path: &Path,
    required_mode: u32,
    policy: &CredentialFilePolicy,
) -> Result<Zeroizing<Vec<u8>>, CredentialError> {
    if path.starts_with("/nix/store") {
        return Err(CredentialError::NixStorePath);
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| CredentialError::Unreadable)?;
    let meta = file.metadata().map_err(|_| CredentialError::Unreadable)?;
    if !meta.file_type().is_file() {
        return Err(CredentialError::BadFileType);
    }
    if meta.mode() & 0o777 != required_mode {
        return Err(CredentialError::BadMode(meta.mode() & 0o777));
    }
    if let Some(uid) = policy.required_uid
        && meta.uid() != uid
    {
        return Err(CredentialError::BadOwner(meta.uid()));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.read_to_end(&mut bytes)
        .map_err(|_| CredentialError::Unreadable)?;
    Ok(bytes)
}

fn cipher(sealing_key: &SealingKey) -> ChaCha20Poly1305 {
    ChaCha20Poly1305::new(Key::from_slice(&sealing_key.0))
}

fn credential_aad(generation: u64, not_after: Option<u64>) -> Vec<u8> {
    let mut aad = Vec::with_capacity(SEALING_AAD_PREFIX.len() + 24);
    aad.extend_from_slice(SEALING_AAD_PREFIX);
    aad.extend_from_slice(&generation.to_be_bytes());
    match not_after {
        Some(not_after) => {
            aad.push(1);
            aad.extend_from_slice(&not_after.to_be_bytes());
        }
        None => {
            aad.push(0);
            aad.extend_from_slice(&0_u64.to_be_bytes());
        }
    }
    aad
}

fn decode_fixed<const N: usize>(encoded: &str) -> Result<[u8; N], CredentialError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.as_bytes())
        .map_err(|_| CredentialError::Malformed)?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| CredentialError::Malformed)
}

fn required_str(v: &Value, path: &[&str]) -> Result<String, CredentialError> {
    let mut cur = v;
    for key in path {
        cur = cur.get(*key).ok_or(CredentialError::Malformed)?;
    }
    cur.as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or(CredentialError::Malformed)
}

fn system_now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn valid_material_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 16 * 1024
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

/// Write one sealed credential envelope for tests.
///
/// The Guest never authors envelopes: it only opens the envelope the gateway
/// bootstrap delivers. Unit tests build that fixture through the same
/// envelope primitives the read path verifies.
#[allow(
    clippy::disallowed_methods,
    reason = "cfg(test) helper"
)]
#[cfg(test)]
pub(crate) fn seal_envelope_for_test(
    path: &Path,
    sealing_key: &SealingKey,
    material: GatewayCredentialMaterial,
    generation: u64,
    not_after: Option<u64>,
) -> Result<(), CredentialError> {
    let plaintext = Zeroizing::new(
        serde_json::to_vec(&serde_json::json!({
            "relayListen": {
                "keyName": material.listen_key_name,
                "key": material.listen_key,
            },
            "relaySend": {
                "keyName": material.send_key_name,
                "key": material.send_key,
            },
        }))
        .map_err(|_| CredentialError::Malformed)?,
    );
    // A nonce reused under one key breaks ChaCha20-Poly1305 outright, so it is
    // generated for every envelope rather than written into the buffer by hand.
    // The loader reads it back from the file, which is why the envelope carries
    // it.
    use chacha20poly1305::aead::{AeadCore, OsRng};
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let aad = credential_aad(generation, not_after);
    let ciphertext = cipher(sealing_key)
        .encrypt(
            &nonce,
            Payload {
                msg: &plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| CredentialError::Crypto)?;
    let envelope = SealedCredentialFile {
        schema_version: GATEWAY_CREDENTIAL_SCHEMA_VERSION,
        generation,
        not_after,
        nonce: base64::engine::general_purpose::STANDARD.encode(nonce.as_slice()),
        ciphertext: base64::engine::general_purpose::STANDARD.encode(ciphertext),
    };
    let body = serde_json::to_vec(&envelope).map_err(|_| CredentialError::Malformed)?;
    std::fs::write(path, &body).map_err(|_| CredentialError::Unreadable)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(GATEWAY_CREDENTIAL_MODE))
        .map_err(|_| CredentialError::Unreadable)
}

/// Key-length entropy for the tests. The sealing key material is generated at
/// run time, so none of it lives in the source tree.
#[allow(
    clippy::disallowed_methods,
    reason = "cfg(test) helper"
)]
#[cfg(test)]
pub(crate) fn sealing_key_bytes() -> [u8; GATEWAY_SEAL_KEY_LEN] {
    static BYTES: std::sync::LazyLock<[u8; GATEWAY_SEAL_KEY_LEN]> =
        std::sync::LazyLock::new(|| {
            let mut bytes = [0_u8; GATEWAY_SEAL_KEY_LEN];
            use std::io::Read;
            std::fs::File::open("/dev/urandom")
                .and_then(|mut file| file.read_exact(&mut bytes))
                .expect("entropy for a test sealing key");
            bytes
        });
    *BYTES
}

/// The sealing key this crate's tests use.
///
/// The bytes are drawn once per test binary and remembered, so repeated calls
/// inside one test agree - the sealing and loading halves have to hold the
/// same key.
#[cfg(test)]
pub(crate) fn sealing_key() -> SealingKey {
    SealingKey::from_bytes(sealing_key_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn fixture(dir: &Path) -> PathBuf {
        let path = dir.join("credential.json");
        fs::write(
            &path,
            r#"{
              "relayListen": { "keyName": "gateway-listen", "key": "listen-secret" },
              "relaySend": { "keyName": "gateway-send", "key": "send-secret" }
            }"#,
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        path
    }

    fn material() -> GatewayCredentialMaterial {
        GatewayCredentialMaterial {
            listen_key_name: "gateway-listen".to_owned(),
            listen_key: "listen-secret".to_owned(),
            send_key_name: "gateway-send".to_owned(),
            send_key: "send-secret".to_owned(),
        }
    }

    #[test]
    fn loads_only_runtime_0600_files_and_redacts_debug() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(dir.path());
        let cred = GatewayCredential::load(&path, &CredentialFilePolicy::default()).unwrap();
        let dbg = format!("{cred:?}");
        assert!(dbg.contains("gateway-listen"));
        assert!(dbg.contains("gateway-send"));
        assert!(!dbg.contains("listen-secret"));
        assert!(!dbg.contains("send-secret"));
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn rejects_group_or_other_readable_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(dir.path());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        assert_eq!(
            GatewayCredential::load(&path, &CredentialFilePolicy::default()).unwrap_err(),
            CredentialError::BadMode(0o640)
        );
    }

    #[test]
    fn rejects_nix_store_credential_path() {
        assert_eq!(
            GatewayCredential::load(
                "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-secret.json",
                &CredentialFilePolicy::default()
            )
            .unwrap_err(),
            CredentialError::NixStorePath
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn rejects_owner_mismatch_when_policy_requires_uid() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(dir.path());
        let uid = fs::metadata(&path).unwrap().uid().saturating_add(1);
        assert!(matches!(
            GatewayCredential::load(
                &path,
                &CredentialFilePolicy {
                    required_uid: Some(uid)
                }
            ),
            Err(CredentialError::BadOwner(_))
        ));
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn unseals_gateway_owned_credential_envelope() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credential.sealed.json");
        seal_envelope_for_test(&path, &sealing_key(), material(), 1, Some(2_000)).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("listen-secret"));
        assert!(!raw.contains("send-secret"));

        let cred = GatewayCredential::load_sealed(
            &path,
            &sealing_key(),
            &CredentialFilePolicy::default(),
            1_000,
        )
        .unwrap();
        assert_eq!(cred.generation(), 1);
        assert_eq!(cred.not_after(), Some(2_000));
        let dbg = format!("{cred:?}");
        assert!(!dbg.contains("listen-secret"));
        assert!(!dbg.contains("send-secret"));
    }

    #[test]
    fn sealed_gateway_credential_expiry_is_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credential.sealed.json");
        seal_envelope_for_test(&path, &sealing_key(), material(), 1, Some(10)).unwrap();
        assert_eq!(
            GatewayCredential::load_sealed(
                &path,
                &sealing_key(),
                &CredentialFilePolicy::default(),
                10,
            )
            .unwrap_err(),
            CredentialError::Expired
        );
    }

    #[test]
    fn credential_aad_distinguishes_absent_and_zero_expiry() {
        assert_ne!(credential_aad(1, None), credential_aad(1, Some(0)));
    }

    #[test]
    fn sealed_envelope_rejects_wrong_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credential.sealed.json");
        seal_envelope_for_test(&path, &sealing_key(), material(), 1, None).unwrap();
        let wrong = {
            let mut bytes = sealing_key_bytes();
            bytes[0] ^= 0xff;
            SealingKey::from_bytes(bytes)
        };
        assert_eq!(
            GatewayCredential::load_sealed(&path, &wrong, &CredentialFilePolicy::default(), 1,)
                .unwrap_err(),
            CredentialError::Crypto
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn guest_port_requires_exact_binding_and_revokes_exact_lease() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credential.sealed.json");
        seal_envelope_for_test(&path, &sealing_key(), material(), 1, None).unwrap();
        let credential = Arc::new(
            GatewayCredential::load_sealed(
                &path,
                &sealing_key(),
                &CredentialFilePolicy::default(),
                1,
            )
            .unwrap(),
        );
        let port = GatewayGuestCredentialPort::with_clock(credential, Arc::new(|| 1_000_000));
        assert!(matches!(
            port.acquire(RelayCredentialRole::Send, 1_000).await,
            Err(RelayCredentialError::BindingRequired)
        ));
        let binding = RelayCredentialBinding::new("link-canary", "session-canary", 4).unwrap();
        let lease = port
            .acquire_bound(RelayCredentialRole::Send, &binding, 1_000)
            .await
            .unwrap();
        assert_eq!(port.active_lease_count(), 1);
        assert_eq!(lease.binding(), Some(&binding));
        let port_debug = format!("{port:?}");
        assert!(!port_debug.contains("send-secret"));
        let debug = format!("{lease:?}");
        assert!(!debug.contains("send-secret"));
        assert!(!debug.contains("link-canary"));
        port.revoke(lease).await.unwrap();
        assert_eq!(port.active_lease_count(), 0);
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn guest_port_removes_active_row_when_lease_drops() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credential.sealed.json");
        seal_envelope_for_test(&path, &sealing_key(), material(), 1, None).unwrap();
        let credential = Arc::new(
            GatewayCredential::load_sealed(
                &path,
                &sealing_key(),
                &CredentialFilePolicy::default(),
                1,
            )
            .unwrap(),
        );
        let port = GatewayGuestCredentialPort::with_clock(credential, Arc::new(|| 1_000_000));
        let binding = RelayCredentialBinding::new("link-drop", "session-drop", 1).unwrap();
        let lease = port
            .acquire_bound(RelayCredentialRole::Listen, &binding, 1_000)
            .await
            .unwrap();
        assert_eq!(port.active_lease_count(), 1);
        drop(lease);
        assert_eq!(port.active_lease_count(), 0);
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn sealed_guest_port_does_not_materialize_canary_in_file_or_debug() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credential.sealed.json");
        seal_envelope_for_test(&path, &sealing_key(), material(), 1, None).unwrap();
        let port = GatewayGuestCredentialPort::from_sealed(
            &path,
            &sealing_key(),
            &CredentialFilePolicy::default(),
            1,
        )
        .unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("listen-secret"));
        assert!(!raw.contains("send-secret"));
        let debug = format!("{port:?}");
        assert!(!debug.contains("listen-secret"));
        assert!(!debug.contains("send-secret"));
        assert!(!debug.contains(path.to_string_lossy().as_ref()));
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn guest_port_rejects_legacy_plaintext_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let credential = Arc::new(
            GatewayCredential::load(fixture(dir.path()), &CredentialFilePolicy::default()).unwrap(),
        );
        let port = GatewayGuestCredentialPort::new(credential);
        let binding = RelayCredentialBinding::new("link", "session", 1).unwrap();
        assert!(matches!(
            port.acquire_bound(RelayCredentialRole::Send, &binding, 1_000)
                .await,
            Err(RelayCredentialError::Unavailable)
        ));
        assert_eq!(port.active_lease_count(), 0);
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn guest_port_rejects_expired_envelope_without_materializing_a_lease() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credential.sealed.json");
        seal_envelope_for_test(&path, &sealing_key(), material(), 1, Some(10)).unwrap();
        let credential = Arc::new(
            GatewayCredential::load_sealed(
                &path,
                &sealing_key(),
                &CredentialFilePolicy::default(),
                2,
            )
            .unwrap(),
        );
        let port = GatewayGuestCredentialPort::with_clock(credential, Arc::new(|| 10_000));
        let binding = RelayCredentialBinding::new("link", "session", 1).unwrap();
        assert!(matches!(
            port.acquire_bound(RelayCredentialRole::Listen, &binding, 1_000)
                .await,
            Err(RelayCredentialError::Expired)
        ));
        assert_eq!(port.active_lease_count(), 0);
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn sealed_envelope_rejects_unsupported_schema_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credential.sealed.json");
        fs::write(
            &path,
            r#"{"schemaVersion":2,"generation":1,"nonce":"AAAA","ciphertext":"AAAA"}"#,
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            GatewayCredential::load_sealed(
                &path,
                &sealing_key(),
                &CredentialFilePolicy::default(),
                1,
            )
            .unwrap_err(),
            CredentialError::BadSchemaVersion(2)
        );
    }

    #[test]
    fn credential_path_must_be_a_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            GatewayCredential::load(dir.path(), &CredentialFilePolicy::default()).unwrap_err(),
            CredentialError::BadFileType
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn sealing_key_load_rejects_wrong_length_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sealing.key");
        fs::write(&path, [0x7f; 16]).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            SealingKey::load(&path, &CredentialFilePolicy::default()).unwrap_err(),
            CredentialError::BadSealKey
        );
    }
}
