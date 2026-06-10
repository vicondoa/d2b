//! End-to-end test for the `nixling-activation-helper
//! build-store-view-farm` verb: the privileged broker serialises a
//! [`BuildStoreViewFarmRequest`] to this binary's stdin (under an
//! `unshare --mount` + `umount -l /nix/store` wrapper on a real host),
//! and the verb deserialises it and runs `build_farm`. This test drives
//! the binary directly (same-filesystem `tempdir`, so no namespace is
//! needed) to lock the wire contract + the typed-error-on-stdout
//! protocol the broker relies on.

use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use nixling_host::hardlink_farm::{BuildStoreViewFarmRequest, GenerationMarker, HardlinkFarmError};
use tempfile::tempdir;

fn fake_closure(root: &Path, n: usize) -> Vec<PathBuf> {
    let store = root.join("nix-store-mock");
    std::fs::create_dir_all(&store).unwrap();
    let mut out = Vec::new();
    for i in 0..n {
        let dir = store.join(format!("aaaaaaaaaaaaaaaa-fake-{i}"));
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin").join("payload"), format!("data-{i}")).unwrap();
        out.push(dir);
    }
    out
}

fn marker(closure_hash: &str) -> GenerationMarker {
    GenerationMarker {
        closure_hash: closure_hash.to_owned(),
        nixling_version: "test".to_owned(),
        activated_at: "unix-0".to_owned(),
        vm: "vm-a".to_owned(),
        generation_number: 1,
    }
}

fn run_helper(request: &BuildStoreViewFarmRequest) -> std::process::Output {
    let payload = serde_json::to_vec(request).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_nixling-activation-helper"))
        .arg("build-store-view-farm")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn helper");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&payload)
        .expect("write request");
    child.wait_with_output().expect("await helper")
}

