//! Hermetic test-runtime ledger and timing gate.
//!
//! Records execution-only timings for individual tests, Provider crates, and
//! Layer-1 shards into a deterministic machine-readable ledger, then enforces
//! the hermetic execution budgets against it, including a historical
//! regression threshold against a previously recorded ledger.
//!
//! Measurement is delegated to the existing `make` targets: this task ingests
//! the timings they produce (either as explicit shard/crate samples or as a
//! `libtest --format=json` stream) and never spawns a test runner itself, so
//! the ledger stays reproducible and free of host paths.

use std::{collections::BTreeMap, fs, path::Path, process::ExitCode};

use serde::{Deserialize, Serialize};

use crate::gen_spec_set::render_json;

pub const ARTIFACT_KIND: &str = "d2b-test-runtime-ledger";
pub const SCHEMA_VERSION: u32 = 1;

/// Individual normal hermetic test: p95 <= 50 ms, no wall-clock sleep.
pub const TEST_BUDGET_MS: u64 = 50;
/// Per Provider crate `--lib --tests` hermetic suite.
pub const CRATE_BUDGET_MS: u64 = 2_000;
/// Each Layer-1 hermetic shard.
pub const SHARD_BUDGET_MS: u64 = 60_000;
/// Default historical regression threshold, as a ratio of the baseline p95.
pub const DEFAULT_REGRESSION_RATIO: f64 = 1.25;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub budget_ms: u64,
    pub id: String,
    pub p95_ms: u64,
    pub samples_ms: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Ledger {
    pub artifact_kind: String,
    pub crates: Vec<Sample>,
    pub repetitions: usize,
    pub runner: String,
    pub schema_version: u32,
    pub shards: Vec<Sample>,
    pub tests: Vec<Sample>,
}

/// Nearest-rank p95 over the recorded samples.
pub fn p95(samples: &[u64]) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let rank = ((sorted.len() as f64) * 0.95).ceil().max(1.0) as usize;
    sorted[rank.min(sorted.len()) - 1]
}

#[derive(Debug, Default)]
struct Collector {
    crates: BTreeMap<String, Vec<u64>>,
    shards: BTreeMap<String, Vec<u64>>,
    tests: BTreeMap<String, Vec<u64>>,
    exceptions: BTreeMap<String, u64>,
}

impl Collector {
    fn into_ledger(self, runner: String, repetitions: usize) -> Ledger {
        let build = |entries: BTreeMap<String, Vec<u64>>, budget: u64| -> Vec<Sample> {
            entries
                .into_iter()
                .map(|(id, mut samples)| {
                    samples.sort_unstable();
                    Sample {
                        budget_ms: budget,
                        p95_ms: p95(&samples),
                        id,
                        samples_ms: samples,
                    }
                })
                .collect()
        };
        let exceptions = self.exceptions.clone();
        let mut tests = build(self.tests, TEST_BUDGET_MS);
        for test in &mut tests {
            if let Some(budget) = exceptions.get(&test.id) {
                test.budget_ms = *budget;
            }
        }
        Ledger {
            artifact_kind: ARTIFACT_KIND.to_string(),
            crates: build(self.crates, CRATE_BUDGET_MS),
            repetitions,
            runner,
            schema_version: SCHEMA_VERSION,
            shards: build(self.shards, SHARD_BUDGET_MS),
            tests,
        }
    }
}

/// A single libtest JSON event carrying an execution time.
#[derive(Debug, Deserialize)]
struct LibtestEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    exec_time: Option<f64>,
}

/// Parses a `libtest --format=json` stream into per-test millisecond samples.
pub fn parse_libtest_json(stream: &str) -> Result<Vec<(String, u64)>, String> {
    let mut out = Vec::new();
    for line in stream.lines() {
        let line = line.trim();
        if line.is_empty() || !line.starts_with('{') {
            continue;
        }
        let event: LibtestEvent = match serde_json::from_str(line) {
            Ok(event) => event,
            Err(error) => return Err(format!("malformed libtest event `{line}`: {error}")),
        };
        if event.kind != "test" {
            continue;
        }
        let (Some(name), Some(seconds)) = (event.name, event.exec_time) else {
            continue;
        };
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(format!("test `{name}` reports a non-measurable exec_time"));
        }
        out.push((name, (seconds * 1000.0).round() as u64));
    }
    Ok(out)
}

