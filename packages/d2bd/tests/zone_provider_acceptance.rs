//! Hermetic Zone acceptance for the four Wave 6 resource boundaries.
//!
//! The ports in this file are small real adapters: they persist their
//! effects below a temporary directory or own a real child process. They do
//! not record expected calls or bypass the Provider controllers.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::FileTypeExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use d2b_contracts::v3::{
    ResourceBundleGenerationId, ResourceGeneration, ResourceRef, ResourceUid,
    execution_policy::BoundedToken,
    guest::GuestSpec,
    network::{
        AttachmentGenerationFence, AttachmentHandle, DhcpSpec, DnsSpec, Ipv4Cidr, IsolationSpec,
        MdnsSpec, NetworkSpec, RoutingSpec,
    },
    process::ProcessSpec,
    volume::{EntryType, VolumeSpec},
};
use d2b_provider_device_tpm::{
    TpmResourceController, TpmResourceEffectError, TpmResourceEffectPort, TpmResourceOutcome,
};
use d2b_provider_network_local::{
    artifact::{ArtifactCatalogEntry, ArtifactKind},
    controller::{
        AttachmentRealization, FinalizerStage, FirewallDigest, FirewallIntent,
        NetworkConfigContent, NetworkEffectError, NetworkEffectPort, NetworkReconciler,
        NetworkResourcePort, ReconcileInput, ReconcileProgress,
    },
};
use d2b_provider_runtime_cloud_hypervisor::{
    CloudHypervisorClock, CloudHypervisorConfig, CloudHypervisorController,
    CloudHypervisorEffectPort, CloudHypervisorError, CloudHypervisorGuestSettings,
    CloudHypervisorPhase, CloudHypervisorReconcileOutcome, ConsoleType, GuestControlHealth,
    GuestControlProbe,
    adoption::ProcessIdentity,
    bootstrap_graph::{AttachmentRef, BootstrapGraph},
    health::GuestControlHealthError,
};
use d2b_provider_volume_local::{
    DriftClass, MarkerState, OwnerProof, QuotaCapability, VolumeLayoutEffectPort,
    VolumeLocalController, VolumeLocalProfile, VolumeRootHandle, VolumeSourceEffectPort,
};
use d2bd::resource_operator_activation::{
    Wave6BoundaryError, Wave6Dependencies, Wave6ProviderBoundary, Wave6ReconcileResult,
    Wave6Resource, Wave6ResourceSet,
};
use serde_json::json;

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    let waker = std::task::Waker::noop();
    let mut context = std::task::Context::from_waker(waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(value) => return value,
            std::task::Poll::Pending => std::thread::yield_now(),
        }
    }
}

struct FilesystemVolume {
    root: PathBuf,
    marker: PathBuf,
}

impl FilesystemVolume {
    fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            marker: root.join(".d2b-provisioned"),
            root,
        }
    }

    fn entry_path(&self, path: &str) -> PathBuf {
        if path.is_empty() {
            self.root.clone()
        } else {
            self.root.join(path)
        }
    }

    fn ensure_parent(path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(())
    }

    fn provision(&self, entry: &d2b_provider_volume_local::EntryRequest) -> io::Result<()> {
        let path = self.entry_path(entry.declared().path());
        Self::ensure_parent(&path)?;
        match entry.entry_type() {
            EntryType::Directory => fs::create_dir_all(path),
            EntryType::File => File::create(path).map(|_| ()),
            EntryType::Symlink => {
                let target = entry.declared().target().unwrap_or("target");
                std::os::unix::fs::symlink(target, path)
            }
            EntryType::UnixSocket => Ok(()),
        }
    }

    fn remove(&self, entry: &d2b_provider_volume_local::EntryRequest) -> io::Result<()> {
        let path = self.entry_path(entry.declared().path());
        if !path.exists() {
            return Ok(());
        }
        match entry.entry_type() {
            EntryType::Directory => fs::remove_dir(path),
            EntryType::File | EntryType::Symlink | EntryType::UnixSocket => fs::remove_file(path),
        }
    }

    fn matches_type(path: &Path, entry_type: EntryType) -> io::Result<bool> {
        let metadata = fs::symlink_metadata(path)?;
        Ok(match entry_type {
            EntryType::Directory => metadata.is_dir(),
            EntryType::File => metadata.is_file(),
            EntryType::Symlink => metadata.file_type().is_symlink(),
            EntryType::UnixSocket => metadata.file_type().is_socket(),
        })
    }
}

impl VolumeSourceEffectPort for &FilesystemVolume {
    async fn resolve_root(
        &self,
        _source_policy_id: Option<&BoundedToken>,
        _kind: d2b_contracts::v3::volume::SourceKind,
    ) -> Result<VolumeRootHandle, d2b_provider_volume_local::VolumeLocalError> {
        fs::create_dir_all(&self.root)
            .map_err(|_| d2b_provider_volume_local::VolumeLocalError::EffectFailed)?;
        Ok(VolumeRootHandle::held())
    }

    async fn quota_capability(
        &self,
        _root: &VolumeRootHandle,
    ) -> Result<QuotaCapability, d2b_provider_volume_local::VolumeLocalError> {
        Ok(QuotaCapability::Enforceable)
    }
}

