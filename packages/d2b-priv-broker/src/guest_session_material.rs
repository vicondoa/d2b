//! Closed guest-session and configured-launch credential materialization.

use std::{
    collections::BTreeMap,
    fmt,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    os::fd::{AsRawFd, OwnedFd},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{
        AttachmentAccess, AttachmentKind, AttachmentPurpose, BootstrapPskBinding,
        GuestBootstrapCredentialV1, GuestBootstrapPsk, GuestIdentityBindingV1,
        GuestSessionCredentialBytes, GuestSessionCredentialV1, KernelObjectType, ServicePackage,
    },
    v2_guest_configured_launches::{
        GuestConfiguredLaunchEntryV1, GuestConfiguredLaunchesBytes, GuestConfiguredLaunchesV1,
    },
    v2_identity::{
        RealmId as V2RealmId, RealmPath as V2RealmPath, WorkloadId as V2WorkloadId, WorkloadName,
    },
    v2_services::{
        StrictWireMessage,
        broker::{
            AllocateRequest, AllocateResponse, SpawnRealmChildrenRequest,
            SpawnRealmChildrenResponse,
        },
        common::{DesiredState, Outcome, ServiceRequest, ServiceResponse},
    },
};
use d2b_core::{
    bundle_resolver::BundleResolver,
    storage::{
        ActorKind, PrincipalKind, SensitivityClass, StorageInvariant, StorageLifecycle,
        StoragePathKind, StoragePathSpec, StoragePersistence,
    },
    unsafe_local_workloads::UnsafeLocalLauncherItem,
};
use d2b_session::{Cancellation, OwnedAttachment};
use d2b_session_unix::{
    DescriptorPolicy, DescriptorPolicyResolver, ObjectIdentity, OwnedUnixAttachment,
    UnixAttachmentPayload, UnixSessionError,
};
use nix::{
    fcntl::{FcntlArg, FdFlag, OFlag, SealFlag, fcntl},
    sys::stat::{SFlag, fstat},
    unistd::{Group, User},
};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::guest_material_authority::{
    FileBootstrapReplayLedger, RealmGuestSessionAuthorityConnector,
};
pub use crate::guest_material_store::{
    EnrollmentSuccessAuditIdentity, FilesystemGuestMaterialStore, GuestMaterialStore,
    GuestMaterialTransaction, NoopGuestMaterialStore,
};
use crate::service_v2::{
    BrokerCallContext, BrokerMethod, BrokerOperationHandler, BrokerReply, BrokerRuntimeDispatch,
    BrokerServiceFailure, RealmBrokerSessionBinding, attachment_descriptor,
};

pub const GUEST_SESSION_STORAGE_PREFIX: &str = "path:workload-guest-session-credential:";
pub const CONFIGURED_LAUNCH_STORAGE_PREFIX: &str = "path:workload-configured-launch-credential:";
pub const GUEST_SESSION_CREDENTIAL_NAME: &str = "d2b-guest-session-v2";
pub const CONFIGURED_LAUNCH_CREDENTIAL_NAME: &str = "d2b-configured-launch-v2";
pub const BROKER_APPLY_METHOD_ID: u32 = 2_253_834_528;
pub const GUEST_ENROLLMENT_WIRE_PREFIX: &str =
    d2b_host::guest_runtime::GUEST_ENROLLMENT_WIRE_PREFIX;

const MAX_REPLAY_RECORDS: usize = 1_024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GuestMaterialError {
    InvalidRequest,
    AuthorityUnavailable,
    AuthorityMismatch,
    GenerationMismatch,
    StorageRefUnknown,
    StorageContractMismatch,
    InventoryUnavailable,
    InventoryInvalid,
    DigestMismatch,
    BootstrapInvalid,
    Replay,
    Cancelled,
    DeadlineExceeded,
    ResourceExhausted,
    MemfdFailed,
    MaterializationFailed,
    AuditFailed,
    HandlerDropped,
}

impl GuestMaterialError {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "guest-material-request-invalid",
            Self::AuthorityUnavailable => "guest-material-authority-unavailable",
            Self::AuthorityMismatch => "guest-material-authority-mismatch",
            Self::GenerationMismatch => "guest-material-generation-mismatch",
            Self::StorageRefUnknown => "guest-material-storage-ref-unknown",
            Self::StorageContractMismatch => "guest-material-storage-contract-mismatch",
            Self::InventoryUnavailable => "guest-material-inventory-unavailable",
            Self::InventoryInvalid => "guest-material-inventory-invalid",
            Self::DigestMismatch => "guest-material-digest-mismatch",
            Self::BootstrapInvalid => "guest-material-bootstrap-invalid",
            Self::Replay => "guest-material-replay-refused",
            Self::Cancelled => "guest-material-cancelled",
            Self::DeadlineExceeded => "guest-material-deadline-exceeded",
            Self::ResourceExhausted => "guest-material-replay-capacity-exhausted",
            Self::MemfdFailed => "guest-material-memfd-failed",
            Self::MaterializationFailed => "guest-material-materialization-failed",
            Self::AuditFailed => "guest-material-audit-failed",
            Self::HandlerDropped => "guest-material-handler-dropped",
        }
    }
}

impl fmt::Debug for GuestMaterialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("GuestMaterialError")
            .field(&self.as_str())
            .finish()
    }
}

impl fmt::Display for GuestMaterialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::error::Error for GuestMaterialError {}

impl From<GuestMaterialError> for BrokerServiceFailure {
    fn from(error: GuestMaterialError) -> Self {
        match error {
            GuestMaterialError::InvalidRequest
            | GuestMaterialError::AuthorityMismatch
            | GuestMaterialError::StorageContractMismatch
            | GuestMaterialError::InventoryInvalid
            | GuestMaterialError::DigestMismatch
            | GuestMaterialError::BootstrapInvalid => Self::InvalidRequest,
            GuestMaterialError::GenerationMismatch => Self::GenerationMismatch,
            GuestMaterialError::StorageRefUnknown | GuestMaterialError::InventoryUnavailable => {
                Self::NotFound
            }
            GuestMaterialError::Replay => Self::Conflict,
            GuestMaterialError::Cancelled => Self::Cancelled,
            GuestMaterialError::DeadlineExceeded => Self::DeadlineExceeded,
            GuestMaterialError::ResourceExhausted => Self::ResourceExhausted,
            GuestMaterialError::AuthorityUnavailable
            | GuestMaterialError::MemfdFailed
            | GuestMaterialError::MaterializationFailed
            | GuestMaterialError::AuditFailed
            | GuestMaterialError::HandlerDropped => Self::Backend,
        }
    }
}

pub struct GuestBootstrapAuthority {
    pub binding: BootstrapPskBinding,
    pub issued_at_unix_ms: u64,
    pub psk: GuestBootstrapPsk,
}

impl fmt::Debug for GuestBootstrapAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GuestBootstrapAuthority(REDACTED)")
    }
}

pub struct GuestSessionAuthority {
    pub realm_id: String,
    pub workload_id: String,
    pub session_generation: u64,
    pub parent_static_public_key: [u8; 32],
    pub channel_binding: [u8; 32],
    pub guest_identity_digest: [u8; 32],
    pub guest_static_public_key: [u8; 32],
    pub bootstrap: Option<GuestBootstrapAuthority>,
}

impl fmt::Debug for GuestSessionAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestSessionAuthority")
            .field("realm_id", &"<closed-id>")
            .field("workload_id", &"<closed-id>")
            .field("session_generation", &"<redacted>")
            .field("key_material", &"<redacted>")
            .field("has_bootstrap", &self.bootstrap.is_some())
            .finish()
    }
}

#[derive(Clone)]
pub struct GuestAuthorityLookup {
    pub realm_id: String,
    pub workload_id: String,
    pub operation_id: String,
    pub storage_ref: String,
    pub request_digest: [u8; 32],
    pub session_generation: u64,
}

impl fmt::Debug for GuestAuthorityLookup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestAuthorityLookup")
            .field("realm_id", &"<closed-id>")
            .field("workload_id", &"<closed-id>")
            .field("operation_id", &"<closed-id>")
            .field("storage_ref", &"<closed-id>")
            .field("request_digest", &hex_digest(&self.request_digest))
            .field("session_generation", &"<redacted>")
            .finish()
    }
}

#[async_trait]
pub trait GuestSessionAuthorityPort: Send + Sync {
    async fn resolve(
        &self,
        lookup: GuestAuthorityLookup,
    ) -> Result<GuestSessionAuthority, GuestMaterialError>;
}

#[derive(Clone)]
pub struct GuestMaterialTarget {
    pub storage_ref: String,
    pub path: PathBuf,
    pub owner_uid: u32,
    pub owner_gid: u32,
    pub mode: u32,
}

impl fmt::Debug for GuestMaterialTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestMaterialTarget")
            .field("storage_ref", &"<closed-id>")
            .field("path", &"<redacted>")
            .field("owner_uid", &self.owner_uid)
            .field("owner_gid", &self.owner_gid)
            .field("mode", &format_args!("{:04o}", self.mode))
            .finish()
    }
}

pub struct GuestMaterialBundle {
    pub session_target: GuestMaterialTarget,
    pub configured_launch_target: GuestMaterialTarget,
    pub configured_launches: GuestConfiguredLaunchesBytes,
    pub configured_launch_digest: [u8; 32],
}

impl fmt::Debug for GuestMaterialBundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestMaterialBundle")
            .field("session_target", &self.session_target)
            .field("configured_launch_target", &self.configured_launch_target)
            .field("configured_launches", &"<redacted>")
            .field(
                "configured_launch_digest",
                &hex_digest(&self.configured_launch_digest),
            )
            .finish()
    }
}

pub trait GuestMaterialBundlePort: Send + Sync {
    fn resolve(
        &self,
        storage_ref: &str,
        realm_id: &str,
        workload_id: &str,
    ) -> Result<GuestMaterialBundle, GuestMaterialError>;
}

pub struct VerifiedBundleGuestMaterialPort {
    resolver: Arc<BundleResolver>,
}

impl VerifiedBundleGuestMaterialPort {
    pub fn new(resolver: Arc<BundleResolver>) -> Self {
        Self { resolver }
    }
}