/// A budget or regression violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub scope: String,
    pub id: String,
    pub detail: String,
}

/// Enforces the hermetic budgets, plus the historical regression threshold
/// when a previously recorded ledger is supplied.
pub fn check(ledger: &Ledger, baseline: Option<&Ledger>, ratio: f64) -> Vec<Violation> {
    let mut violations = Vec::new();
    let scopes: [(&str, &Vec<Sample>); 3] = [
        ("test", &ledger.tests),
        ("crate", &ledger.crates),
        ("shard", &ledger.shards),
    ];
    for (scope, samples) in scopes {
        let baseline_samples: BTreeMap<&str, u64> = baseline
            .map(|base| match scope {
                "test" => &base.tests,
                "crate" => &base.crates,
                _ => &base.shards,
            })
            .into_iter()
            .flatten()
            .map(|sample| (sample.id.as_str(), sample.p95_ms))
            .collect();
        for sample in samples {
            if sample.p95_ms > sample.budget_ms {
                violations.push(Violation {
                    scope: scope.to_string(),
                    id: sample.id.clone(),
                    detail: format!(
                        "p95 {} ms exceeds the {} ms budget",
                        sample.p95_ms, sample.budget_ms
                    ),
                });
            }
            if let Some(previous) = baseline_samples.get(sample.id.as_str()) {
                let ceiling = (*previous as f64 * ratio).ceil() as u64;
                if *previous > 0 && sample.p95_ms > ceiling {
                    violations.push(Violation {
                        scope: scope.to_string(),
                        id: sample.id.clone(),
                        detail: format!(
                            "p95 {} ms regressed past {ceiling} ms ({previous} ms x {ratio})",
                            sample.p95_ms
                        ),
                    });
                }
            }
        }
    }
    violations.sort_by(|a, b| (&a.scope, &a.id, &a.detail).cmp(&(&b.scope, &b.id, &b.detail)));
    violations
}

/// Returns the slowest recorded tests, worst first.
pub fn top_slow_tests(ledger: &Ledger, limit: usize) -> Vec<&Sample> {
    let mut tests: Vec<&Sample> = ledger.tests.iter().collect();
    tests.sort_by(|a, b| b.p95_ms.cmp(&a.p95_ms).then_with(|| a.id.cmp(&b.id)));
    tests.into_iter().take(limit).collect()
}

// ---------------------------------------------------------------------------
// Placement and deterministic-clock lint
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub path: String,
    pub line: usize,
    pub rule: &'static str,
    pub detail: String,
}

/// Markers that disqualify a source file from the hermetic tier.
const PLACEMENT_MARKERS: &[(&str, &str)] = &[
    ("thread::sleep", "wall-clock sleep"),
    ("tokio::time::sleep", "wall-clock sleep"),
    ("std::process::Command", "process spawn"),
    ("Command::new(", "process spawn"),
    ("TcpStream::", "network access"),
    ("TcpListener::", "network access"),
    ("UdpSocket::", "network access"),
    ("nix-build", "Nix build"),
    ("nix-instantiate", "Nix eval"),
    ("systemctl", "systemd access"),
    ("dbus_", "DBus access"),
    ("zbus::", "DBus access"),
    ("podman", "container runtime"),
    ("docker", "container runtime"),
    ("/dev/kvm", "KVM device"),
    ("/dev/dri/", "GPU device"),
    ("/dev/tpm", "TPM device"),
    ("#[ignore]", "hidden slow test"),
];

/// Markers that break deterministic-clock discipline.
const CLOCK_MARKERS: &[(&str, &str)] = &[
    ("Instant::now(", "wall-clock read"),
    ("SystemTime::now(", "wall-clock read"),
    ("Utc::now(", "wall-clock read"),
];

/// Lints one hermetic-tier source file.
///
/// A line may opt out with a trailing `// runtime-budget-exception:` comment,
/// which is how a classified bounded crypto or property test declares itself.
pub fn lint_source(path: &str, text: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.contains("runtime-budget-exception:") {
            continue;
        }
        let code = line.split("//").next().unwrap_or(line);
        for (marker, detail) in PLACEMENT_MARKERS {
            if code.contains(marker) || (marker.starts_with("#[") && line.trim() == *marker) {
                findings.push(Finding {
                    path: path.to_string(),
                    line: index + 1,
                    rule: "placement",
                    detail: format!("{detail} via `{marker}`"),
                });
            }
        }
        for (marker, detail) in CLOCK_MARKERS {
            if code.contains(marker) {
                findings.push(Finding {
                    path: path.to_string(),
                    line: index + 1,
                    rule: "deterministic-clock",
                    detail: format!("{detail} via `{marker}`"),
                });
            }
        }
    }
    findings
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

