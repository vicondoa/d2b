//! Hermetic test-runtime ledger and timing gate.
//!
//! Records advisory per-test wall-clock timings and enforced aggregate crate
//! CPU timings into a deterministic machine-readable ledger.
//!
//! The gate is an absolute aggregate process-CPU budget check on a freshly
//! recorded ledger: it makes no historical-regression claim and holds no
//! baseline. Per-test wall-clock values are diagnostic only. Every run is
//! judged against the frozen crate budgets and the pinned closed census. A
//! genuine cross-machine reference baseline and a real multi-crate shard
//! inventory (with a per-shard budget over that inventory) are deferred; see
//! the `test-runtime-ledger` Makefile target for the named follow-up
//! `runtime-ledger-full-census-and-real-shards`.
//!
//! Measurement is delegated to the existing `make` targets: this task ingests
//! the timings they produce (either as explicit crate samples or as a
//! `libtest --format=json` stream) and never spawns a test runner itself, so
//! the ledger stays reproducible and free of host paths.

use std::{collections::BTreeMap, collections::BTreeSet, fs, path::Path, process::ExitCode};

use serde::{Deserialize, Serialize};

use crate::{diagnostic_redaction::redact_path, gen_spec_set::render_json};

pub const ARTIFACT_KIND: &str = "d2b-test-runtime-ledger";
pub const SCHEMA_VERSION: u32 = 2;

/// Advisory threshold for an individual normal hermetic test.
///
/// Libtest reports wall-clock time, which varies with unrelated machine load,
/// so this threshold ranks diagnostics but never fails the gate.
pub const TEST_ADVISORY_THRESHOLD_MS: u64 = 50;
/// Enforced aggregate process-CPU budget for a Provider crate's hermetic suite.
pub const CRATE_BUDGET_MS: u64 = 2_000;
/// Minimum execution-only repetitions the ledger must carry before its p95 or
/// per-crate budgets mean anything. A single CPU sample is too thin to support
/// a stable nearest-rank p95, so the gate refuses fewer repetitions.
pub const MIN_REPETITIONS: usize = 3;

/// Upper bound on a printable identifier (test id or crate id).
/// Long enough for any crate-qualified `crate::module::submodule::test` path
/// and short enough that a host path, a multi-line log fragment, or an
/// unbounded blob cannot masquerade as an id.
pub const MAX_ID_LEN: usize = 256;
/// Upper bound on the free-form runner label. It is a short human handle for a
/// reference machine, never a hostname or a path.
pub const MAX_RUNNER_LABEL_LEN: usize = 64;
/// Upper bound on how many raw samples a single id may carry. A run records one
/// sample per repetition, so this caps a hostile or corrupt ledger from
/// inflating artifact cardinality without bound.
pub const MAX_SAMPLES_PER_ID: usize = 4_096;
/// Upper bound on a libtest JSON stream this task will ingest. A stream larger
/// than this is rejected before it is parsed so a giant blob cannot be folded
/// into the ledger or echoed into a violation message.
pub const MAX_LIBTEST_BYTES: usize = 8 * 1024 * 1024;

/// Validate the short, closed runner-label grammar.
///
/// A label is `[a-z0-9]` followed by up to [`MAX_RUNNER_LABEL_LEN`]`-1` further
/// `[a-z0-9._-]` characters. That admits `local`, `ci-x86-runner`,
/// `bench.host-1` and refuses anything carrying whitespace, control
/// characters, path separators, or shell metacharacters, so the label can be
/// printed verbatim in a violation line or a ledger without injecting newlines
/// or leaking a host path.
pub fn validate_runner_label(label: &str) -> Result<(), String> {
    if label.is_empty() {
        return Err("runner label must not be empty".to_string());
    }
    if label.len() > MAX_RUNNER_LABEL_LEN {
        return Err(format!(
            "runner label is {} bytes; the maximum is {MAX_RUNNER_LABEL_LEN}",
            label.len()
        ));
    }
    let mut chars = label.chars();
    let first = chars.next().expect("label is non-empty");
    if !first.is_ascii_alphanumeric() {
        return Err(
            "runner label must start with a lowercase letter or digit; got a disallowed \
             leading character"
                .to_string(),
        );
    }
    if first.is_ascii_uppercase() {
        return Err("runner label must be lowercase".to_string());
    }
    for ch in label.chars() {
        let ok = ch.is_ascii_digit() || ch.is_ascii_lowercase() || matches!(ch, '.' | '_' | '-');
        if !ok {
            return Err(
                "runner label admits only lowercase letters, digits, '.', '_', and '-'".to_string(),
            );
        }
    }
    Ok(())
}

