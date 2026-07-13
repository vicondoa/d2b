#![forbid(unsafe_code)]

use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_SCRATCH: AtomicU64 = AtomicU64::new(1);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repository = manifest_dir
            .parent()
            .and_then(std::path::Path::parent)
            .expect("repository root");
        let path = repository.parent().expect("parent").join(format!(
            ".d2b-xtask-cli-test-{}-{}",
            std::process::id(),
            NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create external scratch");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn stack_validate_subcommand_accepts_valid_and_rejects_unknown_dependencies() {
    let scratch = Scratch::new();
    let manifest = serde_json::json!({
        "schema_version": 1,
        "wave": "w1",
        "root_repository": {
            "name": "example/d2b",
            "root": "/workspace/d2b",
            "base": "main",
            "head": "feature"
        },
        "repository_set": [{
            "name": "example/d2b",
            "root": "/workspace/d2b",
            "head": "feature"
        }],
        "stack": [{
            "id": "root",
            "repository": "example/d2b",
            "branch": "feature",
            "pr": 42,
            "head": "feature",
            "depends_on": []
        }],
        "required_validations": [{
            "id": "unit",
            "command": "cargo test -p xtask"
        }],
        "required_checks": [{
            "node": "root",
            "name": "unit"
        }],
        "generated_artifacts": [],
        "dependency_fingerprints": [],
        "contract_fingerprints": []
    });
    let manifest_path = scratch.0.join("stack.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("serialize"),
    )
    .expect("manifest");
    let binary = env!("CARGO_BIN_EXE_xtask");
    let valid = Command::new(binary)
        .args([
            "stack",
            "validate",
            "--manifest",
            manifest_path.to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("run xtask");
    assert!(
        valid.status.success(),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );

    let mut invalid = manifest;
    invalid["stack"][0]["depends_on"] = serde_json::json!(["missing"]);
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&invalid).expect("serialize"),
    )
    .expect("invalid manifest");
    let rejected = Command::new(binary)
        .args([
            "delivery",
            "stack",
            "validate",
            "--manifest",
            manifest_path.to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("run xtask");
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("unknown dependency"));
}