impl GuestMaterialBundlePort for VerifiedBundleGuestMaterialPort {
    fn resolve(
        &self,
        storage_ref: &str,
        realm_id: &str,
        workload_id: &str,
    ) -> Result<GuestMaterialBundle, GuestMaterialError> {
        let expected_session_ref = format!("{GUEST_SESSION_STORAGE_PREFIX}{workload_id}");
        if storage_ref != expected_session_ref {
            return Err(GuestMaterialError::StorageRefUnknown);
        }
        let configured_ref = format!("{CONFIGURED_LAUNCH_STORAGE_PREFIX}{workload_id}");
        let session_spec = self
            .resolver
            .find_storage_path_spec(storage_ref)
            .ok_or(GuestMaterialError::StorageRefUnknown)?;
        let configured_spec = self
            .resolver
            .find_storage_path_spec(&configured_ref)
            .ok_or(GuestMaterialError::StorageRefUnknown)?;
        let session_target = resolve_storage_target(
            session_spec,
            storage_ref,
            realm_id,
            workload_id,
            GUEST_SESSION_CREDENTIAL_NAME,
        )?;
        let configured_launch_target = resolve_storage_target(
            configured_spec,
            &configured_ref,
            realm_id,
            workload_id,
            CONFIGURED_LAUNCH_CREDENTIAL_NAME,
        )?;
        if session_spec.scope != configured_spec.scope
            || session_target.path.parent() != configured_launch_target.path.parent()
        {
            return Err(GuestMaterialError::StorageContractMismatch);
        }

        let inventory = self
            .resolver
            .unsafe_local_workloads
            .as_ref()
            .and_then(|private| {
                private.local_vm_workloads.iter().find(|workload| {
                    workload.identity.realm_id.as_str() == realm_id
                        && workload.identity.workload_id.as_str() == workload_id
                })
            })
            .ok_or(GuestMaterialError::InventoryUnavailable)?;
        inventory
            .validate()
            .map_err(|_| GuestMaterialError::InventoryInvalid)?;
        let inventory_bytes = Zeroizing::new(
            serde_json::to_vec(inventory).map_err(|_| GuestMaterialError::InventoryInvalid)?,
        );
        if inventory_bytes.is_empty() {
            return Err(GuestMaterialError::InventoryInvalid);
        }
        let workload_digest: [u8; 32] = Sha256::digest(&inventory_bytes[..]).into();
        let realm_path = V2RealmPath::parse(format!(
            "{}.local-root",
            inventory.identity.realm_path.target_form()
        ))
        .map_err(|_| GuestMaterialError::InventoryInvalid)?;
        let canonical_realm_id = V2RealmId::derive(&realm_path);
        let workload_name =
            WorkloadName::parse(workload_id).map_err(|_| GuestMaterialError::InventoryInvalid)?;
        let canonical_workload_id = V2WorkloadId::derive(&canonical_realm_id, &workload_name);
        let entries = inventory
            .items
            .iter()
            .map(|item| match item {
                UnsafeLocalLauncherItem::Exec(item) => GuestConfiguredLaunchEntryV1::new(
                    item.id.clone(),
                    item.argv.clone(),
                    item.graphical,
                )
                .map_err(|_| GuestMaterialError::InventoryInvalid),
                UnsafeLocalLauncherItem::Shell(_) => Err(GuestMaterialError::InventoryInvalid),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let configured_launches = GuestConfiguredLaunchesV1::new(
            canonical_realm_id,
            canonical_workload_id,
            workload_digest,
            entries,
        )
        .and_then(|catalog| catalog.encode())
        .map_err(|_| GuestMaterialError::InventoryInvalid)?;
        let configured_launch_digest = configured_launches.sha256();
        Ok(GuestMaterialBundle {
            session_target,
            configured_launch_target,
            configured_launches,
            configured_launch_digest,
        })
    }
}

fn resolve_storage_target(
    spec: &StoragePathSpec,
    storage_ref: &str,
    realm_id: &str,
    workload_id: &str,
    expected_name: &str,
) -> Result<GuestMaterialTarget, GuestMaterialError> {
    if spec.id.as_str() != storage_ref
        || spec.kind != StoragePathKind::RegularFile
        || spec.lifecycle != StorageLifecycle::ProcessScoped
        || spec.persistence != StoragePersistence::ProcessScoped
        || spec.creator.kind != ActorKind::Broker
        || !spec
            .writers
            .iter()
            .any(|writer| writer.kind == ActorKind::Broker)
        || spec.owner.kind != PrincipalKind::User
        || spec.owner.value.as_str() != "root"
        || spec.sensitivity != SensitivityClass::SecretAdjacent
        || ![
            StorageInvariant::NoSymlink,
            StorageInvariant::NoMagicLink,
            StorageInvariant::BrokerOpaqueIdOnly,
            StorageInvariant::RootOwnedParent,
            StorageInvariant::ScopeAuthorizationRequired,
        ]
        .iter()
        .all(|required| spec.invariants.contains(required))
    {
        return Err(GuestMaterialError::StorageContractMismatch);
    }
    let mode = parse_private_mode(&spec.mode)?;
    let path = PathBuf::from(spec.path_template.as_str());
    let expected_parent = PathBuf::from("/run/d2b/r")
        .join(realm_id)
        .join("w")
        .join(workload_id)
        .join("guest-session");
    if path.parent() != Some(expected_parent.as_path())
        || path.file_name().and_then(|name| name.to_str()) != Some(expected_name)
    {
        return Err(GuestMaterialError::StorageContractMismatch);
    }
    let owner_uid = resolve_uid(&spec.owner)?;
    let owner_gid = resolve_gid(&spec.group)?;
    if owner_uid != 0 {
        return Err(GuestMaterialError::StorageContractMismatch);
    }
    Ok(GuestMaterialTarget {
        storage_ref: storage_ref.to_owned(),
        path,
        owner_uid,
        owner_gid,
        mode,
    })
}

fn parse_private_mode(raw: &str) -> Result<u32, GuestMaterialError> {
    let mode = u32::from_str_radix(raw.trim_start_matches('0'), 8)
        .map_err(|_| GuestMaterialError::StorageContractMismatch)?;
    if mode != 0o440 {
        return Err(GuestMaterialError::StorageContractMismatch);
    }
    Ok(mode)
}

fn resolve_uid(principal: &d2b_core::storage::PrincipalRef) -> Result<u32, GuestMaterialError> {
    match principal.kind {
        PrincipalKind::Uid => principal
            .value
            .as_str()
            .parse()
            .map_err(|_| GuestMaterialError::StorageContractMismatch),
        PrincipalKind::User => User::from_name(principal.value.as_str())
            .map_err(|_| GuestMaterialError::StorageContractMismatch)?
            .map(|user| user.uid.as_raw())
            .ok_or(GuestMaterialError::StorageContractMismatch),
        _ => Err(GuestMaterialError::StorageContractMismatch),
    }
}

fn resolve_gid(principal: &d2b_core::storage::PrincipalRef) -> Result<u32, GuestMaterialError> {
    match principal.kind {
        PrincipalKind::Gid => principal
            .value
            .as_str()
            .parse()
            .map_err(|_| GuestMaterialError::StorageContractMismatch),
        PrincipalKind::Group | PrincipalKind::Role => Group::from_name(principal.value.as_str())
            .map_err(|_| GuestMaterialError::StorageContractMismatch)?
            .map(|group| group.gid.as_raw())
            .ok_or(GuestMaterialError::StorageContractMismatch),
        _ => Err(GuestMaterialError::StorageContractMismatch),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestMaterialOutcome {
    Succeeded,
    Failed,
}

#[derive(Clone, PartialEq, Eq)]
pub struct GuestMaterialAuditRecord {
    pub realm_id: String,
    pub workload_id: String,
    pub operation_id: String,
    pub session_storage_ref: String,
    pub configured_storage_ref: String,
    pub session_generation: u64,
    pub request_digest: [u8; 32],
    pub credential_digest: Option<[u8; 32]>,
    pub configured_launch_digest: [u8; 32],
    pub outcome: GuestMaterialOutcome,
    pub error_kind: Option<&'static str>,
}

impl fmt::Debug for GuestMaterialAuditRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestMaterialAuditRecord")
            .field("realm_id", &self.realm_id)
            .field("workload_id", &self.workload_id)
            .field("operation_id", &self.operation_id)
            .field("session_storage_ref", &self.session_storage_ref)
            .field("configured_storage_ref", &self.configured_storage_ref)
            .field("session_generation", &self.session_generation)
            .field("request_digest", &hex_digest(&self.request_digest))
            .field(
                "credential_digest",
                &self.credential_digest.as_ref().map(hex_digest),
            )
            .field(
                "configured_launch_digest",
                &hex_digest(&self.configured_launch_digest),
            )
            .field("outcome", &self.outcome)
            .field("error_kind", &self.error_kind)
            .finish()
    }
}

pub trait GuestMaterialAuditSink: Send + Sync {
    fn record(&self, record: &GuestMaterialAuditRecord) -> Result<(), GuestMaterialError>;
}

pub struct NoopGuestMaterialAuditSink;

impl GuestMaterialAuditSink for NoopGuestMaterialAuditSink {
    fn record(&self, _: &GuestMaterialAuditRecord) -> Result<(), GuestMaterialError> {
        Ok(())
    }
}

pub trait GuestMaterialClock: Send + Sync {
    fn now_unix_ms(&self) -> u64;
}

pub struct SystemGuestMaterialClock;

impl GuestMaterialClock for SystemGuestMaterialClock {
    fn now_unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ReplayKey {
    realm_id: String,
    workload_id: String,
    operation_id: String,
}

enum ReplayRecord {
    InProgress {
        request_digest: [u8; 32],
    },
    Complete {
        request_digest: [u8; 32],
        bootstrap: bool,
    },
    Failed {
        request_digest: [u8; 32],
    },
}

const DROP_REASON_NONE: u8 = 0;
const DROP_REASON_DEADLINE: u8 = 1;

struct ReplayReservation {
    replay: Arc<Mutex<BTreeMap<ReplayKey, ReplayRecord>>>,
    key: ReplayKey,
    request_digest: [u8; 32],
    audit: Arc<dyn GuestMaterialAuditSink>,
    failure_record: GuestMaterialAuditRecord,
    cancellation: Cancellation,
    drop_reason: Arc<AtomicU8>,
    active: bool,
}

impl ReplayReservation {
    fn complete(&mut self, bootstrap: bool) -> Result<(), GuestMaterialError> {
        self.set_terminal(ReplayRecord::Complete {
            request_digest: self.request_digest,
            bootstrap,
        })?;
        self.active = false;
        Ok(())
    }

    fn fail(&mut self, error: GuestMaterialError) -> Result<(), GuestMaterialError> {
        self.set_terminal(ReplayRecord::Failed {
            request_digest: self.request_digest,
        })?;
        self.failure_record.error_kind = Some(error.as_str());
        self.active = false;
        self.audit.record(&self.failure_record)
    }

    fn set_terminal(&self, record: ReplayRecord) -> Result<(), GuestMaterialError> {
        self.replay
            .lock()
            .map_err(|_| GuestMaterialError::ResourceExhausted)?
            .insert(self.key.clone(), record);
        Ok(())
    }
}

impl Drop for ReplayReservation {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let _ = self.set_terminal(ReplayRecord::Failed {
            request_digest: self.request_digest,
        });
        self.failure_record.error_kind = Some(
            if self.cancellation.is_cancelled() {
                GuestMaterialError::Cancelled
            } else if self.drop_reason.load(Ordering::SeqCst) == DROP_REASON_DEADLINE {
                GuestMaterialError::DeadlineExceeded
            } else {
                GuestMaterialError::HandlerDropped
            }
            .as_str(),
        );
        let _ = self.audit.record(&self.failure_record);
        self.active = false;
    }
}

pub struct GuestSessionMaterialBroker {
    authority: Arc<dyn GuestSessionAuthorityPort>,
    bundle: Arc<dyn GuestMaterialBundlePort>,
    store: Arc<dyn GuestMaterialStore>,
    audit: Arc<dyn GuestMaterialAuditSink>,
    clock: Arc<dyn GuestMaterialClock>,
    replay: Arc<Mutex<BTreeMap<ReplayKey, ReplayRecord>>>,
}

pub struct RealmBoundGuestMaterialDispatch<R> {
    realm_id: String,
    authority: Arc<RealmGuestSessionAuthorityConnector>,
    guest_material: Arc<GuestSessionMaterialBroker>,
    fallback: R,
}

impl<R> RealmBoundGuestMaterialDispatch<R> {
    pub fn new(
        realm_id: String,
        authority: Arc<RealmGuestSessionAuthorityConnector>,
        guest_material: Arc<GuestSessionMaterialBroker>,
        fallback: R,
    ) -> Result<Self, GuestMaterialError> {
        d2b_host::realm_children::validate_realm_id(&realm_id)
            .map_err(|_| GuestMaterialError::AuthorityMismatch)?;
        Ok(Self {
            realm_id,
            authority,
            guest_material,
            fallback,
        })
    }
}

pub struct ParentSpawnedRealmGuestRuntime<R> {
    binding: RealmBrokerSessionBinding,
    authority: Arc<RealmGuestSessionAuthorityConnector>,
    dispatch: Arc<RealmBoundGuestMaterialDispatch<R>>,
}

impl<R> ParentSpawnedRealmGuestRuntime<R>
where
    R: BrokerRuntimeDispatch + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub fn production(
        realm_id: String,
        controller_uid: u32,
        controller_gid: u32,
        generation: u64,
        replay_ledger_path: PathBuf,
        replay_owner_uid: u32,
        replay_owner_gid: u32,
        resolver: Arc<BundleResolver>,
        audit: Arc<dyn GuestMaterialAuditSink>,
        fallback: R,
    ) -> Result<Self, GuestMaterialError> {
        Self::with_ports(
            realm_id,
            controller_uid,
            controller_gid,
            generation,
            replay_ledger_path,
            replay_owner_uid,
            replay_owner_gid,
            Arc::new(VerifiedBundleGuestMaterialPort::new(resolver)),
            Arc::new(FilesystemGuestMaterialStore::default()),
            audit,
            Arc::new(SystemGuestMaterialClock),
            fallback,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_ports(
        realm_id: String,
        controller_uid: u32,
        controller_gid: u32,
        generation: u64,
        replay_ledger_path: PathBuf,
        replay_owner_uid: u32,
        replay_owner_gid: u32,
        bundle: Arc<dyn GuestMaterialBundlePort>,
        store: Arc<dyn GuestMaterialStore>,
        audit: Arc<dyn GuestMaterialAuditSink>,
        clock: Arc<dyn GuestMaterialClock>,
        fallback: R,
    ) -> Result<Self, GuestMaterialError> {
        let binding = RealmBrokerSessionBinding::new(
            realm_id.clone(),
            controller_uid,
            controller_gid,
            generation,
        )
        .map_err(|_| GuestMaterialError::AuthorityMismatch)?;
        let ledger = Arc::new(FileBootstrapReplayLedger::open(
            replay_ledger_path,
            replay_owner_uid,
            replay_owner_gid,
        )?);
        let authority = Arc::new(RealmGuestSessionAuthorityConnector::new(
            realm_id.clone(),
            ledger,
        ));
        let material = Arc::new(GuestSessionMaterialBroker::new(
            Arc::clone(&authority) as Arc<dyn GuestSessionAuthorityPort>,
            bundle,
            store,
            audit,
            clock,
        ));
        let dispatch = Arc::new(RealmBoundGuestMaterialDispatch::new(
            realm_id,
            Arc::clone(&authority),
            material,
            fallback,
        )?);
        Ok(Self {
            binding,
            authority,
            dispatch,
        })
    }

    pub fn authority(&self) -> &Arc<RealmGuestSessionAuthorityConnector> {
        &self.authority
    }

    pub fn realm_id(&self) -> &str {
        self.binding.realm_id()
    }

    pub async fn serve(self, fd: OwnedFd) -> Result<(), BrokerServiceFailure> {
        let handler = Arc::new(RealmGuestBrokerOperationHandler {
            runtime: self.dispatch,
        });
        self.binding.serve(fd, handler).await
    }
}

struct RealmGuestBrokerOperationHandler<R> {
    runtime: Arc<RealmBoundGuestMaterialDispatch<R>>,
}

#[async_trait]
impl<R> BrokerOperationHandler for RealmGuestBrokerOperationHandler<R>
where
    R: BrokerRuntimeDispatch + 'static,
{
    async fn handle(
        &self,
        method: BrokerMethod,
        request: ServiceRequest,
        attachments: Vec<OwnedAttachment>,
        context: &BrokerCallContext,
    ) -> Result<BrokerReply<ServiceResponse>, BrokerServiceFailure> {
        self.runtime
            .dispatch(method, request, attachments, context)
            .await
    }

    async fn allocate(
        &self,
        _: AllocateRequest,
        _: &BrokerCallContext,
    ) -> Result<BrokerReply<AllocateResponse>, BrokerServiceFailure> {
        Err(BrokerServiceFailure::PermissionDenied)
    }

    async fn spawn(
        &self,
        _: SpawnRealmChildrenRequest,
        _: Vec<OwnedAttachment>,
        _: &BrokerCallContext,
    ) -> Result<BrokerReply<SpawnRealmChildrenResponse>, BrokerServiceFailure> {
        Err(BrokerServiceFailure::PermissionDenied)
    }

    fn attachment_policy_resolver(&self) -> Option<DescriptorPolicyResolver> {
        Some(guest_material_descriptor_policy_resolver())
    }
}

pub fn guest_material_descriptor_policy_resolver() -> DescriptorPolicyResolver {
    Arc::new(|descriptor| {
        let request =
            descriptor.purpose == AttachmentPurpose::RequestInput && descriptor.index == 0;
        let response =
            descriptor.purpose == AttachmentPurpose::ResponseOutput && descriptor.index <= 1;
        if descriptor.service == ServicePackage::BrokerV2
            && descriptor.method_id == BROKER_APPLY_METHOD_ID
            && descriptor.kind == AttachmentKind::FileDescriptor
            && descriptor.object_type == KernelObjectType::Memfd
            && descriptor.access == AttachmentAccess::ReadOnly
            && descriptor.cloexec_required
            && (request || response)
        {
            Ok(DescriptorPolicy::SealedReadOnlyMemfd)
        } else {
            Err(UnixSessionError::DescriptorMismatch)
        }
    })
}

impl<R> fmt::Debug for RealmBoundGuestMaterialDispatch<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RealmBoundGuestMaterialDispatch")
            .field("realm_id", &"<closed-id>")
            .field("guest_material", &"<redacted>")
            .finish()
    }
}

#[async_trait]
impl<R> BrokerRuntimeDispatch for RealmBoundGuestMaterialDispatch<R>
where
    R: BrokerRuntimeDispatch,
{
    async fn dispatch(
        &self,
        method: BrokerMethod,
        request: ServiceRequest,
        attachments: Vec<OwnedAttachment>,
        context: &BrokerCallContext,
    ) -> Result<BrokerReply<ServiceResponse>, BrokerServiceFailure> {
        if request
            .resource_id
            .starts_with(d2b_host::guest_runtime::GUEST_MATERIAL_WIRE_PREFIX)
        {
            if method != BrokerMethod::Apply
                || !attachments.is_empty()
                || context.peer_role != crate::service_v2::BrokerPeerRole::RealmController
                || request.scope.realm_id != self.realm_id
            {
                return Err(BrokerServiceFailure::PermissionDenied);
            }
            return self.guest_material.apply(request, context).await;
        }
        if request
            .resource_id
            .starts_with(GUEST_ENROLLMENT_WIRE_PREFIX)
        {
            if method != BrokerMethod::Apply
                || context.peer_role != crate::service_v2::BrokerPeerRole::RealmController
                || request.scope.realm_id != self.realm_id
            {
                return Err(BrokerServiceFailure::PermissionDenied);
            }
            return persist_enrolled_identity(
                &self.guest_material,
                &self.authority,
                request,
                attachments,
                context,
            );
        }
        self.fallback
            .dispatch(method, request, attachments, context)
            .await
    }
}

impl fmt::Debug for GuestSessionMaterialBroker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GuestSessionMaterialBroker(REDACTED)")
    }
}

