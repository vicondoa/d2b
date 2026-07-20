//! Authenticated guest control over ComponentSession v2.

use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{
        AttachmentPolicy, BootstrapPskBinding, EndpointPolicy, EndpointPurpose, EndpointRole,
        GuestBootstrapCredentialV1, GuestSessionCredentialBytes, GuestSessionCredentialV1,
        IdentityEvidenceRequirement, LimitProfile, Locality, NoiseProfile, OperationId,
        PurposeClass, ServicePackage, TransportBinding, TransportClass,
    },
    v2_services::{SERVICE_INVENTORY, service_schema_fingerprint},
};
use d2b_session::{
    BootstrapAdmission, BootstrapPsk, ComponentSessionDriver, HandshakeCredentials, OwnedTransport,
    Secret32, SessionEngine, SessionError, TransportDescriptor, TransportError, TransportPacket,
};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const SESSION_MATERIAL_FILE: &str = "d2b-guest-session-v2";
const SEALED_IDENTITY_MAX_BYTES: u64 = 64 * 1024;
const SESSION_MATERIAL_MAX_BYTES: u64 = 16 * 1024;
const PRIVATE_KEY_BYTES: usize = 32;
const BOOTSTRAP_EVIDENCE_MAGIC: &[u8; 8] = b"D2BBEV2\0";
const BOOTSTRAP_EVIDENCE_BYTES: usize = 56;
const FIELD_MASK: u64 = (1_u64 << 51) - 1;
type FieldElement = [u64; 5];

pub const DAEMON_SERVICE_SCHEMA_FINGERPRINT_HEX: &str =
    "4b2834c89162e5a2c17ea879052c066fd546cdc440d1473955a99e2d9521a54a";
pub const GUEST_SERVICE_SCHEMA_FINGERPRINT_HEX: &str =
    "e6d2fd47db903deff84b5b9cb58a0aed17e2f6ef43010182925890878a15dd3d";

fn x25519_public(mut scalar: [u8; 32]) -> [u8; 32] {
    scalar[0] &= 248;
    scalar[31] &= 127;
    scalar[31] |= 64;
    let mut base = [0_u8; 32];
    base[0] = 9;
    let x1 = field_decode(base);
    let mut x2 = [1, 0, 0, 0, 0];
    let mut z2 = [0; 5];
    let mut x3 = x1;
    let mut z3 = [1, 0, 0, 0, 0];
    let mut swap = 0_u64;
    for bit in (0..255).rev() {
        let current = u64::from((scalar[bit / 8] >> (bit & 7)) & 1);
        swap ^= current;
        field_cswap(&mut x2, &mut x3, swap);
        field_cswap(&mut z2, &mut z3, swap);
        swap = current;

        let a = field_add(x2, z2);
        let aa = field_mul(a, a);
        let b = field_sub(x2, z2);
        let bb = field_mul(b, b);
        let e = field_sub(aa, bb);
        let c = field_add(x3, z3);
        let d = field_sub(x3, z3);
        let da = field_mul(d, a);
        let cb = field_mul(c, b);
        x3 = field_mul(field_add(da, cb), field_add(da, cb));
        z3 = field_mul(x1, field_mul(field_sub(da, cb), field_sub(da, cb)));
        x2 = field_mul(aa, bb);
        z2 = field_mul(e, field_add(aa, field_mul_small(e, 121_665)));
    }
    field_cswap(&mut x2, &mut x3, swap);
    field_cswap(&mut z2, &mut z3, swap);
    field_encode(field_mul(x2, field_invert(z2)))
}

fn field_decode(bytes: [u8; 32]) -> FieldElement {
    fn load(bytes: &[u8]) -> u64 {
        let mut word = [0_u8; 8];
        word.copy_from_slice(bytes);
        u64::from_le_bytes(word)
    }
    [
        load(&bytes[0..8]) & FIELD_MASK,
        (load(&bytes[6..14]) >> 3) & FIELD_MASK,
        (load(&bytes[12..20]) >> 6) & FIELD_MASK,
        (load(&bytes[19..27]) >> 1) & FIELD_MASK,
        (load(&bytes[24..32]) >> 12) & FIELD_MASK,
    ]
}

fn field_add(left: FieldElement, right: FieldElement) -> FieldElement {
    field_reduce([
        left[0] + right[0],
        left[1] + right[1],
        left[2] + right[2],
        left[3] + right[3],
        left[4] + right[4],
    ])
}

fn field_sub(left: FieldElement, right: FieldElement) -> FieldElement {
    field_reduce([
        left[0] + 2 * (FIELD_MASK - 18) - right[0],
        left[1] + 2 * FIELD_MASK - right[1],
        left[2] + 2 * FIELD_MASK - right[2],
        left[3] + 2 * FIELD_MASK - right[3],
        left[4] + 2 * FIELD_MASK - right[4],
    ])
}