#[test]
fn build_store_view_farm_verb_populates_farm_from_stdin_request() {
    let tmp = tempdir().unwrap();
    let farm_root = tmp.path().join("vms/vm-a/store-view");
    std::fs::create_dir_all(&farm_root).unwrap();
    let closure = fake_closure(tmp.path(), 2);

    let request = BuildStoreViewFarmRequest {
        farm_root: farm_root.clone(),
        generation: 1,
        closure_paths: closure.clone(),
        marker: marker("closure-xyz"),
    };
    let output = run_helper(&request);
    assert!(
        output.status.success(),
        "helper failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let gen_dir = farm_root.join("generations/1");
    assert!(gen_dir.join("marker.json").exists(), "marker written");
    assert!(gen_dir.join("store-paths").exists(), "store-paths written");
    let farmed = farm_root.join("live/aaaaaaaaaaaaaaaa-fake-0/bin/payload");
    let src = closure[0].join("bin/payload");
    assert!(farmed.exists(), "closure file hardlinked into farm");
    assert_eq!(
        std::fs::metadata(&farmed).unwrap().ino(),
        std::fs::metadata(&src).unwrap().ino(),
        "farm entry shares the source inode (hardlink, not copy)",
    );
}

#[test]
fn private_store_requires_nested_verb_before_unshare() {
    let output = Command::new(env!("CARGO_BIN_EXE_nixling-activation-helper"))
        .arg("private-store")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn helper");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("missing verb"),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn build_store_view_farm_verb_emits_typed_error_json_on_collision() {
    let tmp = tempdir().unwrap();
    let farm_root = tmp.path().join("vms/vm-a/store-view");
    std::fs::create_dir_all(&farm_root).unwrap();
    let closure = fake_closure(tmp.path(), 1);

    // First build at generation 1 with closure hash "first".
    let first = BuildStoreViewFarmRequest {
        farm_root: farm_root.clone(),
        generation: 1,
        closure_paths: closure.clone(),
        marker: marker("first"),
    };
    assert!(run_helper(&first).status.success());

    // Re-build the SAME generation number with a DIFFERENT closure hash
    // -> collision. The verb must exit non-zero AND emit the typed
    // HardlinkFarmError as JSON on stdout so the broker recovers it.
    let collide = BuildStoreViewFarmRequest {
        farm_root,
        generation: 1,
        closure_paths: closure,
        marker: marker("second"),
    };
    let output = run_helper(&collide);
    assert!(!output.status.success(), "collision must fail");
    let line = String::from_utf8_lossy(&output.stdout);
    let parsed: HardlinkFarmError =
        serde_json::from_str(line.trim()).expect("stdout carries typed HardlinkFarmError JSON");
    assert!(
        matches!(parsed, HardlinkFarmError::GenerationCollision { .. }),
        "expected GenerationCollision, got {parsed:?}",
    );
}

use nixling_host::hardlink_farm::{self, BuildStoreViewRequest, StoreViewLinkCounts};

fn run_store_view_helper(request: &BuildStoreViewRequest) -> std::process::Output {
    let payload = serde_json::to_vec(request).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_nixling-activation-helper"))
        .arg("build-store-view")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn helper");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&payload)
        .expect("write request");
    child.wait_with_output().expect("await helper")
}

#[test]
fn build_store_view_verb_writes_split_tree_and_emits_counts() {
    let tmp = tempdir().unwrap();
    let farm_root = tmp.path().join("vms/vm-a/store-view");
    std::fs::create_dir_all(&farm_root).unwrap();
    let closure = fake_closure(tmp.path(), 2);
    let gid = hardlink_farm::generation_id(&closure, hardlink_farm::system_store_path(&closure));

    let request = BuildStoreViewRequest {
        farm_root: farm_root.clone(),
        generation_id: gid.clone(),
        closure_paths: closure.clone(),
        marker: marker("closure-xyz"),
    };
    let output = run_store_view_helper(&request);
    assert!(
        output.status.success(),
        "helper failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // Success: stdout carries the link/skip accounting JSON.
    let line = String::from_utf8_lossy(&output.stdout);
    let counts: StoreViewLinkCounts =
        serde_json::from_str(line.trim()).expect("stdout carries StoreViewLinkCounts JSON");
    assert_eq!(counts.linked, 2);
    assert_eq!(counts.skipped, 0);

    // Split tree: guest meta under meta/, host metadata under state/.
    assert!(farm_root
        .join("meta/generations")
        .join(&gid)
        .join("meta.json")
        .exists());
    assert!(farm_root
        .join("state/generations")
        .join(&gid)
        .join("marker.json")
        .exists());
    let farmed = farm_root.join("live/aaaaaaaaaaaaaaaa-fake-0/bin/payload");
    assert_eq!(
        std::fs::metadata(&farmed).unwrap().ino(),
        std::fs::metadata(closure[0].join("bin/payload"))
            .unwrap()
            .ino(),
    );
    // build_store_view does NOT swap currents or plant the live marker.
    assert!(!farm_root.join("state/current").exists());
    assert!(!farm_root.join("meta/current").exists());
}

#[test]
fn build_store_view_verb_emits_typed_error_json_on_collision() {
    let tmp = tempdir().unwrap();
    let farm_root = tmp.path().join("vms/vm-a/store-view");
    std::fs::create_dir_all(&farm_root).unwrap();
    let closure = fake_closure(tmp.path(), 1);
    let gid = hardlink_farm::generation_id(&closure, hardlink_farm::system_store_path(&closure));

    let first = BuildStoreViewRequest {
        farm_root: farm_root.clone(),
        generation_id: gid.clone(),
        closure_paths: closure.clone(),
        marker: marker("first"),
    };
    assert!(run_store_view_helper(&first).status.success());

    // Reuse the SAME generation id with a DIFFERENT closure hash ->
    // collision; verb exits non-zero and emits the typed error JSON.
    let collide = BuildStoreViewRequest {
        farm_root,
        generation_id: gid,
        closure_paths: closure,
        marker: marker("second"),
    };
    let output = run_store_view_helper(&collide);
    assert!(!output.status.success(), "collision must fail");
    let line = String::from_utf8_lossy(&output.stdout);
    let parsed: HardlinkFarmError =
        serde_json::from_str(line.trim()).expect("stdout carries typed HardlinkFarmError JSON");
    assert!(
        matches!(parsed, HardlinkFarmError::GenerationCollision { .. }),
        "expected GenerationCollision, got {parsed:?}",
    );
}