impl GuestSessionMaterialBroker {
    pub fn new(
        authority: Arc<dyn GuestSessionAuthorityPort>,
        bundle: Arc<dyn GuestMaterialBundlePort>,
        store: Arc<dyn GuestMaterialStore>,
        audit: Arc<dyn GuestMaterialAuditSink>,
        clock: Arc<dyn GuestMaterialClock>,
    ) -> Self {
        Self {
            authority,
            bundle,
            store,
            audit,
            clock,
            replay: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub async fn apply(
        &self,
        request: ServiceRequest,
        context: &BrokerCallContext,
    ) -> Result<BrokerReply<ServiceResponse>, BrokerServiceFailure> {
        if context.peer_role != crate::service_v2::BrokerPeerRole::RealmController {
            return Err(BrokerServiceFailure::PermissionDenied);
        }
        if context.cancellation.is_cancelled() {
            self.record_outer_failure(&request, context, GuestMaterialError::Cancelled)?;
            return Err(BrokerServiceFailure::Cancelled);
        }
        if context.remaining.is_zero() {
            self.record_outer_failure(&request, context, GuestMaterialError::DeadlineExceeded)?;
            return Err(BrokerServiceFailure::DeadlineExceeded);
        }
        let drop_reason = Arc::new(AtomicU8::new(DROP_REASON_NONE));
        let operation = self.apply_inner(request, context, Arc::clone(&drop_reason));
        tokio::pin!(operation);
        tokio::select! {
            result = &mut operation => result.map_err(Into::into),
            () = tokio::time::sleep(context.remaining) => {
                drop_reason.store(DROP_REASON_DEADLINE, Ordering::SeqCst);
                Err(BrokerServiceFailure::DeadlineExceeded)
            }
        }
    }

    fn record_outer_failure(
        &self,
        request: &ServiceRequest,
        context: &BrokerCallContext,
        error: GuestMaterialError,
    ) -> Result<(), BrokerServiceFailure> {
        validate_request(request).map_err(BrokerServiceFailure::from)?;
        let request_digest = request
            .request_digest
            .as_slice()
            .try_into()
            .map_err(|_| BrokerServiceFailure::InvalidRequest)?;
        self.record_failure(request, context, request_digest, error)
            .map_err(BrokerServiceFailure::from)
    }

    async fn apply_inner(
        &self,
        request: ServiceRequest,
        context: &BrokerCallContext,
        drop_reason: Arc<AtomicU8>,
    ) -> Result<BrokerReply<ServiceResponse>, GuestMaterialError> {
        if context.cancellation.is_cancelled() {
            return Err(GuestMaterialError::Cancelled);
        }
        validate_request(&request)?;
        let request_digest: [u8; 32] = request
            .request_digest
            .as_slice()
            .try_into()
            .map_err(|_| GuestMaterialError::InvalidRequest)?;
        let replay_key = ReplayKey {
            realm_id: request.scope.realm_id.clone(),
            workload_id: request.scope.workload_id.clone(),
            operation_id: request.operation_id.clone(),
        };
        let mut reservation =
            match self.begin_replay(replay_key, request_digest, &request, context, drop_reason) {
                Ok(reservation) => reservation,
                Err(error) => {
                    self.record_failure(&request, context, request_digest, error)?;
                    return Err(error);
                }
            };

        let result = self.produce_pair(&request, context, request_digest).await;
        match &result {
            Ok(produced) => reservation.complete(produced.had_bootstrap)?,
            Err(error) => {
                reservation.fail(*error)?;
            }
        }
        result.map(ProducedGuestMaterial::into_reply)
    }

    async fn produce_pair(
        &self,
        request: &ServiceRequest,
        context: &BrokerCallContext,
        request_digest: [u8; 32],
    ) -> Result<ProducedGuestMaterial, GuestMaterialError> {
        let session_storage_ref = format!(
            "{GUEST_SESSION_STORAGE_PREFIX}{}",
            request.scope.workload_id
        );
        let bundle = self.bundle.resolve(
            &session_storage_ref,
            &request.scope.realm_id,
            &request.scope.workload_id,
        )?;
        if context.cancellation.is_cancelled() {
            return Err(GuestMaterialError::Cancelled);
        }
        let authority = self
            .authority
            .resolve(GuestAuthorityLookup {
                realm_id: request.scope.realm_id.clone(),
                workload_id: request.scope.workload_id.clone(),
                operation_id: request.operation_id.clone(),
                storage_ref: request.resource_id.clone(),
                request_digest,
                session_generation: context.session_generation,
            })
            .await?;
        if context.cancellation.is_cancelled() {
            return Err(GuestMaterialError::Cancelled);
        }
        validate_authority(request, context, &authority)?;
        let expected_digest = guest_material_request_digest(GuestMaterialRequestDigestInput {
            realm_id: &request.scope.realm_id,
            workload_id: &request.scope.workload_id,
            operation_id: &request.operation_id,
            session_storage_ref: &request.resource_id,
            session_generation: authority.session_generation,
        });
        if expected_digest != request_digest {
            return Err(GuestMaterialError::DigestMismatch);
        }

        let had_bootstrap = authority.bootstrap.is_some();
        let bootstrap = authority
            .bootstrap
            .map(|bootstrap| {
                GuestBootstrapCredentialV1::new(
                    bootstrap.binding,
                    bootstrap.issued_at_unix_ms,
                    bootstrap.psk,
                )
                .map_err(|_| GuestMaterialError::BootstrapInvalid)
            })
            .transpose()?;
        if let Some(bootstrap) = bootstrap.as_ref() {
            bootstrap
                .admit(self.clock.now_unix_ms())
                .map_err(|_| GuestMaterialError::BootstrapInvalid)?;
        }
        let guest_identity = if authority.guest_identity_digest == [0; 32]
            && authority.guest_static_public_key == [0; 32]
            && bootstrap.is_some()
        {
            GuestIdentityBindingV1::UnboundBootstrap
        } else {
            GuestIdentityBindingV1::Enrolled {
                guest_identity_digest: authority.guest_identity_digest,
                guest_static_public_key: authority.guest_static_public_key,
            }
        };
        let credential = GuestSessionCredentialV1::new(
            authority.session_generation,
            authority.parent_static_public_key,
            authority.channel_binding,
            guest_identity,
            bootstrap,
        )
        .map_err(|_| GuestMaterialError::AuthorityMismatch)?;
        let session_bytes = credential
            .encode()
            .map_err(|_| GuestMaterialError::AuthorityMismatch)?;
        let credential_digest: [u8; 32] = Sha256::digest(session_bytes.as_slice()).into();
        let configured_bytes = bundle.configured_launches.as_slice();
        let result_digest = pair_result_digest(credential_digest, bundle.configured_launch_digest);

        let session_fd = sealed_read_only_memfd(GUEST_SESSION_CREDENTIAL_NAME, &session_bytes)?;
        let configured_fd =
            sealed_read_only_memfd_bytes(CONFIGURED_LAUNCH_CREDENTIAL_NAME, configured_bytes)?;
        let mut transaction = self.store.stage_pair(
            &bundle.session_target,
            &session_bytes,
            &bundle.configured_launch_target,
            configured_bytes,
        )?;
        let attachments = vec![
            response_attachment(context, 0, session_fd)?,
            response_attachment(context, 1, configured_fd)?,
        ];
        let audit = GuestMaterialAuditRecord {
            realm_id: request.scope.realm_id.clone(),
            workload_id: request.scope.workload_id.clone(),
            operation_id: request.operation_id.clone(),
            session_storage_ref: bundle.session_target.storage_ref.clone(),
            configured_storage_ref: bundle.configured_launch_target.storage_ref.clone(),
            session_generation: authority.session_generation,
            request_digest,
            credential_digest: Some(credential_digest),
            configured_launch_digest: bundle.configured_launch_digest,
            outcome: GuestMaterialOutcome::Succeeded,
            error_kind: None,
        };
        if transaction.commit().is_err() || transaction.mark_committed().is_err() {
            transaction.rollback()?;
            return Err(GuestMaterialError::MaterializationFailed);
        }
        if self.audit.record(&audit).is_err() {
            transaction.rollback()?;
            return Err(GuestMaterialError::AuditFailed);
        }
        transaction.finalize();

        let mut response = ServiceResponse::new();
        response.outcome = Outcome::OUTCOME_SUCCEEDED.into();
        response.operation_id = request.operation_id.clone();
        response.result_digest = result_digest.to_vec();
        response.attachment_indexes = vec![0, 1];
        Ok(ProducedGuestMaterial {
            response,
            attachments,
            had_bootstrap,
        })
    }

    fn record_failure(
        &self,
        request: &ServiceRequest,
        context: &BrokerCallContext,
        request_digest: [u8; 32],
        error: GuestMaterialError,
    ) -> Result<(), GuestMaterialError> {
        let mut record = failure_audit_record(request, context, request_digest);
        record.error_kind = Some(error.as_str());
        self.audit
            .record(&record)
            .map_err(|_| GuestMaterialError::AuditFailed)
    }

    fn begin_replay(
        &self,
        key: ReplayKey,
        request_digest: [u8; 32],
        request: &ServiceRequest,
        context: &BrokerCallContext,
        drop_reason: Arc<AtomicU8>,
    ) -> Result<ReplayReservation, GuestMaterialError> {
        let mut replay = self
            .replay
            .lock()
            .map_err(|_| GuestMaterialError::ResourceExhausted)?;
        if let Some(record) = replay.get(&key) {
            match record {
                ReplayRecord::Complete {
                    request_digest: prior,
                    bootstrap: false,
                } if *prior == request_digest => {}
                ReplayRecord::InProgress {
                    request_digest: prior,
                }
                | ReplayRecord::Complete {
                    request_digest: prior,
                    ..
                }
                | ReplayRecord::Failed {
                    request_digest: prior,
                } if *prior != request_digest => {
                    return Err(GuestMaterialError::DigestMismatch);
                }
                _ => return Err(GuestMaterialError::Replay),
            }
            replay.insert(key.clone(), ReplayRecord::InProgress { request_digest });
        } else {
            if replay.len() >= MAX_REPLAY_RECORDS {
                return Err(GuestMaterialError::ResourceExhausted);
            }
            replay.insert(key.clone(), ReplayRecord::InProgress { request_digest });
        }
        drop(replay);
        Ok(ReplayReservation {
            replay: Arc::clone(&self.replay),
            key,
            request_digest,
            audit: Arc::clone(&self.audit),
            failure_record: failure_audit_record(request, context, request_digest),
            cancellation: context.cancellation.clone(),
            drop_reason,
            active: true,
        })
    }
}

fn persist_enrolled_identity(
    broker: &GuestSessionMaterialBroker,
    authority: &RealmGuestSessionAuthorityConnector,
    request: ServiceRequest,
    attachments: Vec<OwnedAttachment>,
    context: &BrokerCallContext,
) -> Result<BrokerReply<ServiceResponse>, BrokerServiceFailure> {
    if context.cancellation.is_cancelled() {
        return Err(BrokerServiceFailure::Cancelled);
    }
    if context.remaining.is_zero() {
        return Err(BrokerServiceFailure::DeadlineExceeded);
    }
    let expected_ref =
        d2b_host::guest_runtime::guest_enrollment_resource_id(&request.scope.workload_id);
    if request.scope.realm_id.is_empty()
        || request.scope.workload_id.is_empty()
        || !request.scope.provider_id.is_empty()
        || !request.scope.role_id.is_empty()
        || request.resource_id != expected_ref
        || request.operation_id.is_empty()
        || request.request_digest.len() != 32
        || request.attachment_indexes != [0]
        || !request.stream_id.is_empty()
        || request.page_size != 0
        || !request.page_cursor.is_empty()
        || request.desired_state.enum_value() != Ok(DesiredState::DESIRED_STATE_PRESENT)
        || attachments.len() != 1
    {
        return Err(BrokerServiceFailure::InvalidRequest);
    }
    let attachment = &attachments[0];
    let descriptor = attachment
        .descriptor()
        .ok_or(BrokerServiceFailure::AttachmentMismatch)?;
    if descriptor.index != 0
        || descriptor.kind != AttachmentKind::FileDescriptor
        || descriptor.object_type != KernelObjectType::Memfd
        || descriptor.access != AttachmentAccess::ReadOnly
        || descriptor.purpose != AttachmentPurpose::RequestInput
        || descriptor.service != ServicePackage::BrokerV2
        || descriptor.method_id != BROKER_APPLY_METHOD_ID
        || descriptor.request_id != context.request_id
        || descriptor.reconnect_generation != context.session_generation
        || !descriptor.cloexec_required
    {
        return Err(BrokerServiceFailure::AttachmentMismatch);
    }
    let fd = attachment
        .payload()
        .and_then(|payload| payload.downcast_ref::<UnixAttachmentPayload>())
        .and_then(UnixAttachmentPayload::file)
        .ok_or(BrokerServiceFailure::AttachmentMismatch)?
        .try_clone_to_owned()
        .map_err(|_| BrokerServiceFailure::AttachmentMismatch)?;
    let mut encoded = Zeroizing::new(Vec::new());
    File::from(fd)
        .take(16 * 1024 + 1)
        .read_to_end(&mut encoded)
        .map_err(|_| BrokerServiceFailure::AttachmentMismatch)?;
    if encoded.is_empty() || encoded.len() > 16 * 1024 {
        return Err(BrokerServiceFailure::AttachmentMismatch);
    }
    let credential = GuestSessionCredentialV1::decode(&encoded)
        .map_err(|_| BrokerServiceFailure::InvalidRequest)?;
    let canonical = credential
        .encode()
        .map_err(|_| BrokerServiceFailure::InvalidRequest)?;
    if canonical.as_slice() != encoded.as_slice() {
        return Err(BrokerServiceFailure::InvalidRequest);
    }
    let credential_digest: [u8; 32] = Sha256::digest(&encoded).into();
    let expected_digest = d2b_host::guest_runtime::guest_enrollment_apply_digest(
        d2b_host::guest_runtime::GuestEnrollmentApplyDigestInput {
            realm_id: &request.scope.realm_id,
            workload_id: &request.scope.workload_id,
            operation_id: &request.operation_id,
            enrollment_ref: &request.resource_id,
            session_generation: context.session_generation,
            credential_digest: &credential_digest,
        },
    );
    if request.request_digest != expected_digest {
        return Err(BrokerServiceFailure::InvalidRequest);
    }
    let storage_ref = format!(
        "{GUEST_SESSION_STORAGE_PREFIX}{}",
        request.scope.workload_id
    );
    let bundle = broker
        .bundle
        .resolve(
            &storage_ref,
            &request.scope.realm_id,
            &request.scope.workload_id,
        )
        .map_err(BrokerServiceFailure::from)?;
    let request_digest: [u8; 32] = request
        .request_digest
        .as_slice()
        .try_into()
        .map_err(|_| BrokerServiceFailure::InvalidRequest)?;
    let success_audit = GuestMaterialAuditRecord {
        realm_id: request.scope.realm_id.clone(),
        workload_id: request.scope.workload_id.clone(),
        operation_id: request.operation_id.clone(),
        session_storage_ref: bundle.session_target.storage_ref.clone(),
        configured_storage_ref: bundle.configured_launch_target.storage_ref.clone(),
        session_generation: credential.session_generation(),
        request_digest,
        credential_digest: Some(credential_digest),
        configured_launch_digest: bundle.configured_launch_digest,
        outcome: GuestMaterialOutcome::Succeeded,
        error_kind: None,
    };
    let response = ServiceResponse {
        outcome: Outcome::OUTCOME_SUCCEEDED.into(),
        operation_id: request.operation_id,
        result_digest: credential_digest.to_vec(),
        ..Default::default()
    };
    response
        .validate_wire(false)
        .map_err(|_| BrokerServiceFailure::Backend)?;
    let mut enrollment = match authority.stage_enrolled_credential(
        &request.scope.realm_id,
        &request.scope.workload_id,
        &credential,
        canonical.as_slice(),
    ) {
        Ok(enrollment) => enrollment,
        Err(error) => {
            record_enrollment_failure(broker.audit.as_ref(), &success_audit, error)?;
            return Err(error.into());
        }
    };
    let success_audit_identity = EnrollmentSuccessAuditIdentity::new(
        credential.session_generation(),
        request_digest,
        credential_digest,
        bundle.configured_launch_digest,
    )
    .map_err(BrokerServiceFailure::from)?;
    let recovery = enrollment.recovery_identity(
        credential_digest,
        bundle.configured_launch_digest,
        success_audit_identity,
    );
    let mut material = match broker.store.stage_enrollment_pair(
        &bundle.session_target,
        &canonical,
        &bundle.configured_launch_target,
        bundle.configured_launches.as_slice(),
        recovery,
    ) {
        Ok(material) => material,
        Err(error) => {
            let rollback = enrollment.rollback();
            record_enrollment_failure(broker.audit.as_ref(), &success_audit, error)?;
            if rollback.is_err() {
                return Err(BrokerServiceFailure::Backend);
            }
            return Err(error.into());
        }
    };
    if context.cancellation.is_cancelled() {
        rollback_enrollment(
            broker.audit.as_ref(),
            &success_audit,
            GuestMaterialError::Cancelled,
            &mut enrollment,
            &mut *material,
        )?;
        return Err(BrokerServiceFailure::Cancelled);
    }
    if let Err(error) = material.commit() {
        rollback_enrollment(
            broker.audit.as_ref(),
            &success_audit,
            error,
            &mut enrollment,
            &mut *material,
        )?;
        return Err(error.into());
    }
    if let Err(error) = enrollment.apply_memory() {
        rollback_enrollment(
            broker.audit.as_ref(),
            &success_audit,
            error,
            &mut enrollment,
            &mut *material,
        )?;
        return Err(error.into());
    }
    if let Err(error) = enrollment.commit() {
        rollback_enrollment(
            broker.audit.as_ref(),
            &success_audit,
            error,
            &mut enrollment,
            &mut *material,
        )?;
        return Err(error.into());
    }
    if let Err(error) = material.mark_committed() {
        rollback_enrollment(
            broker.audit.as_ref(),
            &success_audit,
            error,
            &mut enrollment,
            &mut *material,
        )?;
        return Err(error.into());
    }
    if broker.audit.record(&success_audit).is_err() {
        let rollback_ledger = enrollment.rollback();
        let rollback_material = material.rollback();
        if rollback_ledger.is_err() || rollback_material.is_err() {
            return Err(BrokerServiceFailure::Backend);
        }
        return Err(BrokerServiceFailure::Backend);
    }
    let _ = material.mark_audit_committed();
    enrollment.finalize();
    material.finalize();
    Ok(BrokerReply::message(response))
}

fn rollback_enrollment(
    audit: &dyn GuestMaterialAuditSink,
    success: &GuestMaterialAuditRecord,
    error: GuestMaterialError,
    enrollment: &mut crate::guest_material_authority::AuthorityEnrollmentTransaction,
    material: &mut dyn GuestMaterialTransaction,
) -> Result<(), BrokerServiceFailure> {
    let ledger = enrollment.rollback();
    let pair = material.rollback();
    let audit_result = record_enrollment_failure(audit, success, error);
    if ledger.is_err() || pair.is_err() || audit_result.is_err() {
        return Err(BrokerServiceFailure::Backend);
    }
    Ok(())
}

fn record_enrollment_failure(
    audit: &dyn GuestMaterialAuditSink,
    success: &GuestMaterialAuditRecord,
    error: GuestMaterialError,
) -> Result<(), BrokerServiceFailure> {
    let mut failure = success.clone();
    failure.outcome = GuestMaterialOutcome::Failed;
    failure.error_kind = Some(error.as_str());
    audit
        .record(&failure)
        .map_err(|_| BrokerServiceFailure::Backend)
}

fn failure_audit_record(
    request: &ServiceRequest,
    context: &BrokerCallContext,
    request_digest: [u8; 32],
) -> GuestMaterialAuditRecord {
    GuestMaterialAuditRecord {
        realm_id: request.scope.realm_id.clone(),
        workload_id: request.scope.workload_id.clone(),
        operation_id: request.operation_id.clone(),
        session_storage_ref: request.resource_id.clone(),
        configured_storage_ref: format!(
            "{CONFIGURED_LAUNCH_STORAGE_PREFIX}{}",
            request.scope.workload_id
        ),
        session_generation: context.session_generation,
        request_digest,
        credential_digest: None,
        configured_launch_digest: [0; 32],
        outcome: GuestMaterialOutcome::Failed,
        error_kind: None,
    }
}

struct ProducedGuestMaterial {
    response: ServiceResponse,
    attachments: Vec<OwnedAttachment>,
    had_bootstrap: bool,
}

impl ProducedGuestMaterial {
    fn into_reply(self) -> BrokerReply<ServiceResponse> {
        BrokerReply {
            message: self.response,
            attachments: self.attachments,
        }
    }
}

fn validate_request(request: &ServiceRequest) -> Result<(), GuestMaterialError> {
    if request.scope.realm_id.is_empty()
        || request.scope.workload_id.is_empty()
        || !request.scope.provider_id.is_empty()
        || !request.scope.role_id.is_empty()
        || request.operation_id.is_empty()
        || request.request_digest.len() != 32
        || !request.stream_id.is_empty()
        || !request.attachment_indexes.is_empty()
        || request.page_size != 0
        || !request.page_cursor.is_empty()
        || request.desired_state.enum_value() != Ok(DesiredState::DESIRED_STATE_PRESENT)
        || request.resource_id
            != d2b_host::guest_runtime::guest_material_resource_id(&request.scope.workload_id)
    {
        return Err(GuestMaterialError::InvalidRequest);
    }
    Ok(())
}

fn validate_authority(
    request: &ServiceRequest,
    context: &BrokerCallContext,
    authority: &GuestSessionAuthority,
) -> Result<(), GuestMaterialError> {
    if context.peer_role != crate::service_v2::BrokerPeerRole::RealmController
        || authority.realm_id != request.scope.realm_id
        || authority.workload_id != request.scope.workload_id
    {
        return Err(GuestMaterialError::AuthorityMismatch);
    }
    if authority.session_generation != context.session_generation {
        return Err(GuestMaterialError::GenerationMismatch);
    }
    if authority.parent_static_public_key == [0; 32] || authority.channel_binding == [0; 32] {
        return Err(GuestMaterialError::AuthorityMismatch);
    }
    let unbound =
        authority.guest_identity_digest == [0; 32] && authority.guest_static_public_key == [0; 32];
    let partially_bound = (authority.guest_identity_digest == [0; 32])
        != (authority.guest_static_public_key == [0; 32]);
    if partially_bound || (unbound && authority.bootstrap.is_none()) {
        return Err(GuestMaterialError::AuthorityMismatch);
    }
    Ok(())
}

pub struct GuestMaterialRequestDigestInput<'a> {
    pub realm_id: &'a str,
    pub workload_id: &'a str,
    pub operation_id: &'a str,
    pub session_storage_ref: &'a str,
    pub session_generation: u64,
}

pub fn guest_material_request_digest(input: GuestMaterialRequestDigestInput<'_>) -> [u8; 32] {
    d2b_host::guest_runtime::guest_material_apply_digest(
        d2b_host::guest_runtime::GuestMaterialApplyDigestInput {
            realm_id: input.realm_id,
            workload_id: input.workload_id,
            operation_id: input.operation_id,
            session_storage_ref: input.session_storage_ref,
            session_generation: input.session_generation,
        },
    )
}

fn digest_field(digest: &mut Sha256, field: &[u8]) {
    digest.update(u32::try_from(field.len()).unwrap_or(u32::MAX).to_be_bytes());
    digest.update(field);
}

fn pair_result_digest(session_digest: [u8; 32], configured_digest: [u8; 32]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"d2b-guest-material-pair-v1\0");
    digest.update(session_digest);
    digest.update(configured_digest);
    digest.finalize().into()
}

