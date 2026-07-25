//! The em-dash ban and the tier0 gate that enforces it.
//!
//! `AGENTS.md` bans U+2014 repository-wide and `tests/tools/tier0-first-pass.sh`
//! is the gate. A ban whose gate silently stops matching is worse than no ban,
//! so this lint drives the gate's scan over a fixture tree and requires it to
//! fail on an offending line and pass on a clean one. It also pins the two
//! structural properties the scan depends on: the gate is wired into
//! `make check-tier0`, and it matches on the codepoint rather than on a literal
//! character that would make it flag its own source.
//!
//! Every em-dash in this file is written as `\u{2014}`, never as the character.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use d2b_contract_tests::{read_repo_file, repo_root};

const GATE: &str = "tests/tools/tier0-first-pass.sh";
const EM_DASH: char = '\u{2014}';

fn gate_path() -> PathBuf {
    repo_root().join(GATE)
}

/// Run the gate's scan mode over `root` and return `(success, combined output)`.
fn scan(root: &Path) -> (bool, String) {
    let output = Command::new("bash")
        .arg(gate_path())
        .arg("--scan-em-dash")
        .arg(root)
        .output()
        .expect("tier0 gate must be runnable");
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), combined)
}

fn fixture_tree(name: &str, body: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("em-dash-gate")
        .join(name);
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("nested")).expect("create fixture tree");
    fs::write(root.join("clean.md"), "A spaced hyphen - like this.\n").expect("write clean file");
    fs::write(root.join("nested/sample.md"), body).expect("write sample file");
    root
}

#[test]
fn scan_fails_on_an_em_dash_and_names_the_line() {
    let body = format!("first line is clean\nsecond line has one {EM_DASH} here\n");
    let root = fixture_tree("offending", &body);

    let (success, output) = scan(&root);
    assert!(
        !success,
        "the tier0 em-dash scan must fail closed on an offending file; output:\n{output}"
    );
    assert!(
        output.contains("nested/sample.md:2"),
        "the scan must report the offending file:line; output:\n{output}"
    );
}

#[test]
fn scan_passes_on_a_clean_tree() {
    let root = fixture_tree("clean", "no banned character on this line\n");

    let (success, output) = scan(&root);
    assert!(
        success,
        "the tier0 em-dash scan must pass a tree with no em-dash; output:\n{output}"
    );
}

#[test]
fn scan_ignores_binary_files() {
    let root = fixture_tree("binary", "clean text\n");
    let mut blob: Vec<u8> = vec![0x00, 0x01, 0x02, 0xff];
    blob.extend_from_slice("payload".as_bytes());
    fs::write(root.join("blob.bin"), &blob).expect("write binary fixture");

    let (success, output) = scan(&root);
    assert!(
        success,
        "the scan must not choke on a binary file; output:\n{output}"
    );
}

#[test]
fn the_repository_carries_no_em_dash() {
    let (success, output) = scan(&repo_root());
    assert!(
        success,
        "U+2014 is banned repository-wide (AGENTS.md); output:\n{output}"
    );
}

#[test]
fn the_gate_matches_the_codepoint_not_a_literal_character() {
    let gate = read_repo_file(GATE);
    assert!(
        !gate.contains(EM_DASH),
        "{GATE} must not carry a literal em-dash; it would flag its own source"
    );
    assert!(
        gate.contains(r"$'\u2014'"),
        "{GATE} must match the em-dash by codepoint escape so the pattern survives editing"
    );
    assert!(
        gate.contains("scan_em_dash \"$ROOT\""),
        "{GATE} must run the repository-wide scan in its main body, not only in scan mode"
    );
}

#[test]
fn check_tier0_runs_the_gate() {
    let makefile = read_repo_file("Makefile");
    let wired = makefile
        .lines()
        .any(|line| line.trim_start().starts_with("bash ") && line.contains(GATE));
    assert!(
        wired,
        "the Makefile `check-tier0` target must run {GATE}; the em-dash ban has no other gate"
    );
}

#[test]
fn agents_md_states_the_prohibition() {
    let agents = read_repo_file("AGENTS.md");
    assert!(
        agents.contains("U+2014"),
        "AGENTS.md must name the banned codepoint so the rule is greppable"
    );
    assert!(
        !agents.contains(EM_DASH),
        "AGENTS.md must not use the character it bans"
    );
}