const USAGE: &str = "usage: cargo xtask test-runtime-ledger <command>\n\
     \n\
       record --runner <label> --output <path> [--repetitions <n>]\n\
              [--shard <id>=<ms>]... [--crate <id>=<ms>]... [--test <id>=<ms>]...\n\
              [--libtest-json <path>]... [--exception <test-id>=<ms>]...\n\
       check  --ledger <path> [--baseline <path>] [--regression-ratio <f>] [--top <n>]\n\
       lint   <path>...\n\
       help";

pub fn run_cli(args: &[String]) -> ExitCode {
    let result = match args.first().map(String::as_str) {
        Some("record") => run_record(&args[1..]),
        Some("check") => run_check(&args[1..]),
        Some("lint") => run_lint(&args[1..]),
        Some("help") | None => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some(other) => Err(format!(
            "unknown test-runtime-ledger command `{other}`\n{USAGE}"
        )),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("test-runtime-ledger failed: {message}");
            ExitCode::FAILURE
        }
    }
}

fn pair(value: &str, flag: &str) -> Result<(String, u64), String> {
    let (id, millis) = value
        .rsplit_once('=')
        .ok_or_else(|| format!("{flag} expects `<id>=<milliseconds>`, got `{value}`"))?;
    let millis: u64 = millis
        .parse()
        .map_err(|_| format!("{flag} expects integer milliseconds, got `{millis}`"))?;
    Ok((id.to_string(), millis))
}

fn run_record(args: &[String]) -> Result<(), String> {
    let mut collector = Collector::default();
    let mut runner = String::new();
    let mut output = String::new();
    let mut repetitions = 1usize;
    let mut index = 0usize;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("{flag} expects a value"))?;
        match flag {
            "--runner" => runner = value.clone(),
            "--output" => output = value.clone(),
            "--repetitions" => {
                repetitions = value
                    .parse()
                    .map_err(|_| format!("--repetitions expects an integer, got `{value}`"))?
            }
            "--shard" => {
                let (id, millis) = pair(value, flag)?;
                collector.shards.entry(id).or_default().push(millis);
            }
            "--crate" => {
                let (id, millis) = pair(value, flag)?;
                collector.crates.entry(id).or_default().push(millis);
            }
            "--test" => {
                let (id, millis) = pair(value, flag)?;
                collector.tests.entry(id).or_default().push(millis);
            }
            "--exception" => {
                let (id, millis) = pair(value, flag)?;
                collector.exceptions.insert(id, millis);
            }
            "--libtest-json" => {
                let stream = fs::read_to_string(value)
                    .map_err(|error| format!("cannot read `{value}`: {error}"))?;
                for (id, millis) in parse_libtest_json(&stream)? {
                    collector.tests.entry(id).or_default().push(millis);
                }
            }
            other => return Err(format!("unknown record flag `{other}`\n{USAGE}")),
        }
        index += 2;
    }
    if runner.is_empty() {
        return Err("--runner is required so the ledger records its reference runner".to_string());
    }
    if output.is_empty() {
        return Err("--output is required".to_string());
    }
    let ledger = collector.into_ledger(runner, repetitions);
    let rendered = render_json(&ledger).map_err(|error| error.to_string())?;
    if let Some(parent) = Path::new(&output).parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create `{}`: {error}", parent.display()))?;
    }
    fs::write(&output, rendered).map_err(|error| format!("cannot write `{output}`: {error}"))?;
    println!(
        "recorded {} test, {} crate, and {} shard measurements",
        ledger.tests.len(),
        ledger.crates.len(),
        ledger.shards.len()
    );
    Ok(())
}

