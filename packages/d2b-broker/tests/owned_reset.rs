//! The one-shot ownership-bounded reset (U32, KTD15).
//!
//! Every case runs against a temporary deployment root, a temporary
//! cgroup tree, and the deployment document `nixos-modules/deployment-
//! bootstrap.nix` actually publishes. Nothing here touches a live host,
//! and nothing here opens a SpecStore: the clean break is the property
//! under test, so a test that needed the old store to set itself up would
//! be testing the migration this unit removed.

use std::fs;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use d2b_broker::ops::host_reset::{
    IncarnationRecord, RESET_OPERATION_REF, ResetMode, ResetReport, ResetRequest, run_owned_reset,
};
use d2b_core::deployment_bootstrap::{DEPLOYMENT_BOOTSTRAP_FILE, DeploymentBootstrap};
use d2b_core::resource_authority::{
    AcceptedGraph, GraphAuthority, GraphMutation, MutationKind, MutationSubjectEvidence,
    TransportIdentity,
};
use d2b_contracts_resource::v3::{
    AdmissionDecision, AdmissionStage, AuthoritySubject, AuthoritySubjectKind, RefusalReason,
    ResourceRef,
};

/// The Host publication the daemon installs, captured from the producer
/// rather than re-derived here, so what the reset reads in these tests is
/// what a real host publishes. Re-render it with:
///
/// ```text
/// nix eval --impure --raw \
///   --expr 'let d = import ./nixos-modules/deployment-bootstrap.nix \
///     { lib = (import <nixpkgs> {}).lib; }; in d.documentFor d.systemZone' \
///   --apply 'x: x.documentJson'
/// ```
const HOST_DEPLOYMENT_DOCUMENT: &str = concat!(
    r#"{"graphDigest":"sha256:511d95e68fb11afc6f782159baa72f7cefabde9ff72ee850a9960ae5e5c51811","#,
    r#""implementations":["activation-nixos","audio-binding","audio-service","credential","device","#,
    r#""device-security-key","device-usbip","endpoint","guest","host","network-local","process","#,
    r#""process-systemd","shell-pool","shell-session","user","volume","volume-binding","#,
    r#""wayland-policy","wayland-session"],"roleBindings":[{"admitted":{"roleRef":"#,
    r#""Role/operation-publisher","subjects":["Provider/system-minijail"]},"reference":"#,
    r#""RoleBinding/system-minijail-self-operation-publisher"}],"roles":[{"admitted":{"#,
    r#""operationRefs":[],"rules":[{"executionRefs":[],"resourceNames":[],"resourceTypes":["#,
    r#""Operation"],"sessionVerbs":[],"subresources":[],"verbs":["create"],"zones":[]}]},"#,
    r#""reference":"Role/operation-publisher"}],"schemaVersion":"d2b-deployment-bootstrap/1","#,
    r#""stateVolume":"Volume/d2b-state","storeIncarnation":"foundation-1","zone":"system"}"#
);

/// The identity the published document's accepted grants name.
const PUBLISHER: &str = "Provider/system-minijail";

/// The self-hash the published document carries.
const PUBLISHED_DIGEST: &str =
    "sha256:511d95e68fb11afc6f782159baa72f7cefabde9ff72ee850a9960ae5e5c51811";

/// The exact owned set this release declares, deployment-root-relative and
/// in declaration order. The reset's boundary is auditable, so the test
/// pins it: a surface added to or dropped from the declaration has to be a
/// deliberate edit here too.
const DECLARED_OWNED: &[&str] = &[
    "audit",
    "authority",
    "component-session",
    "current-bundle",
    "daemon-state",
    "guest-audit",
    "guest-broker",
    "guest-state",
    "host-generation",
    "host-generation-handoffs",
    "images",
    "keys",
    "locks",
    "runtime",
    "state-cells",
    "tmp",
    "validated",
    "vms",
    "zones",
];