impl VolumeLayoutEffectPort for &FilesystemVolume {
    async fn observe(
        &self,
        _root: &VolumeRootHandle,
        entry: &d2b_provider_volume_local::EntryRequest,
    ) -> Result<d2b_provider_volume_local::ObservedEntry, d2b_provider_volume_local::VolumeLocalError>
    {
        let path = self.entry_path(entry.declared().path());
        match FilesystemVolume::matches_type(&path, entry.entry_type()) {
            Ok(true) => Ok(d2b_provider_volume_local::ObservedEntry::conformant(
                OwnerProof::NotApplicable,
            )),
            Ok(false) => Ok(d2b_provider_volume_local::ObservedEntry {
                present: true,
                drift: [DriftClass::EntryType].into_iter().collect(),
                symlink_encountered: false,
                foreign_children: false,
                owner_proof: OwnerProof::NotApplicable,
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(d2b_provider_volume_local::ObservedEntry::absent())
            }
            Err(_) => Err(d2b_provider_volume_local::VolumeLocalError::EffectFailed),
        }
    }

    async fn provision(
        &self,
        _root: &VolumeRootHandle,
        entry: &d2b_provider_volume_local::EntryRequest,
    ) -> Result<(), d2b_provider_volume_local::VolumeLocalError> {
        FilesystemVolume::provision(self, entry)
            .map_err(|_| d2b_provider_volume_local::VolumeLocalError::EffectFailed)
    }

    async fn repair(
        &self,
        _root: &VolumeRootHandle,
        entry: &d2b_provider_volume_local::EntryRequest,
        _drift: &std::collections::BTreeSet<DriftClass>,
    ) -> Result<(), d2b_provider_volume_local::VolumeLocalError> {
        FilesystemVolume::remove(self, entry)
            .and_then(|_| FilesystemVolume::provision(self, entry))
            .map_err(|_| d2b_provider_volume_local::VolumeLocalError::EffectFailed)
    }

    async fn apply_acl(
        &self,
        _root: &VolumeRootHandle,
        _entry: &d2b_provider_volume_local::EntryRequest,
    ) -> Result<(), d2b_provider_volume_local::VolumeLocalError> {
        Ok(())
    }

    async fn cleanup(
        &self,
        _root: &VolumeRootHandle,
        entry: &d2b_provider_volume_local::EntryRequest,
    ) -> Result<(), d2b_provider_volume_local::VolumeLocalError> {
        FilesystemVolume::remove(self, entry)
            .map_err(|_| d2b_provider_volume_local::VolumeLocalError::EffectFailed)
    }

    async fn marker_state(
        &self,
        _root: &VolumeRootHandle,
    ) -> Result<MarkerState, d2b_provider_volume_local::VolumeLocalError> {
        if self.marker.exists() {
            Ok(MarkerState::Provisioned)
        } else {
            File::create(&self.marker)
                .and_then(|file| file.sync_all())
                .map_err(|_| d2b_provider_volume_local::VolumeLocalError::EffectFailed)?;
            Ok(MarkerState::NeverProvisioned)
        }
    }
}

pub fn volume_spec() -> VolumeSpec {
    serde_json::from_value(json!({
        "source": {
            "executionRef": "Host/host-system",
            "settings": {
                "kind": "local-path",
                "sourcePolicyId": "zone-state"
            }
        },
        "kind": "state",
        "layout": [
            {
                "path": "",
                "type": "directory",
                "ownerRef": "User/d2bd",
                "groupRef": "User/d2bd",
                "mode": "0700",
                "cleanupPolicy": "never"
            },
            {
                "path": "state.db",
                "type": "file",
                "ownerRef": "User/d2bd",
                "groupRef": "User/d2bd",
                "mode": "0600",
                "cleanupPolicy": "boot"
            }
        ],
        "views": {
            "controller": {
                "path": "",
                "rights": ["read", "write", "create", "delete", "traverse"]
            }
        }
    }))
    .expect("valid Volume acceptance fixture")
}

#[test]
fn volume_zone_activation_ready_restart_and_cleanup_use_real_files() {
    let directory = tempfile::tempdir().expect("Volume backing directory");
    let volume = FilesystemVolume::new(directory.path());
    let uid = ResourceUid::parse("6f9619ff-8b86-4d01-b42d-00cf4fc964ff").unwrap();
    let spec = volume_spec();
    let controller = VolumeLocalController::new(VolumeLocalProfile::shipped(), &volume, &volume);

    let first = block_on(controller.reconcile(&uid, &spec)).expect("initial Volume reconcile");
    assert_eq!(
        first.layout_phase,
        d2b_provider_volume_local::LayoutPhase::Ready
    );
    assert!(directory.path().join("state.db").is_file());

    let restarted = VolumeLocalController::new(VolumeLocalProfile::shipped(), &volume, &volume);
    let adopted = block_on(restarted.reconcile(&uid, &spec)).expect("restart Volume reconcile");
    assert_eq!(
        adopted.layout_phase,
        d2b_provider_volume_local::LayoutPhase::Ready
    );
    assert!(directory.path().join("state.db").is_file());

    let removed = block_on(restarted.cleanup(&uid, &spec)).expect("Volume finalization");
    assert_eq!(removed.len(), 1);
    assert!(!directory.path().join("state.db").exists());
    assert!(directory.path().is_dir(), "never-cleanup root is retained");
}

struct FilesystemNetworkBoundary {
    root: PathBuf,
}

impl FilesystemNetworkBoundary {
    fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn event(&self, name: &str) -> Result<(), NetworkEffectError> {
        fs::create_dir_all(&self.root).map_err(|_| NetworkEffectError::Transient)?;
        let path = self.root.join("events.log");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|_| NetworkEffectError::Transient)?;
        writeln!(file, "{name}").map_err(|_| NetworkEffectError::Transient)?;
        file.sync_all().map_err(|_| NetworkEffectError::Transient)
    }

    fn events(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("events.log"))
            .unwrap_or_default()
            .lines()
            .map(ToOwned::to_owned)
            .collect()
    }
}

impl NetworkEffectPort for &FilesystemNetworkBoundary {
    async fn validate_policy(&self, spec: &NetworkSpec) -> Result<(), NetworkEffectError> {
        if spec.isolation().allow_east_west {
            Err(NetworkEffectError::EastWestHostOptInRequired)
        } else {
            Ok(())
        }
    }