fn load_ledger(path: &str) -> Result<Ledger, String> {
    let bytes = fs::read(path).map_err(|error| format!("cannot read `{path}`: {error}"))?;
    let ledger: Ledger = serde_json::from_slice(&bytes)
        .map_err(|error| format!("`{path}` is not a valid runtime ledger: {error}"))?;
    if ledger.artifact_kind != ARTIFACT_KIND || ledger.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "`{path}` declares {}/{} instead of {ARTIFACT_KIND}/{SCHEMA_VERSION}",
            ledger.artifact_kind, ledger.schema_version
        ));
    }
    Ok(ledger)
}

fn run_check(args: &[String]) -> Result<(), String> {
    let mut ledger_path = String::new();
    let mut baseline_path: Option<String> = None;
    let mut ratio = DEFAULT_REGRESSION_RATIO;
    let mut top = 10usize;
    let mut index = 0usize;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("{flag} expects a value"))?;
        match flag {
            "--ledger" => ledger_path = value.clone(),
            "--baseline" => baseline_path = Some(value.clone()),
            "--regression-ratio" => {
                ratio = value
                    .parse()
                    .map_err(|_| format!("--regression-ratio expects a number, got `{value}`"))?
            }
            "--top" => {
                top = value
                    .parse()
                    .map_err(|_| format!("--top expects an integer, got `{value}`"))?
            }
            other => return Err(format!("unknown check flag `{other}`\n{USAGE}")),
        }
        index += 2;
    }
    if ledger_path.is_empty() {
        return Err("--ledger is required".to_string());
    }
    if !(1.0..=10.0).contains(&ratio) {
        return Err(format!(
            "--regression-ratio must be between 1.0 and 10.0, got {ratio}"
        ));
    }
    let ledger = load_ledger(&ledger_path)?;
    let baseline = baseline_path.as_deref().map(load_ledger).transpose()?;
    for sample in top_slow_tests(&ledger, top) {
        println!(
            "slowest: {} p95 {} ms (budget {} ms)",
            sample.id, sample.p95_ms, sample.budget_ms
        );
    }
    let violations = check(&ledger, baseline.as_ref(), ratio);
    if violations.is_empty() {
        println!(
            "runtime budgets hold for {} test, {} crate, and {} shard measurements on `{}`",
            ledger.tests.len(),
            ledger.crates.len(),
            ledger.shards.len(),
            ledger.runner
        );
        return Ok(());
    }
    for violation in &violations {
        eprintln!(
            "{} `{}`: {}",
            violation.scope, violation.id, violation.detail
        );
    }
    Err(format!("{} runtime budget violation(s)", violations.len()))
}

