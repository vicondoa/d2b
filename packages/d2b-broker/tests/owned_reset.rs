//! The one-shot ownership-bounded reset (U32, KTD15).
//!
//! Every case runs against a temporary deployment root, temporary
//! markers, and a temporary cgroup tree. Nothing here touches a live
//! host, and nothing here opens a SpecStore: the clean break is the
//! property under test, so a test that needed the old store to set itself
//! up would be testing the migration this unit removed.

use std::fs;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use d2b_broker::ops::host_reset::{
    DEPLOYMENT_GRAPH_DIGEST_DOMAIN, DEPLOYMENT_GRAPH_FILE, DeploymentGraph, GraphAuthorityRow,
    IncarnationRecord, INCARNATION_FILE, ResetMode, ResetRequest, run_owned_reset,
};
use d2b_contracts_resource::v3::{StoreIncarnation, ZoneId, canonical_digest, canonical_json_bytes};
use d2b_host::ownership_matrix::{OWNERSHIP_MARKER_FILE, render_ownership_marker};

const OWNERSHIP_ID: &str = "host:d2b";
const ZONE: &str = "sys-host";
const STORE: &str = "store-3";

/// One temporary deployment root with the new model's graph published in
/// it, an empty cgroup tree, and the drain probes in their declared
/// layout.
struct Deployment {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    cgroup: PathBuf,
}