fn sealed_read_only_memfd(
    name: &str,
    bytes: &GuestSessionCredentialBytes,
) -> Result<OwnedFd, GuestMaterialError> {
    sealed_read_only_memfd_bytes(name, bytes.as_slice())
}

fn sealed_read_only_memfd_bytes(name: &str, bytes: &[u8]) -> Result<OwnedFd, GuestMaterialError> {
    let fd = rustix::fs::memfd_create(
        name,
        rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
    )
    .map_err(|_| GuestMaterialError::MemfdFailed)?;
    let mut writer = File::from(fd);
    writer
        .write_all(bytes)
        .and_then(|()| writer.flush())
        .and_then(|()| writer.seek(SeekFrom::Start(0)).map(|_| ()))
        .map_err(|_| GuestMaterialError::MemfdFailed)?;
    let seals = SealFlag::F_SEAL_WRITE
        | SealFlag::F_SEAL_GROW
        | SealFlag::F_SEAL_SHRINK
        | SealFlag::F_SEAL_SEAL;
    fcntl(writer.as_raw_fd(), FcntlArg::F_ADD_SEALS(seals))
        .map_err(|_| GuestMaterialError::MemfdFailed)?;
    let readonly = rustix::fs::open(
        format!("/proc/self/fd/{}", writer.as_raw_fd()),
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| GuestMaterialError::MemfdFailed)?;
    drop(writer);
    validate_sealed_read_only_fd(&readonly)?;
    Ok(readonly)
}