fn field_mul(left: FieldElement, right: FieldElement) -> FieldElement {
    let left = left.map(u128::from);
    let right = right.map(u128::from);
    let mut limbs = [
        left[0] * right[0]
            + 19 * (left[1] * right[4]
                + left[2] * right[3]
                + left[3] * right[2]
                + left[4] * right[1]),
        left[0] * right[1]
            + left[1] * right[0]
            + 19 * (left[2] * right[4] + left[3] * right[3] + left[4] * right[2]),
        left[0] * right[2]
            + left[1] * right[1]
            + left[2] * right[0]
            + 19 * (left[3] * right[4] + left[4] * right[3]),
        left[0] * right[3]
            + left[1] * right[2]
            + left[2] * right[1]
            + left[3] * right[0]
            + 19 * left[4] * right[4],
        left[0] * right[4]
            + left[1] * right[3]
            + left[2] * right[2]
            + left[3] * right[1]
            + left[4] * right[0],
    ];
    for index in 0..4 {
        let carry = limbs[index] >> 51;
        limbs[index] &= u128::from(FIELD_MASK);
        limbs[index + 1] += carry;
    }
    let carry = limbs[4] >> 51;
    limbs[4] &= u128::from(FIELD_MASK);
    limbs[0] += carry * 19;
    let carry = limbs[0] >> 51;
    limbs[0] &= u128::from(FIELD_MASK);
    limbs[1] += carry;
    field_reduce(limbs.map(|limb| limb as u64))
}

fn field_mul_small(value: FieldElement, multiplier: u64) -> FieldElement {
    field_mul(value, [multiplier, 0, 0, 0, 0])
}

fn field_reduce(mut value: FieldElement) -> FieldElement {
    for _ in 0..2 {
        for index in 0..4 {
            let carry = value[index] >> 51;
            value[index] &= FIELD_MASK;
            value[index + 1] += carry;
        }
        let carry = value[4] >> 51;
        value[4] &= FIELD_MASK;
        value[0] += carry * 19;
    }
    value
}

fn field_cswap(left: &mut FieldElement, right: &mut FieldElement, swap: u64) {
    let mask = 0_u64.wrapping_sub(swap);
    for index in 0..5 {
        let difference = mask & (left[index] ^ right[index]);
        left[index] ^= difference;
        right[index] ^= difference;
    }
}

fn field_invert(value: FieldElement) -> FieldElement {
    let mut exponent = [0xff_u8; 32];
    exponent[0] = 0xeb;
    exponent[31] = 0x7f;
    let mut result = [1, 0, 0, 0, 0];
    let mut power = value;
    for bit in 0..255 {
        if (exponent[bit / 8] >> (bit & 7)) & 1 == 1 {
            result = field_mul(result, power);
        }
        power = field_mul(power, power);
    }
    result
}

fn field_encode(value: FieldElement) -> [u8; 32] {
    let value = field_canonical(value);
    let mut output = [0_u8; 32];
    for (limb_index, limb) in value.into_iter().enumerate() {
        for bit in 0..51 {
            if (limb >> bit) & 1 == 1 {
                let output_bit = limb_index * 51 + bit;
                output[output_bit / 8] |= 1 << (output_bit & 7);
            }
        }
    }
    output
}

fn field_canonical(value: FieldElement) -> FieldElement {
    let value = field_reduce(value);
    let modulus = [
        FIELD_MASK - 18,
        FIELD_MASK,
        FIELD_MASK,
        FIELD_MASK,
        FIELD_MASK,
    ];
    let mut difference = [0_u64; 5];
    let mut borrow = 0_i128;
    for index in 0..5 {
        let current = i128::from(value[index]) - i128::from(modulus[index]) - borrow;
        if current < 0 {
            difference[index] = (current + (1_i128 << 51)) as u64;
            borrow = 1;
        } else {
            difference[index] = current as u64;
            borrow = 0;
        }
    }
    let choose_difference = 1_u64.wrapping_sub(borrow as u64);
    let mut canonical = value;
    field_cswap(&mut canonical, &mut difference, choose_difference);
    canonical
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestSessionError {
    InvalidConfiguration,
    IdentityUnavailable,
    IdentityUnsafe,
    IdentitySealFailed,
    BootstrapUnavailable,
    BootstrapReplayed,
    BootstrapExpired,
    Session,
    Service,
    Transport,
}

impl GuestSessionError {
    pub const fn public_message(self) -> &'static str {
        match self {
            Self::InvalidConfiguration => "guest-session configuration is invalid",
            Self::IdentityUnavailable => "guest static identity is unavailable",
            Self::IdentityUnsafe => "guest static identity storage is unsafe",
            Self::IdentitySealFailed => "guest static identity could not be sealed",
            Self::BootstrapUnavailable => "guest bootstrap material is unavailable",
            Self::BootstrapReplayed => "guest bootstrap material was already consumed",
            Self::BootstrapExpired => "guest bootstrap material is expired",
            Self::Session => "guest component session failed",
            Self::Service => "guest service request failed",
            Self::Transport => "guest component transport failed",
        }
    }
}

