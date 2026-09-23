//! Hermetic behaviour tests for `tests/tools/security-scan.sh`.
//!
//! The scan gate (issue #586) is a ~300-line shell script with a pinned
//! contract: exit 0 = no findings, exit 1 = findings, exit 2 = the scan
//! could not run. Each test builds a throwaway git repository, plants a
//! record, and runs the real script against it via `D2B_REPO_ROOT`.
//!
//! Pinned here:
//!
//! 1. *Whole-invocation rule.* The scanner buffers a log-macro invocation
//!    from its opening line to the real closing delimiter, literal-aware:
//!    a multi-line raw string containing `)` with a pinned identifier
//!    later in the record is a finding, and a `)` char literal on the
//!    opening line must still open the record.
//! 2. *Exit statuses.* A clean tree exits 0, a planted finding exits 1,
//!    and a scan outside a git checkout (enumeration failure) exits 2 -
//!    never a clean report from a scan that did not run.
//! 3. *Renames.* The changed-line diff keeps R statuses, so a violation
//!    that arrives with a rename is still a finding.

use std::path::PathBuf;
use std::process::Command;

mod common;

use common::repo_root;

fn scratch_root() -> PathBuf {
    std::env::var_os("CARGO_TARGET_TMPDIR")
        .or_else(|| std::env::var_os("TEST_TMPDIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join(".agent-tmp"))
}

/// A throwaway git repository under the test runner's writable scratch
/// root, removed on drop even when a test panics.
struct FixtureRepo {
    root: PathBuf,
}

impl FixtureRepo {
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn new(tag: &str) -> Self {
        let base = scratch_root().join("security-scan-gate");
        std::fs::create_dir_all(&base).expect("create scratch base");
        let unique = format!(
            "{tag}.{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = base.join(unique);
        std::fs::create_dir_all(root.join("src")).expect("create src");
        let repo = FixtureRepo { root };
        repo.git(&["init", "-q"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .env("GIT_AUTHOR_NAME", "fixture")
            .env("GIT_AUTHOR_EMAIL", "[EMAIL]")
            .env("GIT_COMMITTER_NAME", "fixture")
            .env("GIT_COMMITTER_EMAIL", "[EMAIL]")
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn write(&self, rel: &str, body: &str) {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(path, body).expect("write file");
    }

    fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--no-gpg-sign", "-m", message]);
    }

    fn head_sha(&self) -> String {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.root)
            .output()
            .expect("git rev-parse runs");
        assert!(output.status.success(), "rev-parse failed");
        String::from_utf8(output.stdout).expect("sha is utf-8").trim().to_owned()
    }

    /// Run the real scan gate against this fixture. Returns (exit code,
    /// combined output). `base_sha` selects changed-line mode.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn run_scan(&self, base_sha: Option<&str>) -> (i32, String) {
        run_scan_in(&self.root, base_sha)
    }
}

impl Drop for FixtureRepo {
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn run_scan_in(root: &PathBuf, base_sha: Option<&str>) -> (i32, String) {
    let script = repo_root().join("tests/tools/security-scan.sh");
    let scrubber = repo_root().join("tests/tools/scrub-shell-environment");
    let mut command = Command::new(scrubber);
    command
        .args(["-c", "exec bash \"$@\"", "security-scan-gate"])
        .arg(&script)
        .current_dir(root)
        .env("D2B_REPO_ROOT", root)
        // Stop git from discovering an enclosing repository above the
        // scratch root: the non-repo fixture must genuinely fail its
        // enumeration instead of scanning the runner's own checkout.
        .env("GIT_CEILING_DIRECTORIES", scratch_root())
        .env_remove("D2B_SCAN_BASE_SHA");
    if let Some(base) = base_sha {
        command.env("D2B_SCAN_BASE_SHA", base);
    }
    let output = command.output().expect("scan gate runs");
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.code().unwrap_or(-1), combined)
}

fn assert_exit(code: i32, want: i32, context: &str, output: &str) {
    assert_eq!(
        code, want,
        "{context}: expected exit {want}, got {code}\n{output}"
    );
}

// ---------------------------------------------------------------------------
// Whole-invocation rule (finding: per-line paren balance missed records)
// ---------------------------------------------------------------------------

#[test]
fn a_clean_tree_exits_zero() {
    let repo = FixtureRepo::new("clean");
    repo.write(
        "src/lib.rs",
        "pub fn seed() {\n    tracing::info!(\"all good\");\n}\n",
    );
    repo.commit("seed");
    let (code, out) = repo.run_scan(None);
    assert_exit(code, 0, "clean tree", &out);
}

#[test]
fn a_single_line_finding_exits_one() {
    let repo = FixtureRepo::new("single-line");
    repo.write(
        "src/lib.rs",
        "pub fn seed() {\n    tracing::info!(\"op {} closed\", operation_id);\n}\n",
    );
    repo.commit("plant single-line finding");
    let (code, out) = repo.run_scan(None);
    assert_exit(code, 1, "single-line finding", &out);
    assert!(out.contains("operation_id"), "expected the finding:\n{out}");
}

#[test]
fn a_multi_line_raw_string_record_finding_exits_one() {
    // A multi-line raw string containing ')' with a pinned identifier
    // later in the record: the record must stay open until the real
    // closing delimiter and the identifier must be found.
    let repo = FixtureRepo::new("multi-line-raw");
    repo.write(
        "src/lib.rs",
        "pub fn seed() {\n    tracing::warn!(\n        r#\"multi-line raw\n        ) inside raw string\n        more text\"#,\n        id = operation_id,\n    );\n}\n",
    );
    repo.commit("plant multi-line finding");
    let (code, out) = repo.run_scan(None);
    assert_exit(code, 1, "multi-line raw string finding", &out);
    assert!(out.contains("operation_id"), "expected the finding:\n{out}");
}

#[test]
fn a_char_literal_on_the_opening_line_still_opens_the_record() {
    // A ')' char literal on the opening line must not stop the record
    // from opening: the identifier on a continuation line is a finding.
    let repo = FixtureRepo::new("char-literal");
    repo.write(
        "src/lib.rs",
        "pub fn seed() {\n    tracing::warn!(ch = ')',\n        id = operation_id,\n    );\n}\n",
    );
    repo.commit("plant char-literal opening line");
    let (code, out) = repo.run_scan(None);
    assert_exit(code, 1, "char literal on opening line", &out);
    assert!(out.contains("operation_id"), "expected the finding:\n{out}");
}

#[test]
fn a_multi_line_record_without_an_identifier_stays_clean() {
    // Same shape as the finding fixtures but no pinned identifier: the
    // literal-aware buffering must not invent a finding.
    let repo = FixtureRepo::new("multi-line-clean");
    repo.write(
        "src/lib.rs",
        "pub fn seed() {\n    tracing::warn!(\n        r#\"multi-line raw\n        ) inside raw string\n        more text\"#,\n        id = some_other_id,\n    );\n}\n",
    );
    repo.commit("seed multi-line clean record");
    let (code, out) = repo.run_scan(None);
    assert_exit(code, 0, "multi-line record without identifier", &out);
}

// ---------------------------------------------------------------------------
// Could-not-run status (finding: a failed enumeration reported clean)
// ---------------------------------------------------------------------------

#[test]
fn a_non_repo_dir_exits_two_never_clean() {
    let base = scratch_root().join("security-scan-nonrepo");
    std::fs::create_dir_all(&base).expect("create non-repo dir");
    std::fs::write(base.join("x.rs"), "pub fn f() { tracing::info!(\"x\"); }\n")
        .expect("write x.rs");
    let (code, out) = run_scan_in(&base, None);
    assert_exit(code, 2, "non-repo dir", &out);
    assert!(
        !out.contains("security-scan: clean"),
        "a scan that could not run must never report clean:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

// ---------------------------------------------------------------------------
// Renames (finding: R status must be scanned in changed-line mode)
// ---------------------------------------------------------------------------

#[test]
fn a_rename_with_planted_violation_exits_one() {
    let repo = FixtureRepo::new("rename");
    let mut baseline = String::from("pub fn seed() {\n");
    for i in 0..20 {
        baseline.push_str(&format!("    tracing::info!(\"baseline {i}\");\n"));
    }
    baseline.push_str("}\n");
    repo.write("src/old.rs", &baseline);
    repo.commit("seed baseline");
    let base = repo.head_sha();

    repo.git(&["mv", "src/old.rs", "src/new.rs"]);
    let mut renamed = baseline.clone();
    renamed.push_str("    tracing::warn!(id = operation_id, \"renamed with violation\");\n");
    repo.write("src/new.rs", &renamed);
    repo.commit("rename with violation");

    // Prove the fixture really exercises an R-status diff.
    let diff = Command::new("git")
        .args([
            "diff",
            "--diff-filter=ACMR",
            "-M",
            "-C",
            "--unified=0",
            &base,
            "--",
            "*.rs",
        ])
        .current_dir(&repo.root)
        .output()
        .expect("git diff runs");
    assert!(diff.status.success(), "diff failed");
    let diff_text = String::from_utf8_lossy(&diff.stdout);
    assert!(
        diff_text.contains("rename from src/old.rs"),
        "fixture must produce an R-status diff:\n{diff_text}"
    );

    let (code, out) = repo.run_scan(Some(&base));
    assert_exit(code, 1, "rename with planted violation", &out);
    assert!(out.contains("operation_id"), "expected the finding:\n{out}");
}

#[test]
fn a_rename_without_a_violation_stays_clean() {
    let repo = FixtureRepo::new("rename-clean");
    let mut baseline = String::from("pub fn seed() {\n");
    for i in 0..20 {
        baseline.push_str(&format!("    tracing::info!(\"baseline {i}\");\n"));
    }
    baseline.push_str("}\n");
    repo.write("src/old.rs", &baseline);
    repo.commit("seed baseline");
    let base = repo.head_sha();

    repo.git(&["mv", "src/old.rs", "src/new.rs"]);
    repo.write("src/new.rs", &baseline);
    repo.commit("rename without violation");

    let (code, out) = repo.run_scan(Some(&base));
    assert_exit(code, 0, "rename without violation", &out);
}