/// Validate a printable, bounded, control-free identifier.
///
/// Every id (test or crate) must be non-empty, at most [`MAX_ID_LEN`]
/// bytes, and composed only of printable ASCII (`0x20..=0x7e`). Rejecting
/// control characters and non-ASCII bytes stops a newline or terminal escape
/// from being serialized into the ledger and then echoed verbatim into a
/// slow-test or violation line; bounding the length stops a host path or log
/// blob from being smuggled in as an id.
pub fn validate_id(scope: &str, id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err(format!("{scope} id must not be empty"));
    }
    if id.len() > MAX_ID_LEN {
        return Err(format!(
            "{scope} id is {} bytes; the maximum is {MAX_ID_LEN}",
            id.len()
        ));
    }
    for byte in id.bytes() {
        if !(0x20..=0x7e).contains(&byte) {
            return Err(format!(
                "{scope} id carries a non-printable or non-ASCII byte (0x{byte:02x}); ids must \
                 be printable ASCII so they cannot inject control characters into output"
            ));
        }
    }
    Ok(())
}

/// Validate every identifier, label, and bound in a ledger.
///
/// Applied both when a ledger is emitted and when one is loaded at check time,
/// so a hand-edited or hostile ledger cannot slip an oversized, control-bearing,
/// or unbounded row past the gate.
pub fn validate_ledger(ledger: &Ledger) -> Result<(), String> {
    validate_runner_label(&ledger.runner)?;
    let scopes: [(&str, &Vec<Sample>); 2] = [("test", &ledger.tests), ("crate", &ledger.crates)];
    for (scope, samples) in scopes {
        for sample in samples {
            validate_id(scope, &sample.id)?;
            if sample.samples_ms.len() > MAX_SAMPLES_PER_ID {
                return Err(format!(
                    "{scope} id `{}` carries {} samples; the maximum is {MAX_SAMPLES_PER_ID}",
                    sample.id,
                    sample.samples_ms.len()
                ));
            }
            let expected_shape = match scope {
                "test" => (TimingBasis::WallClock, Enforcement::Advisory),
                "crate" => (TimingBasis::ProcessCpu, Enforcement::Budget),
                _ => unreachable!("the scope table is closed"),
            };
            if (sample.basis, sample.enforcement) != expected_shape {
                return Err(format!(
                    "{scope} id `{}` declares {:?}/{:?}; expected {:?}/{:?}",
                    sample.id, sample.basis, sample.enforcement, expected_shape.0, expected_shape.1
                ));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub basis: TimingBasis,
    pub enforcement: Enforcement,
    pub id: String,
    pub p95_ms: u64,
    pub samples_ms: Vec<u64>,
    pub threshold_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TimingBasis {
    ProcessCpu,
    WallClock,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Enforcement {
    Advisory,
    Budget,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Ledger {
    pub artifact_kind: String,
    pub crates: Vec<Sample>,
    pub repetitions: usize,
    pub runner: String,
    pub schema_version: u32,
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
    tests: BTreeMap<String, Vec<u64>>,
    advisory_thresholds: BTreeMap<String, u64>,
}

impl Collector {
    fn into_ledger(self, runner: String, repetitions: usize) -> Ledger {
        let build = |entries: BTreeMap<String, Vec<u64>>,
                     budget: u64,
                     basis: TimingBasis,
                     enforcement: Enforcement|
         -> Vec<Sample> {
            entries
                .into_iter()
                .map(|(id, mut samples)| {
                    samples.sort_unstable();
                    Sample {
                        basis,
                        enforcement,
                        p95_ms: p95(&samples),
                        id,
                        samples_ms: samples,
                        threshold_ms: budget,
                    }
                })
                .collect()
        };
        let advisory_thresholds = self.advisory_thresholds.clone();
        let mut tests = build(
            self.tests,
            TEST_ADVISORY_THRESHOLD_MS,
            TimingBasis::WallClock,
            Enforcement::Advisory,
        );
        for test in &mut tests {
            if let Some(budget) = advisory_thresholds.get(&test.id) {
                test.threshold_ms = *budget;
            }
        }
        Ledger {
            artifact_kind: ARTIFACT_KIND.to_string(),
            crates: build(
                self.crates,
                CRATE_BUDGET_MS,
                TimingBasis::ProcessCpu,
                Enforcement::Budget,
            ),
            repetitions,
            runner,
            schema_version: SCHEMA_VERSION,
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
///
/// The stream is bounded to [`MAX_LIBTEST_BYTES`] before parsing and every test
/// name is validated as a printable, bounded id, so neither an unbounded blob
/// nor a control-bearing name can be folded into the ledger.
pub fn parse_libtest_json(stream: &str) -> Result<Vec<(String, u64)>, String> {
    if stream.len() > MAX_LIBTEST_BYTES {
        return Err(format!(
            "libtest stream is {} bytes; the maximum is {MAX_LIBTEST_BYTES}",
            stream.len()
        ));
    }
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
        validate_id("test", &name)?;
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(format!("test `{name}` reports a non-measurable exec_time"));
        }
        out.push((name, (seconds * 1000.0).round() as u64));
    }
    Ok(out)
}

/// A budget or census violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub scope: String,
    pub id: String,
    pub detail: String,
}

/// Enforces the aggregate process-CPU crate budgets.
///
/// This is a budget gate, not a regression gate: each recorded p95 is judged
/// only against its frozen CPU budget, with no historical anchor. Per-test
/// wall-clock samples are explicitly advisory and never produce a violation.
pub fn check(ledger: &Ledger) -> Vec<Violation> {
    let mut violations = Vec::new();
    for sample in &ledger.crates {
        if sample.p95_ms > sample.threshold_ms {
            violations.push(Violation {
                scope: "crate".to_string(),
                id: sample.id.clone(),
                detail: format!(
                    "p95 {} ms exceeds the {} ms process-CPU budget",
                    sample.p95_ms, sample.threshold_ms
                ),
            });
        }
    }
    violations.sort_by(|a, b| (&a.scope, &a.id, &a.detail).cmp(&(&b.scope, &b.id, &b.detail)));
    violations
}

/// Enforces that the ledger is a *complete* census before its budgets are
/// trusted, independent of whether any single budget is exceeded.
///
/// `check` only rules on samples that are present; this rules on what must be
/// present. Without it a ledger can pass every budget while measuring nothing
/// meaningful: one un-repeated sample, or an empty scope.
///
/// A run fails the audit when any of the following holds:
///
/// * it records fewer than [`MIN_REPETITIONS`] execution-only repetitions;
/// * either the test or crate census is empty; or
/// * any sample carries a number of samples other than the declared
///   repetition count, so every id is measured every repetition.
pub fn audit_census(ledger: &Ledger) -> Vec<Violation> {
    let mut violations = Vec::new();

    if ledger.repetitions < MIN_REPETITIONS {
        violations.push(Violation {
            scope: "ledger".to_string(),
            id: "repetitions".to_string(),
            detail: format!(
                "{} execution-only repetition(s) recorded; at least {MIN_REPETITIONS} are \
                 required so a single sample cannot flap the gate",
                ledger.repetitions
            ),
        });
    }

    let scopes: [(&str, &Vec<Sample>); 2] = [("test", &ledger.tests), ("crate", &ledger.crates)];
    for (scope, samples) in scopes {
        if samples.is_empty() {
            violations.push(Violation {
                scope: scope.to_string(),
                id: "*".to_string(),
                detail: format!(
                    "the {scope} census is empty; every granularity must be measured so the \
                     advertised {scope} budget is actually enforced"
                ),
            });
        }
        for sample in samples {
            if sample.samples_ms.len() != ledger.repetitions {
                violations.push(Violation {
                    scope: scope.to_string(),
                    id: sample.id.clone(),
                    detail: format!(
                        "carries {} sample(s) but the ledger declares {} repetition(s); every \
                         id must be measured on every repetition",
                        sample.samples_ms.len(),
                        ledger.repetitions
                    ),
                });
            }
        }
    }

    violations.sort_by(|a, b| (&a.scope, &a.id, &a.detail).cmp(&(&b.scope, &b.id, &b.detail)));
    violations
}

/// The pinned, closed crate census the ledger must reproduce exactly.
///
/// Loaded from a committed JSON pin so the set of measured crates is
/// fixed in the repository, not chosen per run. `audit_closed_census` enforces
/// exact-set equality against it, which is what stops the gate from passing on
/// "one arbitrary repeated id per scope": a census that drops a pinned crate,
/// substitutes a different one, or adds an unpinned one fails closed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpectedCensus {
    pub crates: Vec<String>,
    pub tests: Vec<String>,
}

impl ExpectedCensus {
    /// Validate the pin's own ids so a malformed pin cannot smuggle a
    /// control-bearing or oversized id into a violation line.
    pub fn validate(&self) -> Result<(), String> {
        if self.crates.is_empty() {
            return Err(
                "the expected census pins no crates; it must pin the closed \
                        hermetic crate set"
                    .to_string(),
            );
        }
        for id in &self.crates {
            validate_id("crate", id)?;
        }
        if self.tests.is_empty() {
            return Err(
                "the expected census pins no tests; a crate name alone cannot prove any test ran"
                    .to_string(),
            );
        }
        for id in &self.tests {
            validate_id("test", id)?;
        }
        Ok(())
    }
}

/// Enforce that the ledger's crate and test censuses reproduce the pinned
/// closed sets *exactly* - no missing pinned id, and no unpinned extra.
///
/// `audit_census` proves the run is complete and repeated; this proves it is
/// measuring the pinned set and only the pinned set. Together they close the
/// "accepts one arbitrary repeated id" hole: a run cannot shrink a scope to a
/// single convenient crate, and it cannot pad a scope with an id that is not in
/// the committed census.
pub fn audit_closed_census(ledger: &Ledger, expected: &ExpectedCensus) -> Vec<Violation> {
    let mut violations = Vec::new();
    let compared: [(&str, &Vec<String>, &Vec<Sample>); 2] = [
        ("crate", &expected.crates, &ledger.crates),
        ("test", &expected.tests, &ledger.tests),
    ];
    for (scope, expected_ids, samples) in compared {
        let want: BTreeSet<&str> = expected_ids.iter().map(String::as_str).collect();
        let have: BTreeSet<&str> = samples.iter().map(|s| s.id.as_str()).collect();
        for id in want.difference(&have) {
            violations.push(Violation {
                scope: scope.to_string(),
                id: (*id).to_string(),
                detail: "pinned in the closed census but missing from this run; the census \
                         cannot shrink to evade a budget"
                    .to_string(),
            });
        }
        for id in have.difference(&want) {
            violations.push(Violation {
                scope: scope.to_string(),
                id: (*id).to_string(),
                detail: "measured but not pinned in the closed census; add it to the census \
                         pin (or the regeneration helper) before the gate will accept it"
                    .to_string(),
            });
        }
    }
    violations.sort_by(|a, b| (&a.scope, &a.id, &a.detail).cmp(&(&b.scope, &b.id, &b.detail)));
    violations
}

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
              [--crate <id>=<ms>]... [--test <id>=<ms>]...\n\
              [--libtest-json <path>]... [--crate-libtest-json <crate>=<path>]...\n\
              [--advisory-threshold <test-id>=<ms>]...\n\
       check  --ledger <path> --expected-census <path> [--top <n>]\n\
       census --expected-census <path> --field crates\n\
       lint   <path>...\n\
       help";

pub fn run_cli(args: &[String]) -> ExitCode {
    let result = match args.first().map(String::as_str) {
        Some("record") => run_record(&args[1..]),
        Some("check") => run_check(&args[1..]),
        Some("census") => run_census(&args[1..]),
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

/// Load and validate the pinned closed census from a committed JSON pin.
fn load_expected_census(path: &str) -> Result<ExpectedCensus, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("cannot read the expected census pin: {error}"))?;
    let census: ExpectedCensus = serde_json::from_slice(&bytes).map_err(|error| {
        format!("the expected census pin is not a valid expected census: {error}")
    })?;
    census.validate()?;
    Ok(census)
}

fn run_census(args: &[String]) -> Result<(), String> {
    let mut census_path = String::new();
    let mut field = String::new();
    let mut index = 0usize;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("{flag} expects a value"))?;
        match flag {
            "--expected-census" => census_path = value.clone(),
            "--field" => field = value.clone(),
            other => return Err(format!("unknown census flag `{other}`\n{USAGE}")),
        }
        index += 2;
    }
    if census_path.is_empty() {
        return Err("--expected-census is required".to_string());
    }
    let census = load_expected_census(&census_path)?;
    let ids = match field.as_str() {
        "crates" => &census.crates,
        other => {
            return Err(format!("--field expects `crates`, got `{other}`"));
        }
    };
    for id in ids {
        println!("{id}");
    }
    Ok(())
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
            "--crate" => {
                let (id, millis) = pair(value, flag)?;
                validate_id("crate", &id)?;
                collector.crates.entry(id).or_default().push(millis);
            }
            "--test" => {
                let (id, millis) = pair(value, flag)?;
                validate_id("test", &id)?;
                collector.tests.entry(id).or_default().push(millis);
            }
            "--advisory-threshold" => {
                let (id, millis) = pair(value, flag)?;
                validate_id("test", &id)?;
                collector.advisory_thresholds.insert(id, millis);
            }
            "--libtest-json" => {
                let stream = fs::read_to_string(value)
                    .map_err(|error| format!("cannot read the libtest stream: {error}"))?;
                let measurements = parse_libtest_json(&stream)?;
                if measurements.is_empty() {
                    return Err(
                        "the libtest stream contains no timed test events; refusing to record an \
                         empty measurement set"
                            .to_string(),
                    );
                }
                for (id, millis) in measurements {
                    collector.tests.entry(id).or_default().push(millis);
                }
            }
            "--crate-libtest-json" => {
                let (crate_id, path) = value
                    .split_once('=')
                    .ok_or_else(|| format!("{flag} expects `<crate>=<path>`"))?;
                validate_id("crate", crate_id)?;
                let stream = fs::read_to_string(path).map_err(|error| {
                    format!("cannot read the libtest stream for crate `{crate_id}`: {error}")
                })?;
                let measurements = parse_libtest_json(&stream)?;
                if measurements.is_empty() {
                    return Err(format!(
                        "the libtest stream for crate `{crate_id}` contains no timed test events; \
                         refusing to record an empty measurement set"
                    ));
                }
                for (name, millis) in measurements {
                    // Crate-qualify the libtest name so `d2b-core::foo::bar`
                    // cannot collide with an identically-named test in another
                    // crate's suite.
                    let id = format!("{crate_id}::{name}");
                    validate_id("test", &id)?;
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
    validate_runner_label(&runner)?;
    if output.is_empty() {
        return Err("--output is required".to_string());
    }
    let ledger = collector.into_ledger(runner, repetitions);
    // Validate before writing so an oversized, control-bearing, or unbounded
    // row is refused at emit time, not only when the ledger is later loaded.
    validate_ledger(&ledger)?;
    let rendered = render_json(&ledger).map_err(|error| error.to_string())?;
    if let Some(parent) = Path::new(&output).parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!("cannot create the runtime ledger output directory: {error}")
        })?;
    }
    fs::write(&output, rendered)
        .map_err(|error| format!("cannot write the runtime ledger output: {error}"))?;
    println!(
        "recorded {} test and {} crate measurements",
        ledger.tests.len(),
        ledger.crates.len()
    );
    Ok(())
}

fn load_ledger(path: &str) -> Result<Ledger, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("cannot read the runtime ledger: {error}"))?;
    let ledger: Ledger = serde_json::from_slice(&bytes)
        .map_err(|error| format!("the runtime ledger is not valid JSON: {error}"))?;
    if ledger.artifact_kind != ARTIFACT_KIND || ledger.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "the runtime ledger declares {}/{} instead of {ARTIFACT_KIND}/{SCHEMA_VERSION}",
            ledger.artifact_kind, ledger.schema_version
        ));
    }
    // Validate on load, so a hand-edited or hostile ledger cannot slip an
    // oversized or control-bearing id past the gate.
    validate_ledger(&ledger).map_err(|error| format!("the runtime ledger: {error}"))?;
    Ok(ledger)
}