impl From<SessionError> for GuestSessionError {
    fn from(_: SessionError) -> Self {
        Self::Session
    }
}

pub struct GuestStaticIdentity {
    private_key: [u8; PRIVATE_KEY_BYTES],
}

impl GuestStaticIdentity {
    pub fn from_private_key(
        private_key: [u8; PRIVATE_KEY_BYTES],
    ) -> Result<Self, GuestSessionError> {
        if private_key == [0; PRIVATE_KEY_BYTES] {
            return Err(GuestSessionError::IdentityUnavailable);
        }
        Ok(Self { private_key })
    }

    pub fn generate() -> Result<Self, GuestSessionError> {
        let mut private_key = [0_u8; PRIVATE_KEY_BYTES];
        File::open("/dev/urandom")
            .and_then(|mut source| source.read_exact(&mut private_key))
            .map_err(|_| GuestSessionError::IdentityUnavailable)?;
        Self::from_private_key(private_key)
    }

    pub fn public_key(&self) -> Result<[u8; PRIVATE_KEY_BYTES], GuestSessionError> {
        Ok(x25519_public(self.private_key))
    }

    fn handshake_secret(&self) -> Result<Secret32, GuestSessionError> {
        Secret32::new(self.private_key).map_err(GuestSessionError::from)
    }
}

impl fmt::Debug for GuestStaticIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GuestStaticIdentity(<redacted>)")
    }
}

impl Drop for GuestStaticIdentity {
    fn drop(&mut self) {
        self.private_key.fill(0);
    }
}

pub trait SealedIdentityStore: Send + Sync {
    fn load(&self) -> Result<Option<GuestStaticIdentity>, GuestSessionError>;
    fn seal(&self, identity: &GuestStaticIdentity) -> Result<(), GuestSessionError>;

    fn generate(&self) -> Result<GuestStaticIdentity, GuestSessionError> {
        GuestStaticIdentity::generate()
    }
}

#[derive(Debug, Clone)]
pub struct TpmSealedIdentityStore {
    sealed_path: PathBuf,
    systemd_creds_path: PathBuf,
}

impl TpmSealedIdentityStore {
    pub fn new(
        sealed_path: impl Into<PathBuf>,
        systemd_creds_path: impl Into<PathBuf>,
    ) -> Result<Self, GuestSessionError> {
        let sealed_path = sealed_path.into();
        let systemd_creds_path = systemd_creds_path.into();
        if !sealed_path.is_absolute() || !systemd_creds_path.is_absolute() {
            return Err(GuestSessionError::InvalidConfiguration);
        }
        Ok(Self {
            sealed_path,
            systemd_creds_path,
        })
    }

    fn validate_existing(&self) -> Result<(), GuestSessionError> {
        let metadata = fs::symlink_metadata(&self.sealed_path)
            .map_err(|_| GuestSessionError::IdentityUnavailable)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.mode() & 0o077 != 0
            || metadata.uid() != 0
            || metadata.len() == 0
            || metadata.len() > SEALED_IDENTITY_MAX_BYTES
        {
            return Err(GuestSessionError::IdentityUnsafe);
        }
        Ok(())
    }
}