/// One temporary deployment root with the published document in it, an
/// empty cgroup tree, and the drain probes in their declared layout.
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
        deployment.publish(HOST_DEPLOYMENT_DOCUMENT);
        deployment
    }

    /// Install the bytes a real installation puts at the deployment root.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn publish(&self, document: &str) {
        fs::write(self.path(DEPLOYMENT_BOOTSTRAP_FILE), document).expect("publish document");
    }

    /// Republish the Host document with one row rewritten and its self-hash
    /// recomputed: what a different deployment publishing a different
    /// document looks like. It is a different document, never an edit of
    /// this one after verification.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn republish_with(&self, edit: impl FnOnce(&mut DeploymentBootstrap)) {
        let mut document =
            DeploymentBootstrap::decode(HOST_DEPLOYMENT_DOCUMENT.as_bytes(), "captured")
                .expect("decode the captured document");
        edit(&mut document);
        document.graph_digest = document.seal().expect("seal the document");
        let bytes = serde_json::to_vec(&document).expect("render document");
        self.publish(std::str::from_utf8(&bytes).expect("utf8"));
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

    /// The deployment-root-relative names the report's inventory names.
    fn inventory(&self, report: &ResetReport) -> Vec<String> {
        report
            .inventory
            .iter()
            .map(|entry| {
                entry
                    .path
                    .strip_prefix(&self.root)
                    .expect("an owned path is beneath the deployment root")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }
}

/// Populate the state a drained-out previous deployment leaves behind.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn seed_drained_state(deployment: &Deployment) {
    for directory in [
        "audit",
        "authority",
        "state-cells",
        "host-generation-handoffs",
        "zones",
    ] {
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
    fs::create_dir_all(deployment.path("zones/one")).expect("zone");
    fs::write(deployment.path("zones/one/state.json"), b"{}").expect("zone state");
}

/// One declared inventory line, addressed by its deployment-root-relative
/// name.
fn entry<'a>(
    report: &'a ResetReport,
    relative: &str,
) -> &'a d2b_broker::ops::host_reset::ResetInventoryEntry {
    report
        .inventory
        .iter()
        .find(|entry| entry.path.ends_with(relative))
        .unwrap_or_else(|| panic!("{relative} is not in the declared inventory"))
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn apply_succeeds_against_the_document_the_deployment_actually_installs() {
    let deployment = Deployment::new();
    seed_drained_state(&deployment);

    let report = run_owned_reset(&deployment.request(ResetMode::Apply)).expect("apply");
    assert!(report.ok);
    assert_eq!(report.mode, "apply");
    // The report names the document it verified, the identity its accepted
    // grants admitted, and exactly the surfaces it removed.
    assert_eq!(report.zone, "system");
    assert_eq!(report.ownership_id, PUBLISHED_DIGEST);
    assert_eq!(report.admitted_subject, format!("provider:{PUBLISHER}"));
    assert_eq!(deployment.inventory(&report), DECLARED_OWNED);
    assert!(
        report
            .inventory
            .iter()
            .filter(|entry| entry.removed)
            .count()
            >= 5,
        "the seeded state is removed: {:?}",
        report.inventory
    );

    // The projection cursor/digest, the prepared fences, and the effect and
    // reservation journals are gone with the rest of the inventory.
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

    // The document the installation published is left byte for byte: it is
    // the installation's artifact, not runtime state, and the next boot has
    // to verify the very same bytes this reset admitted.
    assert_eq!(
        fs::read_to_string(deployment.path(DEPLOYMENT_BOOTSTRAP_FILE)).expect("document"),
        HOST_DEPLOYMENT_DOCUMENT
    );

    // A NEW incarnation, so the next boot initializes instead of
    // resynchronizing a rollback.
    let incarnation: IncarnationRecord =
        serde_json::from_slice(&fs::read(deployment.path("incarnation.json")).expect("incarnation"))
            .expect("decode incarnation");
    assert_eq!(incarnation.schema_version, "d2b-deployment-incarnation/1");
    assert_eq!(incarnation.zone.as_str(), "system");
    assert_eq!(
        incarnation.store_incarnation.as_str(),
        "foundation-2",
        "the fresh root must not reuse the document's incarnation"
    );
    assert_eq!(incarnation.previous_bootstrap_digest, PUBLISHED_DIGEST);
    assert_eq!(report.store_incarnation, "foundation-2");
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn inspect_reports_the_exact_inventory_and_removes_nothing() {
    let deployment = Deployment::new();
    seed_drained_state(&deployment);

    let report = run_owned_reset(&deployment.request(ResetMode::Inspect)).expect("inspect");
    assert!(report.ok);
    assert_eq!(report.mode, "inspect");
    assert_eq!(report.ownership_id, PUBLISHED_DIGEST);
    assert_eq!(deployment.inventory(&report), DECLARED_OWNED);
    // The declared surfaces that exist are reported present; the ones this
    // deployment never created are reported absent, and neither is removed.
    assert!(entry(&report, "zones").present);
    assert!(entry(&report, "authority").present);
    assert!(entry(&report, "images").present == false);
    assert!(report.inventory.iter().all(|entry| !entry.removed));
    // The half that removes is the half that did not.
    assert!(deployment.path("zones/one/state.json").exists());
    assert!(deployment.path("authority/state.json").exists());
    assert!(!deployment.path("incarnation.json").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_repeated_completed_reset_is_safe() {
    let deployment = Deployment::new();
    seed_drained_state(&deployment);

    let first = run_owned_reset(&deployment.request(ResetMode::Apply)).expect("first apply");
    assert!(entry(&first, "zones").present && entry(&first, "zones").removed);
    assert!(entry(&first, "state-cells").present && entry(&first, "state-cells").removed);
    assert!(!entry(&first, "images").present && !entry(&first, "images").removed);

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
fn a_document_edited_after_verification_is_refused_before_anything_is_read() {
    let deployment = Deployment::new();
    seed_drained_state(&deployment);

    // Re-serialize the document with the owned deployment renamed after
    // verification, leaving the self-hash covering the original bytes.
    let mut document: serde_json::Value =
        serde_json::from_str(HOST_DEPLOYMENT_DOCUMENT).expect("decode");
    document["stateVolume"] = serde_json::json!("Volume/somebody-elses");
    deployment.publish(&serde_json::to_string(&document).expect("render"));

    let failure = run_owned_reset(&deployment.request(ResetMode::Apply)).expect_err("refused");
    assert_eq!(failure.report().code, "reset-document-invalid");
    assert!(deployment.path("zones/one/state.json").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_root_with_no_deployment_document_is_not_a_deployment_root() {
    let deployment = Deployment::new();
    seed_drained_state(&deployment);
    fs::remove_file(deployment.path(DEPLOYMENT_BOOTSTRAP_FILE)).expect("remove document");

    let failure = run_owned_reset(&deployment.request(ResetMode::Apply)).expect_err("refused");
    assert_eq!(failure.report().code, "reset-io");
    assert!(deployment.path("zones/one/state.json").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_deployment_that_grants_the_reset_to_nobody_refuses_and_changes_nothing() {
    let deployment = Deployment::new();
    seed_drained_state(&deployment);

    // A different deployment, publishing a document that verifies over its
    // own bytes and grants no one. The reset may not fall back to its own
    // authority when the deployment names none.
    deployment.republish_with(|document| {
        document.roles.clear();
        document.role_bindings.clear();
    });
    let published = fs::read_to_string(deployment.path(DEPLOYMENT_BOOTSTRAP_FILE)).expect("doc");

    let failure = run_owned_reset(&deployment.request(ResetMode::Apply)).expect_err("refused");
    assert_eq!(failure.report().code, "reset-not-admitted");
    assert!(deployment.path("zones/one/state.json").exists());
    assert_eq!(
        fs::read_to_string(deployment.path(DEPLOYMENT_BOOTSTRAP_FILE)).expect("document"),
        published,
        "a document this reset did not admit is left exactly as its owner wrote it"
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_deployment_that_grants_only_another_operation_refuses_the_reset() {
    let deployment = Deployment::new();
    seed_drained_state(&deployment);

    // The same deployment, publishing a Role that covers exactly one other
    // Operation. The reset's target is pinned to `Operation/local-reset`, so
    // a grant that does not name it admits nothing.
    deployment.republish_with(|document| {
        document.roles[0].admitted = serde_json::json!({
            "operationRefs": [],
            "rules": [{
                "executionRefs": [],
                "resourceNames": ["some-other-operation"],
                "resourceTypes": ["Operation"],
                "sessionVerbs": [],
                "subresources": [],
                "verbs": ["create"],
                "zones": [],
            }],
        });
    });

    let failure = run_owned_reset(&deployment.request(ResetMode::Apply)).expect_err("refused");
    assert_eq!(failure.report().code, "reset-not-admitted");
    assert!(deployment.path("zones/one/state.json").exists());
    assert_eq!(
        RESET_OPERATION_REF, "Operation/local-reset",
        "the target the evaluator is asked about is the whole reset's scope"
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn an_undecodable_authority_row_refuses_instead_of_decoding_without_authority() {
    let deployment = Deployment::new();
    seed_drained_state(&deployment);

    deployment.republish_with(|document| {
        document.roles[0].admitted = serde_json::json!({ "not": "a role" });
    });

    let failure = run_owned_reset(&deployment.request(ResetMode::Apply)).expect_err("refused");
    assert_eq!(failure.report().code, "reset-document-invalid");
    assert!(deployment.path("zones/one/state.json").exists());
}

/// The acceptance case the admission has to express: the same verified
/// document, the same accepted graph, and two DIFFERENT identities - the
/// one its `RoleBinding` names and one it does not.
///
/// Both are asked the same question through the same evaluator, so the
/// answer can only come from the accepted rows. A comparison that passed
/// because the request's subject and the graph's root were the same value
/// would answer this identically for both identities.
#[test]
fn the_published_grant_admits_one_identity_and_refuses_another() {
    let document = DeploymentBootstrap::decode(HOST_DEPLOYMENT_DOCUMENT.as_bytes(), "captured")
        .expect("the published document verifies");
    let accepted: AcceptedGraph = document.accepted_graph().expect("accepted graph");
    let target = ResourceRef::parse(RESET_OPERATION_REF).expect("reset operation");
    let ask = |principal: &str| {
        let subject = AuthoritySubject::named(
            AuthoritySubjectKind::Provider,
            ResourceRef::parse(principal).expect("provider reference"),
        );
        let request = GraphMutation::new(
            document.zone.clone(),
            MutationSubjectEvidence::new(subject, TransportIdentity::OperatorConsole),
            MutationKind::Create,
            target.clone(),
        );
        GraphAuthority::admit_mutation(&request, &accepted)
    };

    assert_eq!(ask(PUBLISHER), AdmissionDecision::Admitted);
    for stranger in [
        "Provider/system-minijail-other",
        "Provider/other",
        "Provider/system-minijail-2",
    ] {
        assert_eq!(
            ask(stranger),
            AdmissionDecision::refuse(AdmissionStage::Authorize, RefusalReason::IdentityNotAuthorized),
            "{stranger} holds no accepted grant"
        );
    }
    // The deployment root itself is the document, and it is not the identity
    // the reset acts as: the two sides of the decision are separate.
    assert_eq!(
        accepted.root_subject(),
        &AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap)
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_live_workload_prevents_the_reset_and_leaves_the_state_alone() {
    let deployment = Deployment::new();
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
    seed_drained_state(&deployment);

    fs::create_dir_all(deployment.path("host-generation")).expect("host generation");
    fs::write(deployment.path("host-generation/active"), "generation-9\n").expect("active");

    let failure = run_owned_reset(&deployment.request(ResetMode::Apply)).expect_err("refused");
    assert_eq!(failure.report().code, "reset-host-generation-active");
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_symlink_standing_on_an_owned_surface_refuses_and_leaves_its_target_alone() {
    let deployment = Deployment::new();
    seed_drained_state(&deployment);

    // A declared directory replaced by a link out of the deployment root.
    let outside = deployment.path("outside");
    fs::create_dir_all(&outside).expect("outside");
    fs::write(outside.join("operator-file"), b"precious").expect("operator data");
    fs::remove_dir_all(deployment.path("zones")).expect("clear the surface");
    std::os::unix::fs::symlink(&outside, deployment.path("zones")).expect("symlink");

    let failure = run_owned_reset(&deployment.request(ResetMode::Apply)).expect_err("refused");
    assert_eq!(failure.report().code, "reset-symlink-escape");
    assert!(outside.join("operator-file").exists());
    assert!(deployment.path("audit/host.jsonl").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn nothing_outside_the_deployment_root_is_reached() {
    let deployment = Deployment::new();
    seed_drained_state(&deployment);

    // A sibling of the deployment root that happens to carry the same
    // surface names is not this deployment's, and the boundary is the
    // deployment root rather than a name search.
    let sibling = deployment.path("../d2b-operator-volume");
    fs::create_dir_all(sibling.join("zones")).expect("operator volume");
    fs::write(sibling.join("zones/state.json"), b"operator data").expect("operator state");

    run_owned_reset(&deployment.request(ResetMode::Apply)).expect("apply");
    assert!(sibling.join("zones/state.json").exists());
}

/// The reset's boundary is the list, not the deployment root. A surface the
/// declaration does not name is not this runner's to remove, whether it sits
/// beside the root or inside it.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_surface_the_declaration_does_not_name_is_never_unlinked() {
    let deployment = Deployment::new();
    seed_drained_state(&deployment);

    // A family-owned subtree this shared crate may not name. It lives inside
    // the deployment root and is not in the declared set, so the reset leaves
    // it alone rather than guessing at who owns it.
    let family_state = deployment.path("device-families/state");
    fs::create_dir_all(&family_state).expect("family state");
    fs::write(family_state.join("identity"), b"family-owned").expect("family identity");

    let report = run_owned_reset(&deployment.request(ResetMode::Apply)).expect("apply");
    assert!(
        !report
            .inventory
            .iter()
            .any(|declared| declared.path.starts_with(&family_state)),
        "an undeclared surface must not appear in the inventory at all"
    );
    assert_eq!(
        fs::read(family_state.join("identity")).expect("family identity"),
        b"family-owned"
    );
    // The surfaces the declaration does name are still removed: the
    // undeclared sibling narrows the inventory rather than replacing it.
    assert!(!deployment.path("zones").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_store_view_hardlink_farm_is_unlinked_without_touching_the_shared_inode() {
    // The peer of the shared inode stands in for the system store: it
    // lives outside the deployment root, so the reset has no ownership of
    // it and must not reach it.
    let system_store = tempfile::tempdir().expect("system store");
    let deployment = Deployment::new();

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
    assert!(!deployment.path("vms").exists());
    assert!(Path::new("/nix/store").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_reset_never_reads_the_previous_release_store() {
    let deployment = Deployment::new();
    seed_drained_state(&deployment);

    // An old-format store, unreadable to anything that wanted to migrate
    // it. Reset has nothing to say about it and does not need it.
    let store = deployment.path("zones/one/spec-store.sqlite3");
    fs::write(&store, b"legacy format this release cannot parse").expect("old store");
    fs::set_permissions(&store, fs::Permissions::from_mode(0o000)).expect("unreadable store");

    let report = run_owned_reset(&deployment.request(ResetMode::Apply)).expect("apply");
    assert!(report.ok);
    assert!(!deployment.path("zones").exists());
    assert!(deployment.path("incarnation.json").exists());
}