fn run_lint(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err(format!("lint expects at least one path\n{USAGE}"));
    }
    let mut findings = Vec::new();
    for path in args {
        let text =
            fs::read_to_string(path).map_err(|error| format!("cannot read `{path}`: {error}"))?;
        findings.extend(lint_source(path, &text));
    }
    if findings.is_empty() {
        println!(
            "hermetic placement lint clean across {} file(s)",
            args.len()
        );
        return Ok(());
    }
    for finding in &findings {
        eprintln!(
            "{}:{}: {} - {}",
            finding.path, finding.line, finding.rule, finding.detail
        );
    }
    Err(format!(
        "{} hermetic placement violation(s)",
        findings.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str, budget: u64, samples: &[u64]) -> Sample {
        Sample {
            budget_ms: budget,
            id: id.to_string(),
            p95_ms: p95(samples),
            samples_ms: samples.to_vec(),
        }
    }

    fn ledger(tests: Vec<Sample>) -> Ledger {
        Ledger {
            artifact_kind: ARTIFACT_KIND.to_string(),
            crates: Vec::new(),
            repetitions: 3,
            runner: "reference".to_string(),
            schema_version: SCHEMA_VERSION,
            shards: Vec::new(),
            tests,
        }
    }

    #[test]
    fn p95_uses_nearest_rank() {
        assert_eq!(p95(&[]), 0);
        assert_eq!(p95(&[7]), 7);
        assert_eq!(p95(&[1, 2, 3, 4]), 4);
        let twenty: Vec<u64> = (1..=20).collect();
        assert_eq!(p95(&twenty), 19);
    }

    #[test]
    fn an_over_budget_hermetic_test_fails_the_gate() {
        let over = ledger(vec![sample("slow::test", TEST_BUDGET_MS, &[10, 20, 90])]);
        let violations = check(&over, None, DEFAULT_REGRESSION_RATIO);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].scope, "test");
        assert!(violations[0].detail.contains("exceeds the 50 ms budget"));
    }

    #[test]
    fn a_classified_exception_may_declare_a_higher_budget() {
        let mut collector = Collector::default();
        collector
            .tests
            .entry("crypto::kdf_vectors".to_string())
            .or_default()
            .push(400);
        collector
            .exceptions
            .insert("crypto::kdf_vectors".to_string(), 500);
        let recorded = collector.into_ledger("reference".to_string(), 1);
        assert_eq!(recorded.tests[0].budget_ms, 500);
        assert!(check(&recorded, None, DEFAULT_REGRESSION_RATIO).is_empty());
    }

    #[test]
    fn a_synthetic_timing_regression_fails_the_gate() {
        let baseline = ledger(vec![sample("steady::test", TEST_BUDGET_MS, &[10])]);
        let regressed = ledger(vec![sample("steady::test", TEST_BUDGET_MS, &[40])]);
        let violations = check(&regressed, Some(&baseline), DEFAULT_REGRESSION_RATIO);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].detail.contains("regressed past"));

        let steady = ledger(vec![sample("steady::test", TEST_BUDGET_MS, &[12])]);
        assert!(check(&steady, Some(&baseline), DEFAULT_REGRESSION_RATIO).is_empty());
    }

    #[test]
    fn libtest_json_streams_become_millisecond_samples() {
        let stream = "{ \"type\": \"suite\", \"event\": \"started\", \"test_count\": 1 }\n\
                      { \"type\": \"test\", \"name\": \"a::b\", \"event\": \"ok\", \"exec_time\": 0.012 }\n\
                      { \"type\": \"suite\", \"event\": \"ok\" }\n";
        assert_eq!(
            parse_libtest_json(stream).expect("parses"),
            vec![("a::b".to_string(), 12)]
        );
        assert!(parse_libtest_json("{ \"type\": \"test\" ").is_err());
    }

    #[test]
    fn placement_lint_rejects_sleep_process_and_network_in_a_hermetic_test() {
        let source = "#[test]\n\
                      fn slow() {\n    \
                      std::thread::sleep(std::time::Duration::from_secs(1));\n    \
                      let _ = std::process::Command::new(\"true\").status();\n    \
                      let _ = std::net::TcpStream::connect(\"127.0.0.1:80\");\n}\n";
        let findings = lint_source("packages/x/tests/slow.rs", source);
        let rules: Vec<&str> = findings.iter().map(|finding| finding.rule).collect();
        assert!(rules.iter().all(|rule| *rule == "placement"));
        assert!(
            findings
                .iter()
                .any(|f| f.detail.contains("wall-clock sleep"))
        );
        assert!(findings.iter().any(|f| f.detail.contains("process spawn")));
        assert!(findings.iter().any(|f| f.detail.contains("network access")));
    }

    #[test]
    fn placement_lint_rejects_wall_clock_reads_and_hidden_slow_tests() {
        let source = "#[ignore]\n#[test]\nfn t() { let _ = std::time::Instant::now(); }\n";
        let findings = lint_source("packages/x/src/lib.rs", source);
        assert!(findings.iter().any(|f| f.rule == "deterministic-clock"));
        assert!(
            findings
                .iter()
                .any(|f| f.detail.contains("hidden slow test"))
        );
    }

    #[test]
    fn a_classified_exception_comment_opts_a_line_out() {
        let source = "let _ = std::time::Instant::now(); // runtime-budget-exception: bounded crypto vector\n";
        assert!(lint_source("packages/x/src/lib.rs", source).is_empty());
    }

    #[test]
    fn the_ledger_round_trips_deterministically() {
        let recorded = ledger(vec![sample("a::b", TEST_BUDGET_MS, &[3, 1, 2])]);
        let rendered = render_json(&recorded).expect("renders");
        let parsed: Ledger = serde_json::from_str(&rendered).expect("parses");
        assert_eq!(parsed, recorded);
        assert_eq!(render_json(&parsed).expect("renders"), rendered);
    }

    #[test]
    fn top_slow_tests_report_the_worst_offenders_first() {
        let recorded = ledger(vec![
            sample("a::fast", TEST_BUDGET_MS, &[1]),
            sample("b::slow", TEST_BUDGET_MS, &[40]),
        ]);
        let top = top_slow_tests(&recorded, 1);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].id, "b::slow");
    }
}
