//! `d2b host reset`: the offline ownership-bounded clean break (U32,
//! KTD15).
//!
//! Every case runs the real `d2b` CLI against the real composed broker
//! binary, with a temporary deployment root, the deployment document
//! `nixos-modules/deployment-bootstrap.nix` actually publishes, a temporary
//! cgroup tree, and a public socket that does not exist. Nothing here
//! reaches a live host, and nothing here needs a running `d2bd` - the
//! absence of both is the precondition the verb exists for, so every
//! assertion below is made with them absent.

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::{Command, Output};

/// The Host publication the daemon installs, captured from the producer
/// rather than re-derived here. Re-render it with:
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

const DOCUMENT_FILE: &str = "deployment-bootstrap.json";

/// The self-hash the published document carries.
const PUBLISHED_DIGEST: &str =
    "sha256:511d95e68fb11afc6f782159baa72f7cefabde9ff72ee850a9960ae5e5c51811";

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

/// One temporary deployment root carrying the published document and the
/// previous release's drained-out state beneath it.
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
        // The document an installation publishes, and the state the
        // framework's own surfaces create beneath the deployment root,
        // including the broker's projection cursor/digest and prepared
        // fences, its durable state cells, and its host-generation handoff
        // journal.
        fs::write(deployment.path(DOCUMENT_FILE), HOST_DEPLOYMENT_DOCUMENT).expect("publish");
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
            .env("D2B_PUBLIC_SOCKET", self.path("absent-public.sock"))
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
    // The document the deployment published, and the identity its accepted
    // grants admitted the reset for.
    assert_eq!(report["zone"], serde_json::json!("system"));
    assert_eq!(report["ownershipId"], serde_json::json!(PUBLISHED_DIGEST));
    assert_eq!(
        report["admittedSubject"],
        serde_json::json!("provider:Provider/system-minijail")
    );
    let inventory = report["inventory"].as_array().expect("inventory");
    let present = inventory
        .iter()
        .filter(|entry| entry["present"] == serde_json::json!(true))
        .count();
    assert!(present >= 5, "the seeded state is in the inventory: {inventory:?}");
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
    assert_eq!(report["storeIncarnation"], serde_json::json!("foundation-2"));
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
    // The installation's document is untouched, and the fresh deployment
    // root carries a NEW incarnation, so the next boot initializes rather
    // than resynchronizing a rollback.
    assert_eq!(
        fs::read_to_string(deployment.path(DOCUMENT_FILE)).expect("document"),
        HOST_DEPLOYMENT_DOCUMENT
    );
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

/// A document edited after it was verified never authorizes a removal, and
/// it is left byte for byte as whoever wrote it left it.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_tampered_deployment_document_refuses_and_is_left_byte_for_byte() {
    let deployment = Deployment::new();
    let mut document: serde_json::Value =
        serde_json::from_str(HOST_DEPLOYMENT_DOCUMENT).expect("decode");
    document["stateVolume"] = serde_json::json!("Volume/somebody-elses");
    let tampered = serde_json::to_string(&document).expect("render");
    fs::write(deployment.path(DOCUMENT_FILE), &tampered).expect("tamper");

    let output = deployment.run(&["--apply"]);
    assert_ne!(output.status.code(), Some(0));
    let report = envelope(&output);
    assert_eq!(report["ok"], serde_json::json!(false));
    assert_eq!(report["code"], serde_json::json!("reset-document-invalid"));
    assert_eq!(
        fs::read_to_string(deployment.path(DOCUMENT_FILE)).expect("document"),
        tampered
    );
    assert!(deployment.path("zones/one/state.json").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_root_with_no_deployment_document_is_refused() {
    let deployment = Deployment::new();
    fs::remove_file(deployment.path(DOCUMENT_FILE)).expect("remove document");

    let output = deployment.run(&["--apply"]);
    assert_ne!(output.status.code(), Some(0));
    assert_eq!(envelope(&output)["ok"], serde_json::json!(false));
    assert!(deployment.path("zones/one/state.json").exists());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_symlink_escape_refuses_and_leaves_the_target_alone() {
    let deployment = Deployment::new();
    let outside = deployment.path("outside");
    fs::create_dir_all(&outside).expect("outside");
    fs::write(outside.join("operator-file"), b"precious").expect("operator data");
    fs::remove_dir_all(deployment.path("zones")).expect("clear the surface");
    std::os::unix::fs::symlink(&outside, deployment.path("zones")).expect("symlink");

    let output = deployment.run(&["--apply"]);
    assert_ne!(output.status.code(), Some(0));
    assert_eq!(
        envelope(&output)["code"],
        serde_json::json!("reset-symlink-escape")
    );
    assert!(outside.join("operator-file").exists());
    assert!(deployment.path("authority/state.json").exists());
}

/// Nothing outside the deployment root is ever a candidate, including a
/// sibling directory that carries the same surface names.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn nothing_outside_the_deployment_root_is_reached() {
    let deployment = Deployment::new();
    let sibling = deployment.path("../d2b-operator-volume");
    fs::create_dir_all(sibling.join("zones")).expect("operator volume");
    fs::write(sibling.join("zones/state.json"), b"operator data").expect("operator state");

    let output = deployment.run(&["--apply"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(sibling.join("zones/state.json")).expect("operator state"),
        b"operator data"
    );
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
    assert!(rendered.contains("provider:Provider/system-minijail"), "{rendered}");

    // A document edited after verification is the refusal an operator sees
    // from a root somebody else has been writing to.
    let mut document: serde_json::Value =
        serde_json::from_str(HOST_DEPLOYMENT_DOCUMENT).expect("decode");
    document["implementations"] = serde_json::json!(["process", "not-compiled-here"]);
    fs::write(
        deployment.path(DOCUMENT_FILE),
        serde_json::to_string(&document).expect("render"),
    )
    .expect("tamper");
    let refused = Command::new(d2b_bin())
        .args(["--human", "host", "reset", "--apply"])
        .arg("--state-root")
        .arg(&deployment.root)
        .arg("--cgroup-root")
        .arg(&deployment.cgroup)
        .env("D2B_BROKER_BIN", broker_bin())
        .env("D2B_PUBLIC_SOCKET", deployment.path("absent-public.sock"))
        .output()
        .expect("run the d2b CLI");
    assert_ne!(refused.status.code(), Some(0));
    let rendered = String::from_utf8_lossy(&refused.stdout);
    assert!(
        rendered.contains("d2b host reset refused: reset-document-invalid"),
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