impl SealedIdentityStore for TpmSealedIdentityStore {
    fn load(&self) -> Result<Option<GuestStaticIdentity>, GuestSessionError> {
        if !self.sealed_path.exists() {
            return Ok(None);
        }
        self.validate_existing()?;
        let output = Command::new(&self.systemd_creds_path)
            .args(["decrypt", "--with-key=tpm2"])
            .arg(&self.sealed_path)
            .arg("-")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| GuestSessionError::IdentityUnavailable)?;
        if !output.status.success() || output.stdout.len() != PRIVATE_KEY_BYTES {
            return Err(GuestSessionError::IdentityUnavailable);
        }
        let private_key: [u8; PRIVATE_KEY_BYTES] = output
            .stdout
            .try_into()
            .map_err(|_| GuestSessionError::IdentityUnavailable)?;
        GuestStaticIdentity::from_private_key(private_key).map(Some)
    }

    fn seal(&self, identity: &GuestStaticIdentity) -> Result<(), GuestSessionError> {
        let parent = self
            .sealed_path
            .parent()
            .ok_or(GuestSessionError::IdentityUnsafe)?;
        let metadata =
            fs::symlink_metadata(parent).map_err(|_| GuestSessionError::IdentityUnsafe)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || metadata.uid() != 0
            || metadata.mode() & 0o022 != 0
        {
            return Err(GuestSessionError::IdentityUnsafe);
        }
        if self.sealed_path.exists() {
            return Err(GuestSessionError::IdentityUnsafe);
        }

        let staging = self.sealed_path.with_extension("sealed-new");
        let _ = fs::remove_file(&staging);
        let mut child = Command::new(&self.systemd_creds_path)
            .args(["encrypt", "--with-key=tpm2", "-"])
            .arg(&staging)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| GuestSessionError::IdentitySealFailed)?;
        let write_result = child
            .stdin
            .take()
            .ok_or(GuestSessionError::IdentitySealFailed)
            .and_then(|mut stdin| {
                stdin
                    .write_all(&identity.private_key)
                    .map_err(|_| GuestSessionError::IdentitySealFailed)
            });
        let status = child
            .wait()
            .map_err(|_| GuestSessionError::IdentitySealFailed)?;
        if write_result.is_err() || !status.success() {
            let _ = fs::remove_file(&staging);
            return Err(GuestSessionError::IdentitySealFailed);
        }
        fs::set_permissions(&staging, fs::Permissions::from_mode(0o600))
            .and_then(|()| OpenOptions::new().read(true).open(&staging)?.sync_all())
            .and_then(|()| fs::rename(&staging, &self.sealed_path))
            .map_err(|_| {
                let _ = fs::remove_file(&staging);
                GuestSessionError::IdentitySealFailed
            })
    }
}

pub struct GuestSessionMaterial {
    generation: u64,
    parent_static_public: [u8; PRIVATE_KEY_BYTES],
    channel_binding: [u8; PRIVATE_KEY_BYTES],
    guest_identity_digest: Option<[u8; 32]>,
    guest_static_public_key: Option<[u8; 32]>,
    credential: Mutex<Option<GuestSessionCredentialV1>>,
    bootstrap_claimed: AtomicBool,
}

pub struct GuestRuntimeCredential {
    bytes: Vec<u8>,
}

impl GuestRuntimeCredential {
    pub(crate) fn expose(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for GuestRuntimeCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GuestRuntimeCredential(<redacted>)")
    }
}

impl Drop for GuestRuntimeCredential {
    fn drop(&mut self) {
        self.bytes.fill(0);
    }
}

pub trait GuestRuntimeCredentialSource {
    fn load(&self) -> Result<GuestRuntimeCredential, GuestSessionError>;
}

pub struct SystemdGuestCredentialSource {
    directory: PathBuf,
}

impl fmt::Debug for SystemdGuestCredentialSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SystemdGuestCredentialSource(<redacted>)")
    }
}

impl SystemdGuestCredentialSource {
    pub fn from_environment() -> Result<Self, GuestSessionError> {
        let directory = std::env::var_os("CREDENTIALS_DIRECTORY")
            .map(PathBuf::from)
            .ok_or(GuestSessionError::BootstrapUnavailable)?;
        if !directory.is_absolute() {
            return Err(GuestSessionError::InvalidConfiguration);
        }
        Ok(Self { directory })
    }

    pub(crate) fn load_named(
        &self,
        name: &str,
        max_bytes: u64,
    ) -> Result<GuestRuntimeCredential, GuestSessionError> {
        let valid_name = !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if !valid_name || max_bytes == 0 {
            return Err(GuestSessionError::InvalidConfiguration);
        }
        Ok(GuestRuntimeCredential {
            bytes: GuestSessionMaterial::read_runtime_credential(&self.directory, name, max_bytes)?,
        })
    }
}

impl GuestRuntimeCredentialSource for SystemdGuestCredentialSource {
    fn load(&self) -> Result<GuestRuntimeCredential, GuestSessionError> {
        let bytes = GuestSessionMaterial::read_runtime_credential(
            &self.directory,
            SESSION_MATERIAL_FILE,
            SESSION_MATERIAL_MAX_BYTES,
        )?;
        Ok(GuestRuntimeCredential { bytes })
    }
}

impl fmt::Debug for GuestSessionMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestSessionMaterial")
            .field("generation", &"<redacted>")
            .field("key_material", &"<redacted>")
            .field(
                "bootstrap_claimed",
                &self.bootstrap_claimed.load(Ordering::Acquire),
            )
            .finish()
    }
}

