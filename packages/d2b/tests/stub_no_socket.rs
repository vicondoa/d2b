use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::tempdir;

fn snapshot_directory(path: &Path) -> BTreeSet<PathBuf> {
    let Ok(entries) = std::fs::read_dir(path) else {
        return BTreeSet::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect()
}

#[test]
fn d2b_stub_does_not_create_socket_or_runtime_state() {
    let scratch = tempdir().expect("stub scratch directory");
    let home = scratch.path().join("home");
    let tmp = scratch.path().join("tmp");
    let runtime = scratch.path().join("xdg-runtime");
    for path in [&home, &tmp, &runtime] {
        std::fs::create_dir(path).expect("stub scratch child directory");
    }

    let run_before = snapshot_directory(Path::new("/run/d2b"));
    let var_lib_before = snapshot_directory(Path::new("/var/lib/d2b"));
    let output = Command::new(env!("CARGO_BIN_EXE_d2b"))
        .env("HOME", &home)
        .env("TMPDIR", &tmp)
        .env("XDG_RUNTIME_DIR", &runtime)
        .output()
        .expect("run d2b stub");

    assert!(
        output.status.success(),
        "d2b stub failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("d2b 0.0.0-bootstrap"),
        "d2b stub did not report its version"
    );
    assert_eq!(run_before, snapshot_directory(Path::new("/run/d2b")));
    assert_eq!(
        var_lib_before,
        snapshot_directory(Path::new("/var/lib/d2b"))
    );
    assert!(
        snapshot_directory(&home).is_empty(),
        "d2b stub created HOME state"
    );
    assert!(
        snapshot_directory(&tmp).is_empty(),
        "d2b stub created temporary state"
    );
    assert!(
        snapshot_directory(&runtime).is_empty(),
        "d2b stub created runtime state"
    );
}