    async fn create_bridges(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        fs::create_dir_all(self.root.join("bridges"))
            .map_err(|_| NetworkEffectError::BridgeCreate)?;
        self.event("bridges")
    }

    async fn apply_sysctls(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.event("sysctls")
    }

    async fn apply_host_firewall(
        &self,
        intent: &FirewallIntent,
    ) -> Result<FirewallDigest, NetworkEffectError> {
        fs::write(
            self.root.join("firewall-generation"),
            intent.expected_generation_id().as_str(),
        )
        .map_err(|_| NetworkEffectError::Transient)?;
        self.event("firewall-apply")?;
        Ok(FirewallDigest::new([1; 32]))
    }

    async fn remove_host_firewall(&self, _: &FirewallIntent) -> Result<(), NetworkEffectError> {
        let _ = fs::remove_file(self.root.join("firewall-generation"));
        self.event("firewall-remove")
    }

    async fn apply_nm_unmanaged(&self) -> Result<(), NetworkEffectError> {
        self.event("nm")
    }

    async fn apply_routes(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.event("routes")
    }

    async fn remove_routes(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.event("routes-remove")
    }

    async fn update_hosts(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.event("hosts")
    }

    async fn seed_dhcp(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.event("dhcp")
    }

    async fn delete_persistent_tap(
        &self,
        _: &AttachmentHandle,
        _: &AttachmentGenerationFence,
    ) -> Result<(), NetworkEffectError> {
        self.event("tap-delete")
    }

    async fn delete_bridges(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        let _ = fs::remove_dir(self.root.join("bridges"));
        self.event("bridge-delete")
    }
}

impl NetworkResourcePort for &FilesystemNetworkBoundary {
    async fn upsert_volume_backing(
        &self,
        _: &d2b_contracts::v3::volume::VolumeSpec,
    ) -> Result<(), NetworkEffectError> {
        self.event("volume-upsert")
    }

    async fn write_volume_content(
        &self,
        content: &NetworkConfigContent,
    ) -> Result<(), NetworkEffectError> {
        fs::write(self.root.join("dnsmasq.conf"), &content.dnsmasq)
            .and_then(|_| fs::write(self.root.join("nftables.conf"), &content.nftables))
            .and_then(|_| fs::write(self.root.join("routing.conf"), &content.routing))
            .and_then(|_| fs::write(self.root.join("attachments.conf"), &content.attachments))
            .map_err(|_| NetworkEffectError::Transient)?;
        self.event("volume-write")
    }

    async fn upsert_guest(&self, _: &GuestSpec) -> Result<(), NetworkEffectError> {
        self.event("guest-upsert")
    }

    async fn attach_volume(
        &self,
        _: &d2b_contracts::v3::volume::VolumeAttachment,
    ) -> Result<(), NetworkEffectError> {
        self.event("volume-attach")
    }

    async fn upsert_agent(&self, _: &ProcessSpec) -> Result<(), NetworkEffectError> {
        self.event("agent-upsert")
    }

    async fn reconcile_mdns(&self, _: bool) -> Result<(), NetworkEffectError> {
        self.event("mdns")
    }

    async fn delete_processes(&self) -> Result<(), NetworkEffectError> {
        self.event("process-delete")
    }

    async fn detach_volume(&self) -> Result<(), NetworkEffectError> {
        self.event("volume-detach")
    }

    async fn delete_guest(&self) -> Result<(), NetworkEffectError> {
        self.event("guest-delete")
    }

    async fn delete_volume(&self) -> Result<(), NetworkEffectError> {
        self.event("volume-delete")
    }
}

pub fn network_spec() -> NetworkSpec {
    NetworkSpec::minimal(
        Ipv4Cidr::parse("10.20.0.0/24").unwrap(),
        Ipv4Cidr::parse("192.0.2.0/30").unwrap(),
        BoundedToken::parse("net-vm-base").unwrap(),
    )
    .unwrap()
}

