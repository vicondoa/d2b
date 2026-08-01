use std::cell::RefCell;
use std::collections::BTreeMap;

use d2b_contracts::v3::execution_policy::BoundedToken;
use d2b_contracts::v3::provider::{
    ComponentDescriptor, ComponentStateKind, ComponentStateNamespace, ComponentStateView,
    MIN_COMPONENT_STATE_QUOTA_BYTES, ProviderContractError, StatePlacementMode,
    StateSchemaCustodyClass, StorageNeed,
};
use d2b_contracts::v3::volume::{AttachmentAccess, ViewRight, VolumeSpec};
use d2b_contracts::v3::{
    MigrationPolicy, PersistenceClass, ResourceRef, ResourceUid, SchemaFingerprint, SchemaVersion,
    SensitivityClass, StateDigest, StateEnvelope, VolumeStateError, VolumeStateSchemaId,
};
use d2b_provider_volume_local::atomic::{
    AtomicFilesystem, AtomicWrite, AtomicWriteError, CanonicalJson, WritePolicy, check_soft_quota,
};
use d2b_provider_volume_local::effect_port::{ExecutionDomain, VolumeEffectError, validate_domain};
use d2b_provider_volume_local::lock::{
    LockError, LockId, LockSet, LockSpec, LockTransferPolicy, OfdLockBackend, OfdLockHandle,
};
use d2b_provider_volume_local::marker::{
    MarkerBinding, MarkerError, MarkerStore, VerifiedMarkerFile, VolumeRootIdentity,
    provision_marker, verify_marker,
};
use d2b_provider_volume_local::path::{PathError, RelativePath};
use d2b_provider_volume_local::{VolumeLocalError, admit_attachments};
use serde_json::json;

const DIGEST: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

fn volume_uid() -> ResourceUid {
    ResourceUid::parse("6f9619ff-8b86-4d01-b42d-00cf4fc964ff").unwrap()
}

struct RecordingFilesystem {
    uid: ResourceUid,
    calls: RefCell<Vec<&'static str>>,
}

impl RecordingFilesystem {
    fn new() -> Self {
        Self {
            uid: volume_uid(),
            calls: RefCell::new(Vec::new()),
        }
    }

    fn record(&self, call: &'static str) {
        self.calls.borrow_mut().push(call);
    }
}

impl AtomicFilesystem for RecordingFilesystem {
    type Temp = Vec<u8>;

    fn resource_uid(&self) -> &ResourceUid {
        self.record("resource-uid");
        &self.uid
    }

    fn read_target(&mut self, _maximum: usize) -> Result<Vec<u8>, AtomicWriteError> {
        self.record("read-target");
        Ok(Vec::new())
    }

    fn current_generation(&mut self) -> Result<Option<u64>, AtomicWriteError> {
        self.record("current-generation");
        Ok(None)
    }

    fn current_charged_bytes(&mut self) -> Result<u64, AtomicWriteError> {
        self.record("current-charged-bytes");
        Ok(0)
    }

    fn current_target_bytes(&mut self) -> Result<u64, AtomicWriteError> {
        self.record("current-target-bytes");
        Ok(0)
    }

    fn create_temp(&mut self) -> Result<Self::Temp, AtomicWriteError> {
        self.record("create-temp");
        Ok(Vec::new())
    }

    fn write_temp(
        &mut self,
        temp: &mut Self::Temp,
        bytes: &[u8],
    ) -> Result<usize, AtomicWriteError> {
        self.record("write-temp");
        temp.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn sync_temp(&mut self, _temp: &mut Self::Temp) -> Result<(), AtomicWriteError> {
        self.record("sync-temp");
        Ok(())
    }

    fn replace_temp(&mut self, _temp: &mut Self::Temp) -> Result<(), AtomicWriteError> {
        self.record("replace-temp");
        Ok(())
    }

    fn sync_parent(&mut self) -> Result<(), AtomicWriteError> {
        self.record("sync-parent");
        Ok(())
    }

    fn remove_temp(&mut self, _temp: &mut Self::Temp) {
        self.record("remove-temp");
    }
}

struct NoopLockHandle;

impl OfdLockHandle for NoopLockHandle {
    fn release(&mut self) -> Result<(), LockError> {
        Ok(())
    }

