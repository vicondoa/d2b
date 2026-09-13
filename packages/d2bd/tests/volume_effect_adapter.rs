//! Production Volume controller and anchored effect-adapter composition.

use std::os::unix::fs::PermissionsExt;

use d2b_contracts_resource::v3::{
    ResourceGeneration, ResourceRef, ResourceUid,
    volume::SourceKind,
};
use d2b_provider_volume_local::{
    ContentFile, ContentProjection, ContentProvenance, EntryRequest, VolumeLayoutEffectPort,
    VolumeLocalController, VolumeLocalError, VolumeLocalProfile, VolumeSourceEffectPort,
};
use d2bd::resource_runtime::{
    AnchoredVolumeEffectAdapter, FdRootResolver, ResolvedVolumeRoot, VolumeRootResolver,
};

#[derive(Debug)]
struct FixedPrincipalResolver {
    inner: FdRootResolver,
    uid: u32,
    gid: u32,
}

impl VolumeRootResolver for FixedPrincipalResolver {
    fn resolve_root(
        &self,
        volume_uid: &ResourceUid,
        source_policy_id: Option<&d2b_contracts_resource::v3::execution_policy::BoundedToken>,
        system_artifact_id: Option<&d2b_contracts_resource::v3::execution_policy::BoundedToken>,
        kind: SourceKind,
    ) -> Result<ResolvedVolumeRoot, VolumeLocalError> {
        self.inner
            .resolve_root(volume_uid, source_policy_id, system_artifact_id, kind)
    }

    fn resolve_principal(&self, reference: &ResourceRef) -> Result<u32, VolumeLocalError> {
        if reference.resource_type().as_str() != "User" {
            return Err(VolumeLocalError::InvalidSpec);
        }
        Ok(self.uid)
    }

    fn resolve_group(&self, reference: &ResourceRef) -> Result<u32, VolumeLocalError> {
        if reference.resource_type().as_str() != "User" {
            return Err(VolumeLocalError::InvalidSpec);
        }
        Ok(self.gid)
    }
}

fn volume_uid() -> ResourceUid {
    ResourceUid::parse("6f9619ff-8b86-4d01-b42d-00cf4fc964ff").expect("volume uid")
}

fn volume_spec() -> d2b_contracts_resource::v3::volume::VolumeSpec {
    serde_json::from_value(serde_json::json!({
        "source": {
            "executionRef": "Host/host-system",
            "settings": { "kind": "local-path", "sourcePolicyId": "state-root" }
        },
        "kind": "state",
        "layout": [{
            "path": "",
            "type": "directory",
            "ownerRef": "User/d2bd",
            "groupRef": "User/d2bd",
            "mode": "0700"
        }],
        "views": {
            "controller": {
                "path": "",
                "rights": ["read", "write", "create", "delete", "traverse"]
            }
        }
    }))
    .expect("Volume spec")
}

fn adapter_root(
    name: &str,
) -> (
    std::path::PathBuf,
    AnchoredVolumeEffectAdapter<FdRootResolver>,
) {
    adapter_root_with_options(name, false)
}

fn adapter_root_with_options(
    name: &str,
    preexisting_state: bool,
) -> (
    std::path::PathBuf,
    AnchoredVolumeEffectAdapter<FdRootResolver>,
) {
    let base = std::path::PathBuf::from(
        std::env::var_os("CARGO_TARGET_TMPDIR").unwrap_or_else(|| "target/u7-volume-tests".into()),
    )
    .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("volume root");
    let resolver =
        FdRootResolver::new(std::fs::File::open(&base).expect("open root"), volume_uid())
            .expect("root resolver");
    let resolver = if preexisting_state {
        resolver.with_preexisting_state()
    } else {
        resolver
    };
    (base, AnchoredVolumeEffectAdapter::new(resolver))
}