impl GuestSessionMaterial {
    pub fn from_credential_bytes(
        encoded: &GuestSessionCredentialBytes,
    ) -> Result<Self, GuestSessionError> {
        Self::decode(encoded.as_slice())
    }

    pub fn decode(encoded: &[u8]) -> Result<Self, GuestSessionError> {
        let credential = GuestSessionCredentialV1::decode(encoded)
            .map_err(|_| GuestSessionError::InvalidConfiguration)?;
        let guest_identity_digest = credential.guest_identity_digest().copied();
        let guest_static_public_key = credential.guest_static_public_key().copied();
        if let (Some(identity_digest), Some(public_key)) =
            (guest_identity_digest, guest_static_public_key)
            && identity_digest != Sha256::digest(public_key).as_slice()
        {
            return Err(GuestSessionError::InvalidConfiguration);
        }
        Ok(Self {
            generation: credential.session_generation(),
            parent_static_public: *credential.parent_static_public_key(),
            channel_binding: *credential.channel_binding(),
            guest_identity_digest,
            guest_static_public_key,
            credential: Mutex::new(Some(credential)),
            bootstrap_claimed: AtomicBool::new(false),
        })
    }

    pub fn from_runtime_credentials(
        source: &dyn GuestRuntimeCredentialSource,
    ) -> Result<Self, GuestSessionError> {
        let credential = source.load()?;
        Self::decode(credential.expose())
    }

    pub fn from_credentials_directory(directory: &Path) -> Result<Self, GuestSessionError> {
        let credential = GuestRuntimeCredential {
            bytes: Self::read_runtime_credential(
                directory,
                SESSION_MATERIAL_FILE,
                SESSION_MATERIAL_MAX_BYTES,
            )?,
        };
        Self::decode(credential.expose())
    }

    fn read_runtime_credential(
        directory: &Path,
        name: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>, GuestSessionError> {
        if !directory.is_absolute() {
            return Err(GuestSessionError::InvalidConfiguration);
        }
        let metadata =
            fs::symlink_metadata(directory).map_err(|_| GuestSessionError::BootstrapUnavailable)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || metadata.uid() != 0
            || metadata.mode() & 0o022 != 0
        {
            return Err(GuestSessionError::IdentityUnsafe);
        }
        let path = directory.join(name);
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| GuestSessionError::BootstrapUnavailable)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.uid() != 0
            || metadata.mode() & 0o077 != 0
            || metadata.len() == 0
            || metadata.len() > max_bytes
        {
            return Err(GuestSessionError::IdentityUnsafe);
        }
        let mut encoded = Vec::with_capacity(metadata.len() as usize);
        File::open(path)
            .and_then(|file| file.take(max_bytes + 1).read_to_end(&mut encoded))
            .map_err(|_| GuestSessionError::BootstrapUnavailable)?;
        Ok(encoded)
    }

    fn claim_bootstrap(
        &self,
        evidence: &BootstrapPeerEvidence,
        now_unix_ms: u64,
    ) -> Result<(BootstrapPskBinding, d2b_session::AdmittedBootstrapPsk), GuestSessionError> {
        if self.bootstrap_claimed.swap(true, Ordering::AcqRel) {
            return Err(GuestSessionError::BootstrapReplayed);
        }
        let credential = self
            .credential
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .ok_or(GuestSessionError::BootstrapUnavailable)?;
        let secrets: &GuestBootstrapCredentialV1 = credential
            .bootstrap()
            .ok_or(GuestSessionError::BootstrapUnavailable)?;
        secrets
            .admit(now_unix_ms)
            .map_err(|_| GuestSessionError::BootstrapExpired)?;
        let binding = secrets.binding().clone();
        let psk_bytes = *secrets.expose_psk();
        let mut admission = BootstrapAdmission::new(
            binding.clone(),
            BootstrapPsk::new(psk_bytes).map_err(GuestSessionError::from)?,
        )
        .map_err(GuestSessionError::from)?;
        let psk = admission
            .consume(&evidence.operation_id, &evidence.replay_nonce, now_unix_ms)
            .map_err(GuestSessionError::from)?;
        Ok((binding, psk))
    }
}

pub struct BootstrapPeerEvidence {
    operation_id: OperationId,
    replay_nonce: [u8; 32],
}

impl BootstrapPeerEvidence {
    pub fn new(operation_id: Vec<u8>, replay_nonce: [u8; 32]) -> Result<Self, GuestSessionError> {
        let operation_id =
            OperationId::new(operation_id).map_err(|_| GuestSessionError::InvalidConfiguration)?;
        if replay_nonce == [0; 32] {
            return Err(GuestSessionError::InvalidConfiguration);
        }
        Ok(Self {
            operation_id,
            replay_nonce,
        })
    }

