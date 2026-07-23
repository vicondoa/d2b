#![cfg(target_os = "linux")]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

static NEXT_SCRATCH_ID: AtomicU64 = AtomicU64::new(0);

struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("guest-control-token-materializer-tests");
        fs::create_dir_all(&base).expect("create guest-control token materializer scratch base");
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after UNIX_EPOCH")
            .as_nanos();
        for attempt in 0..100 {
            let id = NEXT_SCRATCH_ID.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("{}-{nanos}-{id}-{attempt}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => panic!("create scratch dir {}: {err}", path.display()),
            }
        }
        panic!("could not allocate unique guest-control token materializer scratch dir");
    }

    fn join(&self, rel: &str) -> PathBuf {
        self.path.join(rel)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("canonicalize repository root")
}

fn run_materializer(spec: &Path) -> Output {
    let root = repo_root();
    let materializer = root.join("nixos-modules/guest-control-token-materialize.py");
    match Command::new("python3")
        .arg(&materializer)
        .arg(spec)
        .output()
    {
        Ok(output) => output,
        Err(err) if err.kind() == io::ErrorKind::NotFound => Command::new("nix")
            .args(["shell", "--quiet", "--inputs-from"])
            .arg(&root)
            .arg("nixpkgs#python3")
            .args(["--command", "python3"])
            .arg(&materializer)
            .arg(spec)
            .output()
            .expect("spawn python3 via nix shell"),
        Err(err) => panic!("spawn python3: {err}"),
    }
}

fn expect_redacted_source_validation_failure(label: &str, source: &str, kind: &str) {
    let scratch = Scratch::new();
    let spec = scratch.join(&format!("{label}.json"));
    let target = scratch.join(&format!("{label}-target/token"));
    let payload = json!([
        {
            "name": "corp-vm",
            "source": source,
            "target": target.to_string_lossy(),
        }
    ]);
    fs::write(
        &spec,
        serde_json::to_vec(&payload).expect("serialize materializer spec"),
    )
    .expect("write materializer spec");

    let output = run_materializer(&spec);
    assert!(
        !output.status.success(),
        "guest-control-token-materializer: {label} unexpectedly succeeded"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(kind),
        "guest-control-token-materializer: {label} did not report {kind}; stderr={stderr}"
    );
    assert!(
        !stderr.contains(source),
        "guest-control-token-materializer: {label} leaked source path in stderr={stderr}"
    );
    let target = target.to_string_lossy();
    assert!(
        !stderr.contains(target.as_ref()),
        "guest-control-token-materializer: {label} leaked target path in stderr={stderr}"
    );
}

#[test]
fn guest_control_token_materializer_rejects_relative_source_without_path_leak() {
    expect_redacted_source_validation_failure(
        "relative-source",
        "relative-token",
        "source-not-absolute",
    );
}

#[test]
fn guest_control_token_materializer_rejects_store_root_source_without_path_leak() {
    expect_redacted_source_validation_failure("store-root", "/nix/store", "source-in-nix-store");
}

#[test]
fn guest_control_token_materializer_rejects_store_child_source_without_path_leak() {
    expect_redacted_source_validation_failure(
        "store-child",
        "/nix/store/not-a-token",
        "source-in-nix-store",
    );
}

#[test]
fn guest_control_token_materializer_rejects_missing_source_without_path_leak() {
    expect_redacted_source_validation_failure(
        "missing-source",
        "/definitely-missing-d2b-token",
        "path-component-missing",
    );
}
