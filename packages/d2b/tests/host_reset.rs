//! `d2b host reset`: the offline ownership-bounded clean break (U32,
//! KTD15).
//!
//! Every case runs the real `d2b` CLI against the real composed broker
//! binary, with a temporary deployment root, temporary markers, a
//! temporary cgroup tree, and a public socket that does not exist. Nothing
//! here reaches a live host, and nothing here needs a running `d2bd` - the
//! absence of both is the precondition the verb exists for, so every
//! assertion below is made with them absent.

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::{Command, Output};

use d2b_contracts_resource::v3::{
    StoreIncarnation, ZoneId, canonical_digest, canonical_json_bytes,
};

/// The deployment-graph document shape the one-shot runner verifies.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeploymentGraph {
    schema_version: String,
    zone: ZoneId,
    store_incarnation: StoreIncarnation,
    ownership_id: String,
    owned: Vec<DeclaredOwnedPath>,
    external_sources: Vec<PathBuf>,
    roles: Vec<serde_json::Value>,
    role_bindings: Vec<serde_json::Value>,
    graph_digest: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DeclaredOwnedPath {
    path: PathBuf,
    kind: String,
}

const DEPLOYMENT_GRAPH_DIGEST_DOMAIN: &str = "d2b:v3:deployment-graph";

/// The CLI binary under test, as Cargo or Bazel hands it to the test.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn d2b_bin() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_d2b")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/debug/d2b"))
}

/// The composed broker binary the CLI spawns for the one-shot runner.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn broker_bin() -> PathBuf {
    std::env::var_os("D2B_BROKER_BIN")
        .map(PathBuf::from)
        .expect("D2B_BROKER_BIN must name the composed broker for this oracle")
}

const OWNERSHIP_ID: &str = "host:d2b";
const ZONE: &str = "sys-host";
const STORE: &str = "store-3";