    pub fn framed_bytes(&self) -> Vec<u8> {
        let mut payload = Vec::with_capacity(BOOTSTRAP_EVIDENCE_BYTES);
        payload.extend_from_slice(BOOTSTRAP_EVIDENCE_MAGIC);
        payload.extend_from_slice(self.operation_id.as_bytes());
        payload.extend_from_slice(&self.replay_nonce);
        let mut framed = Vec::with_capacity(BOOTSTRAP_EVIDENCE_BYTES + 2);
        framed.extend_from_slice(&(BOOTSTRAP_EVIDENCE_BYTES as u16).to_be_bytes());
        framed.extend_from_slice(&payload);
        framed
    }
}

impl fmt::Debug for BootstrapPeerEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BootstrapPeerEvidence(<redacted>)")
    }
}

pub struct GuestSessionAuthority {
    material: Arc<GuestSessionMaterial>,
    identity_store: Arc<dyn SealedIdentityStore>,
}

impl GuestSessionAuthority {
    pub fn new(
        material: GuestSessionMaterial,
        identity_store: Arc<dyn SealedIdentityStore>,
    ) -> Self {
        Self {
            material: Arc::new(material),
            identity_store,
        }
    }

    pub async fn establish_responder<T: OwnedTransport + 'static>(
        &self,
        transport: T,
        now: Instant,
        _: u64,
    ) -> Result<EstablishedGuestSession, GuestSessionError> {
        match self.identity_store.load()? {
            Some(identity) => {
                let guest_public = identity.public_key()?;
                let expected_public = self
                    .material
                    .guest_static_public_key
                    .ok_or(GuestSessionError::IdentityUnsafe)?;
                let expected_identity_digest = self
                    .material
                    .guest_identity_digest
                    .ok_or(GuestSessionError::IdentityUnsafe)?;
                if guest_public != expected_public {
                    return Err(GuestSessionError::IdentityUnsafe);
                }
                let policy = guest_policy(
                    GuestSessionPhase::Enrolled,
                    self.material.generation,
                    self.material.channel_binding,
                );
                let engine = SessionEngine::establish_responder(
                    transport,
                    policy,
                    HandshakeCredentials::Kk {
                        local_private: identity.handshake_secret()?,
                        remote_public: self.material.parent_static_public,
                    },
                    now,
                )
                .await?;
                Ok(EstablishedGuestSession {
                    driver: Arc::new(engine.into_driver()),
                    bootstrap_commit: None,
                    identity: SessionIdentityEvidence::new(
                        GuestSessionPhase::Enrolled,
                        self.material.parent_static_public,
                        guest_public,
                        expected_identity_digest,
                    ),
                    owner_key: random_secret32()?,
                })
            }
            None => Err(GuestSessionError::IdentityUnavailable),
        }
    }

    pub async fn establish_bootstrap_initiator<T: OwnedTransport + 'static>(
        &self,
        transport: T,
        evidence: BootstrapPeerEvidence,
        now: Instant,
        now_unix_ms: u64,
    ) -> Result<EstablishedGuestSession, GuestSessionError> {
        if self.identity_store.load()?.is_some() {
            return Err(GuestSessionError::IdentityUnsafe);
        }
        if self.material.guest_identity_digest.is_some()
            || self.material.guest_static_public_key.is_some()
        {
            return Err(GuestSessionError::IdentityUnsafe);
        }
        let (_, psk) = self.material.claim_bootstrap(&evidence, now_unix_ms)?;
        let identity = self.identity_store.generate()?;
        let guest_public = identity.public_key()?;
        let guest_identity_digest: [u8; 32] = Sha256::digest(guest_public).into();
        let policy = guest_policy(
            GuestSessionPhase::Bootstrap,
            self.material.generation,
            self.material.channel_binding,
        );
        let engine = SessionEngine::establish_initiator(
            transport,
            policy,
            HandshakeCredentials::IkPsk2Initiator {
                local_private: identity.handshake_secret()?,
                remote_public: self.material.parent_static_public,
                psk,
            },
            now,
        )
        .await?;
        Ok(EstablishedGuestSession {
            driver: Arc::new(engine.into_driver()),
            bootstrap_commit: Some(Arc::new(BootstrapCommit {
                identity: Mutex::new(Some(identity)),
                store: Arc::clone(&self.identity_store),
                committed: AtomicBool::new(false),
            })),
            identity: SessionIdentityEvidence::new(
                GuestSessionPhase::Bootstrap,
                self.material.parent_static_public,
                guest_public,
                guest_identity_digest,
            ),
            owner_key: random_secret32()?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuestSessionPhase {
    Bootstrap,
    Enrolled,
}

fn guest_service_spec() -> &'static d2b_contracts::v2_services::ServiceSpec {
    SERVICE_INVENTORY
        .iter()
        .find(|service| service.package == "d2b.guest.v2")
        .expect("frozen guest service inventory")
}

fn assert_frozen_service_fingerprints() {
    for (package, expected) in [
        ("d2b.daemon.v2", DAEMON_SERVICE_SCHEMA_FINGERPRINT_HEX),
        ("d2b.guest.v2", GUEST_SERVICE_SCHEMA_FINGERPRINT_HEX),
    ] {
        let service = SERVICE_INVENTORY
            .iter()
            .find(|service| service.package == package)
            .expect("frozen service inventory entry");
        assert_eq!(hex(&service_schema_fingerprint(service)), expected);
    }
}

fn guest_policy(
    phase: GuestSessionPhase,
    generation: u64,
    channel_binding: [u8; 32],
) -> EndpointPolicy {
    assert_frozen_service_fingerprints();
    let (purpose, purpose_class, noise_profile, identity_evidence) = match phase {
        GuestSessionPhase::Bootstrap => (
            EndpointPurpose::GuestBootstrap,
            PurposeClass::Bootstrap,
            NoiseProfile::Ikpsk2_25519ChaChaPolySha256,
            IdentityEvidenceRequirement::ParentStaticAndSingleUsePsk,
        ),
        GuestSessionPhase::Enrolled => (
            EndpointPurpose::GuestControl,
            PurposeClass::Enrolled,
            NoiseProfile::Kk25519ChaChaPolySha256,
            IdentityEvidenceRequirement::EnrolledStaticKeys,
        ),
    };
    EndpointPolicy {
        purpose,
        purpose_class,
        initiator_role: match phase {
            GuestSessionPhase::Bootstrap => EndpointRole::GuestAgent,
            GuestSessionPhase::Enrolled => EndpointRole::RealmController,
        },
        responder_role: match phase {
            GuestSessionPhase::Bootstrap => EndpointRole::RealmController,
            GuestSessionPhase::Enrolled => EndpointRole::GuestAgent,
        },
        service: ServicePackage::GuestV2,
        schema_fingerprint: service_schema_fingerprint(guest_service_spec()),
        noise_profile,
        limits: LimitProfile::local_default(),
        transport_binding: TransportBinding {
            transport: TransportClass::NativeVsock,
            locality: Locality::GuestLocal,
            channel_binding,
            identity_evidence,
        },
        reconnect_generation: generation,
        attachment_policy: AttachmentPolicy::disabled(),
    }
}

pub fn guest_bootstrap_policy(generation: u64, channel_binding: [u8; 32]) -> EndpointPolicy {
    guest_policy(GuestSessionPhase::Bootstrap, generation, channel_binding)
}

pub fn guest_enrolled_policy(generation: u64, channel_binding: [u8; 32]) -> EndpointPolicy {
    guest_policy(GuestSessionPhase::Enrolled, generation, channel_binding)
}

pub struct EstablishedGuestSession {
    pub driver: Arc<dyn ComponentSessionDriver>,
    pub(crate) bootstrap_commit: Option<Arc<BootstrapCommit>>,
    pub(crate) identity: SessionIdentityEvidence,
    pub(crate) owner_key: [u8; 32],
}

impl fmt::Debug for EstablishedGuestSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EstablishedGuestSession")
            .field("generation", &"<redacted>")
            .field("bootstrap", &self.bootstrap_commit.is_some())
            .field("identity", &"<redacted>")
            .finish()
    }
}

