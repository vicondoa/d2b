//! The non-ASCII dash ban and the tier0 gate that enforces it.
//!
//! `AGENTS.md` permits only the plain ASCII hyphen to spell a dash, and
//! `tests/tools/tier0-first-pass.sh` is the gate. A ban whose gate silently
//! stops matching is worse than no ban, so this lint drives the gate's scan
//! over a fixture tree and requires it to fail on every banned codepoint and
//! pass on a clean one. It also pins the structural properties the scan depends
//! on: the gate is wired into `make check-tier0`, it runs repository-wide in
//! its main body, and it matches on codepoints rather than on literal
//! characters that would make it flag its own source.
//!
//! Every banned character in this file is written as a `\u{...}` escape, never
//! as the character.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use d2b_contract_tests::{read_repo_file, repo_root};

const GATE: &str = "tests/tools/tier0-first-pass.sh";

/// The banned class, paired with the codepoint label the gate documents. Only
/// U+2011, U+2013, U+2014 and U+2212 were ever present in this repository; the
/// rest are banned pre-emptively so a future paste of any of them fails the
/// same way, and each still has to be proven rejected.
const BANNED: &[(char, &str)] = &[
    ('\u{2010}', "U+2010"),
    ('\u{2011}', "U+2011"),
    ('\u{2012}', "U+2012"),
    ('\u{2013}', "U+2013"),
    ('\u{2014}', "U+2014"),
    ('\u{2015}', "U+2015"),
    ('\u{2212}', "U+2212"),
    ('\u{FE58}', "U+FE58"),
    ('\u{FF0D}', "U+FF0D"),
];

fn gate_path() -> PathBuf {
    repo_root().join(GATE)
}

/// Run the gate's scan mode over `root` and return `(success, combined output)`.
fn scan(root: &Path) -> (bool, String) {
    let output = Command::new("bash")
        .arg(gate_path())
        .arg("--scan-dashes")
        .arg(root)
        .output()
        .expect("tier0 gate must be runnable");
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), combined)
}

fn fixture_tree(name: &str, body: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("dash-gate")
        .join(name);
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("nested")).expect("create fixture tree");
    fs::write(root.join("clean.md"), "A spaced hyphen - like this.\n").expect("write clean file");
    fs::write(root.join("nested/sample.md"), body).expect("write sample file");
    root
}

#[test]
fn scan_fails_on_every_banned_codepoint_and_names_the_line() {
    for (dash, label) in BANNED {
        let body = format!("first line is clean\nsecond line has one {dash} here\n");
        let root = fixture_tree(&format!("offending-{label}"), &body);

        let (success, output) = scan(&root);
        assert!(
            !success,
            "the tier0 dash scan must fail closed on {label}; output:\n{output}"
        );
        assert!(
            output.contains("nested/sample.md:2"),
            "the scan must report the offending file:line for {label}; output:\n{output}"
        );
    }
}

#[test]
fn scan_passes_on_a_clean_tree() {
    let root = fixture_tree("clean", "no banned character on this line\n");

    let (success, output) = scan(&root);
    assert!(
        success,
        "the tier0 dash scan must pass a tree whose only dash is the ASCII hyphen; output:\n{output}"
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
fn the_repository_carries_no_non_ascii_dash() {
    let (success, output) = scan(&repo_root());
    assert!(
        success,
        "only the ASCII hyphen may spell a dash (AGENTS.md); output:\n{output}"
    );
}

#[test]
fn the_gate_matches_codepoints_not_literal_characters() {
    let gate = read_repo_file(GATE);
    for (dash, label) in BANNED {
        assert!(
            !gate.contains(*dash),
            "{GATE} must not carry a literal {label}; it would flag its own source"
        );
        let escape = format!(r"$'\u{}'", label.trim_start_matches("U+"));
        assert!(
            gate.contains(&escape),
            "{GATE} must match {label} by codepoint escape ({escape}) so the whole class is \
             rejected and the pattern survives editing"
        );
    }
    assert!(
        gate.contains("scan_dashes \"$ROOT\""),
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
        "the Makefile `check-tier0` target must run {GATE}; the dash ban has no other gate"
    );
}

#[test]
fn agents_md_states_the_prohibition() {
    let agents = read_repo_file("AGENTS.md");
    for (dash, label) in BANNED {
        assert!(
            agents.contains(label),
            "AGENTS.md must name {label} so the rule is greppable"
        );
        assert!(
            !agents.contains(*dash),
            "AGENTS.md must not use the {label} character it bans"
        );
    }
}
