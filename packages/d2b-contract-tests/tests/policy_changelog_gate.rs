//! Hermetic behaviour tests for `scripts/changelog-check.sh`.
//!
//! Two properties are pinned here, both of which a prior version of the gate
//! failed to enforce:
//!
//! 1. *Change classification.* A code change ships a release note or the gate
//!    fails. "Code" is any executable or configuration surface, and a deletion
//!    counts as a change. The earlier gate used `--diff-filter=ACMR` (no
//!    deletions) and recognized only Rust, Nix, and Cargo files, so a deleted
//!    module, a shell edit, or a Makefile change shipped silently.
//!
//! 2. *Fragment-parser parity.* The pure-shell fragment validator is the second
//!    parser of the `changelog.d/` format; the canonical one is the Rust
//!    assembler (`cargo xtask changelog-fold --check`). They must agree on
//!    discovery, file types, and encoding. The Rust side is pinned by the
//!    `load_fragments_*` tests in `packages/xtask/src/changelog.rs`; the shell
//!    side is pinned here against the same corpus.
//!
//! Each test builds a throwaway git repository, replays a change, and runs the
//! real gate against it via `ROOT=<fixture>`.

use std::path::PathBuf;
use std::process::Command;

use d2b_contract_tests::repo_root;

const VALID_CHANGELOG: &str = "\
# Changelog

## [Unreleased]

### Added

- a seeded unreleased entry

## [1.0.0] - 2026-01-01

### Added

- the first release
";

const VALID_FRAGMENT: &str = "### Added\n\n- a fragment entry\n";

/// A throwaway git repository under the gitignored `.agent-tmp/` scratch root,
/// removed on drop even when a test panics.
struct FixtureRepo {
    root: PathBuf,
}