#[derive(Clone)]
pub(crate) struct SessionIdentityEvidence {
    pub(crate) phase: GuestSessionPhase,
    pub(crate) parent_static_public_key_digest: [u8; 32],
    pub(crate) guest_static_public_key_digest: [u8; 32],
    pub(crate) guest_identity_handle: String,
    pub(crate) guest_identity_digest: [u8; 32],
    pub(crate) guest_static_public_key: [u8; 32],
}

impl SessionIdentityEvidence {
    fn new(
        phase: GuestSessionPhase,
        parent_static_public: [u8; 32],
        guest_static_public: [u8; 32],
        guest_identity_digest: [u8; 32],
    ) -> Self {
        let guest_digest: [u8; 32] = Sha256::digest(guest_static_public).into();
        Self {
            phase,
            parent_static_public_key_digest: Sha256::digest(parent_static_public).into(),
            guest_static_public_key_digest: guest_digest,
            guest_identity_handle: format!("guest-{}", hex(&guest_identity_digest[..28])),
            guest_identity_digest,
            guest_static_public_key: guest_static_public,
        }
    }
}

impl fmt::Debug for SessionIdentityEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionIdentityEvidence(<redacted>)")
    }
}

fn random_secret32() -> Result<[u8; 32], GuestSessionError> {
    let mut bytes = [0_u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .map_err(|_| GuestSessionError::IdentityUnavailable)?;
    if bytes == [0; 32] {
        return Err(GuestSessionError::IdentityUnavailable);
    }
    Ok(bytes)
}

pub(crate) struct BootstrapCommit {
    identity: Mutex<Option<GuestStaticIdentity>>,
    store: Arc<dyn SealedIdentityStore>,
    committed: AtomicBool,
}

impl BootstrapCommit {
    pub(crate) fn preview_public_key(&self) -> Result<[u8; 32], GuestSessionError> {
        if self.committed.load(Ordering::Acquire) {
            return Err(GuestSessionError::BootstrapReplayed);
        }
        self.identity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .ok_or(GuestSessionError::BootstrapReplayed)?
            .public_key()
    }

    pub(crate) fn commit(&self) -> Result<[u8; 32], GuestSessionError> {
        if self.committed.swap(true, Ordering::AcqRel) {
            return Err(GuestSessionError::BootstrapReplayed);
        }
        let identity = self
            .identity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .ok_or(GuestSessionError::BootstrapReplayed)?;
        let public = identity.public_key()?;
        self.store.seal(&identity)?;
        Ok(public)
    }
}

pub struct FramedGuestTransport<S> {
    stream: S,
}

impl<S> FramedGuestTransport<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }
}

