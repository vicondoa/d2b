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
fn scan_ignores_dash_bytes_inside_a_binary_file() {
    // A leading NUL marks the file binary to `grep -I`. The banned dash bytes
    // sit right after it, so if `grep -I` were dropped the scan would match
    // them and fail; the pass here is binary-skip, not a dead scan.
    let dash = '\u{2014}';
    let root = fixture_tree("binary", "clean text\n");
    let mut buf = [0u8; 4];
    let mut blob: Vec<u8> = vec![0x00];
    blob.extend_from_slice(dash.encode_utf8(&mut buf).as_bytes());
    blob.extend_from_slice(&[0x01, 0x02, 0xff]);
    blob.extend_from_slice(b"payload");
    fs::write(root.join("blob.bin"), &blob).expect("write binary fixture");

    let (success, output) = scan(&root);
    assert!(
        success,
        "grep -I must skip the binary blob even though it embeds {dash:?}; output:\n{output}"
    );

    // The identical codepoint in a text file (no NUL) must still fail, which
    // proves the pass above is binary-skip and not a scan that stopped matching.
    let text_root = fixture_tree("binary-text-control", &format!("has one {dash} here\n"));
    let (text_success, text_output) = scan(&text_root);
    assert!(
        !text_success,
        "the same codepoint in a text file must fail; output:\n{text_output}"
    );
    assert!(
        text_output.contains("nested/sample.md:1"),
        "the text control must report the offending file:line; output:\n{text_output}"
    );
}

#[test]
fn scan_fails_closed_on_an_unreadable_file() {
    use std::os::unix::fs::PermissionsExt;

    let root = fixture_tree("unreadable", "clean text\n");
    let secret = root.join("nested/secret.md");
    fs::write(&secret, "clean text\n").expect("write secret fixture");
    let mut perms = fs::metadata(&secret).expect("metadata").permissions();
    perms.set_mode(0o000);
    fs::set_permissions(&secret, perms).expect("chmod 000");

    // Running as root bypasses the mode bits, so the grep-error path cannot be
    // exercised; skip rather than assert a pass we cannot reach.
    if fs::read(&secret).is_ok() {
        let mut restore = fs::metadata(&secret).expect("metadata").permissions();
        restore.set_mode(0o644);
        let _ = fs::set_permissions(&secret, restore);
        eprintln!("skipping: cannot make a file unreadable to this uid (running as root?)");
        return;
    }

    let (success, output) = scan(&root);

    let mut restore = fs::metadata(&secret).expect("metadata").permissions();
    restore.set_mode(0o644);
    let _ = fs::set_permissions(&secret, restore);

    assert!(
        !success,
        "the scan must fail closed when grep cannot read a file, not report a pass having \
         scanned nothing; output:\n{output}"
    );
    assert!(
        output.contains("grep exited"),
        "the scan must name the grep error status; output:\n{output}"
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