impl FixtureRepo {
    fn new(tag: &str) -> Self {
        let base = repo_root().join(".agent-tmp").join("changelog-gate");
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
        std::fs::create_dir_all(root.join("changelog.d")).expect("create changelog.d");
        let repo = FixtureRepo { root };
        repo.git(&["init", "-q"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo
    }

    fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .env("GIT_AUTHOR_NAME", "fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write(&self, rel: &str, body: &str) {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(path, body).expect("write file");
    }

    fn write_bytes(&self, rel: &str, bytes: &[u8]) {
        std::fs::write(self.root.join(rel), bytes).expect("write bytes");
    }

    fn remove(&self, rel: &str) {
        std::fs::remove_file(self.root.join(rel)).expect("remove file");
    }

    fn symlink(&self, target: &str, link: &str) {
        std::os::unix::fs::symlink(target, self.root.join(link)).expect("create symlink");
    }

    fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--no-gpg-sign", "-m", message]);
    }

    /// Seed a valid baseline: a valid changelog plus a source file, committed.
    fn seed(&self) {
        self.write("CHANGELOG.md", VALID_CHANGELOG);
        self.write("src/lib.rs", "pub fn seed() {}\n");
        self.write("run.sh", "#!/usr/bin/env bash\necho seed\n");
        self.write("Makefile", "seed:\n\techo seed\n");
        self.commit("seed");
    }

    /// Run the real gate against this fixture. Returns (passed, combined output).
    fn run_gate(&self) -> (bool, String) {
        let script = repo_root().join("scripts/changelog-check.sh");
        let scrubber = repo_root().join("tests/tools/scrub-shell-environment");
        let output = Command::new(scrubber)
            .args(["-c", "exec bash \"$@\"", "policy-changelog-gate"])
            .arg(&script)
            .current_dir(&self.root)
            .env("ROOT", &self.root)
            .env_remove("GITHUB_BASE_REF")
            .output()
            .expect("gate runs");
        let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
        (output.status.success(), combined)
    }
}

impl Drop for FixtureRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn assert_gate(passed: bool, output: &str, expect_pass: bool, context: &str) {
    assert_eq!(
        passed,
        expect_pass,
        "{context}: expected gate to {}, got {}\n{output}",
        if expect_pass { "PASS" } else { "FAIL" },
        if passed { "PASS" } else { "FAIL" },
    );
}

fn path_is_symlink_capable() -> bool {
    // The fixtures rely on POSIX symlinks; skip cleanly where they cannot exist.
    let probe = repo_root().join(".agent-tmp").join("symlink-probe");
    let _ = std::fs::create_dir_all(probe.parent().unwrap());
    let _ = std::fs::remove_file(&probe);
    let ok = std::os::unix::fs::symlink("target", &probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

// ---------------------------------------------------------------------------
// Change classification (finding: the gate missed whole categories of change)
// ---------------------------------------------------------------------------

#[test]
fn a_rust_edit_without_notes_fails_and_either_note_form_passes() {
    let repo = FixtureRepo::new("rust-edit");
    repo.seed();
    repo.write("src/lib.rs", "pub fn seed() {}\npub fn added() {}\n");
    repo.commit("rust edit, no notes");
    let (passed, out) = repo.run_gate();
    assert_gate(passed, &out, false, "rust edit without notes");

    // A CHANGELOG.md entry satisfies the gate.
    let repo = FixtureRepo::new("rust-edit-changelog");
    repo.seed();
    repo.write("src/lib.rs", "pub fn seed() {}\npub fn added() {}\n");
    repo.write(
        "CHANGELOG.md",
        &VALID_CHANGELOG.replacen(
            "- a seeded unreleased entry\n",
            "- a seeded unreleased entry\n- a new note\n",
            1,
        ),
    );
    repo.commit("rust edit with changelog");
    let (passed, out) = repo.run_gate();
    assert_gate(passed, &out, true, "rust edit with CHANGELOG entry");

    // A changelog.d fragment satisfies the gate too.
    let repo = FixtureRepo::new("rust-edit-fragment");
    repo.seed();
    repo.write("src/lib.rs", "pub fn seed() {}\npub fn added() {}\n");
    repo.write("changelog.d/branch.md", VALID_FRAGMENT);
    repo.commit("rust edit with fragment");
    let (passed, out) = repo.run_gate();
    assert_gate(passed, &out, true, "rust edit with changelog.d fragment");
}

#[test]
fn a_shell_edit_without_notes_fails_and_a_fragment_passes() {
    let repo = FixtureRepo::new("shell-edit");
    repo.seed();
    repo.write("run.sh", "#!/usr/bin/env bash\necho changed\n");
    repo.commit("shell edit, no notes");
    let (passed, out) = repo.run_gate();
    assert_gate(passed, &out, false, "shell edit without notes");

    let repo = FixtureRepo::new("shell-edit-fragment");
    repo.seed();
    repo.write("run.sh", "#!/usr/bin/env bash\necho changed\n");
    repo.write("changelog.d/branch.md", VALID_FRAGMENT);
    repo.commit("shell edit with fragment");
    let (passed, out) = repo.run_gate();
    assert_gate(passed, &out, true, "shell edit with fragment");
}

#[test]
fn a_makefile_edit_without_notes_fails() {
    let repo = FixtureRepo::new("makefile-edit");
    repo.seed();
    repo.write("Makefile", "seed:\n\techo changed\n");
    repo.commit("makefile edit, no notes");
    let (passed, out) = repo.run_gate();
    assert_gate(passed, &out, false, "makefile edit without notes");
}

#[test]
fn a_deletion_without_notes_fails_and_a_fragment_passes() {
    let repo = FixtureRepo::new("delete");
    repo.seed();
    repo.remove("src/lib.rs");
    repo.commit("delete module, no notes");
    let (passed, out) = repo.run_gate();
    assert_gate(passed, &out, false, "deletion without notes");

    let repo = FixtureRepo::new("delete-fragment");
    repo.seed();
    repo.remove("src/lib.rs");
    repo.write("changelog.d/branch.md", VALID_FRAGMENT);
    repo.commit("delete module with fragment");
    let (passed, out) = repo.run_gate();
    assert_gate(passed, &out, true, "deletion with fragment");
}

#[test]
fn a_patch_or_proto_change_without_notes_fails() {
    // A patch-only change ships behaviour and must carry a note.
    let repo = FixtureRepo::new("patch-edit");
    repo.seed();
    repo.write(
        "0001-fix-thing.patch",
        "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n",
    );
    repo.commit("patch add, no notes");
    let (passed, out) = repo.run_gate();
    assert_gate(passed, &out, false, "patch edit without notes");

    // A protocol-definition change is code too.
    let repo = FixtureRepo::new("proto-edit");
    repo.seed();
    repo.write(
        "wire.proto",
        "syntax = \"proto3\";\nmessage M { int32 a = 1; }\n",
    );
    repo.commit("proto add, no notes");
    let (passed, out) = repo.run_gate();
    assert_gate(passed, &out, false, "proto edit without notes");

    // And a fragment satisfies the gate for a proto change.
    let repo = FixtureRepo::new("proto-fragment");
    repo.seed();
    repo.write(
        "wire.proto",
        "syntax = \"proto3\";\nmessage M { int32 a = 1; }\n",
    );
    repo.write("changelog.d/branch.md", VALID_FRAGMENT);
    repo.commit("proto add with fragment");
    let (passed, out) = repo.run_gate();
    assert_gate(passed, &out, true, "proto edit with fragment");
}

#[test]
fn a_patch_or_proto_deletion_without_notes_fails() {
    // Seed a repo that already carries a patch and a proto, then delete them.
    let repo = FixtureRepo::new("patch-delete");
    repo.write("CHANGELOG.md", VALID_CHANGELOG);
    repo.write(
        "0001-fix-thing.patch",
        "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n",
    );
    repo.write(
        "wire.proto",
        "syntax = \"proto3\";\nmessage M { int32 a = 1; }\n",
    );
    repo.commit("seed patch and proto");
    repo.remove("0001-fix-thing.patch");
    repo.remove("wire.proto");
    repo.commit("delete patch and proto, no notes");
    let (passed, out) = repo.run_gate();
    assert_gate(passed, &out, false, "patch/proto deletion without notes");
}

#[test]
fn a_prose_only_change_needs_no_notes() {
    let repo = FixtureRepo::new("prose");
    repo.seed();
    repo.write("docs/guide.md", "# Guide\n\nUpdated prose.\n");
    repo.commit("docs only");
    let (passed, out) = repo.run_gate();
    assert_gate(passed, &out, true, "prose-only change");
}

// ---------------------------------------------------------------------------
// Fragment-parser parity (finding: two parsers asserted equivalent, untested)
// ---------------------------------------------------------------------------
//
// Each case sets up a fragment-only change so the note-requirement never fires,
// isolating the shell fragment validator. The expected verdict matches the Rust
// `load_fragments` semantics pinned in packages/xtask/src/changelog.rs.

#[test]
fn parity_a_valid_fragment_passes() {
    let repo = FixtureRepo::new("parity-valid");
    repo.seed();
    repo.write("changelog.d/branch.md", VALID_FRAGMENT);
    repo.commit("add valid fragment");
    let (passed, out) = repo.run_gate();
    assert_gate(passed, &out, true, "valid fragment");
}

#[test]
fn parity_a_non_md_fragment_is_rejected() {
    let repo = FixtureRepo::new("parity-nonmd");
    repo.seed();
    repo.write("changelog.d/notes.txt", VALID_FRAGMENT);
    repo.commit("add non-.md entry");
    let (passed, out) = repo.run_gate();
    assert_gate(passed, &out, false, "non-.md fragment");
    assert!(
        out.contains("must be named"),
        "expected a naming diagnostic:\n{out}"
    );
}

#[test]
fn parity_a_symlink_fragment_is_rejected() {
    if !path_is_symlink_capable() {
        eprintln!("skipping symlink parity test: symlinks unavailable");
        return;
    }
    let repo = FixtureRepo::new("parity-symlink");
    repo.seed();
    repo.symlink("../CHANGELOG.md", "changelog.d/link.md");
    repo.commit("add symlink fragment");
    let (passed, out) = repo.run_gate();
    assert_gate(passed, &out, false, "symlink fragment");
    assert!(
        out.contains("not a regular file"),
        "expected a file-type diagnostic:\n{out}"
    );
}

#[test]
fn parity_an_invalid_utf8_fragment_is_rejected() {
    let repo = FixtureRepo::new("parity-utf8");
    repo.seed();
    repo.write_bytes("changelog.d/bad.md", &[0xff, 0xfe, 0x00, 0x41]);
    repo.commit("add invalid utf-8 fragment");
    let (passed, out) = repo.run_gate();
    assert_gate(passed, &out, false, "invalid utf-8 fragment");
    assert!(
        out.contains("not valid UTF-8"),
        "expected an encoding diagnostic:\n{out}"
    );
}

#[test]
fn parity_a_structurally_invalid_fragment_is_rejected() {
    let repo = FixtureRepo::new("parity-structure");
    repo.seed();
    repo.write(
        "changelog.d/branch.md",
        "### Improved\n\n- not a real section\n",
    );
    repo.commit("add unknown-section fragment");
    let (passed, out) = repo.run_gate();
    assert_gate(passed, &out, false, "unknown-section fragment");
    assert!(
        out.contains("unknown section"),
        "expected a structural diagnostic:\n{out}"
    );
}

#[test]
fn parity_a_star_bullet_fragment_is_rejected() {
    let repo = FixtureRepo::new("parity-star-bullet");
    repo.seed();
    repo.write(
        "changelog.d/branch.md",
        "### Added\n\n* non-canonical bullet\n",
    );
    repo.commit("add star-bullet fragment");
    let (passed, out) = repo.run_gate();
    assert_gate(passed, &out, false, "star-bullet fragment");
    assert!(
        out.contains("must start with a '- ' bullet"),
        "expected a canonical-bullet diagnostic:\n{out}"
    );
}
