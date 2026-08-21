//! Deterministic-time policy for the hermetic runtime-ledger surface.
//!
//! The runtime ledger owns the placement vocabulary and the exception rule.
//! This policy keeps the currently pinned census aligned with that checker
//! without creating a second timing framework or a historical baseline.

use std::fs;

use d2b_contract_tests::repo_root;
use serde::Deserialize;

const PLACEMENT_MARKERS: &[&str] = &[
    "thread::sleep",
    "thread::park_timeout",
    "tokio::time::sleep(",
    "tokio::time::sleep_until",
    "tokio::time::interval(",
    "tokio::time::interval_at",
    "tokio::time::timeout",
    "recv_timeout(",
    "wait_timeout(",
    "std::process::Command",
    "Command::new(",
    "TcpStream::",
    "TcpListener::",
    "UdpSocket::",
    "systemctl",
    "zbus::",
    "#[ignore]",
];

const CLOCK_MARKERS: &[&str] = &["Instant::now(", "SystemTime::now(", "Utc::now("];

#[derive(Debug, Deserialize)]
struct RuntimeCensus {
    crates: Vec<String>,
}

fn code_line(line: &str) -> &str {
    line.split("//").next().unwrap_or_default()
}

fn hermetic_violations(path: &str, source: &str) -> Vec<String> {
    let mut violations = Vec::new();
    for (line_number, line) in source.lines().enumerate() {
        let code = code_line(line);
        for marker in PLACEMENT_MARKERS {
            if code.contains(marker) {
                violations.push(format!(
                    "{path}:{}: placement marker `{marker}`",
                    line_number + 1
                ));
            }
        }
        if !line.contains("runtime-budget-exception:") {
            for marker in CLOCK_MARKERS {
                if code.contains(marker) {
                    violations.push(format!(
                        "{path}:{}: deterministic-clock marker `{marker}`",
                        line_number + 1
                    ));
                }
            }
        }
    }
    violations
}

fn census_test_files() -> Vec<(String, String)> {
    let census: RuntimeCensus = serde_json::from_str(
        &fs::read_to_string(repo_root().join("tests/runtime-ledger-census.json"))
            .expect("read runtime-ledger census"),
    )
    .expect("parse runtime-ledger census");
    assert!(
        !census.crates.is_empty(),
        "runtime-ledger census must name at least one crate"
    );

    let mut files = Vec::new();
    for crate_name in census.crates {
        let directory = repo_root().join("packages").join(&crate_name).join("tests");
        assert!(
            directory.is_dir(),
            "runtime-ledger census crate has no tests directory: {crate_name}"
        );
        for entry in fs::read_dir(&directory).expect("read census tests directory") {
            let entry = entry.expect("read census test entry");
            let path = entry.path();
            if !path.extension().is_some_and(|extension| extension == "rs") {
                continue;
            }
            let relative = path
                .strip_prefix(repo_root())
                .expect("census test is below repository root")
                .to_string_lossy()
                .replace('\\', "/");
            files.push((
                relative,
                fs::read_to_string(path).expect("read census test source"),
            ));
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

#[test]
fn pinned_runtime_census_is_clean_under_the_canonical_placement_lint() {
    let runtime_ledger =
        fs::read_to_string(repo_root().join("packages/xtask/src/test_runtime_ledger.rs"))
            .expect("read runtime-ledger implementation");
    assert!(runtime_ledger.contains("const PLACEMENT_MARKERS"));
    assert!(runtime_ledger.contains("const CLOCK_MARKERS"));
    assert!(runtime_ledger.contains("runtime-budget-exception:"));
    for marker in PLACEMENT_MARKERS.iter().chain(CLOCK_MARKERS) {
        assert!(
            runtime_ledger.contains(marker),
            "runtime-ledger placement policy lost marker `{marker}`"
        );
    }

    let violations = census_test_files()
        .into_iter()
        .flat_map(|(path, source)| hermetic_violations(&path, &source))
        .collect::<Vec<_>>();
    assert!(
        violations.is_empty(),
        "pinned runtime-ledger census contains nondeterministic hermetic tests:\n{}",
        violations.join("\n")
    );
}

#[test]
fn synthetic_wall_clock_sleep_and_reads_are_rejected() {
    let violations = hermetic_violations(
        "fixture.rs",
        "#[test]\nfn t() { std::thread::sleep(delay); let _ = std::time::Instant::now(); }\n",
    );
    assert_eq!(violations.len(), 2);
    assert!(violations.iter().any(|value| value.contains("placement")));
    assert!(
        violations
            .iter()
            .any(|value| value.contains("deterministic-clock"))
    );
}

#[test]
fn timing_exceptions_do_not_allow_wall_clock_sleep() {
    let violations = hermetic_violations(
        "fixture.rs",
        "let _ = std::time::Instant::now(); // runtime-budget-exception: bounded crypto vector\n\
         std::thread::sleep(delay); // runtime-budget-exception: bounded crypto vector\n",
    );
    assert_eq!(violations.len(), 1);
    assert!(violations[0].contains("wall-clock") || violations[0].contains("thread::sleep"));
}