impl<S> FramedGuestTransport<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    pub async fn receive_bootstrap_evidence(
        &mut self,
    ) -> Result<BootstrapPeerEvidence, GuestSessionError> {
        let mut length = [0_u8; 2];
        self.stream
            .read_exact(&mut length)
            .await
            .map_err(|_| GuestSessionError::Transport)?;
        if usize::from(u16::from_be_bytes(length)) != BOOTSTRAP_EVIDENCE_BYTES {
            return Err(GuestSessionError::InvalidConfiguration);
        }
        let mut payload = [0_u8; BOOTSTRAP_EVIDENCE_BYTES];
        self.stream
            .read_exact(&mut payload)
            .await
            .map_err(|_| GuestSessionError::Transport)?;
        if &payload[..8] != BOOTSTRAP_EVIDENCE_MAGIC {
            return Err(GuestSessionError::InvalidConfiguration);
        }
        BootstrapPeerEvidence::new(
            payload[8..24].to_vec(),
            payload[24..56]
                .try_into()
                .map_err(|_| GuestSessionError::InvalidConfiguration)?,
        )
    }
}

#[async_trait]
impl<S> OwnedTransport for FramedGuestTransport<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    fn descriptor(&self) -> TransportDescriptor {
        TransportDescriptor {
            class: TransportClass::NativeVsock,
            locality: Locality::GuestLocal,
            packet_atomic: false,
            supports_attachments: false,
        }
    }

    async fn receive(&mut self, protected_limit: usize) -> Result<TransportPacket, TransportError> {
        let mut length = [0_u8; 2];
        self.stream
            .read_exact(&mut length)
            .await
            .map_err(map_transport_io)?;
        let length = usize::from(u16::from_be_bytes(length));
        if length == 0 || length > protected_limit {
            return Err(TransportError::LimitExceeded);
        }
        let mut bytes = vec![0_u8; length];
        self.stream
            .read_exact(&mut bytes)
            .await
            .map_err(map_transport_io)?;
        Ok(TransportPacket::new(bytes))
    }

    async fn send(&mut self, packet: TransportPacket) -> Result<(), TransportError> {
        let (bytes, attachments) = packet.into_parts();
        if !attachments.is_empty() || bytes.is_empty() || bytes.len() > usize::from(u16::MAX) {
            return Err(TransportError::LimitExceeded);
        }
        self.stream
            .write_all(&(bytes.len() as u16).to_be_bytes())
            .await
            .map_err(map_transport_io)?;
        self.stream
            .write_all(&bytes)
            .await
            .map_err(map_transport_io)?;
        self.stream.flush().await.map_err(map_transport_io)
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        self.stream.shutdown().await.map_err(map_transport_io)
    }
}

fn map_transport_io(error: std::io::Error) -> TransportError {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::UnexpectedEof
        | ErrorKind::BrokenPipe
        | ErrorKind::ConnectionAborted
        | ErrorKind::ConnectionReset
        | ErrorKind::NotConnected => TransportError::Disconnected,
        ErrorKind::WouldBlock => TransportError::WouldBlock,
        _ => TransportError::Other,
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}