fn validate_sealed_read_only_fd(fd: &impl AsRawFd) -> Result<(), GuestMaterialError> {
    let status = OFlag::from_bits_truncate(
        fcntl(fd.as_raw_fd(), FcntlArg::F_GETFL).map_err(|_| GuestMaterialError::MemfdFailed)?,
    );
    let descriptor = FdFlag::from_bits_truncate(
        fcntl(fd.as_raw_fd(), FcntlArg::F_GETFD).map_err(|_| GuestMaterialError::MemfdFailed)?,
    );
    let seals = SealFlag::from_bits_truncate(
        fcntl(fd.as_raw_fd(), FcntlArg::F_GET_SEALS)
            .map_err(|_| GuestMaterialError::MemfdFailed)?,
    );
    let required = SealFlag::F_SEAL_WRITE
        | SealFlag::F_SEAL_GROW
        | SealFlag::F_SEAL_SHRINK
        | SealFlag::F_SEAL_SEAL;
    let stat = fstat(fd.as_raw_fd()).map_err(|_| GuestMaterialError::MemfdFailed)?;
    if status & OFlag::O_ACCMODE != OFlag::O_RDONLY
        || !descriptor.contains(FdFlag::FD_CLOEXEC)
        || !seals.contains(required)
        || !SFlag::from_bits_truncate(stat.st_mode).contains(SFlag::S_IFREG)
    {
        return Err(GuestMaterialError::MemfdFailed);
    }
    Ok(())
}

fn response_attachment(
    context: &BrokerCallContext,
    index: u32,
    fd: OwnedFd,
) -> Result<OwnedAttachment, GuestMaterialError> {
    let descriptor = attachment_descriptor(
        crate::service_v2::BrokerMethod::Apply,
        context,
        index,
        crate::service_v2::DescriptorShape {
            object_type: KernelObjectType::Memfd,
            access: AttachmentAccess::ReadOnly,
        },
        AttachmentPurpose::ResponseOutput,
    )
    .map_err(|_| GuestMaterialError::MemfdFailed)?;
    let policy = DescriptorPolicy::File(
        ObjectIdentity::from_trusted(&fd, descriptor.object_type, descriptor.access)
            .map_err(|_| GuestMaterialError::MemfdFailed)?,
    );
    OwnedUnixAttachment::file(descriptor, fd, policy).map_err(|_| GuestMaterialError::MemfdFailed)
}