    fn commit_transfer(&mut self) -> Result<(), LockError> {
        Ok(())
    }
}

struct NoopLockBackend;

impl OfdLockBackend for NoopLockBackend {
    fn acquire(&self, _spec: &LockSpec) -> Result<Box<dyn OfdLockHandle>, LockError> {
        Ok(Box::new(NoopLockHandle))
    }
}

fn lock_spec(id: &str, order: u32, acquire_after: Vec<LockId>) -> LockSpec {
    LockSpec::new(
        LockId::parse(id).unwrap(),
        volume_uid(),
        order,
        acquire_after,
        500,
        LockTransferPolicy::Never,
    )
    .unwrap()
}

#[derive(Default)]
struct MemoryMarkerStore {
    marker: Option<Vec<u8>>,
}

impl MarkerStore for MemoryMarkerStore {
    fn read_marker(
        &mut self,
        _volume_uid: &ResourceUid,
    ) -> Result<Option<VerifiedMarkerFile>, MarkerError> {
        Ok(self
            .marker
            .clone()
            .map(VerifiedMarkerFile::from_verified_regular_file))
    }

    fn create_marker_exclusive(
        &mut self,
        _volume_uid: &ResourceUid,
        bytes: &[u8],
    ) -> Result<(), MarkerError> {
        if self.marker.is_some() {
            return Err(MarkerError::MarkerWriteFailed);
        }
        self.marker = Some(bytes.to_vec());
        Ok(())
    }
}

fn marker_binding(root: VolumeRootIdentity) -> MarkerBinding {
    MarkerBinding::new(
        volume_uid(),
        root,
        VolumeStateSchemaId::parse("example-provider.d2bus.org/controller/main-state").unwrap(),
        SchemaVersion::new(1, 0).unwrap(),
        SchemaFingerprint::parse(DIGEST).unwrap(),
    )
}

fn state_namespace(
    placement: StatePlacementMode,
    host_custody_permitted: bool,
) -> Result<ComponentStateNamespace, ProviderContractError> {
    ComponentStateNamespace::new(
        BoundedToken::parse("main-state").unwrap(),
        ComponentStateKind::State,
        VolumeStateSchemaId::parse("example-provider.d2bus.org/controller/main-state").unwrap(),
        SchemaVersion::new(1, 0).unwrap(),
        SchemaFingerprint::parse(DIGEST).unwrap(),
        PersistenceClass::Persistent,
        SensitivityClass::Private,
        MigrationPolicy::PreLaunchRequired,
        MIN_COMPONENT_STATE_QUOTA_BYTES,
        Some(StorageNeed::Secret),
        true,
        Some(placement),
        host_custody_permitted,
        BTreeMap::from([(
            "main".to_owned(),
            ComponentStateView::new(
                "",
                vec![
                    ViewRight::Read,
                    ViewRight::Write,
                    ViewRight::Create,
                    ViewRight::Delete,
                    ViewRight::Traverse,
                ],
            )
            .unwrap(),
        )]),
    )
}

#[test]
fn unavailable_digest_domain_blocks_every_filesystem_effect() {
    let envelope = StateEnvelope::new(
        1,
        StateDigest::parse(DIGEST).unwrap(),
        json!({"private": "state-canary"}),
    )
    .unwrap();
    let first = lock_spec("state", 10, Vec::new());
    let mut locks = LockSet::new();
    let guard = locks.acquire(&NoopLockBackend, &first).unwrap();
    let mut writer = AtomicWrite::new(RecordingFilesystem::new());

    assert_eq!(
        writer.write(
            &envelope,
            WritePolicy {
                expected_previous: None,
                quota_bytes: Some(4096),
            },
            guard,
        ),
        Err(AtomicWriteError::StateContract(
            VolumeStateError::DigestDomainUnavailable
        ))
    );
    let filesystem = writer.into_inner();
    assert!(
        filesystem.calls.borrow().is_empty(),
        "digest rejection must precede filesystem observation"
    );
}

#[test]
fn state_generation_canonical_json_and_quota_bounds_reject_invalid_inputs() {
    assert_eq!(
        StateEnvelope::new(
            0,
            StateDigest::parse(DIGEST).unwrap(),
            json!({"ready": true}),
        ),
        Err(VolumeStateError::InvalidGeneration)
    );
    assert_eq!(
        CanonicalJson::decode::<serde_json::Value>(br#"{ "ready": true }"#),
        Err(AtomicWriteError::NonCanonical)
    );
    assert!(check_soft_quota(8192, 4096, 4096, 8192).is_ok());
    assert_eq!(
        check_soft_quota(8192, 4096, 4097, 8192),
        Err(AtomicWriteError::QuotaExceeded)
    );
}

#[test]
fn anchored_paths_reject_escape_and_ambiguous_separator_forms() {
    for candidate in [
        "",
        "/state",
        "state/",
        "state//data",
        "../state",
        "state/..",
        "state\\data",
        "state/data name",
    ] {
        assert!(
            RelativePath::parse(candidate).is_err(),
            "accepted invalid anchored path"
        );
    }
    assert_eq!(
        RelativePath::from_components(Vec::<String>::new()),
        Err(PathError::EmptyPath)
    );
    let path = RelativePath::parse("state/public").unwrap();
    assert_eq!(path.components().len(), 2);
    assert_eq!(path.leaf().as_str(), "public");
}

#[test]
fn lock_set_rejects_missing_dependencies_and_nonincreasing_order() {
    let first = lock_spec("first", 10, Vec::new());
    let second = lock_spec("second", 20, vec![first.lock_id().clone()]);
    let mut locks = LockSet::new();

    assert_eq!(
        locks.acquire(&NoopLockBackend, &second).unwrap_err(),
        LockError::DependencyMissing
    );
    locks.acquire(&NoopLockBackend, &first).unwrap();
    locks.acquire(&NoopLockBackend, &second).unwrap();
    assert_eq!(
        locks
            .acquire(&NoopLockBackend, &lock_spec("lower", 15, Vec::new()))
            .unwrap_err(),
        LockError::OrderViolation
    );
}

#[test]
fn marker_evidence_outliving_its_root_refuses_reprovision() {
    let root = VolumeRootIdentity {
        device: 31,
        inode: 47,
    };
    let binding = marker_binding(root);
    let mut store = MemoryMarkerStore::default();
    provision_marker(&mut store, &binding).unwrap();

    assert_eq!(
        verify_marker(&mut store, None, &binding),
        Err(MarkerError::PreviouslyProvisionedStateMissing)
    );
    assert!(store.marker.is_some(), "marker evidence must be preserved");
}

#[test]
fn stateless_component_round_trip_declares_no_state_volume() {
    let descriptor: ComponentDescriptor = serde_json::from_value(json!({
        "componentId": "volume-controller",
        "componentType": "controller",
        "exportedResourceTypes": ["Volume"],
        "exportedMethods": ["assess-update"],
        "allowedDomains": ["system"],
        "cardinality": 1,
        "configDigest": DIGEST,
        "dependencies": [],
        "declaresStateVolume": false,
        "stateNamespaces": [],
    }))
    .unwrap();

    let encoded = serde_json::to_vec(&descriptor).unwrap();
    let decoded: ComponentDescriptor = serde_json::from_slice(&encoded).unwrap();
    assert!(!decoded.declares_state_volume());
    assert!(decoded.state_namespaces().is_empty());
}

#[test]
fn guest_local_projection_keeps_source_in_guest_and_creates_no_export() {
    let namespace = state_namespace(StatePlacementMode::GuestLocal, false).unwrap();
    let guest = ResourceRef::parse("Guest/work-vm").unwrap();
    let projection = namespace.project_volume(&guest, None, 2, 4096).unwrap();

    assert_eq!(projection.source_execution_ref(), &guest);
    assert_eq!(projection.export_count(), 0);
    assert_eq!(
        projection.quota_max_bytes(),
        MIN_COMPONENT_STATE_QUOTA_BYTES
    );
    assert_eq!(projection.quota_max_inodes(), 4096);
    assert!(!format!("{projection:?}").contains("work-vm"));
}

#[test]
fn host_backed_projection_requires_custody_and_exports_each_attachment() {
    assert_eq!(
        state_namespace(StatePlacementMode::HostBackedGuest, false),
        Err(ProviderContractError::PlacementHostCustodyViolation)
    );

    let namespace = state_namespace(StatePlacementMode::HostBackedGuest, true).unwrap();
    let guest = ResourceRef::parse("Guest/work-vm").unwrap();
    let host = ResourceRef::parse("Host/host-system").unwrap();
    let projection = namespace
        .project_volume(&guest, Some(&host), 2, 4096)
        .unwrap();
    assert_eq!(projection.source_execution_ref(), &host);
    assert_eq!(projection.export_count(), 2);
}

#[test]
fn protected_schema_classes_reject_host_backed_guest_custody() {
    let namespace = state_namespace(StatePlacementMode::HostBackedGuest, true).unwrap();
    for class in [
        StateSchemaCustodyClass::Credential,
        StateSchemaCustodyClass::Audit,
        StateSchemaCustodyClass::RemoteNode,
        StateSchemaCustodyClass::CloudControl,
    ] {
        assert_eq!(
            namespace.validate_schema_custody(class),
            Err(ProviderContractError::GuestLocalRequired)
        );
    }
    assert!(
        namespace
            .validate_schema_custody(StateSchemaCustodyClass::Ordinary)
            .is_ok()
    );
}

#[test]
fn guest_local_domain_rejects_host_and_other_guest_observation() {
    let local = ExecutionDomain::Guest(BoundedToken::parse("work-vm").unwrap());
    let same_guest = ExecutionDomain::Guest(BoundedToken::parse("work-vm").unwrap());
    assert!(validate_domain(&local, &same_guest).is_ok());
    assert_eq!(
        validate_domain(
            &local,
            &ExecutionDomain::Host(BoundedToken::parse("host-system").unwrap())
        ),
        Err(VolumeEffectError::DomainMismatch)
    );
    assert_eq!(
        validate_domain(
            &local,
            &ExecutionDomain::Guest(BoundedToken::parse("personal-vm").unwrap())
        ),
        Err(VolumeEffectError::DomainMismatch)
    );
}

#[test]
fn shipped_provider_rejects_shared_write_state_attachment() {
    let spec: VolumeSpec = serde_json::from_value(json!({
        "source": {
            "executionRef": "Host/host-system",
            "settings": { "kind": "local-path", "sourcePolicyId": "state-root" }
        },
        "kind": "state",
        "layout": [],
        "views": {
            "controller": {
                "path": "",
                "rights": ["read", "write", "create", "delete", "traverse"]
            }
        },
        "attachments": [
            {
                "executionRef": "Guest/work-vm",
                "transport": "virtiofs",
                "view": "controller",
                "access": "read-write",
                "mountPath": "/state"
            },
            {
                "executionRef": "Guest/personal-vm",
                "transport": "virtiofs",
                "view": "controller",
                "access": "shared-write",
                "mountPath": "/state"
            }
        ]
    }))
    .unwrap();

    assert_eq!(
        admit_attachments(&spec, false),
        Err(VolumeLocalError::SharedWriteUnsupported)
    );
    assert_eq!(spec.attachments()[0].access(), AttachmentAccess::ReadWrite);
}