fn run_check(args: &[String]) -> Result<(), String> {
    let mut ledger_path = String::new();
    let mut census_path: Option<String> = None;
    let mut top = 10usize;
    let mut index = 0usize;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("{flag} expects a value"))?;
        match flag {
            "--ledger" => ledger_path = value.clone(),
            "--expected-census" => census_path = Some(value.clone()),
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
    // A pinned closed census is mandatory: without it the gate could pass on a
    // scope shrunk to one convenient id.
    let census_path = census_path.ok_or_else(|| {
        "--expected-census is required so the crate census is enforced against a \
         pinned closed set"
            .to_string()
    })?;
    let ledger = load_ledger(&ledger_path)?;
    let expected = load_expected_census(&census_path)?;
    for sample in top_slow_tests(&ledger, top) {
        println!(
            "advisory wall-clock: {} p95 {} ms (diagnostic threshold {} ms)",
            sample.id, sample.p95_ms, sample.threshold_ms
        );
    }
    for sample in &ledger.crates {
        println!(
            "enforced process CPU: {} p95 {} ms (budget {} ms; samples {:?})",
            sample.id, sample.p95_ms, sample.threshold_ms, sample.samples_ms
        );
    }
    let mut violations = check(&ledger);
    violations.extend(audit_census(&ledger));
    violations.extend(audit_closed_census(&ledger, &expected));
    violations.sort_by(|a, b| (&a.scope, &a.id, &a.detail).cmp(&(&b.scope, &b.id, &b.detail)));
    violations.dedup();
    if violations.is_empty() {
        println!(
            "runtime CPU budgets hold for {} crate measurement(s) on `{}`; \
             captured {} advisory per-test wall-clock measurement(s)",
            ledger.crates.len(),
            ledger.runner,
            ledger.tests.len(),
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
        let text = fs::read_to_string(path).map_err(|error| {
            format!(
                "cannot read lint target `{}`: {error}",
                redact_path(Path::new(path))
            )
        })?;
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
            redact_path(Path::new(&finding.path)),
            finding.line,
            finding.rule,
            finding.detail
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

    fn test_sample(id: &str, budget: u64, samples: &[u64]) -> Sample {
        Sample {
            basis: TimingBasis::WallClock,
            enforcement: Enforcement::Advisory,
            id: id.to_string(),
            p95_ms: p95(samples),
            samples_ms: samples.to_vec(),
            threshold_ms: budget,
        }
    }

    fn crate_sample(id: &str, budget: u64, samples: &[u64]) -> Sample {
        Sample {
            basis: TimingBasis::ProcessCpu,
            enforcement: Enforcement::Budget,
            id: id.to_string(),
            p95_ms: p95(samples),
            samples_ms: samples.to_vec(),
            threshold_ms: budget,
        }
    }

    fn ledger(tests: Vec<Sample>) -> Ledger {
        Ledger {
            artifact_kind: ARTIFACT_KIND.to_string(),
            crates: Vec::new(),
            repetitions: 3,
            runner: "reference".to_string(),
            schema_version: SCHEMA_VERSION,
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
    fn an_over_threshold_wall_clock_test_is_advisory() {
        let over = ledger(vec![test_sample(
            "slow::test",
            TEST_ADVISORY_THRESHOLD_MS,
            &[10, 20, 90],
        )]);
        assert!(
            check(&over).is_empty(),
            "per-test wall-clock contention must not fail a CPU budget gate"
        );
    }

    #[test]
    fn an_over_budget_crate_cpu_measurement_fails_the_gate() {
        let mut over = ledger(Vec::new());
        over.crates
            .push(crate_sample("d2b-core", CRATE_BUDGET_MS, &[1_000, 2_001]));
        let violations = check(&over);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].scope, "crate");
        assert!(
            violations[0]
                .detail
                .contains("exceeds the 2000 ms process-CPU budget")
        );
    }

    #[test]
    fn a_classified_test_may_declare_a_higher_advisory_threshold() {
        let mut collector = Collector::default();
        collector
            .tests
            .entry("crypto::kdf_vectors".to_string())
            .or_default()
            .push(400);
        collector
            .advisory_thresholds
            .insert("crypto::kdf_vectors".to_string(), 500);
        let recorded = collector.into_ledger("reference".to_string(), 1);
        assert_eq!(recorded.tests[0].threshold_ms, 500);
        assert_eq!(recorded.tests[0].enforcement, Enforcement::Advisory);
        assert!(check(&recorded).is_empty());
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
        let recorded = ledger(vec![test_sample(
            "a::b",
            TEST_ADVISORY_THRESHOLD_MS,
            &[3, 1, 2],
        )]);
        let rendered = render_json(&recorded).expect("renders");
        let parsed: Ledger = serde_json::from_str(&rendered).expect("parses");
        assert_eq!(parsed, recorded);
        assert_eq!(render_json(&parsed).expect("renders"), rendered);
    }

    #[test]
    fn top_slow_tests_report_the_worst_offenders_first() {
        let recorded = ledger(vec![
            test_sample("a::fast", TEST_ADVISORY_THRESHOLD_MS, &[1]),
            test_sample("b::slow", TEST_ADVISORY_THRESHOLD_MS, &[40]),
        ]);
        let top = top_slow_tests(&recorded, 1);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].id, "b::slow");
    }

    /// A complete, three-repetition census across every granularity.
    fn full_ledger(repetitions: usize) -> Ledger {
        let three =
            |id: &str, budget: u64, basis: TimingBasis, enforcement: Enforcement| -> Sample {
                Sample {
                    basis,
                    enforcement,
                    id: id.to_string(),
                    p95_ms: 1,
                    samples_ms: vec![1; repetitions],
                    threshold_ms: budget,
                }
            };
        Ledger {
            artifact_kind: ARTIFACT_KIND.to_string(),
            crates: vec![three(
                "d2b-core",
                CRATE_BUDGET_MS,
                TimingBasis::ProcessCpu,
                Enforcement::Budget,
            )],
            repetitions,
            runner: "reference".to_string(),
            schema_version: SCHEMA_VERSION,
            tests: vec![three(
                "a::b",
                TEST_ADVISORY_THRESHOLD_MS,
                TimingBasis::WallClock,
                Enforcement::Advisory,
            )],
        }
    }

    #[test]
    fn audit_passes_a_complete_repeated_census() {
        let complete = full_ledger(MIN_REPETITIONS);
        assert!(
            audit_census(&complete).is_empty(),
            "a complete, repeated census across every scope is accepted"
        );
    }

    #[test]
    fn audit_rejects_a_single_sample_that_could_flap() {
        let thin = full_ledger(1);
        let violations = audit_census(&thin);
        assert!(
            violations
                .iter()
                .any(|v| v.scope == "ledger" && v.id == "repetitions"),
            "one repetition is below the floor: {violations:?}"
        );
    }

    #[test]
    fn audit_rejects_an_empty_scope() {
        let mut missing = full_ledger(MIN_REPETITIONS);
        missing.crates.clear();
        let violations = audit_census(&missing);
        assert!(
            violations
                .iter()
                .any(|v| v.scope == "crate" && v.detail.contains("empty")),
            "an unmeasured crate census is rejected: {violations:?}"
        );
    }

    #[test]
    fn audit_rejects_a_sample_count_that_does_not_match_the_repetitions() {
        let mut ragged = full_ledger(MIN_REPETITIONS);
        ragged.tests[0].samples_ms.pop();
        let violations = audit_census(&ragged);
        assert!(
            violations
                .iter()
                .any(|v| v.scope == "test" && v.detail.contains("every id must be measured")),
            "a short sample vector is rejected: {violations:?}"
        );
    }

    fn expected_census() -> ExpectedCensus {
        ExpectedCensus {
            crates: vec!["d2b-core".to_string()],
            tests: vec!["a::b".to_string()],
        }
    }

    #[test]
    fn closed_census_accepts_an_exact_match() {
        let complete = full_ledger(MIN_REPETITIONS);
        assert!(
            audit_closed_census(&complete, &expected_census()).is_empty(),
            "a ledger reproducing the pinned closed set exactly is accepted"
        );
    }

    #[test]
    fn closed_census_rejects_a_shrunk_scope() {
        let mut shrunk = full_ledger(MIN_REPETITIONS);
        shrunk.crates.clear();
        let violations = audit_closed_census(&shrunk, &expected_census());
        assert!(
            violations
                .iter()
                .any(|v| v.scope == "crate" && v.id == "d2b-core" && v.detail.contains("missing")),
            "dropping a pinned crate is rejected: {violations:?}"
        );
    }

    #[test]
    fn closed_census_rejects_a_disappeared_test() {
        let mut shrunk = full_ledger(MIN_REPETITIONS);
        shrunk.tests.clear();
        let violations = audit_closed_census(&shrunk, &expected_census());
        assert!(
            violations
                .iter()
                .any(|v| v.scope == "test" && v.id == "a::b" && v.detail.contains("missing")),
            "dropping a pinned test is rejected: {violations:?}"
        );
    }

    #[test]
    fn closed_census_rejects_an_unpinned_extra() {
        let mut padded = full_ledger(MIN_REPETITIONS);
        padded.crates.push(Sample {
            basis: TimingBasis::ProcessCpu,
            enforcement: Enforcement::Budget,
            id: "not-pinned".to_string(),
            p95_ms: 1,
            samples_ms: vec![1; MIN_REPETITIONS],
            threshold_ms: CRATE_BUDGET_MS,
        });
        let violations = audit_closed_census(&padded, &expected_census());
        assert!(
            violations.iter().any(|v| v.scope == "crate"
                && v.id == "not-pinned"
                && v.detail.contains("not pinned")),
            "padding a scope with an unpinned id is rejected: {violations:?}"
        );
    }

    #[test]
    fn closed_census_rejects_a_substituted_crate() {
        let mut swapped = full_ledger(MIN_REPETITIONS);
        swapped.crates[0].id = "d2b-other".to_string();
        let violations = audit_closed_census(&swapped, &expected_census());
        // Exactly one missing (pinned) and one extra (unpinned).
        assert!(
            violations
                .iter()
                .any(|v| v.id == "d2b-core" && v.detail.contains("missing")),
            "the pinned crate is reported missing: {violations:?}"
        );
        assert!(
            violations
                .iter()
                .any(|v| v.id == "d2b-other" && v.detail.contains("not pinned")),
            "the substitute crate is reported unpinned: {violations:?}"
        );
    }

    #[test]
    fn an_empty_census_pin_is_rejected() {
        let empty = ExpectedCensus {
            crates: Vec::new(),
            tests: Vec::new(),
        };
        assert!(
            empty.validate().is_err(),
            "a census with no crates is refused"
        );
    }

    #[test]
    fn a_census_pin_with_no_tests_is_rejected() {
        let empty = ExpectedCensus {
            crates: vec!["d2b-core".to_string()],
            tests: Vec::new(),
        };
        assert!(
            empty.validate().is_err(),
            "a census with no tests cannot attest that the crate executed tests"
        );
    }

    #[test]
    fn runner_label_grammar_admits_short_handles_and_rejects_everything_else() {
        for ok in ["local", "ci-x86-runner", "bench.host-1", "r0"] {
            assert!(validate_runner_label(ok).is_ok(), "`{ok}` is a valid label");
        }
        for bad in [
            "",
            "-leading",
            ".leading",
            "UPPER",
            "has space",
            "has/slash",
            "has\nnewline",
            "trailing\u{7f}",
        ] {
            assert!(
                validate_runner_label(bad).is_err(),
                "`{bad:?}` must be rejected"
            );
        }
        let too_long = "a".repeat(MAX_RUNNER_LABEL_LEN + 1);
        assert!(
            validate_runner_label(&too_long).is_err(),
            "an over-length label is rejected"
        );
    }

    #[test]
    fn id_validation_rejects_control_characters_and_oversize() {
        assert!(validate_id("test", "crate::module::test").is_ok());
        assert!(validate_id("test", "").is_err(), "an empty id is rejected");
        assert!(
            validate_id("test", "line\nbreak").is_err(),
            "a newline in an id is rejected"
        );
        assert!(
            validate_id("test", "tab\tstop").is_err(),
            "a tab in an id is rejected"
        );
        assert!(
            validate_id("test", "esc\u{1b}[0m").is_err(),
            "a terminal escape in an id is rejected"
        );
        assert!(
            validate_id("test", "sn\u{f6}w").is_err(),
            "a non-ASCII byte in an id is rejected"
        );
        let too_long = "a".repeat(MAX_ID_LEN + 1);
        assert!(
            validate_id("test", &too_long).is_err(),
            "an over-length id (a smuggled host path or log blob) is rejected"
        );
    }

    #[test]
    fn validate_ledger_rejects_a_control_bearing_or_unbounded_row() {
        let mut hostile = full_ledger(MIN_REPETITIONS);
        hostile.tests[0].id = "inject\nnewline".to_string();
        assert!(
            validate_ledger(&hostile).is_err(),
            "a control-bearing id is rejected on validation"
        );

        let mut flooded = full_ledger(MIN_REPETITIONS);
        flooded.tests[0].samples_ms = vec![1; MAX_SAMPLES_PER_ID + 1];
        assert!(
            validate_ledger(&flooded).is_err(),
            "an unbounded sample vector is rejected on validation"
        );

        let mut bad_runner = full_ledger(MIN_REPETITIONS);
        bad_runner.runner = "Bad Runner".to_string();
        assert!(
            validate_ledger(&bad_runner).is_err(),
            "an invalid runner label is rejected on validation"
        );

        let mut mislabeled = full_ledger(MIN_REPETITIONS);
        mislabeled.crates[0].enforcement = Enforcement::Advisory;
        assert!(
            validate_ledger(&mislabeled).is_err(),
            "an enforced crate measurement cannot be relabeled as advisory"
        );
    }

    #[test]
    fn parse_libtest_json_rejects_an_oversized_stream() {
        let giant = "x".repeat(MAX_LIBTEST_BYTES + 1);
        assert!(
            parse_libtest_json(&giant).is_err(),
            "a stream larger than the ingest bound is rejected before parsing"
        );
    }

    #[test]
    fn parse_libtest_json_rejects_a_control_bearing_test_name() {
        let stream = "{ \"type\": \"test\", \"name\": \"a::b\\ninjected\", \"event\": \"ok\", \"exec_time\": 0.01 }\n";
        assert!(
            parse_libtest_json(stream).is_err(),
            "a libtest name carrying a newline is rejected"
        );
    }

    #[test]
    fn a_missing_census_pin_does_not_leak_its_absolute_directory() {
        let dir = "/home/redaction-sentinel-census";
        let error = load_expected_census(&format!("{dir}/runtime-census.json"))
            .expect_err("a missing census pin must fail");
        assert!(
            !error.contains(dir),
            "the census diagnostic leaked its absolute directory: {error}"
        );
    }

    #[test]
    fn a_missing_ledger_does_not_leak_its_absolute_directory() {
        let dir = "/home/redaction-sentinel-ledger";
        let error = load_ledger(&format!("{dir}/runtime-ledger.json"))
            .expect_err("a missing ledger must fail");
        assert!(
            !error.contains(dir),
            "the ledger diagnostic leaked its absolute directory: {error}"
        );
    }

    #[test]
    fn a_missing_libtest_stream_does_not_leak_its_absolute_directory() {
        let dir = "/home/redaction-sentinel-stream";
        let args = vec![
            "--runner".to_string(),
            "reference".to_string(),
            "--libtest-json".to_string(),
            format!("{dir}/stream.json"),
        ];
        let error = run_record(&args).expect_err("a missing libtest stream must fail");
        assert!(
            !error.contains(dir),
            "the record diagnostic leaked its absolute directory: {error}"
        );
    }

    #[test]
    fn a_missing_lint_target_keeps_unambiguous_repository_context() {
        let repo = crate::repo_root().unwrap();
        let target = repo.join(".scratch/redaction-sentinel-lint/one/entrypoint.sh");
        let args = vec![target.display().to_string()];
        let error = run_lint(&args).expect_err("a missing lint target must fail");
        assert!(
            !error.contains(repo.to_str().unwrap()),
            "the lint diagnostic leaked its absolute checkout: {error}"
        );
        assert!(
            error.contains("<repo>/.scratch/redaction-sentinel-lint/one/entrypoint.sh"),
            "the lint diagnostic must preserve repository-relative context: {error}"
        );
    }
}