/// One temporary deployment root with the new model's graph published in
/// it and the previous release's drained-out state beneath it.
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
            deployment.root.join("d2b-ownership"),
            format!(
                "# d2b-managed begin\n# d2b managed: {OWNERSHIP_ID}\n# d2b-managed end\n"
            ),
        )
        .expect("ownership marker");
        // The verified new graph declares the whole owned set, including
        // the broker's projection cursor/digest and prepared fences, its
        // durable state cells (the effect and reservation journals), and
        // its host-generation handoff journal.
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
        for directory in [
            "audit",
            "authority",
            "state-cells",
            "host-generation-handoffs",
        ] {
            fs::create_dir_all(deployment.path(directory)).expect("state dir");
        }
        fs::write(deployment.path("audit/host.jsonl"), b"{}\n").expect("audit record");
        fs::write(deployment.path("authority/state.json"), b"{\"cursor\":1}").expect("authority");
        fs::write(deployment.path("state-cells/cells.json"), b"{\"cells\":[]}").expect("cells");
        fs::create_dir_all(deployment.path("zones/one")).expect("zone");
        fs::write(deployment.path("zones/one/state.json"), b"{}").expect("zone state");
        deployment
    }

    /// Publish the verified new deployment graph, self-hashed exactly as
    /// the broker's own contract requires: the digest covers the canonical
    /// bytes of the document with its own `graphDigest` cleared.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn publish(&self, owned: &[(&str, &str)], external_sources: &[&str]) {
        let mut graph = DeploymentGraph {
            schema_version: "d2b-deployment-graph/1".to_owned(),
            zone: ZoneId::parse(ZONE).expect("zone"),
            store_incarnation: StoreIncarnation::parse(STORE).expect("store"),
            ownership_id: OWNERSHIP_ID.to_owned(),
            owned: owned
                .iter()
                .map(|(relative, kind)| DeclaredOwnedPath {
                    path: self.path(relative),
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
        let bytes = canonical_json_bytes(&graph).expect("canonical graph");
        graph.graph_digest = canonical_digest(DEPLOYMENT_GRAPH_DIGEST_DOMAIN, &bytes);
        fs::write(
            self.root.join("deployment-graph.json"),
            serde_json::to_vec_pretty(&graph).expect("render graph"),
        )
        .expect("publish graph");
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// Run the real CLI against this deployment root.
    ///
    /// `D2B_PUBLIC_SOCKET` points at a path that does not exist, so any
    /// accidental Zone resolution fails loudly instead of reaching a
    /// daemon that happens to be running on the test host.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn run(&self, args: &[&str]) -> Output {
        Command::new(d2b_bin())
            .args(["--json", "host", "reset"])
            .args(args)
            .arg("--state-root")
            .arg(&self.root)
            .arg("--cgroup-root")
            .arg(&self.cgroup)
            .env("D2B_BROKER_BIN", broker_bin())
            .env("D2B_PUBLIC_SOCKET", self.root.join("absent-public.sock"))
            .env("D2B_ZONE", ZONE)
            .env_remove("D2B_BROKER_SOCKET_PATH")
            .output()
            .expect("run the d2b CLI")
    }
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn envelope(output: &Output) -> serde_json::Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "the CLI did not render one JSON envelope: {error}\nstdout:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_missing_mutation_mode_is_refused_with_no_daemon_and_no_broker() {
    let deployment = Deployment::new();
    // The socket path does not exist and `D2B_BROKER_BIN` is deliberately
    // unset by pointing it at a path that does not exist, so an invocation
    // that resolved a Zone or ran the runner would fail differently.
    let output = Command::new(d2b_bin())
        .args(["--json", "host", "reset"])
        .arg("--state-root")
        .arg(&deployment.root)
        .env("D2B_BROKER_BIN", deployment.path("absent-broker"))
        .env("D2B_PUBLIC_SOCKET", deployment.path("absent-public.sock"))
        .output()
        .expect("run the d2b CLI");

    assert_eq!(
        output.status.code(),
        Some(78),
        "the missing-mode refusal is owed before anything else"
    );
    let rendered = String::from_utf8_lossy(&output.stdout);
    assert!(rendered.contains("--apply-or-dry-run-required"), "{rendered}");
    // Refused before the runner was reachable, so nothing was touched.
    assert!(deployment.path("zones/one/state.json").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn dry_run_reports_the_exact_inventory_and_removes_nothing() {
    let deployment = Deployment::new();
    let output = deployment.run(&["--dry-run"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let report = envelope(&output);
    assert_eq!(report["ok"], serde_json::json!(true));
    assert_eq!(report["mode"], serde_json::json!("inspect"));
    let inventory = report["inventory"].as_array().expect("inventory");
    assert_eq!(inventory.len(), 5);
    assert!(inventory.iter().all(|entry| entry["present"] == serde_json::json!(true)));
    assert!(inventory.iter().all(|entry| entry["removed"] == serde_json::json!(false)));
    // The inspect half inspected; it did not remove.
    assert!(deployment.path("zones/one/state.json").exists());
    assert!(deployment.path("authority/state.json").exists());
    assert!(!deployment.path("incarnation.json").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn apply_succeeds_with_the_daemon_stopped_and_an_unreadable_old_store() {
    let deployment = Deployment::new();
    // An old-format store the previous release could parse and this one
    // cannot, made unreadable so even an accidental reader would fail.
    let store = deployment.path("zones/one/spec-store.sqlite3");
    fs::write(&store, b"legacy format this release cannot parse").expect("old store");
    fs::set_permissions(&store, fs::Permissions::from_mode(0o000)).expect("unreadable store");

    let output = deployment.run(&["--apply"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let report = envelope(&output);
    assert_eq!(report["mode"], serde_json::json!("apply"));
    assert_eq!(report["storeIncarnation"], serde_json::json!("store-4"));
    let inventory = report["inventory"].as_array().expect("inventory");
    assert!(inventory.iter().all(|entry| entry["removed"] == serde_json::json!(true)));
    // The projection cursor/digest and prepared fences, the effect and
    // reservation journals, and the host-generation handoff journal are
    // gone with the rest of the inventory.
    for gone in [
        "zones",
        "audit",
        "authority",
        "state-cells",
        "host-generation-handoffs",
    ] {
        assert!(!deployment.path(gone).exists(), "{gone} survived the reset");
    }
    // The fresh deployment root carries a NEW incarnation, so the next
    // boot initializes rather than resynchronizing a rollback.
    assert!(deployment.path("incarnation.json").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_repeated_completed_reset_is_safe() {
    let deployment = Deployment::new();
    assert_eq!(
        deployment.run(&["--apply"]).status.code(),
        Some(0),
        "first apply"
    );
    let second = deployment.run(&["--apply"]);
    assert_eq!(second.status.code(), Some(0), "{}", String::from_utf8_lossy(&second.stderr));
    let report = envelope(&second);
    let inventory = report["inventory"].as_array().expect("inventory");
    assert!(inventory.iter().all(|entry| entry["present"] == serde_json::json!(false)));
    assert!(inventory.iter().all(|entry| entry["removed"] == serde_json::json!(false)));
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_foreign_marker_refuses_and_is_left_byte_for_byte() {
    let deployment = Deployment::new();
    let foreign = "# d2b-managed begin\n# d2b managed: host:somebody-else\n# d2b-managed end\n";
    fs::write(deployment.path("d2b-ownership"), foreign).expect("foreign marker");

    let output = deployment.run(&["--apply"]);
    assert_ne!(output.status.code(), Some(0));
    let report = envelope(&output);
    assert_eq!(report["ok"], serde_json::json!(false));
    assert_eq!(report["code"], serde_json::json!("reset-foreign-ownership-marker"));
    // A foreign marker never authorizes an overwrite.
    assert_eq!(
        fs::read_to_string(deployment.path("d2b-ownership")).expect("marker"),
        foreign
    );
    assert!(deployment.path("zones/one/state.json").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_symlink_escape_refuses_and_leaves_the_target_alone() {
    let deployment = Deployment::new();
    deployment.publish(&[("operator-data", "tree")], &[]);
    let outside = deployment.path("outside");
    fs::create_dir_all(&outside).expect("outside");
    fs::write(outside.join("operator-file"), b"precious").expect("operator data");
    std::os::unix::fs::symlink(&outside, deployment.path("operator-data")).expect("symlink");

    let output = deployment.run(&["--apply"]);
    assert_ne!(output.status.code(), Some(0));
    assert_eq!(
        envelope(&output)["code"],
        serde_json::json!("reset-symlink-escape")
    );
    assert!(outside.join("operator-file").exists());
    assert!(deployment.path("zones/one/state.json").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn an_external_volume_source_is_reported_untouched() {
    let deployment = Deployment::new();
    let external = tempfile::tempdir().expect("external source");
    fs::write(external.path().join("volume.img"), b"operator volume").expect("volume");
    deployment.publish(
        &[("zones", "tree")],
        &[external.path().to_str().expect("utf8")],
    );

    let output = deployment.run(&["--dry-run"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        envelope(&output)["externalSources"],
        serde_json::json!([external.path().to_string_lossy()])
    );
    assert_eq!(fs::read(external.path().join("volume.img")).expect("volume"), b"operator volume");
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_live_workload_refuses_the_apply_before_anything_is_removed() {
    let deployment = Deployment::new();
    let live = deployment.cgroup.join("zones/one/guest");
    fs::create_dir_all(&live).expect("live cgroup");
    fs::write(live.join("cgroup.procs"), "4242\n").expect("members");

    let output = deployment.run(&["--apply"]);
    assert_ne!(output.status.code(), Some(0));
    assert_eq!(
        envelope(&output)["code"],
        serde_json::json!("reset-workload-live")
    );
    // A reset that refuses but has already removed state is worse than one
    // that never acted.
    assert!(deployment.path("zones/one/state.json").exists());
    assert!(deployment.path("authority/state.json").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_human_invocation_renders_the_inventory_and_the_refusal() {
    let deployment = Deployment::new();
    let planned = Command::new(d2b_bin())
        .args(["--human", "host", "reset", "--dry-run"])
        .arg("--state-root")
        .arg(&deployment.root)
        .arg("--cgroup-root")
        .arg(&deployment.cgroup)
        .env("D2B_BROKER_BIN", broker_bin())
        .env("D2B_PUBLIC_SOCKET", deployment.path("absent-public.sock"))
        .output()
        .expect("run the d2b CLI");
    assert_eq!(planned.status.code(), Some(0));
    let rendered = String::from_utf8_lossy(&planned.stdout);
    assert!(rendered.contains("ownership boundary verified (inspect)"), "{rendered}");
    assert!(rendered.contains("[tree]"), "{rendered}");
    assert!(rendered.contains("(present)"), "{rendered}");

    fs::write(
        deployment.path("d2b-ownership"),
        "# d2b-managed begin\n# d2b managed: host:somebody-else\n# d2b-managed end\n",
    )
    .expect("foreign marker");
    let refused = Command::new(d2b_bin())
        .args(["--human", "host", "reset", "--apply"])
        .arg("--state-root")
        .arg(&deployment.root)
        .env("D2B_BROKER_BIN", broker_bin())
        .env("D2B_PUBLIC_SOCKET", deployment.path("absent-public.sock"))
        .output()
        .expect("run the d2b CLI");
    assert_ne!(refused.status.code(), Some(0));
    let rendered = String::from_utf8_lossy(&refused.stdout);
    assert!(
        rendered.contains("d2b host reset refused: reset-foreign-ownership-marker"),
        "{rendered}"
    );
    assert!(rendered.contains("nothing was removed"), "{rendered}");
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn the_reset_verb_is_in_the_parser_and_the_host_surface() {
    // The verb is part of the published command surface, and it is a
    // subcommand of `host` rather than a new top-level namespace.
    let help = Command::new(d2b_bin())
        .args(["host", "--help"])
        .output()
        .expect("run the d2b CLI");
    assert!(help.status.success());
    let rendered = String::from_utf8_lossy(&help.stdout);
    assert!(rendered.contains("reset"), "{rendered}");

    // It belongs to `host`, not to the top-level registry: the top-level
    // help would list it among the built-in commands.
    let top = Command::new(d2b_bin())
        .args(["--help"])
        .output()
        .expect("run the d2b CLI");
    assert!(top.status.success());
    let top_rendered = String::from_utf8_lossy(&top.stdout);
    assert!(
        !top_rendered.contains("reset"),
        "reset must not have become a top-level command:\n{top_rendered}"
    );
}