fn network_input(
    spec: NetworkSpec,
    volume_ready: bool,
    guest_ready: bool,
    attachment_ready: bool,
) -> ReconcileInput {
    let network_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let attachment_uid = ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap();
    ReconcileInput {
        spec,
        mdns_enabled: false,
        network_uid: network_uid.clone(),
        network_generation: ResourceGeneration::new(4).unwrap(),
        installed_generation: ResourceBundleGenerationId::parse(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap(),
        artifact_catalog: vec![ArtifactCatalogEntry::new(
            BoundedToken::parse("net-vm-base").unwrap(),
            ArtifactKind::NixosSystem,
        )],
        peer_networks: Vec::new(),
        user_ready: true,
        host_memory_budget_available: 8 * 1024 * 1024,
        volume_ready,
        guest_ready,
        volume_attachment_ready: attachment_ready,
        workload_fds_closed: true,
        agent_deleted: true,
        mdns_deleted: true,
        volume_attachment_removed: true,
        guest_deleted: true,
        volume_deleted: true,
        attachments: vec![AttachmentRealization {
            handle: AttachmentHandle::new(
                attachment_uid.clone(),
                AttachmentGenerationFence::new(
                    network_uid,
                    ResourceGeneration::new(4).unwrap(),
                    attachment_uid,
                    ResourceGeneration::new(7).unwrap(),
                ),
            ),
            vmm_fd_closed: true,
        }],
    }
}

#[test]
fn network_zone_waits_for_children_refuses_unauthorized_policy_and_finalizes_ordered() {
    let directory = tempfile::tempdir().expect("Network state directory");
    let boundary = FilesystemNetworkBoundary::new(directory.path());
    let reconciler = NetworkReconciler::new(&boundary, &boundary);

    let waiting = network_input(network_spec(), false, true, true);
    assert_eq!(
        block_on(reconciler.reconcile(&waiting)).unwrap(),
        ReconcileProgress::Pending(
            d2b_provider_network_local::controller::NetworkConditionReason::VolumeNotReady
        )
    );
    assert!(
        !boundary
            .events()
            .iter()
            .any(|event| event == "guest-upsert"),
        "dependency wait must not reach Guest effects"
    );

    let ready = network_input(network_spec(), true, true, true);
    assert_eq!(
        block_on(reconciler.reconcile(&ready)).unwrap(),
        ReconcileProgress::Ready
    );

    let mut unauthorized = network_input(network_spec(), true, true, true);
    unauthorized.spec = NetworkSpec::new(
        Ipv4Cidr::parse("10.21.0.0/24").unwrap(),
        Ipv4Cidr::parse("192.0.2.4/30").unwrap(),
        None,
        false,
        IsolationSpec {
            allow_east_west: true,
        },
        RoutingSpec::default(),
        DhcpSpec::default(),
        DnsSpec::default(),
        None,
        MdnsSpec::default(),
        None,
        BoundedToken::parse("net-vm-base").unwrap(),
        Vec::new(),
    )
    .unwrap();
    let event_count = boundary.events().len();
    assert_eq!(
        block_on(reconciler.reconcile(&unauthorized)),
        Err(NetworkEffectError::EastWestHostOptInRequired)
    );
    assert_eq!(boundary.events().len(), event_count);

    let mut finalizing = ready;
    finalizing.agent_deleted = false;
    finalizing.mdns_deleted = false;
    finalizing.volume_attachment_removed = false;
    finalizing.guest_deleted = false;
    finalizing.volume_deleted = false;
    assert_eq!(
        block_on(reconciler.finalize(&finalizing)).unwrap(),
        FinalizerStage::Processes
    );
    finalizing.agent_deleted = true;
    finalizing.mdns_deleted = true;
    assert_eq!(
        block_on(reconciler.finalize(&finalizing)).unwrap(),
        FinalizerStage::VolumeAttachment
    );
    finalizing.volume_attachment_removed = true;
    assert_eq!(
        block_on(reconciler.finalize(&finalizing)).unwrap(),
        FinalizerStage::Guest
    );
    finalizing.guest_deleted = true;
    assert_eq!(
        block_on(reconciler.finalize(&finalizing)).unwrap(),
        FinalizerStage::Volume
    );
    finalizing.volume_deleted = true;
    assert_eq!(
        block_on(reconciler.finalize(&finalizing)).unwrap(),
        FinalizerStage::Complete
    );
    let events = boundary.events();
    let detach = events
        .iter()
        .position(|event| event == "volume-detach")
        .unwrap();
    let guest = events
        .iter()
        .position(|event| event == "guest-delete")
        .unwrap();
    let volume = events
        .iter()
        .position(|event| event == "volume-delete")
        .unwrap();
    let bridge = events
        .iter()
        .position(|event| event == "bridge-delete")
        .unwrap();
    assert!(detach < guest && guest < volume && volume < bridge);
}

struct FilesystemTpm {
    root: PathBuf,
    process: Mutex<Option<Child>>,
}

impl FilesystemTpm {
    fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            process: Mutex::new(None),
        }
    }
}

impl TpmResourceEffectPort for FilesystemTpm {
    async fn ensure_state_volume(
        &self,
        _: &ResourceUid,
        _: &ResourceRef,
        _: &ResourceRef,
    ) -> Result<ResourceRef, TpmResourceEffectError> {
        fs::create_dir_all(self.root.join("tpm-state"))
            .map_err(|_| TpmResourceEffectError::Transient)?;
        Ok(ResourceRef::parse("Volume/device-tpm-state").unwrap())
    }

    async fn request_swtpm_process(
        &self,
        _: &ResourceUid,
        _: &ResourceRef,
        _: &ResourceRef,
    ) -> Result<ResourceRef, TpmResourceEffectError> {
        if let Some(process) = self
            .process
            .lock()
            .map_err(|_| TpmResourceEffectError::Transient)?
            .as_mut()
            && process
                .try_wait()
                .map_err(|_| TpmResourceEffectError::Transient)?
                .is_none()
        {
            return Ok(ResourceRef::parse("Process/device-swtpm").unwrap());
        }
        let child = Command::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| TpmResourceEffectError::Transient)?;
        fs::write(self.root.join("swtpm.pid"), child.id().to_string())
            .map_err(|_| TpmResourceEffectError::Transient)?;
        *self
            .process
            .lock()
            .map_err(|_| TpmResourceEffectError::Transient)? = Some(child);
        Ok(ResourceRef::parse("Process/device-swtpm").unwrap())
    }

    async fn request_flush_process(
        &self,
        _: &ResourceUid,
        _: &ResourceRef,
    ) -> Result<ResourceRef, TpmResourceEffectError> {
        Command::new("true")
            .status()
            .map_err(|_| TpmResourceEffectError::Transient)?;
        fs::write(self.root.join("flush.complete"), b"ok")
            .map_err(|_| TpmResourceEffectError::Transient)?;
        Ok(ResourceRef::parse("EphemeralProcess/device-tpm-flush").unwrap())
    }

    async fn stop_swtpm_process(&self, _: &ResourceRef) -> Result<(), TpmResourceEffectError> {
        let Some(mut child) = self
            .process
            .lock()
            .map_err(|_| TpmResourceEffectError::Transient)?
            .take()
        else {
            return Ok(());
        };
        child
            .kill()
            .and_then(|_| child.wait())
            .map_err(|_| TpmResourceEffectError::Transient)?;
        fs::write(self.root.join("swtpm.stopped"), b"ok")
            .map_err(|_| TpmResourceEffectError::Transient)
    }

    async fn delete_flush_process(&self, _: &ResourceRef) -> Result<(), TpmResourceEffectError> {
        let _ = fs::remove_file(self.root.join("flush.complete"));
        Ok(())
    }

    async fn watch_tpm_endpoint(
        &self,
        _: &ResourceRef,
    ) -> Result<ResourceRef, TpmResourceEffectError> {
        let mut process = self
            .process
            .lock()
            .map_err(|_| TpmResourceEffectError::Transient)?;
        if process
            .as_mut()
            .and_then(|child| child.try_wait().ok())
            .flatten()
            .is_some()
        {
            return Err(TpmResourceEffectError::Transient);
        }
        Ok(ResourceRef::parse("Endpoint/device-tpm").unwrap())
    }
}