#[test]
fn production_controller_materializes_and_adopts_a_marker_bound_root() {
    let (base, adapter) = adapter_root("production");
    let controller = VolumeLocalController::new(VolumeLocalProfile::shipped(), &adapter, &adapter);
    let uid = volume_uid();
    let spec = volume_spec();

    let first =
        d2b_provider_volume_local::testing::block_on(controller.reconcile(&uid, &spec, None, None))
        .expect("first reconcile");
    let restarted = d2b_provider_volume_local::testing::block_on(
        controller.reconcile(&uid, &spec, None, None),
    )
        .expect("restart adoption");
    assert_eq!(
        first.layout_phase,
        d2b_provider_volume_local::LayoutPhase::Ready
    );
    assert_eq!(
        restarted.layout_phase,
        d2b_provider_volume_local::LayoutPhase::Ready
    );
    assert!(base.join(".d2b-volume-marker").is_file());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn production_content_status_is_published_only_after_full_readback() {
    let (base, adapter) = adapter_root("content");
    let controller = VolumeLocalController::new(VolumeLocalProfile::shipped(), &adapter, &adapter);
    let uid = volume_uid();
    let spec = volume_spec();
    d2b_provider_volume_local::testing::block_on(controller.reconcile(&uid, &spec, None, None))
        .expect("layout reconcile");
    let owner = ResourceRef::parse("User/d2bd").expect("owner");
    let projection = ContentProjection::new(
        uid.clone(),
        ContentProvenance::new(
            ResourceRef::parse("Network/work").expect("network"),
            ResourceUid::parse("7f9619ff-8b86-4d01-b42d-00cf4fc964ff").expect("network uid"),
            ResourceGeneration::new(3).expect("generation"),
            "assignment-7",
            None,
        )
        .expect("provenance"),
        "network:config:owned",
        [ContentFile::new(
            "config",
            owner.clone(),
            owner,
            "0640",
            b"declared\n".to_vec(),
        )
        .expect("content file")],
    )
    .expect("projection");

    let evidence = d2b_provider_volume_local::testing::block_on(controller.reconcile_content(
        &uid,
        &spec,
        &projection,
    ))
    .expect("materialization evidence");
    let adopted = d2b_provider_volume_local::testing::block_on(controller.reconcile_content(
        &uid,
        &spec,
        &projection,
    ))
    .expect("restart evidence");
    assert!(evidence.matches(&projection));
    assert_eq!(adopted, evidence);
    assert_eq!(
        std::fs::read(base.join("config")).expect("readback"),
        b"declared\n"
    );
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn production_network_content_materializes_and_preserves_a_foreign_marker() {
    let (base, adapter) = adapter_root("network-content");
    let controller =
        VolumeLocalController::new(VolumeLocalProfile::shipped(), &adapter, &adapter);
    let volume_uid = volume_uid();
    let volume_spec =
        d2b_provider_network_local::controller::config_volume_spec("host-system", None)
            .expect("Network Volume spec");
    let network_ref = ResourceRef::parse("Network/work").expect("Network ref");
    let provenance = d2b_contracts_resource::v3::network::NetworkProvenance::new(
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("Zone UID"),
        ResourceUid::parse("223e4567-e89b-42d3-a456-426614174000").expect("Network UID"),
        ResourceGeneration::new(2).expect("Network generation"),
        ResourceGeneration::new(3).expect("Attachment generation"),
        d2b_contracts_resource::v3::ResourceBundleGenerationId::parse(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("bundle generation"),
    );
    let network_spec = d2b_contracts_resource::v3::network::NetworkSpec::minimal(
        d2b_contracts_resource::v3::network::Ipv4Cidr::parse("10.20.0.0/24")
            .expect("LAN CIDR"),
        d2b_contracts_resource::v3::network::Ipv4Cidr::parse("192.0.2.0/30")
            .expect("uplink CIDR"),
        d2b_contracts_resource::v3::execution_policy::BoundedToken::parse("net-vm-base")
            .expect("artifact"),
    )
    .expect("Network spec");
    let content =
        d2b_provider_network_local::controller::render_config_with_provenance(
            &network_spec,
            &provenance,
        )
        .expect("Network content");
    let file_owner =
        ResourceRef::parse(d2b_provider_volume_local::NETWORK_CONFIG_FILE_OWNER)
            .expect("content owner");
    let projection = d2b_provider_volume_local::NetworkConfigContentProjection::new(
        volume_uid.clone(),
        network_ref.clone(),
        provenance,
        d2b_contracts_resource::v3::derive_network_ownership_marker(
            &content
                .provenance
                .clone()
                .expect("content provenance"),
            "network-config",
        ),
        file_owner.clone(),
        file_owner,
        d2b_provider_volume_local::NETWORK_CONFIG_FILE_MODE,
        content.dnsmasq.clone(),
        content.nftables.clone(),
        content.routing.clone(),
        content.attachments.clone(),
        content.digest(),
    )
    .expect("typed Network content projection");
    let provider = serde_json::json!({
        "schemaId": d2b_provider_volume_local::VOLUME_CONTENT_SCHEMA_ID,
        "schemaVersion": d2b_provider_volume_local::VOLUME_CONTENT_SCHEMA_VERSION,
        "settings": {
            "kind": d2b_provider_volume_local::NETWORK_CONFIG_CONTENT_KIND,
            "content": projection,
        },
    });
    let status = d2b_provider_volume_local::testing::block_on(controller.reconcile(
        &volume_uid,
        &volume_spec,
        Some(&provider),
        Some(&network_ref),
    ))
    .expect("Network content reconcile");
    let evidence = status.content.as_ref().expect("durable content evidence");
    assert!(evidence.matches(
        &d2b_provider_volume_local::NetworkConfigContentProjection::from_settings(
            &provider["settings"]["content"],
        )
        .expect("projection round trip")
    ));
    for (path, bytes) in [
        ("dnsmasq.conf", content.dnsmasq.as_slice()),
        ("nftables.rules", content.nftables.as_slice()),
        ("routing.conf", content.routing.as_slice()),
        ("attachments.json", content.attachments.as_slice()),
    ] {
        assert_eq!(std::fs::read(base.join(path)).expect("materialized file"), bytes);
    }
    let before = [
        "dnsmasq.conf",
        "nftables.rules",
        "routing.conf",
        "attachments.json",
    ]
    .into_iter()
    .map(|path| std::fs::read(base.join(path)).expect("materialized file"))
    .collect::<Vec<_>>();

    std::fs::write(base.join(".d2b-volume-marker"), b"foreign-marker")
        .expect("foreign marker");
    assert_eq!(
        d2b_provider_volume_local::testing::block_on(controller.reconcile(
            &volume_uid,
            &volume_spec,
            Some(&provider),
            Some(&network_ref),
        )),
        Err(VolumeLocalError::EffectFailed)
    );
    assert_eq!(
        std::fs::read(base.join(".d2b-volume-marker")).expect("marker readback"),
        b"foreign-marker"
    );
    for (index, path) in [
        "dnsmasq.conf",
        "nftables.rules",
        "routing.conf",
        "attachments.json",
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            std::fs::read(base.join(path)).expect("materialized file"),
            before[index]
        );
    }
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn production_store_view_marker_evidence_requires_a_zero_length_file() {
    let (base, adapter) = adapter_root("store-view-marker");
    let uid = volume_uid();
    let spec = volume_spec();
    std::fs::create_dir_all(base.join("live")).expect("live directory");
    let root = d2b_provider_volume_local::testing::block_on(adapter.resolve_root_for(
        &uid,
        spec.source().settings().source_policy_id(),
        spec.source().settings().system_artifact_id(),
        spec.source().settings().kind(),
    ))
    .expect("resolve anchored root");
    let marker_path = "live/.d2b-marker-work-vm";

    let missing = d2b_provider_volume_local::testing::block_on(
        adapter.observe_store_view_marker(&root, marker_path),
    )
    .expect("missing marker evidence");
    assert!(!missing.present);
    assert!(!missing.zero_length);

    std::fs::write(base.join(marker_path), b"not-ready").expect("non-empty marker");
    let non_empty = d2b_provider_volume_local::testing::block_on(
        adapter.observe_store_view_marker(&root, marker_path),
    )
    .expect("non-empty marker evidence");
    assert!(non_empty.present);
    assert!(!non_empty.zero_length);

    std::fs::write(base.join(marker_path), []).expect("zero-length marker");
    let ready = d2b_provider_volume_local::testing::block_on(
        adapter.observe_store_view_marker(&root, marker_path),
    )
    .expect("ready marker evidence");
    assert!(ready.present);
    assert!(ready.zero_length);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn broker_owned_final_symlink_is_observed_without_metadata_repair() {
    let base = std::path::PathBuf::from(
        std::env::var_os("CARGO_TARGET_TMPDIR").unwrap_or_else(|| "target/u7-volume-tests".into()),
    )
    .join(format!("final-symlink-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("meta/generations")).expect("metadata directories");
    std::os::unix::fs::symlink("generations/0", base.join("meta/current"))
        .expect("broker current symlink");

    let uid = volume_uid();
    let inner = FdRootResolver::new(std::fs::File::open(&base).expect("open root"), uid.clone())
        .expect("root resolver");
    let resolver = FixedPrincipalResolver {
        inner,
        uid: 0,
        gid: 0,
    };
    let adapter = AnchoredVolumeEffectAdapter::new(resolver);
    let spec = d2b_provider_volume_local::testing::fixtures::store_view_volume();
    let declared = spec
        .layout()
        .iter()
        .find(|entry| entry.path() == "meta/current")
        .expect("current layout entry");
    let entry = EntryRequest::resolve(&uid, declared).expect("entry request");
    let root = d2b_provider_volume_local::testing::block_on(adapter.resolve_root_for(
        &uid,
        spec.source().settings().source_policy_id(),
        spec.source().settings().system_artifact_id(),
        spec.source().settings().kind(),
    ))
    .expect("resolve root");

    let observed = d2b_provider_volume_local::testing::block_on(adapter.observe(&root, &entry))
        .expect("observe final symlink");
    assert!(observed.present);
    assert!(observed.drift.is_empty());
    assert!(base.join("meta/current").is_symlink());

    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn production_nix_closure_store_view_materializes_without_a_source_policy() {
    let (base, adapter) = adapter_root_with_options("nix-closure-store-view", true);
    let controller = VolumeLocalController::new(VolumeLocalProfile::shipped(), &adapter, &adapter);
    let uid = volume_uid();
    let spec = d2b_provider_volume_local::testing::fixtures::nix_closure_store_view_volume();

    let first =
        d2b_provider_volume_local::testing::block_on(controller.reconcile(&uid, &spec, None, None))
            .expect("Nix closure Volume reconcile");
    let restarted =
        d2b_provider_volume_local::testing::block_on(controller.reconcile(&uid, &spec, None, None))
            .expect("Nix closure Volume restart adoption");

    assert_eq!(
        first.layout_phase,
        d2b_provider_volume_local::LayoutPhase::Ready
    );
    assert_eq!(
        restarted.layout_phase,
        d2b_provider_volume_local::LayoutPhase::Ready
    );
    assert!(base.join(".d2b-volume-marker").is_file());
    assert!(base.join("live").is_dir());
    assert!(base.join("meta/current").is_symlink());
    let _ = std::fs::remove_dir_all(base);
}

/// The store-view `sync.lock` is a `file-record` lease: it is adopted only
/// with a verifiable live owner record, exactly as the broker writes it
/// under the exclusive flock while it materializes the farm. With the
/// broker's lock present the layout converges to `Ready`; without adopting
/// it the Volume would stay Degraded and its attachment Pending.
#[test]
fn broker_owned_sync_lock_record_converges_the_store_view_layout() {
    let (base, adapter) = adapter_root_with_options("broker-sync-lock", true);
    let record = d2b_host::hardlink_farm::SyncLockOwnerRecord::for_current_process()
        .expect("broker owner record");
    let lock = base.join("sync.lock");
    std::fs::write(&lock, record.to_bytes()).expect("broker sync.lock record");
    std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o640)).expect("lock mode");

    let controller = VolumeLocalController::new(VolumeLocalProfile::shipped(), &adapter, &adapter);
    let uid = volume_uid();
    let spec = d2b_provider_volume_local::testing::fixtures::store_view_volume();
    let report = d2b_provider_volume_local::testing::block_on(
        controller.reconcile(&uid, &spec, None, None),
    )
    .expect("store-view Volume reconcile");

    assert_eq!(
        report.layout_phase,
        d2b_provider_volume_local::LayoutPhase::Ready
    );
    assert!(
        report.layout_conditions.is_empty(),
        "no entry may stay quarantined: {:?}",
        report.layout_conditions
    );
    // The broker's record is neither removed nor rewritten by adoption.
    assert_eq!(std::fs::read(&lock).expect("lock readback"), record.to_bytes());
    let _ = std::fs::remove_dir_all(base);
}

/// A `file-record` lock whose owner cannot be proven stays quarantined: an
/// unknown payload, and a record naming a process that is gone, are both
/// ambiguity and neither is adopted, repaired, or removed.
#[test]
fn unprovable_sync_lock_owner_still_quarantines_the_store_view_layout() {
    let (base, adapter) = adapter_root_with_options("foreign-sync-lock", true);
    let mut stale = d2b_host::hardlink_farm::SyncLockOwnerRecord::for_current_process()
        .expect("owner record");
    // No such process: the record's owner is not live.
    stale.pid = i32::MAX;
    let lock = base.join("sync.lock");
    std::fs::write(&lock, stale.to_bytes()).expect("stale sync.lock record");

    let controller = VolumeLocalController::new(VolumeLocalProfile::shipped(), &adapter, &adapter);
    let uid = volume_uid();
    let spec = d2b_provider_volume_local::testing::fixtures::store_view_volume();
    let report = d2b_provider_volume_local::testing::block_on(
        controller.reconcile(&uid, &spec, None, None),
    )
    .expect("store-view Volume reconcile");

    assert_eq!(
        report.layout_phase,
        d2b_provider_volume_local::LayoutPhase::Degraded
    );
    assert_eq!(report.layout_conditions.len(), 1, "{:?}", report.layout_conditions);
    let condition = &report.layout_conditions[0];
    assert_eq!(
        condition.entry,
        d2b_provider_volume_local::EntryDigest::derive(&uid, "sync.lock")
    );
    assert_eq!(condition.reason, VolumeLocalError::EntryQuarantined);
    assert_eq!(
        condition.severity,
        d2b_provider_volume_local::ConditionSeverity::Degraded
    );
    // Quarantine mutates nothing.
    assert_eq!(std::fs::read(&lock).expect("lock readback"), stale.to_bytes());
    let _ = std::fs::remove_dir_all(base);
}

/// The same quarantine holds for a lock that carries no record at all: the
/// daemon-created file is adopted only because its creator recorded itself.
#[test]
fn unrecorded_sync_lock_owner_still_quarantines_the_store_view_layout() {
    let (base, adapter) = adapter_root_with_options("unrecorded-sync-lock", true);
    let lock = base.join("sync.lock");
    std::fs::write(&lock, b"foreign-bytes").expect("foreign sync.lock");

    let controller = VolumeLocalController::new(VolumeLocalProfile::shipped(), &adapter, &adapter);
    let uid = volume_uid();
    let spec = d2b_provider_volume_local::testing::fixtures::store_view_volume();
    let report = d2b_provider_volume_local::testing::block_on(
        controller.reconcile(&uid, &spec, None, None),
    )
    .expect("store-view Volume reconcile");

    assert_eq!(
        report.layout_phase,
        d2b_provider_volume_local::LayoutPhase::Degraded
    );
    assert_eq!(report.layout_conditions.len(), 1, "{:?}", report.layout_conditions);
    assert_eq!(
        report.layout_conditions[0].reason,
        VolumeLocalError::EntryQuarantined
    );
    assert_eq!(
        std::fs::read(&lock).expect("lock readback"),
        b"foreign-bytes",
        "an unprovable foreign lock is never rewritten"
    );
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn foreign_marker_is_preserved_and_blocks_the_controller() {
    let (base, adapter) = adapter_root("foreign-marker");
    std::fs::write(base.join(".d2b-volume-marker"), b"foreign-marker").expect("foreign marker");
    let controller = VolumeLocalController::new(VolumeLocalProfile::shipped(), &adapter, &adapter);
    let error = d2b_provider_volume_local::testing::block_on(
        controller.reconcile(&volume_uid(), &volume_spec(), None, None),
    )
    .expect_err("foreign marker must fail closed");
    assert_eq!(error, VolumeLocalError::EffectFailed);
    assert_eq!(
        std::fs::read(base.join(".d2b-volume-marker")).expect("marker readback"),
        b"foreign-marker"
    );
    let _ = std::fs::remove_dir_all(base);
}