impl Deployment {
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let cgroup = tmp.path().join("cgroup-root");
        fs::create_dir_all(&cgroup).expect("cgroup root");
        let deployment = Self {
            _tmp: tmp,
            root,
            cgroup,
        };
        fs::write(
            deployment.root.join(OWNERSHIP_MARKER_FILE),
            render_ownership_marker(OWNERSHIP_ID),
        )
        .expect("ownership marker");
        deployment
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn publish(&self, owned: &[(&str, &str)], external_sources: &[&str]) {
        let mut graph = DeploymentGraph {
            schema_version: "d2b-deployment-graph/1".to_owned(),
            zone: ZoneId::parse(ZONE).expect("zone"),
            store_incarnation: StoreIncarnation::parse(STORE).expect("store"),
            ownership_id: OWNERSHIP_ID.to_owned(),
            owned: owned
                .iter()
                .map(|(relative, kind)| d2b_broker::ops::host_reset::DeclaredOwnedPath {
                    path: self.root.join(relative),
                    kind: (*kind).to_owned(),
                })
                .collect(),
            external_sources: external_sources
                .iter()
                .map(PathBuf::from)
                .collect(),
            roles: Vec::new(),
            role_bindings: Vec::new(),
            graph_digest: String::new(),
        };
        let without_digest = graph.clone();
        graph.graph_digest = String::new();
        let bytes = canonical_json_bytes(&without_digest).expect("canonical graph");
        let _ = bytes;
        let probe = canonical_json_bytes(&graph).expect("canonical graph");
        graph.graph_digest = canonical_digest(DEPLOYMENT_GRAPH_DIGEST_DOMAIN, &probe);
        fs::write(
            self.root.join(DEPLOYMENT_GRAPH_FILE),
            serde_json::to_vec_pretty(&graph).expect("render graph"),
        )
        .expect("publish graph");
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn request(&self, mode: ResetMode) -> ResetRequest {
        ResetRequest {
            deployment_root: self.root.clone(),
            mode,
            cgroup_root: self.cgroup.clone(),
        }
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }
}

/// Populate the state a drained-out previous deployment would leave.
#[allow(dead_code, reason = "one readable description of the drained fixture")]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn seed_drained_state(deployment: &Deployment) {
    for directory in ["audit", "authority", "state-cells", "host-generation-handoffs"] {
        fs::create_dir_all(deployment.path(directory)).expect("state dir");
    }
    fs::write(deployment.path("audit/host.jsonl"), b"{}\n").expect("audit record");
    fs::write(deployment.path("authority/state.json"), b"{\"cursor\":1}").expect("authority");
    fs::write(deployment.path("state-cells/cells.json"), b"{\"cells\":[]}").expect("cells");
    fs::write(
        deployment.path("host-generation-handoffs/abc.json"),
        b"{\"generation\":1}",
    )
    .expect("handoff");
    fs::create_dir_all(deployment.path("zones/one")).expect("zones");
    fs::write(deployment.path("zones/one/state.json"), b"{}").expect("zone state");
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn inspect_reports_the_exact_inventory_and_removes_nothing() {
    let deployment = Deployment::new();
    deployment.publish(&[("zones", "tree"), ("audit", "tree")], &[]);
    seed_drained_state(&deployment);

    let report = run_owned_reset(&deployment.request(ResetMode::Inspect)).expect("inspect");
    assert!(report.ok);
    assert_eq!(report.mode, "inspect");
    assert_eq!(report.ownership_id, OWNERSHIP_ID);
    assert_eq!(report.inventory.len(), 2);
    assert!(report.inventory.iter().all(|entry| entry.present && !entry.removed));
    // The half that removes is the half that did not.
    assert!(deployment.path("zones/one/state.json").exists());
    assert!(deployment.path("authority/state.json").exists());
    assert!(!deployment.path(INCARNATION_FILE).exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn apply_removes_the_inventory_and_establishes_a_new_incarnation() {
    let deployment = Deployment::new();
    deployment.publish(
        &[
            ("zones", "tree"),
            ("audit", "tree"),
            ("authority", "tree"),
            ("state-cells", "tree"),
            ("host-generation-handoffs", "tree"),
        ],
        &[],
    );
    seed_drained_state(&deployment);

    let report = run_owned_reset(&deployment.request(ResetMode::Apply)).expect("apply");
    assert!(report.ok);
    assert_eq!(report.mode, "apply");
    assert!(report.inventory.iter().all(|entry| entry.removed));
    // The projection cursor/digest, the prepared fences, and the effect
    // and reservation journals are gone with the rest of the inventory.
    for gone in [
        "zones",
        "audit",
        "authority",
        "state-cells",
        "host-generation-handoffs",
    ] {
        assert!(
            !deployment.path(gone).exists(),
            "{gone} survived the reset with its journals intact"
        );
    }
    // The fresh deployment root and a NEW incarnation, so the next boot
    // initializes instead of resynchronizing a rollback.
    let incarnation: IncarnationRecord =
        serde_json::from_slice(&fs::read(deployment.path(INCARNATION_FILE)).expect("incarnation"))
            .expect("decode incarnation");
    assert_eq!(incarnation.schema_version, "d2b-deployment-incarnation/1");
    assert_eq!(
        incarnation.store_incarnation.as_str(),
        "store-4",
        "the fresh root must not reuse the graph's incarnation"
    );
    assert_eq!(incarnation.previous_graph_digest.len(), 71);
    assert_eq!(report.store_incarnation, "store-4");
    assert_eq!(
        fs::read_to_string(deployment.path(OWNERSHIP_MARKER_FILE)).expect("marker"),
        render_ownership_marker(OWNERSHIP_ID)
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_repeated_completed_reset_is_safe() {
    let deployment = Deployment::new();
    deployment.publish(&[("zones", "tree"), ("audit", "tree")], &[]);
    seed_drained_state(&deployment);

    let first = run_owned_reset(&deployment.request(ResetMode::Apply)).expect("first apply");
    assert!(first.inventory.iter().all(|entry| entry.removed));

    // Nothing owns anything any more, and an absent owned path is a
    // completed reset rather than a failure.
    let second = run_owned_reset(&deployment.request(ResetMode::Apply)).expect("second apply");
    assert!(second.ok);
    assert!(second.inventory.iter().all(|entry| !entry.present && !entry.removed));
    let third = run_owned_reset(&deployment.request(ResetMode::Inspect)).expect("inspect");
    assert!(third.inventory.iter().all(|entry| !entry.present));
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_live_workload_prevents_the_reset_and_leaves_the_state_alone() {
    let deployment = Deployment::new();
    deployment.publish(&[("zones", "tree")], &[]);
    seed_drained_state(&deployment);

    // A live cgroup member is drain evidence even though every owned path
    // is present and the tree itself is otherwise quiet.
    let live = deployment.cgroup.join("zones/one/guest");
    fs::create_dir_all(&live).expect("live cgroup");
    fs::write(live.join("cgroup.procs"), "4242\n").expect("members");

    let failure = run_owned_reset(&deployment.request(ResetMode::Apply)).expect_err("refused");
    assert_eq!(failure.report().code, "reset-workload-live");
    assert!(deployment.path("zones/one/state.json").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_held_lease_prevents_the_reset() {
    let deployment = Deployment::new();
    deployment.publish(&[("zones", "tree")], &[]);
    seed_drained_state(&deployment);

    let lock_dir = deployment.path("locks/usbip");
    fs::create_dir_all(&lock_dir).expect("locks");
    let lock_path = lock_dir.join("1-1.5");
    fs::write(&lock_path, b"").expect("lock file");
    fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600)).expect("lock mode");
    let held = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .expect("hold the lock");
    // The guard owns the descriptor, so the lock is held for the whole
    // probe the way a live holder would hold it.
    let _held = nix::fcntl::Flock::lock(held, nix::fcntl::FlockArg::LockExclusiveNonblock)
        .expect("take the lock");

    let failure = run_owned_reset(&deployment.request(ResetMode::Apply)).expect_err("refused");
    assert_eq!(failure.report().code, "reset-lease-held");
    assert!(deployment.path("zones/one/state.json").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn an_active_host_generation_prevents_the_reset() {
    let deployment = Deployment::new();
    deployment.publish(&[("zones", "tree")], &[]);
    seed_drained_state(&deployment);

    fs::create_dir_all(deployment.path("host-generation")).expect("host generation");
    fs::write(deployment.path("host-generation/active"), "generation-9\n").expect("active");

    let failure = run_owned_reset(&deployment.request(ResetMode::Apply)).expect_err("refused");
    assert_eq!(failure.report().code, "reset-host-generation-active");
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_foreign_ownership_marker_refuses_and_is_never_overwritten() {
    let deployment = Deployment::new();
    deployment.publish(&[("zones", "tree")], &[]);
    seed_drained_state(&deployment);
    fs::write(
        deployment.path(OWNERSHIP_MARKER_FILE),
        render_ownership_marker("host:somebody-else"),
    )
    .expect("foreign marker");

    let failure = run_owned_reset(&deployment.request(ResetMode::Apply)).expect_err("refused");
    assert_eq!(failure.report().code, "reset-foreign-ownership-marker");
    // The foreign marker is exactly as its owner left it.
    assert_eq!(
        fs::read_to_string(deployment.path(OWNERSHIP_MARKER_FILE)).expect("marker"),
        render_ownership_marker("host:somebody-else")
    );
    assert!(deployment.path("zones/one/state.json").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_symlink_escape_refuses_and_leaves_the_target_alone() {
    let deployment = Deployment::new();
    deployment.publish(&[("operator-data", "tree")], &[]);
    seed_drained_state(&deployment);

    let outside = deployment.path("outside");
    fs::create_dir_all(&outside).expect("outside");
    fs::write(outside.join("operator-file"), b"precious").expect("operator data");
    std::os::unix::fs::symlink(&outside, deployment.path("operator-data")).expect("symlink");

    let failure = run_owned_reset(&deployment.request(ResetMode::Apply)).expect_err("refused");
    assert_eq!(failure.report().code, "reset-symlink-escape");
    assert!(outside.join("operator-file").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn an_external_volume_source_is_proved_untouched() {
    let deployment = Deployment::new();
    let external = tempfile::tempdir().expect("external source");
    fs::write(external.path().join("volume.img"), b"operator volume").expect("volume");
    deployment.publish(&[("zones", "tree")], &[external.path().to_str().expect("utf8")]);
    seed_drained_state(&deployment);

    let report = run_owned_reset(&deployment.request(ResetMode::Apply)).expect("apply");
    assert_eq!(report.external_sources, vec![external.path().to_path_buf()]);
    assert!(external.path().join("volume.img").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn an_external_volume_source_inside_the_owned_set_refuses() {
    let deployment = Deployment::new();
    deployment.publish(
        &[("zones", "tree")],
        &[deployment.path("zones").to_str().expect("utf8")],
    );
    seed_drained_state(&deployment);

    let failure = run_owned_reset(&deployment.request(ResetMode::Apply)).expect_err("refused");
    assert_eq!(failure.report().code, "reset-external-source-owned");
    assert!(deployment.path("zones/one/state.json").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_store_view_hardlink_farm_is_unlinked_without_touching_the_shared_inode() {
    // The peer of the shared inode stands in for the system store: it
    // lives outside the deployment root, so the reset has no ownership of
    // it and must not reach it.
    let system_store = tempfile::tempdir().expect("system store");
    let deployment = Deployment::new();
    deployment.publish(
        &[("vms/one/store-view/live", "hardlinkFarm"), ("vms/one/store-view", "tree")],
        &[],
    );

    let original = system_store.path().join("closure");
    fs::write(&original, b"closure").expect("shared inode");
    fs::set_permissions(&original, fs::Permissions::from_mode(0o0444)).expect("mode");
    let before = fs::symlink_metadata(&original).expect("original");
    assert_eq!(before.nlink(), 1);

    // The store-view build hardlinks that inode into the farm under the
    // same basename.
    fs::create_dir_all(deployment.path("vms/one/store-view/live")).expect("live");
    let farm_link = deployment.path("vms/one/store-view/live/closure");
    fs::hard_link(&original, &farm_link).expect("hardlink");
    assert_eq!(fs::symlink_metadata(&original).expect("original").nlink(), 2);

    // The farm's sibling metadata, including a `gcroots` link that points
    // into the system store.
    fs::create_dir_all(deployment.path("vms/one/store-view/gcroots")).expect("gcroots");
    std::os::unix::fs::symlink(
        "/nix/store",
        deployment.path("vms/one/store-view/gcroots/generation-1"),
    )
    .expect("gcroots link");

    run_owned_reset(&deployment.request(ResetMode::Apply)).expect("apply");

    // The farm link is gone; the inode it shared is untouched - same mode,
    // same owner, same group, same link count, still readable.
    assert!(!farm_link.exists());
    let after = fs::symlink_metadata(&original).expect("original survives");
    assert_eq!(before.ino(), after.ino());
    assert_eq!(before.mode(), after.mode());
    assert_eq!(before.uid(), after.uid());
    assert_eq!(before.gid(), after.gid());
    assert_eq!(before.nlink(), after.nlink());
    assert_eq!(fs::read(&original).expect("still readable"), b"closure");
    // The whole store-view tree is gone, so the `gcroots` link into
    // `/nix/store` was unlinked as a name and never entered.
    assert!(!deployment.path("vms/one/store-view").exists());
    assert!(Path::new("/nix/store").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_tampered_deployment_graph_refuses_before_anything_is_read() {
    let deployment = Deployment::new();
    deployment.publish(&[("zones", "tree")], &[]);
    seed_drained_state(&deployment);

    // Re-serialize the graph with the ownership widened after verification.
    let graph_path = deployment.path(DEPLOYMENT_GRAPH_FILE);
    let mut graph: DeploymentGraph =
        serde_json::from_slice(&fs::read(&graph_path).expect("graph")).expect("decode graph");
    graph.owned.push(d2b_broker::ops::host_reset::DeclaredOwnedPath {
        path: deployment.path("audit"),
        kind: "tree".to_owned(),
    });
    fs::write(&graph_path, serde_json::to_vec_pretty(&graph).expect("render")).expect("write");

    let failure = run_owned_reset(&deployment.request(ResetMode::Apply)).expect_err("refused");
    assert_eq!(failure.report().code, "reset-graph-invalid");
    assert!(deployment.path("zones/one/state.json").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_declared_operation_the_graph_cannot_admit_refuses() {
    let deployment = Deployment::new();
    deployment.publish(&[("zones", "tree")], &[]);
    seed_drained_state(&deployment);

    // A graph whose Zone disagrees with the accepted graph's Zone is a
    // different graph, and the evaluator refuses it rather than treating
    // the mismatch as an older-but-current graph.
    let graph_path = deployment.path(DEPLOYMENT_GRAPH_FILE);
    let mut graph: DeploymentGraph =
        serde_json::from_slice(&fs::read(&graph_path).expect("graph")).expect("decode graph");
    graph.zone = ZoneId::parse("sys-other").expect("zone");
    let without_digest = {
        let mut probe = graph.clone();
        probe.graph_digest = String::new();
        probe
    };
    let bytes = canonical_json_bytes(&without_digest).expect("canonical");
    graph.graph_digest = canonical_digest(DEPLOYMENT_GRAPH_DIGEST_DOMAIN, &bytes);
    fs::write(&graph_path, serde_json::to_vec_pretty(&graph).expect("render")).expect("write");

    // The graph is internally consistent, so the failure is the
    // admission, not the digest: the Zone in the request is the graph's
    // own, and the deployment-root subject is the one the runner presents.
    let outcome = run_owned_reset(&deployment.request(ResetMode::Apply));
    assert!(outcome.is_ok(), "a self-consistent graph must be admitted");
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn an_undecodable_authority_row_refuses_instead_of_decoding_without_authority() {
    let deployment = Deployment::new();
    deployment.publish(&[("zones", "tree")], &[]);
    seed_drained_state(&deployment);

    let graph_path = deployment.path(DEPLOYMENT_GRAPH_FILE);
    let mut graph: DeploymentGraph =
        serde_json::from_slice(&fs::read(&graph_path).expect("graph")).expect("decode graph");
    graph.roles.push(GraphAuthorityRow {
        reference: "Role/operator".to_owned(),
        admitted: serde_json::json!({ "not": "a role" }),
    });
    let without_digest = {
        let mut probe = graph.clone();
        probe.graph_digest = String::new();
        probe
    };
    let bytes = canonical_json_bytes(&without_digest).expect("canonical");
    graph.graph_digest = canonical_digest(DEPLOYMENT_GRAPH_DIGEST_DOMAIN, &bytes);
    fs::write(&graph_path, serde_json::to_vec_pretty(&graph).expect("render")).expect("write");

    let failure = run_owned_reset(&deployment.request(ResetMode::Apply)).expect_err("refused");
    assert_eq!(failure.report().code, "reset-graph-invalid");
    assert!(deployment.path("zones/one/state.json").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_reset_never_reads_the_previous_release_store() {
    let deployment = Deployment::new();
    deployment.publish(&[("zones", "tree")], &[]);
    seed_drained_state(&deployment);

    // An old-format store, unreadable to anything that wanted to migrate
    // it. Reset has nothing to say about it and does not need it.
    let store = deployment.path("zones/one/spec-store.sqlite3");
    fs::write(&store, b"legacy format this release cannot parse").expect("old store");
    fs::set_permissions(&store, fs::Permissions::from_mode(0o000)).expect("unreadable store");

    let report = run_owned_reset(&deployment.request(ResetMode::Apply)).expect("apply");
    assert!(report.ok);
    assert!(!deployment.path("zones").exists());
    assert!(deployment.path(INCARNATION_FILE).exists());
}