impl Drop for FilesystemTpm {
    fn drop(&mut self) {
        if let Ok(mut process) = self.process.lock()
            && let Some(mut child) = process.take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[tokio::test]
async fn device_tpm_zone_activation_ready_and_state_preserving_removal() {
    let directory = tempfile::tempdir().expect("TPM state directory");
    let effects = FilesystemTpm::new(directory.path());
    let device = ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").unwrap();
    let device_ref = ResourceRef::parse("Device/work-tpm").unwrap();
    let execution = ResourceRef::parse("Host/host-system").unwrap();
    let mut controller = TpmResourceController::new(device, device_ref, execution).unwrap();

    assert_eq!(
        controller.reconcile(&effects).await.unwrap(),
        TpmResourceOutcome::Ready
    );
    assert!(directory.path().join("tpm-state").is_dir());
    assert!(controller.endpoint_ref().is_some());
    assert!(directory.path().join("flush.complete").is_file());

    assert_eq!(
        controller.finalize(&effects).await.unwrap(),
        TpmResourceOutcome::VolumeRetained
    );
    assert!(!directory.path().join("flush.complete").exists());
    assert!(directory.path().join("tpm-state").is_dir());
    assert!(directory.path().join("swtpm.stopped").is_file());
}

struct RealCloudHypervisorEffect {
    process: Mutex<Option<Child>>,
    pidfds: Mutex<Vec<rustix::fd::OwnedFd>>,
}

impl RealCloudHypervisorEffect {
    fn identity_for(child: &Child) -> Result<ProcessIdentity, CloudHypervisorError> {
        let pid = child.id();
        let stat = fs::read_to_string(format!("/proc/{pid}/stat"))
            .map_err(|_| CloudHypervisorError::Effect)?;
        let fields = stat
            .split_once(") ")
            .map(|(_, rest)| rest.split_whitespace().collect::<Vec<_>>())
            .ok_or(CloudHypervisorError::Effect)?;
        let start_time_ticks = fields
            .get(19)
            .and_then(|value| value.parse().ok())
            .ok_or(CloudHypervisorError::Effect)?;
        let executable =
            fs::read_link(format!("/proc/{pid}/exe")).map_err(|_| CloudHypervisorError::Effect)?;
        let cgroup =
            fs::read(format!("/proc/{pid}/cgroup")).map_err(|_| CloudHypervisorError::Effect)?;
        let executable_digest = digest(executable.to_string_lossy().as_bytes());
        let cgroup_digest = digest(&cgroup);
        Ok(ProcessIdentity {
            pid,
            start_time_ticks,
            cgroup_digest,
            executable_digest,
            template_digest: [3; 32],
            generation: 1,
        })
    }

    fn current_identity(&self) -> Result<Option<ProcessIdentity>, CloudHypervisorError> {
        let mut process = self
            .process
            .lock()
            .map_err(|_| CloudHypervisorError::Effect)?;
        let Some(child) = process.as_mut() else {
            return Ok(None);
        };
        if child
            .try_wait()
            .map_err(|_| CloudHypervisorError::Effect)?
            .is_some()
        {
            *process = None;
            return Ok(None);
        }
        Ok(Some(Self::identity_for(child)?))
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(bytes);
    digest.finalize().into()
}

#[async_trait]
impl CloudHypervisorEffectPort for RealCloudHypervisorEffect {
    async fn launch(
        &self,
        _: &BootstrapGraph,
        _: &CloudHypervisorConfig,
        _: &CloudHypervisorGuestSettings,
    ) -> Result<ProcessIdentity, CloudHypervisorError> {
        let child = Command::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| CloudHypervisorError::Effect)?;
        let identity = Self::identity_for(&child)?;
        *self
            .process
            .lock()
            .map_err(|_| CloudHypervisorError::Effect)? = Some(child);
        Ok(identity)
    }

    async fn observe(&self) -> Result<Option<ProcessIdentity>, CloudHypervisorError> {
        self.current_identity()
    }

    async fn open_pidfd(&self, identity: &ProcessIdentity) -> Result<(), CloudHypervisorError> {
        let pidfd = rustix::process::pidfd_open(
            rustix::process::Pid::from_raw(identity.pid as i32)
                .ok_or(CloudHypervisorError::Effect)?,
            rustix::process::PidfdFlags::empty(),
        )
        .map_err(|_| CloudHypervisorError::Effect)?;
        self.pidfds
            .lock()
            .map_err(|_| CloudHypervisorError::Effect)?
            .push(pidfd);
        Ok(())
    }

    async fn stop(&self, identity: &ProcessIdentity) -> Result<(), CloudHypervisorError> {
        let mut process = self
            .process
            .lock()
            .map_err(|_| CloudHypervisorError::Effect)?;
        let Some(mut child) = process.take() else {
            return Ok(());
        };
        if child.id() != identity.pid {
            return Err(CloudHypervisorError::AdoptionAmbiguous);
        }
        child
            .kill()
            .and_then(|_| child.wait())
            .map(|_| ())
            .map_err(|_| CloudHypervisorError::Effect)
    }
}

impl Drop for RealCloudHypervisorEffect {
    fn drop(&mut self) {
        if let Ok(mut process) = self.process.lock()
            && let Some(mut child) = process.take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct FixedCloudClock(u64);

impl CloudHypervisorClock for FixedCloudClock {
    fn now_unix_ms(&self) -> u64 {
        self.0
    }
}

struct FilesystemGuestControl {
    ready: PathBuf,
}

#[async_trait]
impl GuestControlProbe for FilesystemGuestControl {
    async fn probe(&self, _: u32, _: u32) -> Result<GuestControlHealth, GuestControlHealthError> {
        match fs::read_to_string(&self.ready) {
            Ok(value) if value == "ready" => Ok(GuestControlHealth::Ready),
            Ok(_) => Ok(GuestControlHealth::Degraded),
            Err(_) => Err(GuestControlHealthError::Disconnected),
        }
    }

    async fn close(&self, _: u32) -> Result<(), GuestControlHealthError> {
        fs::write(&self.ready, b"closed").map_err(|_| GuestControlHealthError::Disconnected)
    }
}

fn cloud_controller(
    effect: Arc<RealCloudHypervisorEffect>,
    probe: Arc<FilesystemGuestControl>,
    expected: Option<ProcessIdentity>,
) -> CloudHypervisorController<RealCloudHypervisorEffect, FilesystemGuestControl> {
    let config = CloudHypervisorConfig {
        controller_execution_ref: ResourceRef::parse("Host/host-system").unwrap(),
        default_vcpus: 2,
        default_memory_mb: 512,
        default_machine_type: d2b_contracts::v3::credential::OpaqueAzureRef::parse("q35").unwrap(),
        watchdog: true,
        adoption_window_ms: 30_000,
        health_check_interval_ms: 30_000,
        health_check_timeout_ms: 5_000,
        health_check_failure_threshold: 3,
        startup_deadline_ms: 30_000,
    };
    let settings = CloudHypervisorGuestSettings {
        vcpus: Some(2),
        memory_mb: Some(512),
        machine_type: None,
        console_type: ConsoleType::Null,
        serial_port: true,
        pvpanic: false,
        watchdog_override: None,
        memory_shared: true,
        has_virtiofs_attachment: false,
        system_artifact_id: Some("system-artifact".to_owned()),
    };
    let graph = BootstrapGraph::new(
        vec![ResourceRef::parse("Device/kvm").unwrap()],
        vec![ResourceRef::parse("Network/work").unwrap()],
        vec![ResourceRef::parse("Volume/store").unwrap()],
        vec![AttachmentRef::new("launch-ticket").unwrap()],
    )
    .unwrap();
    let controller = CloudHypervisorController::new(config, settings, graph, effect, probe)
        .unwrap()
        .with_clock(Arc::new(FixedCloudClock(1_000)));
    match expected {
        Some(identity) => controller.with_expected_identity(identity),
        None => controller,
    }
}

#[tokio::test]
async fn cloud_hypervisor_zone_waits_dependencies_reaches_ready_and_adopts_process() {
    let directory = tempfile::tempdir().expect("Guest-control state directory");
    let ready_path = directory.path().join("guest-control");
    fs::write(&ready_path, b"ready").unwrap();
    let effect = Arc::new(RealCloudHypervisorEffect {
        process: Mutex::new(None),
        pidfds: Mutex::new(Vec::new()),
    });
    let probe = Arc::new(FilesystemGuestControl { ready: ready_path });
    let mut controller = cloud_controller(Arc::clone(&effect), Arc::clone(&probe), None);

    assert_eq!(
        controller.reconcile(false, true, true, 14).await.unwrap(),
        CloudHypervisorReconcileOutcome::Retry { after_ms: 500 }
    );
    assert_eq!(controller.phase(), CloudHypervisorPhase::Pending);
    assert_eq!(
        controller.reconcile(true, true, true, 14).await.unwrap(),
        CloudHypervisorReconcileOutcome::Converged
    );
    assert_eq!(controller.phase(), CloudHypervisorPhase::Ready);
    let identity = effect
        .current_identity()
        .unwrap()
        .expect("running VMM identity");
    let recovery = controller.recovery_state();
    drop(controller);

    let mut restarted = cloud_controller(Arc::clone(&effect), probe, Some(identity))
        .restore_recovery_state(recovery)
        .unwrap();
    assert_eq!(
        restarted.adopt(identity, 14).await.unwrap(),
        CloudHypervisorReconcileOutcome::Converged
    );
    assert_eq!(restarted.phase(), CloudHypervisorPhase::Ready);
    restarted.finalize().await.unwrap();
    assert_eq!(restarted.phase(), CloudHypervisorPhase::Finalized);
    assert!(effect.current_identity().unwrap().is_none());
}

/// Shared real-effect boundary used by the daemon-level operator acceptance.
///
/// The boundary deliberately owns filesystem and child-process effects and
/// reconstructs controller state during `adopt_after_restart`; it is not a
/// call-recording test double.
pub struct Wave6RealBoundary {
    root: PathBuf,
    volume: FilesystemVolume,
    network: FilesystemNetworkBoundary,
    tpm: FilesystemTpm,
    tpm_controller: Mutex<Option<TpmResourceController>>,
    cloud_effect: Arc<RealCloudHypervisorEffect>,
    cloud_probe: Arc<FilesystemGuestControl>,
    guest_controller:
        Mutex<Option<CloudHypervisorController<RealCloudHypervisorEffect, FilesystemGuestControl>>>,
}

impl Wave6RealBoundary {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        fs::create_dir_all(&root).expect("create Wave 6 provider effect root");
        let guest_control = root.join("guest-control");
        fs::write(&guest_control, b"ready").expect("seed guest-control readiness");
        Self {
            volume: FilesystemVolume::new(root.join("volume")),
            network: FilesystemNetworkBoundary::new(root.join("network")),
            tpm: FilesystemTpm::new(root.join("tpm")),
            tpm_controller: Mutex::new(None),
            cloud_effect: Arc::new(RealCloudHypervisorEffect {
                process: Mutex::new(None),
                pidfds: Mutex::new(Vec::new()),
            }),
            cloud_probe: Arc::new(FilesystemGuestControl {
                ready: guest_control,
            }),
            guest_controller: Mutex::new(None),
            root,
        }
    }

    fn tpm_controller() -> Result<TpmResourceController, Wave6BoundaryError> {
        TpmResourceController::new(
            ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002")
                .map_err(|_| Wave6BoundaryError::Effect)?,
            ResourceRef::parse("Device/work-tpm").map_err(|_| Wave6BoundaryError::Effect)?,
            ResourceRef::parse("Host/host-system").map_err(|_| Wave6BoundaryError::Effect)?,
        )
        .map_err(|_| Wave6BoundaryError::Effect)
    }

    fn guest_controller(
        &self,
        expected: Option<ProcessIdentity>,
    ) -> Result<
        CloudHypervisorController<RealCloudHypervisorEffect, FilesystemGuestControl>,
        Wave6BoundaryError,
    > {
        Ok(cloud_controller(
            Arc::clone(&self.cloud_effect),
            Arc::clone(&self.cloud_probe),
            expected,
        ))
    }

    fn ready_network_input(&self, dependencies: Wave6Dependencies) -> ReconcileInput {
        network_input(
            network_spec(),
            dependencies.volume_ready,
            dependencies.guest_ready,
            dependencies.attachment_ready,
        )
    }
}

#[async_trait]
impl Wave6ProviderBoundary for Wave6RealBoundary {
    async fn reconcile_volume(
        &self,
        resource: &Wave6Resource,
    ) -> Result<Wave6ReconcileResult, Wave6BoundaryError> {
        let controller =
            VolumeLocalController::new(VolumeLocalProfile::shipped(), &self.volume, &self.volume);
        controller
            .reconcile(&resource.uid, &volume_spec())
            .await
            .map_err(|_| Wave6BoundaryError::Effect)?;
        Ok(Wave6ReconcileResult::Ready)
    }

    async fn reconcile_network(
        &self,
        _resource: &Wave6Resource,
        dependencies: Wave6Dependencies,
    ) -> Result<Wave6ReconcileResult, Wave6BoundaryError> {
        let reconciler = NetworkReconciler::new(&self.network, &self.network);
        match reconciler
            .reconcile(&self.ready_network_input(dependencies))
            .await
            .map_err(|_| Wave6BoundaryError::Effect)?
        {
            ReconcileProgress::Pending(_) => Ok(Wave6ReconcileResult::Waiting),
            ReconcileProgress::Requeue(_) => Ok(Wave6ReconcileResult::Waiting),
            ReconcileProgress::Ready => Ok(Wave6ReconcileResult::Ready),
            ReconcileProgress::Blocked(_) => Err(Wave6BoundaryError::Lifecycle),
        }
    }

    async fn reconcile_device_tpm(
        &self,
        _resource: &Wave6Resource,
    ) -> Result<Wave6ReconcileResult, Wave6BoundaryError> {
        let mut controller = self
            .tpm_controller
            .lock()
            .map_err(|_| Wave6BoundaryError::Effect)?
            .take()
            .unwrap_or(Self::tpm_controller()?);
        let result = controller
            .reconcile(&self.tpm)
            .await
            .map_err(|_| Wave6BoundaryError::Effect);
        self.tpm_controller
            .lock()
            .map_err(|_| Wave6BoundaryError::Effect)?
            .replace(controller);
        result?;
        Ok(Wave6ReconcileResult::Ready)
    }

    async fn reconcile_cloud_hypervisor_guest(
        &self,
        _resource: &Wave6Resource,
        dependencies: Wave6Dependencies,
    ) -> Result<Wave6ReconcileResult, Wave6BoundaryError> {
        let mut controller = self
            .guest_controller
            .lock()
            .map_err(|_| Wave6BoundaryError::Effect)?
            .take()
            .map(Ok)
            .unwrap_or_else(|| self.guest_controller(None))?;
        let outcome = controller
            .reconcile(
                dependencies.network_ready,
                dependencies.volume_ready,
                dependencies.attachment_ready,
                14,
            )
            .await
            .map_err(|_| Wave6BoundaryError::Effect)?;
        self.guest_controller
            .lock()
            .map_err(|_| Wave6BoundaryError::Effect)?
            .replace(controller);
        match outcome {
            CloudHypervisorReconcileOutcome::Retry { .. }
            | CloudHypervisorReconcileOutcome::Progressing { .. } => {
                Ok(Wave6ReconcileResult::Waiting)
            }
            CloudHypervisorReconcileOutcome::Converged => Ok(Wave6ReconcileResult::Ready),
        }
    }

    async fn adopt_after_restart(
        &self,
        resources: &Wave6ResourceSet,
    ) -> Result<(), Wave6BoundaryError> {
        let volume =
            VolumeLocalController::new(VolumeLocalProfile::shipped(), &self.volume, &self.volume);
        volume
            .reconcile(&resources.volume.uid, &volume_spec())
            .await
            .map_err(|_| Wave6BoundaryError::Effect)?;

        let network = NetworkReconciler::new(&self.network, &self.network);
        let network_result = network
            .reconcile(&self.ready_network_input(Wave6Dependencies::guest_ready_for_adoption()))
            .await
            .map_err(|_| Wave6BoundaryError::Effect)?;
        if !matches!(network_result, ReconcileProgress::Ready) {
            return Err(Wave6BoundaryError::Lifecycle);
        }

        let mut tpm_controller = Self::tpm_controller()?;
        tpm_controller
            .reconcile(&self.tpm)
            .await
            .map_err(|_| Wave6BoundaryError::Effect)?;
        self.tpm_controller
            .lock()
            .map_err(|_| Wave6BoundaryError::Effect)?
            .replace(tpm_controller);

        let (recovery, identity) = {
            let mut guest_guard = self
                .guest_controller
                .lock()
                .map_err(|_| Wave6BoundaryError::Effect)?;
            let controller = guest_guard.take().ok_or(Wave6BoundaryError::Lifecycle)?;
            let identity = self
                .cloud_effect
                .current_identity()
                .map_err(|_| Wave6BoundaryError::Effect)?
                .ok_or(Wave6BoundaryError::Lifecycle)?;
            (controller.recovery_state(), identity)
        };
        let mut restarted = self
            .guest_controller(Some(identity))?
            .restore_recovery_state(recovery)
            .map_err(|_| Wave6BoundaryError::Lifecycle)?;
        restarted
            .adopt(identity, 14)
            .await
            .map_err(|_| Wave6BoundaryError::Effect)?;
        if restarted.phase() != CloudHypervisorPhase::Ready {
            return Err(Wave6BoundaryError::Lifecycle);
        }
        *self
            .guest_controller
            .lock()
            .map_err(|_| Wave6BoundaryError::Effect)? = Some(restarted);
        Ok(())
    }

    async fn remove_cloud_hypervisor_guest(
        &self,
        _resource: &Wave6Resource,
    ) -> Result<(), Wave6BoundaryError> {
        let mut controller = self
            .guest_controller
            .lock()
            .map_err(|_| Wave6BoundaryError::Effect)?
            .take()
            .ok_or(Wave6BoundaryError::Lifecycle)?;
        controller
            .finalize()
            .await
            .map_err(|_| Wave6BoundaryError::Effect)?;
        if controller.phase() != CloudHypervisorPhase::Finalized
            || self
                .cloud_effect
                .current_identity()
                .map_err(|_| Wave6BoundaryError::Effect)?
                .is_some()
        {
            return Err(Wave6BoundaryError::Lifecycle);
        }
        Ok(())
    }

    async fn remove_network(&self, _resource: &Wave6Resource) -> Result<(), Wave6BoundaryError> {
        let reconciler = NetworkReconciler::new(&self.network, &self.network);
        let mut input = self.ready_network_input(Wave6Dependencies::guest_ready_for_adoption());
        input.agent_deleted = false;
        input.mdns_deleted = false;
        input.volume_attachment_removed = false;
        input.guest_deleted = false;
        input.volume_deleted = false;
        if !matches!(
            reconciler
                .finalize(&input)
                .await
                .map_err(|_| Wave6BoundaryError::Effect)?,
            FinalizerStage::Processes
        ) {
            return Err(Wave6BoundaryError::Lifecycle);
        }
        input.agent_deleted = true;
        input.mdns_deleted = true;
        if !matches!(
            reconciler
                .finalize(&input)
                .await
                .map_err(|_| Wave6BoundaryError::Effect)?,
            FinalizerStage::VolumeAttachment
        ) {
            return Err(Wave6BoundaryError::Lifecycle);
        }
        input.volume_attachment_removed = true;
        if !matches!(
            reconciler
                .finalize(&input)
                .await
                .map_err(|_| Wave6BoundaryError::Effect)?,
            FinalizerStage::Guest
        ) {
            return Err(Wave6BoundaryError::Lifecycle);
        }
        input.guest_deleted = true;
        if !matches!(
            reconciler
                .finalize(&input)
                .await
                .map_err(|_| Wave6BoundaryError::Effect)?,
            FinalizerStage::Volume
        ) {
            return Err(Wave6BoundaryError::Lifecycle);
        }
        input.volume_deleted = true;
        if !matches!(
            reconciler
                .finalize(&input)
                .await
                .map_err(|_| Wave6BoundaryError::Effect)?,
            FinalizerStage::Complete
        ) {
            return Err(Wave6BoundaryError::Lifecycle);
        }
        Ok(())
    }

    async fn remove_device_tpm(
        &self,
        _resource: &Wave6Resource,
    ) -> Result<bool, Wave6BoundaryError> {
        let mut controller = self
            .tpm_controller
            .lock()
            .map_err(|_| Wave6BoundaryError::Effect)?
            .take()
            .ok_or(Wave6BoundaryError::Lifecycle)?;
        let outcome = controller
            .finalize(&self.tpm)
            .await
            .map_err(|_| Wave6BoundaryError::Effect)?;
        if !matches!(outcome, TpmResourceOutcome::VolumeRetained)
            || !self.root.join("tpm/tpm-state").is_dir()
        {
            return Err(Wave6BoundaryError::DeviceStateNotRetained);
        }
        Ok(true)
    }

    async fn remove_volume(&self, resource: &Wave6Resource) -> Result<(), Wave6BoundaryError> {
        let controller =
            VolumeLocalController::new(VolumeLocalProfile::shipped(), &self.volume, &self.volume);
        controller
            .cleanup(&resource.uid, &volume_spec())
            .await
            .map_err(|_| Wave6BoundaryError::Effect)?;
        if self.root.join("volume/state.db").exists() {
            return Err(Wave6BoundaryError::Lifecycle);
        }
        Ok(())
    }
}