fn hex_digest(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::{
        io::Read,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use d2b_contracts::{
        v2_component_session::{
            GUEST_SESSION_CREDENTIAL_V1_WITH_BOOTSTRAP_BYTES, OperationId, RequestId,
            ServicePackage,
        },
        v2_services::{SERVICE_INVENTORY, common::IdentityScope},
    };
    use d2b_core::configured_argv::ConfiguredArgv;
    use d2b_realm_core::ProtocolToken;
    use d2b_session::RequestRegistry;
    use d2b_session_unix::UnixAttachmentPayload;

    use super::*;
    use crate::guest_material_authority::{
        BootstrapReplayKey, BootstrapReplayLedger, EnrollmentLedgerTransaction,
        InMemoryBootstrapReplayLedger, RealmGuestSessionAuthorityConnector,
    };
    use crate::service_v2::BrokerPeerRole;

    const GENERATION: u64 = 9;
    const NOW_MS: u64 = 10_000;
    const REALM: &str = "work";
    const WORKLOAD: &str = "editor";
    const OPERATION: &str = "guest-material-operation";
    const INVENTORY: &[u8] =
        br#"{"identity":"editor.work.d2b","items":[{"argv":["private-canary"]}]}"#;

    struct FixedClock;

    impl GuestMaterialClock for FixedClock {
        fn now_unix_ms(&self) -> u64 {
            NOW_MS
        }
    }

    #[derive(Default)]
    struct CommitFailLedger {
        rolled_back: Arc<AtomicBool>,
    }

    impl BootstrapReplayLedger for CommitFailLedger {
        fn is_consumed(&self, _: &BootstrapReplayKey) -> Result<bool, GuestMaterialError> {
            Ok(false)
        }

        fn stage_enrollment(
            &self,
            _: &BootstrapReplayKey,
            _: &str,
            _: &str,
            _: &[u8],
        ) -> Result<Box<dyn EnrollmentLedgerTransaction>, GuestMaterialError> {
            Ok(Box::new(CommitFailTransaction {
                rolled_back: Arc::clone(&self.rolled_back),
                active: true,
            }))
        }

        fn restore_enrollment(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Option<GuestSessionCredentialV1>, GuestMaterialError> {
            Ok(None)
        }
    }

    struct CommitFailTransaction {
        rolled_back: Arc<AtomicBool>,
        active: bool,
    }

    impl EnrollmentLedgerTransaction for CommitFailTransaction {
        fn commit(&mut self) -> Result<(), GuestMaterialError> {
            Err(GuestMaterialError::AuthorityUnavailable)
        }

        fn finalize(&mut self) {}

        fn rollback(&mut self) -> Result<(), GuestMaterialError> {
            if self.active {
                self.rolled_back.store(true, Ordering::SeqCst);
                self.active = false;
            }
            Ok(())
        }
    }

    impl Drop for CommitFailTransaction {
        fn drop(&mut self) {
            let _ = self.rollback();
        }
    }

    #[derive(Clone, Copy)]
    struct AuthorityTemplate {
        realm: &'static str,
        workload: &'static str,
        generation: u64,
        bootstrap: bool,
        delay_ms: u64,
    }

    struct TestAuthority {
        template: AuthorityTemplate,
        calls: AtomicUsize,
        source_zeroized: AtomicBool,
    }

    impl TestAuthority {
        fn new(template: AuthorityTemplate) -> Self {
            Self {
                template,
                calls: AtomicUsize::new(0),
                source_zeroized: AtomicBool::new(false),
            }
        }
    }

    #[async_trait]
    impl GuestSessionAuthorityPort for TestAuthority {
        async fn resolve(
            &self,
            _: GuestAuthorityLookup,
        ) -> Result<GuestSessionAuthority, GuestMaterialError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.template.delay_ms != 0 {
                tokio::time::sleep(Duration::from_millis(self.template.delay_ms)).await;
            }
            let bootstrap = if self.template.bootstrap {
                let mut source = [0x88; 32];
                let psk = GuestBootstrapPsk::copy_from_and_zeroize(&mut source)
                    .map_err(|_| GuestMaterialError::BootstrapInvalid)?;
                self.source_zeroized
                    .store(source == [0; 32], Ordering::SeqCst);
                Some(GuestBootstrapAuthority {
                    binding: bootstrap_binding(),
                    issued_at_unix_ms: NOW_MS,
                    psk,
                })
            } else {
                None
            };
            Ok(GuestSessionAuthority {
                realm_id: self.template.realm.to_owned(),
                workload_id: self.template.workload.to_owned(),
                session_generation: self.template.generation,
                parent_static_public_key: [0x11; 32],
                channel_binding: [0x22; 32],
                guest_identity_digest: [0x33; 32],
                guest_static_public_key: [0x44; 32],
                bootstrap,
            })
        }
    }

    struct ExcessLifetimeAuthority;

    #[async_trait]
    impl GuestSessionAuthorityPort for ExcessLifetimeAuthority {
        async fn resolve(
            &self,
            _: GuestAuthorityLookup,
        ) -> Result<GuestSessionAuthority, GuestMaterialError> {
            Ok(GuestSessionAuthority {
                realm_id: REALM.to_owned(),
                workload_id: WORKLOAD.to_owned(),
                session_generation: GENERATION,
                parent_static_public_key: [0x11; 32],
                channel_binding: [0x22; 32],
                guest_identity_digest: [0x33; 32],
                guest_static_public_key: [0x44; 32],
                bootstrap: Some(GuestBootstrapAuthority {
                    binding: excess_lifetime_binding(),
                    issued_at_unix_ms: NOW_MS,
                    psk: GuestBootstrapPsk::generate_with(|psk| {
                        psk.fill(0x88);
                        Ok(())
                    })
                    .unwrap(),
                }),
            })
        }
    }

    struct TestBundle {
        inventory: &'static [u8],
        reject_storage: bool,
        calls: AtomicUsize,
    }

    impl TestBundle {
        fn new() -> Self {
            Self {
                inventory: INVENTORY,
                reject_storage: false,
                calls: AtomicUsize::new(0),
            }
        }

        fn configured_digest(&self) -> [u8; 32] {
            self.configured_launches().sha256()
        }

        fn configured_launches(&self) -> GuestConfiguredLaunchesBytes {
            let entry = GuestConfiguredLaunchEntryV1::new(
                ProtocolToken::parse("editor").unwrap(),
                ConfiguredArgv::new(vec!["private-canary".to_owned()]).unwrap(),
                false,
            )
            .unwrap();
            GuestConfiguredLaunchesV1::new(
                V2RealmId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap(),
                V2WorkloadId::parse("bbbbbbbbbbbbbbbbbbba").unwrap(),
                Sha256::digest(self.inventory).into(),
                vec![entry],
            )
            .unwrap()
            .encode()
            .unwrap()
        }
    }

    impl GuestMaterialBundlePort for TestBundle {
        fn resolve(
            &self,
            storage_ref: &str,
            realm_id: &str,
            workload_id: &str,
        ) -> Result<GuestMaterialBundle, GuestMaterialError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let expected = format!("{GUEST_SESSION_STORAGE_PREFIX}{workload_id}");
            if self.reject_storage || storage_ref != expected {
                return Err(GuestMaterialError::StorageRefUnknown);
            }
            let root = PathBuf::from("broker-test-material")
                .join(realm_id)
                .join(workload_id);
            Ok(GuestMaterialBundle {
                session_target: GuestMaterialTarget {
                    storage_ref: expected,
                    path: root.join(GUEST_SESSION_CREDENTIAL_NAME),
                    owner_uid: 0,
                    owner_gid: 77,
                    mode: 0o440,
                },
                configured_launch_target: GuestMaterialTarget {
                    storage_ref: format!("{CONFIGURED_LAUNCH_STORAGE_PREFIX}{workload_id}"),
                    path: root.join(CONFIGURED_LAUNCH_CREDENTIAL_NAME),
                    owner_uid: 0,
                    owner_gid: 77,
                    mode: 0o440,
                },
                configured_launches: self.configured_launches(),
                configured_launch_digest: self.configured_digest(),
            })
        }
    }

    type MaterialDigestPair = ([u8; 32], [u8; 32]);
    type CommittedMaterial = Arc<Mutex<Vec<MaterialDigestPair>>>;

    struct AtomicTestStore {
        fail: AtomicBool,
        fail_commit: Arc<AtomicBool>,
        committed: CommittedMaterial,
        rolled_back: Arc<AtomicBool>,
    }

    impl AtomicTestStore {
        fn new() -> Self {
            Self {
                fail: AtomicBool::new(false),
                fail_commit: Arc::new(AtomicBool::new(false)),
                committed: Arc::new(Mutex::new(Vec::new())),
                rolled_back: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    impl GuestMaterialStore for AtomicTestStore {
        fn stage_pair(
            &self,
            _: &GuestMaterialTarget,
            session_bytes: &GuestSessionCredentialBytes,
            _: &GuestMaterialTarget,
            configured_bytes: &[u8],
        ) -> Result<Box<dyn GuestMaterialTransaction>, GuestMaterialError> {
            if self.fail.load(Ordering::SeqCst) {
                return Err(GuestMaterialError::MaterializationFailed);
            }
            Ok(Box::new(AtomicTestTransaction {
                committed: Arc::clone(&self.committed),
                digests: (
                    Sha256::digest(session_bytes.as_slice()).into(),
                    Sha256::digest(configured_bytes).into(),
                ),
                rolled_back: Arc::clone(&self.rolled_back),
                fail_commit: Arc::clone(&self.fail_commit),
                pending: true,
            }))
        }
    }

    struct AtomicTestTransaction {
        committed: CommittedMaterial,
        digests: MaterialDigestPair,
        rolled_back: Arc<AtomicBool>,
        fail_commit: Arc<AtomicBool>,
        pending: bool,
    }

    impl GuestMaterialTransaction for AtomicTestTransaction {
        fn commit(&mut self) -> Result<(), GuestMaterialError> {
            if self.fail_commit.load(Ordering::SeqCst) {
                return Err(GuestMaterialError::MaterializationFailed);
            }
            Ok(())
        }

        fn mark_committed(&mut self) -> Result<(), GuestMaterialError> {
            Ok(())
        }

        fn mark_audit_committed(&mut self) -> Result<(), GuestMaterialError> {
            Ok(())
        }

        fn finalize(&mut self) {
            if self.pending {
                self.committed.lock().unwrap().push(self.digests);
                self.pending = false;
            }
        }

        fn rollback(&mut self) -> Result<(), GuestMaterialError> {
            self.rolled_back.store(true, Ordering::SeqCst);
            self.pending = false;
            Ok(())
        }
    }

    #[derive(Default)]
    struct CapturingAudit {
        records: Mutex<Vec<GuestMaterialAuditRecord>>,
        fail: AtomicBool,
    }

    impl GuestMaterialAuditSink for CapturingAudit {
        fn record(&self, record: &GuestMaterialAuditRecord) -> Result<(), GuestMaterialError> {
            if self.fail.load(Ordering::SeqCst) {
                return Err(GuestMaterialError::AuditFailed);
            }
            self.records.lock().unwrap().push(record.clone());
            Ok(())
        }
    }

    fn bootstrap_binding() -> BootstrapPskBinding {
        BootstrapPskBinding {
            operation_id: OperationId::new(vec![0x66; 16]).unwrap(),
            replay_nonce: [0x77; 32],
            expires_at_unix_ms: NOW_MS + 5 * 60 * 1_000,
        }
    }

    fn excess_lifetime_binding() -> BootstrapPskBinding {
        BootstrapPskBinding {
            expires_at_unix_ms: NOW_MS + 5 * 60 * 1_000 + 1,
            ..bootstrap_binding()
        }
    }

    fn context(request_byte: u8) -> BrokerCallContext {
        let request_id = RequestId::new(vec![request_byte; 16]).unwrap();
        let cancellation = RequestRegistry::new(GENERATION)
            .unwrap()
            .register(request_id.clone())
            .unwrap();
        BrokerCallContext {
            peer_role: BrokerPeerRole::RealmController,
            request_id,
            session_generation: GENERATION,
            remaining: Duration::from_secs(2),
            cancellation,
        }
    }

    fn request(bundle: &TestBundle, bootstrap: bool) -> ServiceRequest {
        let binding = bootstrap.then(bootstrap_binding);
        request_with_binding(bundle, binding.as_ref())
    }

    fn request_with_binding(
        bundle: &TestBundle,
        bootstrap_binding: Option<&BootstrapPskBinding>,
    ) -> ServiceRequest {
        let _ = (bundle, bootstrap_binding);
        let session_ref = d2b_host::guest_runtime::guest_material_resource_id(WORKLOAD);
        let digest = guest_material_request_digest(GuestMaterialRequestDigestInput {
            realm_id: REALM,
            workload_id: WORKLOAD,
            operation_id: OPERATION,
            session_storage_ref: &session_ref,
            session_generation: GENERATION,
        });
        ServiceRequest {
            scope: protobuf::MessageField::some(IdentityScope {
                realm_id: REALM.to_owned(),
                workload_id: WORKLOAD.to_owned(),
                ..Default::default()
            }),
            resource_id: session_ref,
            operation_id: OPERATION.to_owned(),
            request_digest: digest.to_vec(),
            desired_state: DesiredState::DESIRED_STATE_PRESENT.into(),
            ..Default::default()
        }
    }

    fn enrollment_call(
        credential: &GuestSessionCredentialV1,
        context: &BrokerCallContext,
    ) -> (ServiceRequest, OwnedAttachment) {
        let encoded = credential.encode().unwrap();
        let credential_digest: [u8; 32] = Sha256::digest(encoded.as_slice()).into();
        let operation_id = "persist-enrollment";
        let resource_id = d2b_host::guest_runtime::guest_enrollment_resource_id(WORKLOAD);
        let request_digest = d2b_host::guest_runtime::guest_enrollment_apply_digest(
            d2b_host::guest_runtime::GuestEnrollmentApplyDigestInput {
                realm_id: REALM,
                workload_id: WORKLOAD,
                operation_id,
                enrollment_ref: &resource_id,
                session_generation: GENERATION,
                credential_digest: &credential_digest,
            },
        );
        let fd = sealed_read_only_memfd_bytes("enrollment-test", encoded.as_slice()).unwrap();
        let descriptor = attachment_descriptor(
            BrokerMethod::Apply,
            context,
            0,
            crate::service_v2::DescriptorShape {
                object_type: KernelObjectType::Memfd,
                access: AttachmentAccess::ReadOnly,
            },
            AttachmentPurpose::RequestInput,
        )
        .unwrap();
        let policy = DescriptorPolicy::File(
            ObjectIdentity::from_trusted(&fd, KernelObjectType::Memfd, AttachmentAccess::ReadOnly)
                .unwrap(),
        );
        let attachment = OwnedUnixAttachment::file(descriptor, fd, policy).unwrap();
        (
            ServiceRequest {
                scope: protobuf::MessageField::some(IdentityScope {
                    realm_id: REALM.to_owned(),
                    workload_id: WORKLOAD.to_owned(),
                    ..Default::default()
                }),
                resource_id,
                operation_id: operation_id.to_owned(),
                request_digest: request_digest.to_vec(),
                attachment_indexes: vec![0],
                desired_state: DesiredState::DESIRED_STATE_PRESENT.into(),
                ..Default::default()
            },
            attachment,
        )
    }

    fn broker(
        authority: Arc<TestAuthority>,
        bundle: Arc<TestBundle>,
        store: Arc<AtomicTestStore>,
        audit: Arc<CapturingAudit>,
    ) -> GuestSessionMaterialBroker {
        GuestSessionMaterialBroker::new(authority, bundle, store, audit, Arc::new(FixedClock))
    }

    fn attachment_bytes(attachment: &OwnedAttachment) -> Vec<u8> {
        let fd = attachment
            .payload()
            .and_then(|payload| payload.downcast_ref::<UnixAttachmentPayload>())
            .and_then(UnixAttachmentPayload::file)
            .expect("file attachment")
            .try_clone_to_owned()
            .expect("clone attachment");
        let mut bytes = Vec::new();
        File::from(fd).read_to_end(&mut bytes).unwrap();
        bytes
    }

    struct RejectingFallback;

    #[async_trait]
    impl BrokerRuntimeDispatch for RejectingFallback {
        async fn dispatch(
            &self,
            _: BrokerMethod,
            _: ServiceRequest,
            _: Vec<OwnedAttachment>,
            _: &BrokerCallContext,
        ) -> Result<BrokerReply<ServiceResponse>, BrokerServiceFailure> {
            Err(BrokerServiceFailure::InvalidRequest)
        }
    }

    #[tokio::test]
    async fn realm_bound_dispatch_accepts_authenticated_child_and_denies_local_root() {
        let connector = Arc::new(RealmGuestSessionAuthorityConnector::new(
            REALM.to_owned(),
            Arc::new(InMemoryBootstrapReplayLedger::default()),
        ));
        connector
            .install(GuestSessionAuthority {
                realm_id: REALM.to_owned(),
                workload_id: WORKLOAD.to_owned(),
                session_generation: GENERATION,
                parent_static_public_key: [0x11; 32],
                channel_binding: [0x22; 32],
                guest_identity_digest: [0x33; 32],
                guest_static_public_key: [0x44; 32],
                bootstrap: None,
            })
            .unwrap();
        let bundle = Arc::new(TestBundle::new());
        let material = Arc::new(GuestSessionMaterialBroker::new(
            Arc::clone(&connector) as Arc<dyn GuestSessionAuthorityPort>,
            Arc::clone(&bundle) as Arc<dyn GuestMaterialBundlePort>,
            Arc::new(AtomicTestStore::new()),
            Arc::new(CapturingAudit::default()),
            Arc::new(FixedClock),
        ));
        let dispatch = RealmBoundGuestMaterialDispatch::new(
            REALM.to_owned(),
            connector,
            material,
            RejectingFallback,
        )
        .unwrap();
        let response = dispatch
            .dispatch(
                BrokerMethod::Apply,
                request(&bundle, false),
                Vec::new(),
                &context(19),
            )
            .await
            .unwrap();
        assert_eq!(response.message.attachment_indexes, vec![0, 1]);

        let mut local_root = context(20);
        local_root.peer_role = BrokerPeerRole::LocalRootController;
        assert_eq!(
            dispatch
                .dispatch(
                    BrokerMethod::Apply,
                    request(&bundle, false),
                    Vec::new(),
                    &local_root,
                )
                .await
                .unwrap_err(),
            BrokerServiceFailure::PermissionDenied
        );
    }

    #[tokio::test]
    async fn realm_dispatch_persists_exact_enrolled_credential_attachment() {
        let connector = Arc::new(RealmGuestSessionAuthorityConnector::new(
            REALM.to_owned(),
            Arc::new(InMemoryBootstrapReplayLedger::default()),
        ));
        connector
            .install(GuestSessionAuthority {
                realm_id: REALM.to_owned(),
                workload_id: WORKLOAD.to_owned(),
                session_generation: GENERATION,
                parent_static_public_key: [0x11; 32],
                channel_binding: [0x22; 32],
                guest_identity_digest: [0; 32],
                guest_static_public_key: [0; 32],
                bootstrap: Some(GuestBootstrapAuthority {
                    binding: bootstrap_binding(),
                    issued_at_unix_ms: NOW_MS,
                    psk: GuestBootstrapPsk::generate_with(|psk| {
                        psk.fill(0x88);
                        Ok(())
                    })
                    .unwrap(),
                }),
            })
            .unwrap();
        let bundle = Arc::new(TestBundle::new());
        let material = Arc::new(GuestSessionMaterialBroker::new(
            Arc::clone(&connector) as Arc<dyn GuestSessionAuthorityPort>,
            Arc::clone(&bundle) as Arc<dyn GuestMaterialBundlePort>,
            Arc::new(AtomicTestStore::new()),
            Arc::new(CapturingAudit::default()),
            Arc::new(FixedClock),
        ));
        let dispatch = RealmBoundGuestMaterialDispatch::new(
            REALM.to_owned(),
            Arc::clone(&connector),
            material,
            RejectingFallback,
        )
        .unwrap();
        dispatch
            .dispatch(
                BrokerMethod::Apply,
                request(&bundle, true),
                Vec::new(),
                &context(30),
            )
            .await
            .unwrap();

        let guest_public = [0x44; 32];
        let guest_identity: [u8; 32] = Sha256::digest(guest_public).into();
        let credential = GuestSessionCredentialV1::new(
            GENERATION,
            [0x11; 32],
            [0x22; 32],
            GuestIdentityBindingV1::Enrolled {
                guest_identity_digest: guest_identity,
                guest_static_public_key: guest_public,
            },
            None,
        )
        .unwrap();
        let encoded = credential.encode().unwrap();
        let credential_digest: [u8; 32] = Sha256::digest(encoded.as_slice()).into();
        let operation_id = "persist-enrollment";
        let resource_id = d2b_host::guest_runtime::guest_enrollment_resource_id(WORKLOAD);
        let request_digest = d2b_host::guest_runtime::guest_enrollment_apply_digest(
            d2b_host::guest_runtime::GuestEnrollmentApplyDigestInput {
                realm_id: REALM,
                workload_id: WORKLOAD,
                operation_id,
                enrollment_ref: &resource_id,
                session_generation: GENERATION,
                credential_digest: &credential_digest,
            },
        );
        let persist_context = context(31);
        let fd = sealed_read_only_memfd_bytes("enrollment-test", encoded.as_slice()).unwrap();
        let descriptor = attachment_descriptor(
            BrokerMethod::Apply,
            &persist_context,
            0,
            crate::service_v2::DescriptorShape {
                object_type: KernelObjectType::Memfd,
                access: AttachmentAccess::ReadOnly,
            },
            AttachmentPurpose::RequestInput,
        )
        .unwrap();
        let policy = DescriptorPolicy::File(
            ObjectIdentity::from_trusted(&fd, KernelObjectType::Memfd, AttachmentAccess::ReadOnly)
                .unwrap(),
        );
        let attachment = OwnedUnixAttachment::file(descriptor, fd, policy).unwrap();
        let response = dispatch
            .dispatch(
                BrokerMethod::Apply,
                ServiceRequest {
                    scope: protobuf::MessageField::some(IdentityScope {
                        realm_id: REALM.to_owned(),
                        workload_id: WORKLOAD.to_owned(),
                        ..Default::default()
                    }),
                    resource_id,
                    operation_id: operation_id.to_owned(),
                    request_digest: request_digest.to_vec(),
                    attachment_indexes: vec![0],
                    desired_state: DesiredState::DESIRED_STATE_PRESENT.into(),
                    ..Default::default()
                },
                vec![attachment],
                &persist_context,
            )
            .await
            .unwrap();
        assert_eq!(
            response.message.outcome.enum_value().ok(),
            Some(Outcome::OUTCOME_SUCCEEDED)
        );
        let enrolled = connector
            .resolve(GuestAuthorityLookup {
                realm_id: REALM.to_owned(),
                workload_id: WORKLOAD.to_owned(),
                operation_id: "verify-enrollment".to_owned(),
                storage_ref: "verify".to_owned(),
                request_digest: [1; 32],
                session_generation: GENERATION,
            })
            .await
            .unwrap();
        assert_eq!(enrolled.guest_identity_digest, guest_identity);
        assert_eq!(enrolled.guest_static_public_key, guest_public);
        assert!(enrolled.bootstrap.is_none());
    }

    #[tokio::test]
    async fn enrollment_audit_failure_rolls_back_material_and_durable_ledger() {
        let ledger: Arc<dyn crate::guest_material_authority::BootstrapReplayLedger> =
            Arc::new(InMemoryBootstrapReplayLedger::default());
        let connector = Arc::new(RealmGuestSessionAuthorityConnector::new(
            REALM.to_owned(),
            Arc::clone(&ledger),
        ));
        let unbound_authority = || GuestSessionAuthority {
            realm_id: REALM.to_owned(),
            workload_id: WORKLOAD.to_owned(),
            session_generation: GENERATION,
            parent_static_public_key: [0x11; 32],
            channel_binding: [0x22; 32],
            guest_identity_digest: [0; 32],
            guest_static_public_key: [0; 32],
            bootstrap: Some(GuestBootstrapAuthority {
                binding: bootstrap_binding(),
                issued_at_unix_ms: NOW_MS,
                psk: GuestBootstrapPsk::generate_with(|psk| {
                    psk.fill(0x88);
                    Ok(())
                })
                .unwrap(),
            }),
        };
        connector.install(unbound_authority()).unwrap();
        let bundle = Arc::new(TestBundle::new());
        let store = Arc::new(AtomicTestStore::new());
        let audit = Arc::new(CapturingAudit::default());
        let material = Arc::new(GuestSessionMaterialBroker::new(
            Arc::clone(&connector) as Arc<dyn GuestSessionAuthorityPort>,
            Arc::clone(&bundle) as Arc<dyn GuestMaterialBundlePort>,
            Arc::clone(&store) as Arc<dyn GuestMaterialStore>,
            Arc::clone(&audit) as Arc<dyn GuestMaterialAuditSink>,
            Arc::new(FixedClock),
        ));
        let dispatch = RealmBoundGuestMaterialDispatch::new(
            REALM.to_owned(),
            Arc::clone(&connector),
            material,
            RejectingFallback,
        )
        .unwrap();
        dispatch
            .dispatch(
                BrokerMethod::Apply,
                request(&bundle, true),
                Vec::new(),
                &context(40),
            )
            .await
            .unwrap();
        audit.fail.store(true, Ordering::SeqCst);

        let guest_public = [0x44; 32];
        let credential = GuestSessionCredentialV1::new(
            GENERATION,
            [0x11; 32],
            [0x22; 32],
            GuestIdentityBindingV1::Enrolled {
                guest_identity_digest: Sha256::digest(guest_public).into(),
                guest_static_public_key: guest_public,
            },
            None,
        )
        .unwrap();
        let persist_context = context(41);
        let (request, attachment) = enrollment_call(&credential, &persist_context);
        assert_eq!(
            dispatch
                .dispatch(
                    BrokerMethod::Apply,
                    request,
                    vec![attachment],
                    &persist_context,
                )
                .await
                .unwrap_err(),
            BrokerServiceFailure::Backend
        );
        assert!(store.rolled_back.load(Ordering::SeqCst));
        assert!(
            ledger
                .restore_enrollment(REALM, WORKLOAD)
                .unwrap()
                .is_none()
        );
        {
            let records = audit.records.lock().unwrap();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].outcome, GuestMaterialOutcome::Succeeded);
        }

        let restored =
            RealmGuestSessionAuthorityConnector::new(REALM.to_owned(), Arc::clone(&ledger));
        restored.install(unbound_authority()).unwrap();
        assert!(
            restored
                .resolve(GuestAuthorityLookup {
                    realm_id: REALM.to_owned(),
                    workload_id: WORKLOAD.to_owned(),
                    operation_id: "retry-bootstrap".to_owned(),
                    storage_ref: "retry".to_owned(),
                    request_digest: [1; 32],
                    session_generation: GENERATION,
                })
                .await
                .unwrap()
                .bootstrap
                .is_some()
        );
    }

    #[tokio::test]
    async fn enrollment_ledger_commit_failure_rolls_back_material_and_identity() {
        let ledger = Arc::new(CommitFailLedger::default());
        let connector = Arc::new(RealmGuestSessionAuthorityConnector::new(
            REALM.to_owned(),
            Arc::clone(&ledger) as Arc<dyn BootstrapReplayLedger>,
        ));
        let unbound = || GuestSessionAuthority {
            realm_id: REALM.to_owned(),
            workload_id: WORKLOAD.to_owned(),
            session_generation: GENERATION,
            parent_static_public_key: [0x11; 32],
            channel_binding: [0x22; 32],
            guest_identity_digest: [0; 32],
            guest_static_public_key: [0; 32],
            bootstrap: Some(GuestBootstrapAuthority {
                binding: bootstrap_binding(),
                issued_at_unix_ms: NOW_MS,
                psk: GuestBootstrapPsk::generate_with(|psk| {
                    psk.fill(0x88);
                    Ok(())
                })
                .unwrap(),
            }),
        };
        connector.install(unbound()).unwrap();
        let bundle = Arc::new(TestBundle::new());
        let store = Arc::new(AtomicTestStore::new());
        let audit = Arc::new(CapturingAudit::default());
        let material = Arc::new(GuestSessionMaterialBroker::new(
            Arc::clone(&connector) as Arc<dyn GuestSessionAuthorityPort>,
            Arc::clone(&bundle) as Arc<dyn GuestMaterialBundlePort>,
            Arc::clone(&store) as Arc<dyn GuestMaterialStore>,
            Arc::clone(&audit) as Arc<dyn GuestMaterialAuditSink>,
            Arc::new(FixedClock),
        ));
        let dispatch = RealmBoundGuestMaterialDispatch::new(
            REALM.to_owned(),
            Arc::clone(&connector),
            material,
            RejectingFallback,
        )
        .unwrap();
        dispatch
            .dispatch(
                BrokerMethod::Apply,
                request(&bundle, true),
                Vec::new(),
                &context(50),
            )
            .await
            .unwrap();

        let guest_public = [0x44; 32];
        let credential = GuestSessionCredentialV1::new(
            GENERATION,
            [0x11; 32],
            [0x22; 32],
            GuestIdentityBindingV1::Enrolled {
                guest_identity_digest: Sha256::digest(guest_public).into(),
                guest_static_public_key: guest_public,
            },
            None,
        )
        .unwrap();
        let persist_context = context(51);
        let (request, attachment) = enrollment_call(&credential, &persist_context);
        assert_eq!(
            dispatch
                .dispatch(
                    BrokerMethod::Apply,
                    request,
                    vec![attachment],
                    &persist_context,
                )
                .await
                .unwrap_err(),
            BrokerServiceFailure::Backend
        );
        assert!(ledger.rolled_back.load(Ordering::SeqCst));
        assert!(store.rolled_back.load(Ordering::SeqCst));
        assert_eq!(store.committed.lock().unwrap().len(), 1);
        {
            let records = audit.records.lock().unwrap();
            assert_eq!(records.len(), 2);
            assert_eq!(records[0].outcome, GuestMaterialOutcome::Succeeded);
            assert_eq!(records[1].outcome, GuestMaterialOutcome::Failed);
            assert_eq!(
                records[1].error_kind,
                Some(GuestMaterialError::AuthorityUnavailable.as_str())
            );
        }
        let restored = RealmGuestSessionAuthorityConnector::new(
            REALM.to_owned(),
            Arc::clone(&ledger) as Arc<dyn BootstrapReplayLedger>,
        );
        restored.install(unbound()).unwrap();
        assert!(
            restored
                .resolve(GuestAuthorityLookup {
                    realm_id: REALM.to_owned(),
                    workload_id: WORKLOAD.to_owned(),
                    operation_id: "retry-after-ledger-failure".to_owned(),
                    storage_ref: "retry".to_owned(),
                    request_digest: [1; 32],
                    session_generation: GENERATION,
                })
                .await
                .unwrap()
                .bootstrap
                .is_some()
        );
    }

    #[tokio::test]
    async fn enrollment_pair_commit_failure_audits_only_failure_and_rolls_back_ledger() {
        let ledger: Arc<dyn BootstrapReplayLedger> =
            Arc::new(InMemoryBootstrapReplayLedger::default());
        let connector = Arc::new(RealmGuestSessionAuthorityConnector::new(
            REALM.to_owned(),
            Arc::clone(&ledger),
        ));
        connector
            .install(GuestSessionAuthority {
                realm_id: REALM.to_owned(),
                workload_id: WORKLOAD.to_owned(),
                session_generation: GENERATION,
                parent_static_public_key: [0x11; 32],
                channel_binding: [0x22; 32],
                guest_identity_digest: [0; 32],
                guest_static_public_key: [0; 32],
                bootstrap: Some(GuestBootstrapAuthority {
                    binding: bootstrap_binding(),
                    issued_at_unix_ms: NOW_MS,
                    psk: GuestBootstrapPsk::generate_with(|psk| {
                        psk.fill(0x88);
                        Ok(())
                    })
                    .unwrap(),
                }),
            })
            .unwrap();
        let bundle = Arc::new(TestBundle::new());
        let store = Arc::new(AtomicTestStore::new());
        let audit = Arc::new(CapturingAudit::default());
        let material = Arc::new(GuestSessionMaterialBroker::new(
            Arc::clone(&connector) as Arc<dyn GuestSessionAuthorityPort>,
            Arc::clone(&bundle) as Arc<dyn GuestMaterialBundlePort>,
            Arc::clone(&store) as Arc<dyn GuestMaterialStore>,
            Arc::clone(&audit) as Arc<dyn GuestMaterialAuditSink>,
            Arc::new(FixedClock),
        ));
        let dispatch = RealmBoundGuestMaterialDispatch::new(
            REALM.to_owned(),
            connector,
            material,
            RejectingFallback,
        )
        .unwrap();
        dispatch
            .dispatch(
                BrokerMethod::Apply,
                request(&bundle, true),
                Vec::new(),
                &context(60),
            )
            .await
            .unwrap();
        store.fail_commit.store(true, Ordering::SeqCst);

        let guest_public = [0x44; 32];
        let credential = GuestSessionCredentialV1::new(
            GENERATION,
            [0x11; 32],
            [0x22; 32],
            GuestIdentityBindingV1::Enrolled {
                guest_identity_digest: Sha256::digest(guest_public).into(),
                guest_static_public_key: guest_public,
            },
            None,
        )
        .unwrap();
        let persist_context = context(61);
        let (request, attachment) = enrollment_call(&credential, &persist_context);
        assert_eq!(
            dispatch
                .dispatch(
                    BrokerMethod::Apply,
                    request,
                    vec![attachment],
                    &persist_context,
                )
                .await
                .unwrap_err(),
            BrokerServiceFailure::Backend
        );
        assert!(store.rolled_back.load(Ordering::SeqCst));
        assert!(
            ledger
                .restore_enrollment(REALM, WORKLOAD)
                .unwrap()
                .is_none()
        );
        let records = audit.records.lock().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].outcome, GuestMaterialOutcome::Succeeded);
        assert_eq!(records[1].outcome, GuestMaterialOutcome::Failed);
        assert_eq!(
            records[1].error_kind,
            Some(GuestMaterialError::MaterializationFailed.as_str())
        );
    }

    #[tokio::test]
    async fn emits_shared_codec_bytes_and_bound_configured_pair() {
        let authority = Arc::new(TestAuthority::new(AuthorityTemplate {
            realm: REALM,
            workload: WORKLOAD,
            generation: GENERATION,
            bootstrap: true,
            delay_ms: 0,
        }));
        let bundle = Arc::new(TestBundle::new());
        let store = Arc::new(AtomicTestStore::new());
        let audit = Arc::new(CapturingAudit::default());
        let reply = broker(
            Arc::clone(&authority),
            Arc::clone(&bundle),
            Arc::clone(&store),
            Arc::clone(&audit),
        )
        .apply(request(&bundle, true), &context(1))
        .await
        .expect("material pair");

        assert_eq!(reply.message.attachment_indexes, vec![0, 1]);
        assert_eq!(reply.attachments.len(), 2);
        let session = attachment_bytes(&reply.attachments[0]);
        assert_eq!(
            session.len(),
            GUEST_SESSION_CREDENTIAL_V1_WITH_BOOTSTRAP_BYTES
        );
        let decoded = GuestSessionCredentialV1::decode(&session).unwrap();
        assert_eq!(decoded.session_generation(), GENERATION);
        assert_eq!(decoded.parent_static_public_key(), &[0x11; 32]);
        assert_eq!(decoded.channel_binding(), &[0x22; 32]);
        assert_eq!(decoded.guest_identity_digest(), Some(&[0x33; 32]));
        assert_eq!(decoded.guest_static_public_key(), Some(&[0x44; 32]));
        assert_eq!(decoded.bootstrap().unwrap().expose_psk(), &[0x88; 32]);

        let configured = attachment_bytes(&reply.attachments[1]);
        let configured_catalog = GuestConfiguredLaunchesV1::decode(&configured).unwrap();
        assert_eq!(
            configured_catalog.realm_id().as_str(),
            "aaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(
            configured_catalog.workload_id().as_str(),
            "bbbbbbbbbbbbbbbbbbba"
        );
        assert_eq!(
            configured_catalog
                .resolve_id("editor")
                .unwrap()
                .argv()
                .as_slice(),
            &["private-canary"]
        );
        assert_eq!(
            Sha256::digest(&configured).as_slice(),
            bundle.configured_digest().as_slice()
        );
        assert!(authority.source_zeroized.load(Ordering::SeqCst));
        assert_eq!(store.committed.lock().unwrap().len(), 1);
        let audits = audit.records.lock().unwrap();
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].outcome, GuestMaterialOutcome::Succeeded);
        assert_eq!(
            audits[0].credential_digest,
            Some(Sha256::digest(session).into())
        );
    }

    #[tokio::test]
    async fn response_memfds_are_read_only_cloexec_and_fully_sealed() {
        let authority = Arc::new(TestAuthority::new(AuthorityTemplate {
            realm: REALM,
            workload: WORKLOAD,
            generation: GENERATION,
            bootstrap: false,
            delay_ms: 0,
        }));
        let bundle = Arc::new(TestBundle::new());
        let reply = broker(
            authority,
            Arc::clone(&bundle),
            Arc::new(AtomicTestStore::new()),
            Arc::new(CapturingAudit::default()),
        )
        .apply(request(&bundle, false), &context(2))
        .await
        .unwrap();
        for attachment in &reply.attachments {
            let fd = attachment
                .payload()
                .and_then(|payload| payload.downcast_ref::<UnixAttachmentPayload>())
                .and_then(UnixAttachmentPayload::file)
                .unwrap();
            validate_sealed_read_only_fd(&fd).unwrap();
        }
    }

    #[tokio::test]
    async fn mismatches_storage_identity_generation_and_digest_fail_closed() {
        let bundle = Arc::new(TestBundle::new());
        let store = Arc::new(AtomicTestStore::new());

        let mismatched_realm = Arc::new(TestAuthority::new(AuthorityTemplate {
            realm: "personal",
            workload: WORKLOAD,
            generation: GENERATION,
            bootstrap: false,
            delay_ms: 0,
        }));
        assert_eq!(
            broker(
                mismatched_realm,
                Arc::clone(&bundle),
                Arc::clone(&store),
                Arc::new(CapturingAudit::default()),
            )
            .apply(request(&bundle, false), &context(3))
            .await
            .unwrap_err(),
            BrokerServiceFailure::InvalidRequest
        );

        let wrong_generation = Arc::new(TestAuthority::new(AuthorityTemplate {
            realm: REALM,
            workload: WORKLOAD,
            generation: GENERATION + 1,
            bootstrap: false,
            delay_ms: 0,
        }));
        assert_eq!(
            broker(
                wrong_generation,
                Arc::clone(&bundle),
                Arc::clone(&store),
                Arc::new(CapturingAudit::default()),
            )
            .apply(request(&bundle, false), &context(4))
            .await
            .unwrap_err(),
            BrokerServiceFailure::GenerationMismatch
        );

        let authority = Arc::new(TestAuthority::new(AuthorityTemplate {
            realm: REALM,
            workload: WORKLOAD,
            generation: GENERATION,
            bootstrap: false,
            delay_ms: 0,
        }));
        let mut local_root = context(5);
        local_root.peer_role = BrokerPeerRole::LocalRootController;
        assert_eq!(
            broker(
                Arc::clone(&authority),
                Arc::clone(&bundle),
                Arc::clone(&store),
                Arc::new(CapturingAudit::default()),
            )
            .apply(request(&bundle, false), &local_root)
            .await
            .unwrap_err(),
            BrokerServiceFailure::PermissionDenied
        );
        let mut tampered = request(&bundle, false);
        tampered.request_digest[0] ^= 0xff;
        assert_eq!(
            broker(
                authority,
                Arc::clone(&bundle),
                Arc::clone(&store),
                Arc::new(CapturingAudit::default()),
            )
            .apply(tampered, &context(14))
            .await
            .unwrap_err(),
            BrokerServiceFailure::InvalidRequest
        );
        assert!(store.committed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn pair_failure_commits_neither_material_and_audits_redacted_error() {
        let authority = Arc::new(TestAuthority::new(AuthorityTemplate {
            realm: REALM,
            workload: WORKLOAD,
            generation: GENERATION,
            bootstrap: true,
            delay_ms: 0,
        }));
        let bundle = Arc::new(TestBundle::new());
        let store = Arc::new(AtomicTestStore::new());
        store.fail.store(true, Ordering::SeqCst);
        let audit = Arc::new(CapturingAudit::default());
        let error = broker(
            Arc::clone(&authority),
            Arc::clone(&bundle),
            Arc::clone(&store),
            Arc::clone(&audit),
        )
        .apply(request(&bundle, true), &context(6))
        .await
        .unwrap_err();
        assert_eq!(error, BrokerServiceFailure::Backend);
        assert!(store.committed.lock().unwrap().is_empty());
        assert!(authority.source_zeroized.load(Ordering::SeqCst));
        let records = audit.records.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].outcome, GuestMaterialOutcome::Failed);
        let debug = format!("{records:?}");
        assert!(!debug.contains("private-canary"));
        assert!(!debug.contains("88888888"));
    }

    #[tokio::test]
    async fn mandatory_audit_failure_rolls_back_staged_pair() {
        let authority = Arc::new(TestAuthority::new(AuthorityTemplate {
            realm: REALM,
            workload: WORKLOAD,
            generation: GENERATION,
            bootstrap: false,
            delay_ms: 0,
        }));
        let bundle = Arc::new(TestBundle::new());
        let store = Arc::new(AtomicTestStore::new());
        let audit = Arc::new(CapturingAudit::default());
        audit.fail.store(true, Ordering::SeqCst);
        assert_eq!(
            broker(
                authority,
                Arc::clone(&bundle),
                Arc::clone(&store),
                Arc::clone(&audit),
            )
            .apply(request(&bundle, false), &context(16))
            .await
            .unwrap_err(),
            BrokerServiceFailure::Backend
        );
        assert!(store.committed.lock().unwrap().is_empty());
        assert!(store.rolled_back.load(Ordering::SeqCst));
        assert!(audit.records.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn bootstrap_lifetime_over_five_minutes_is_rejected() {
        let bundle = Arc::new(TestBundle::new());
        let service = GuestSessionMaterialBroker::new(
            Arc::new(ExcessLifetimeAuthority),
            Arc::clone(&bundle) as Arc<dyn GuestMaterialBundlePort>,
            Arc::new(AtomicTestStore::new()),
            Arc::new(CapturingAudit::default()),
            Arc::new(FixedClock),
        );
        assert_eq!(
            service
                .apply(
                    request_with_binding(&bundle, Some(&excess_lifetime_binding())),
                    &context(15),
                )
                .await
                .unwrap_err(),
            BrokerServiceFailure::InvalidRequest
        );
    }

    #[tokio::test]
    async fn bootstrap_is_one_time_while_enrolled_retry_is_idempotent() {
        let bundle = Arc::new(TestBundle::new());
        let bootstrap_authority = Arc::new(TestAuthority::new(AuthorityTemplate {
            realm: REALM,
            workload: WORKLOAD,
            generation: GENERATION,
            bootstrap: true,
            delay_ms: 0,
        }));
        let bootstrap_broker = broker(
            Arc::clone(&bootstrap_authority),
            Arc::clone(&bundle),
            Arc::new(AtomicTestStore::new()),
            Arc::new(CapturingAudit::default()),
        );
        bootstrap_broker
            .apply(request(&bundle, true), &context(7))
            .await
            .unwrap();
        assert_eq!(
            bootstrap_broker
                .apply(request(&bundle, true), &context(8))
                .await
                .unwrap_err(),
            BrokerServiceFailure::Conflict
        );
        assert_eq!(bootstrap_authority.calls.load(Ordering::SeqCst), 1);

        let enrolled_authority = Arc::new(TestAuthority::new(AuthorityTemplate {
            realm: REALM,
            workload: WORKLOAD,
            generation: GENERATION,
            bootstrap: false,
            delay_ms: 0,
        }));
        let enrolled_broker = broker(
            Arc::clone(&enrolled_authority),
            Arc::clone(&bundle),
            Arc::new(AtomicTestStore::new()),
            Arc::new(CapturingAudit::default()),
        );
        let first = enrolled_broker
            .apply(request(&bundle, false), &context(9))
            .await
            .unwrap();
        let second = enrolled_broker
            .apply(request(&bundle, false), &context(10))
            .await
            .unwrap();
        assert_eq!(first.message.result_digest, second.message.result_digest);
        assert_eq!(enrolled_authority.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn cancellation_deadline_and_unknown_storage_fail_without_fallback() {
        let bundle = Arc::new(TestBundle::new());
        let authority = Arc::new(TestAuthority::new(AuthorityTemplate {
            realm: REALM,
            workload: WORKLOAD,
            generation: GENERATION,
            bootstrap: false,
            delay_ms: 0,
        }));
        let service = broker(
            Arc::clone(&authority),
            Arc::clone(&bundle),
            Arc::new(AtomicTestStore::new()),
            Arc::new(CapturingAudit::default()),
        );
        let cancelled = context(11);
        assert!(cancelled.cancellation.cancel());
        assert_eq!(
            service
                .apply(request(&bundle, false), &cancelled)
                .await
                .unwrap_err(),
            BrokerServiceFailure::Cancelled
        );

        let delayed = Arc::new(TestAuthority::new(AuthorityTemplate {
            delay_ms: 50,
            ..authority.template
        }));
        let deadline_service = broker(
            delayed,
            Arc::clone(&bundle),
            Arc::new(AtomicTestStore::new()),
            Arc::new(CapturingAudit::default()),
        );
        let mut deadline = context(12);
        deadline.remaining = Duration::from_millis(1);
        assert_eq!(
            deadline_service
                .apply(request(&bundle, false), &deadline)
                .await
                .unwrap_err(),
            BrokerServiceFailure::DeadlineExceeded
        );

        let rejecting_bundle = Arc::new(TestBundle {
            reject_storage: true,
            ..TestBundle::new()
        });
        assert_eq!(
            broker(
                authority,
                Arc::clone(&rejecting_bundle),
                Arc::new(AtomicTestStore::new()),
                Arc::new(CapturingAudit::default()),
            )
            .apply(request(&rejecting_bundle, false), &context(13))
            .await
            .unwrap_err(),
            BrokerServiceFailure::NotFound
        );
    }

    #[tokio::test]
    async fn cancelled_handler_drop_terminally_records_replay_and_audits() {
        let authority = Arc::new(TestAuthority::new(AuthorityTemplate {
            realm: REALM,
            workload: WORKLOAD,
            generation: GENERATION,
            bootstrap: false,
            delay_ms: 100,
        }));
        let bundle = Arc::new(TestBundle::new());
        let audit = Arc::new(CapturingAudit::default());
        let service = Arc::new(broker(
            authority,
            Arc::clone(&bundle),
            Arc::new(AtomicTestStore::new()),
            Arc::clone(&audit),
        ));
        let call_context = context(17);
        let cancellation = call_context.cancellation.clone();
        let pending_service = Arc::clone(&service);
        let pending_request = request(&bundle, false);
        let operation =
            tokio::spawn(
                async move { pending_service.apply(pending_request, &call_context).await },
            );
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(cancellation.cancel());
        operation.abort();
        assert!(operation.await.unwrap_err().is_cancelled());

        assert_eq!(
            service
                .apply(request(&bundle, false), &context(18))
                .await
                .unwrap_err(),
            BrokerServiceFailure::Conflict
        );
        let records = audit.records.lock().unwrap();
        assert!(
            records.iter().any(|record| {
                record.error_kind == Some(GuestMaterialError::Cancelled.as_str())
            })
        );
    }

    #[test]
    fn registered_apply_method_id_and_debug_surfaces_are_stable() {
        let method_id = SERVICE_INVENTORY
            .iter()
            .find(|service| service.package == "d2b.broker.v2")
            .unwrap()
            .methods
            .iter()
            .find(|method| method.name == "Apply")
            .unwrap()
            .method_id("d2b.broker.v2", "BrokerService");
        assert_eq!(method_id, BROKER_APPLY_METHOD_ID);
        assert_eq!(ServicePackage::BrokerV2.as_str(), "d2b.broker.v2");

        let authority = GuestSessionAuthority {
            realm_id: "private-realm-canary".to_owned(),
            workload_id: "private-workload-canary".to_owned(),
            session_generation: GENERATION,
            parent_static_public_key: [0xaa; 32],
            channel_binding: [0xbb; 32],
            guest_identity_digest: [0xcc; 32],
            guest_static_public_key: [0xdd; 32],
            bootstrap: None,
        };
        let debug = format!("{authority:?}");
        assert!(!debug.contains("private-realm-canary"));
        assert!(!debug.contains("private-workload-canary"));
        assert!(!debug.contains("aaaaaaaa"));
        assert!(!debug.contains("bbbbbbbb"));
        assert!(!format!("{:?}", GuestMaterialError::AuthorityMismatch).contains("canary"));
    }
}
