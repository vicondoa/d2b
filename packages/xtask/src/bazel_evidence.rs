#![forbid(unsafe_code)]

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

const DEFAULT_POLICY: &str = "tests/golden/bazel/cache-policy.json";
const DEFAULT_U9_REPORT_DIGEST: &str =
    "sha256:b95aa3c27f9dda7947f303da7792a09416ce4d1e4092eca75fc9a5e36898f241";

type Result<T> = std::result::Result<T, String>;

pub fn run_cli(args: &[String]) -> ExitCode {
    match run(args) {
        Ok(value) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("Bazel evidence serializes")
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("bazel-evidence failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<Value> {
    match args.first().map(String::as_str) {
        Some("check-u9") => {
            let options = CheckU9Options::parse(&args[1..])?;
            check_u9(&options)
        }
        Some("check-security") => {
            let policy = option_path(&args[1..], "--policy")?
                .unwrap_or_else(|| PathBuf::from(DEFAULT_POLICY));
            check_security(&policy)
        }
        Some("security-digest") => {
            let policy = option_path(&args[1..], "--policy")?
                .unwrap_or_else(|| PathBuf::from(DEFAULT_POLICY));
            security_digest(&policy)
        }
        Some("classify-failure") => {
            let log =
                option_path(&args[1..], "--log")?.ok_or_else(|| "--log is required".to_owned())?;
            let dispatch_evidence = has_flag(&args[1..], "--dispatch-evidence")?;
            let text = fs::read_to_string(resolve_path(&repo_root(), &log))
                .map_err(|error| format!("read failure log {}: {error}", log.display()))?;
            Ok(classification_value(classify_failure(
                &text,
                dispatch_evidence,
            )))
        }
        Some("redact-log") => {
            let input = option_path(&args[1..], "--input")?
                .ok_or_else(|| "--input is required".to_owned())?;
            let output = option_path(&args[1..], "--output")?
                .ok_or_else(|| "--output is required".to_owned())?;
            let input = resolve_path(&repo_root(), &input);
            let output = resolve_path(&repo_root(), &output);
            let text = fs::read_to_string(&input)
                .map_err(|error| format!("read evidence {}: {error}", input.display()))?;
            let redacted = redact_text(&text);
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("create evidence directory: {error}"))?;
            }
            fs::write(&output, redacted.as_bytes())
                .map_err(|error| format!("write evidence {}: {error}", output.display()))?;
            Ok(json!({
                "status": "pass",
                "redactedBytes": redacted.len(),
            }))
        }
        Some("--help" | "-h") | None => {
            print_usage();
            Err("help requested".to_owned())
        }
        Some(command) => Err(format!("unknown bazel-evidence command {command}")),
    }
}

fn print_usage() {
    println!("usage: xtask bazel-evidence check-u9 [--policy <path>] [--report <path>]");
    println!("       xtask bazel-evidence check-security [--policy <path>]");
    println!("       xtask bazel-evidence security-digest [--policy <path>]");
    println!("       xtask bazel-evidence classify-failure --log <path> [--dispatch-evidence]");
    println!("       xtask bazel-evidence redact-log --input <path> --output <path>");
}

struct CheckU9Options {
    policy: PathBuf,
    report: Option<PathBuf>,
}

impl CheckU9Options {
    fn parse(args: &[String]) -> Result<Self> {
        reject_unknown_flags(args, &["--policy", "--report"])?;
        Ok(Self {
            policy: option_path(args, "--policy")?.unwrap_or_else(|| PathBuf::from(DEFAULT_POLICY)),
            report: option_path(args, "--report")?,
        })
    }
}

fn reject_unknown_flags(args: &[String], valued: &[&str]) -> Result<()> {
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        if !valued.contains(&flag) {
            return Err(format!("unknown option {flag}"));
        }
        index += 2;
        if index > args.len() {
            return Err(format!("{flag} requires a value"));
        }
    }
    Ok(())
}

fn option_path(args: &[String], flag: &str) -> Result<Option<PathBuf>> {
    let mut found = None;
    let mut index = 0;
    while index < args.len() {
        if args[index] == flag {
            if found.is_some() {
                return Err(format!("{flag} may be specified only once"));
            }
            let value = args
                .get(index + 1)
                .ok_or_else(|| format!("{flag} requires a value"))?;
            if value.starts_with("--") {
                return Err(format!("{flag} requires a value"));
            }
            found = Some(PathBuf::from(value));
            index += 2;
        } else {
            index += 1;
        }
    }
    Ok(found)
}

fn has_flag(args: &[String], flag: &str) -> Result<bool> {
    let count = args.iter().filter(|value| value.as_str() == flag).count();
    if count > 1 {
        return Err(format!("{flag} may be specified only once"));
    }
    Ok(count == 1)
}

fn repo_root() -> PathBuf {
    let mut candidates = Vec::new();
    if let Some(root) = env::var_os("D2B_REPO_ROOT") {
        candidates.push(PathBuf::from(root));
    }
    if let Some(manifest_dir) = option_env!("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .or_else(|| env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from))
    {
        candidates.push(
            manifest_dir
                .parent()
                .and_then(Path::parent)
                .expect("xtask lives under packages/xtask")
                .to_path_buf(),
        );
    }
    if let Ok(current_dir) = env::current_dir() {
        candidates.push(current_dir);
    }
    for candidate in candidates {
        let mut path = candidate;
        loop {
            if path.join("Cargo.toml").is_file()
                && path.join("BUILD.bazel").is_file()
                && path.join("flake.nix").is_file()
            {
                return path;
            }
            if !path.pop() {
                break;
            }
        }
    }
    panic!("repository root with Cargo.toml, BUILD.bazel, and flake.nix is not discoverable")
}

fn resolve_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn read_json(path: &Path) -> Result<Value> {
    let bytes = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("parse {}: {error}", path.display()))
}

fn object<'a>(value: &'a Value, context: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| format!("{context} must be an object"))
}

fn string<'a>(object: &'a Map<String, Value>, key: &str, context: &str) -> Result<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{context}.{key} must be a string"))
}

fn u64_field(object: &Map<String, Value>, key: &str, context: &str) -> Result<u64> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{context}.{key} must be an unsigned integer"))
}

fn check_u9(options: &CheckU9Options) -> Result<Value> {
    let root = repo_root();
    let policy_path = resolve_path(&root, &options.policy);
    let policy = read_json(&policy_path)?;
    let policy_object = object(&policy, "cache policy")?;
    if u64_field(policy_object, "schemaVersion", "cache policy")? != 1 {
        return Err("u9-evidence-stale:policy-schema".to_owned());
    }
    let gate = object(
        policy_object
            .get("u9Gate")
            .ok_or_else(|| "u9-evidence-stale:missing-gate".to_owned())?,
        "cache policy u9Gate",
    )?;
    let eligibility_relative = PathBuf::from(string(gate, "eligibility", "u9Gate")?);
    let eligibility_path = resolve_path(&root, &eligibility_relative);
    let eligibility_bytes =
        fs::read(&eligibility_path).map_err(|_| "u9-evidence-missing:eligibility".to_owned())?;
    let actual_eligibility_digest = digest_bytes(&eligibility_bytes);
    let expected_eligibility_digest = string(gate, "eligibilityDigest", "u9Gate")?;
    if actual_eligibility_digest != expected_eligibility_digest {
        return Err("u9-evidence-stale:eligibility-digest".to_owned());
    }

    let report_relative = match &options.report {
        Some(report) => report.clone(),
        None => PathBuf::from(string(gate, "report", "u9Gate")?),
    };
    let report_path = resolve_path(&root, &report_relative);
    let report_bytes = fs::read(&report_path)
        .map_err(|_| "u9-evidence-missing:representative-report".to_owned())?;
    if options.report.is_none() {
        if digest_bytes(&report_bytes) != DEFAULT_U9_REPORT_DIGEST {
            return Err("u9-evidence-stale:report-digest".to_owned());
        }
    }
    let report: Value = serde_json::from_slice(&report_bytes)
        .map_err(|_| "u9-evidence-missing:representative-report".to_owned())?;
    let report_object = object(&report, "representative report")?;
    if u64_field(report_object, "schemaVersion", "representative report")? != 1
        || string(report_object, "reportKind", "representative report")? != "representative-summary"
    {
        return Err("u9-evidence-stale:report-schema".to_owned());
    }
    let source = object(
        report_object
            .get("source")
            .ok_or_else(|| "u9-evidence-stale:report-source".to_owned())?,
        "representative report source",
    )?;
    if string(source, "eligibility", "report source")? != string(gate, "eligibility", "u9Gate")?
        || string(source, "eligibilityDigest", "report source")? != expected_eligibility_digest
    {
        return Err("u9-evidence-stale:eligibility-digest".to_owned());
    }
    for (field, context) in [
        ("graphDigest", "graph"),
        ("configuration", "configuration"),
        ("platform", "platform"),
        ("toolchain", "toolchain"),
    ] {
        if source.get(field) != gate.get(field) {
            return Err(format!("u9-evidence-stale:{context}"));
        }
    }

    let report_graph = object(
        report_object
            .get("wholeGraph")
            .ok_or_else(|| "u9-evidence-stale:whole-graph".to_owned())?,
        "representative report wholeGraph",
    )?;
    let expected_graph = object(
        gate.get("wholeGraph")
            .ok_or_else(|| "u9-evidence-stale:expected-graph".to_owned())?,
        "u9Gate wholeGraph",
    )?;
    for field in ["actionCount", "grossInputBytes", "uniqueInputBytes"] {
        if report_graph.get(field) != expected_graph.get(field) {
            return Err(format!("u9-evidence-stale:whole-graph-{field}"));
        }
    }

    let pipelining = object(
        report_object
            .get("pipeliningRejected")
            .ok_or_else(|| "u9-evidence-stale:pipelining".to_owned())?,
        "representative report pipeliningRejected",
    )?;
    let expected_pipelining = object(
        gate.get("pipelining")
            .ok_or_else(|| "u9-evidence-stale:expected-pipelining".to_owned())?,
        "u9Gate pipelining",
    )?;
    if pipelining.get("reason") != expected_pipelining.get("reason")
        || pipelining.get("pipelinedGrossInputBytes")
            != expected_pipelining.get("pipelinedGrossInputBytes")
        || pipelining.get("pipelinedUniqueInputBytes")
            != expected_pipelining.get("pipelinedUniqueInputBytes")
        || pipelining.get("pipelinedActionCount") != expected_pipelining.get("pipelinedActionCount")
        || pipelining.get("pipelinedFanOutRatio") != expected_pipelining.get("pipelinedFanOutRatio")
    {
        return Err("u9-evidence-stale:pipelining".to_owned());
    }

    Ok(json!({
        "status": "pass",
        "gate": "u9",
        "report": report_relative,
        "eligibility": eligibility_relative,
        "eligibilityDigest": actual_eligibility_digest,
        "graphDigest": string(source, "graphDigest", "report source")?,
        "actionCount": u64_field(report_graph, "actionCount", "wholeGraph")?,
        "grossInputBytes": u64_field(report_graph, "grossInputBytes", "wholeGraph")?,
        "uniqueInputBytes": u64_field(report_graph, "uniqueInputBytes", "wholeGraph")?,
    }))
}

fn security_digest(policy_path: &Path) -> Result<Value> {
    let root = repo_root();
    let policy = read_json(&resolve_path(&root, policy_path))?;
    let policy_object = object(&policy, "cache policy")?;
    let trusted = object(
        policy_object
            .get("trustedInjection")
            .ok_or_else(|| "security policy missing trustedInjection".to_owned())?,
        "trustedInjection",
    )?;
    let files = trusted
        .get("allowlistedSecurityFiles")
        .and_then(Value::as_array)
        .ok_or_else(|| "trustedInjection.allowlistedSecurityFiles must be an array".to_owned())?;
    let mut paths = Vec::new();
    for value in files {
        let relative = value
            .as_str()
            .ok_or_else(|| "allowlisted security file must be a string".to_owned())?;
        paths.push(relative.to_owned());
    }
    paths.sort();
    let mut preimage = Vec::new();
    for relative in &paths {
        let bytes = fs::read(root.join(relative))
            .map_err(|error| format!("read security file {relative}: {error}"))?;
        preimage.extend_from_slice(relative.as_bytes());
        preimage.push(0);
        preimage.extend_from_slice(&bytes);
        preimage.push(0);
    }
    let digest = digest_bytes(&preimage);
    Ok(json!({
        "status": "pass",
        "digest": digest,
        "files": paths,
    }))
}

fn check_security(policy_path: &Path) -> Result<Value> {
    let digest_value = security_digest(policy_path)?;
    let digest = digest_value
        .get("digest")
        .and_then(Value::as_str)
        .ok_or_else(|| "security digest missing".to_owned())?;
    let policy = read_json(&resolve_path(&repo_root(), policy_path))?;
    let trusted = object(
        object(&policy, "cache policy")?
            .get("trustedInjection")
            .ok_or_else(|| "security policy missing trustedInjection".to_owned())?,
        "trustedInjection",
    )?;
    let allowed = trusted
        .get("allowedSecurityDigests")
        .and_then(Value::as_array)
        .ok_or_else(|| "allowed security digests must be an array".to_owned())?;
    if !allowed.iter().any(|value| value.as_str() == Some(digest)) {
        return Err("security-digest-not-allowlisted".to_owned());
    }
    Ok(json!({
        "status": "pass",
        "digest": digest,
        "allowlisted": true,
    }))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FailureKind {
    MissingCredentials,
    Authentication,
    Endpoint,
    Worker,
    Transport,
    Analysis,
    Test,
    Policy,
    Build,
}

impl FailureKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::MissingCredentials => "missing-credentials",
            Self::Authentication => "authentication",
            Self::Endpoint => "endpoint",
            Self::Worker => "worker",
            Self::Transport => "transport",
            Self::Analysis => "analysis",
            Self::Test => "test",
            Self::Policy => "policy",
            Self::Build => "build",
        }
    }

    fn permits_local_retry(self) -> bool {
        matches!(
            self,
            Self::MissingCredentials
                | Self::Authentication
                | Self::Endpoint
                | Self::Worker
                | Self::Transport
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FailureClassification {
    kind: Option<FailureKind>,
    dispatch_evidence: bool,
}

fn classify_failure(log: &str, explicit_dispatch_evidence: bool) -> FailureClassification {
    let normalized = log.to_ascii_lowercase();
    let dispatch_evidence = explicit_dispatch_evidence
        || [
            "remote execution started",
            "remote action dispatched",
            "spawnexec",
            "action cache hit",
            "action cache miss",
            "completed remote",
            "remote output",
        ]
        .iter()
        .any(|marker| normalized.contains(marker));

    let kind = if normalized.contains("missing credential")
        || normalized.contains("credential helper unavailable")
        || normalized.contains("no credential")
    {
        Some(FailureKind::MissingCredentials)
    } else if normalized.contains("unauthenticated")
        || normalized.contains("permission denied")
        || normalized.contains("authentication")
        || normalized.contains("status code 401")
        || normalized.contains("status code 403")
    {
        Some(FailureKind::Authentication)
    } else if normalized.contains("no such host")
        || normalized.contains("name or service not known")
        || normalized.contains("dns")
        || normalized.contains("connection refused")
        || normalized.contains("endpoint unavailable")
    {
        Some(FailureKind::Endpoint)
    } else if normalized.contains("worker unavailable")
        || normalized.contains("executor unavailable")
        || normalized.contains("worker image")
        || normalized.contains("platform mismatch")
    {
        Some(FailureKind::Worker)
    } else if normalized.contains("transport")
        || normalized.contains("grpc")
        || normalized.contains("broken pipe")
        || normalized.contains("connection reset")
        || normalized.contains("deadline exceeded")
    {
        Some(FailureKind::Transport)
    } else if normalized.contains("analysis")
        || normalized.contains("loading package")
        || normalized.contains("no such target")
    {
        Some(FailureKind::Analysis)
    } else if normalized.contains("policy")
        || normalized.contains("assertion")
        || normalized.contains("inventory")
    {
        Some(FailureKind::Policy)
    } else if normalized.contains("test failed")
        || normalized.contains("test_runner")
        || normalized.contains("test failure")
    {
        Some(FailureKind::Test)
    } else {
        Some(FailureKind::Build)
    };

    FailureClassification {
        kind,
        dispatch_evidence,
    }
}

fn classification_value(classification: FailureClassification) -> Value {
    let retry = classification
        .kind
        .is_some_and(FailureKind::permits_local_retry)
        && !classification.dispatch_evidence;
    let kind = if classification.dispatch_evidence && !retry {
        "post-dispatch-uncertainty".to_owned()
    } else {
        classification
            .kind
            .map(FailureKind::as_str)
            .unwrap_or("unknown")
            .to_owned()
    };
    json!({
        "kind": kind,
        "dispatchEvidence": classification.dispatch_evidence,
        "retryLocally": retry,
        "maxLocalRetries": if retry { 1 } else { 0 },
    })
}

fn redact_text(input: &str) -> String {
    let mut output = input.to_owned();
    if let Some(sentinels) = env::var_os("D2B_BUILDBUDDY_SENTINELS") {
        for sentinel in sentinels.to_string_lossy().split('|') {
            if !sentinel.is_empty() {
                output = output.replace(sentinel, "[REDACTED]");
            }
        }
    }

    output
        .lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if lower.contains("x-buildbuddy-api-key")
                || lower.contains("api-key=")
                || lower.contains("api_key=")
                || lower.contains("authorization:")
                || lower.contains("bearer ")
            {
                let failure_hint = if lower.contains("unauthenticated")
                    || lower.contains("permission denied")
                    || lower.contains("status code 401")
                    || lower.contains("status code 403")
                {
                    " authentication"
                } else {
                    ""
                };
                format!("[REDACTED]{failure_hint}")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn digest_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{digest:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infrastructure_failure_before_dispatch_is_retryable() {
        let result = classify_failure("remote authentication failed", false);
        assert_eq!(result.kind, Some(FailureKind::Authentication));
        assert!(!result.dispatch_evidence);
        assert!(
            classification_value(result)["retryLocally"]
                .as_bool()
                .unwrap()
        );
    }

    #[test]
    fn dispatch_evidence_turns_uncertainty_into_fail_closed() {
        let result = classify_failure("remote execution started\nUNAUTHENTICATED", false);
        assert!(
            classification_value(result)["retryLocally"]
                .as_bool()
                .is_some_and(|v| !v)
        );
        assert_eq!(
            classification_value(result)["kind"],
            Value::String("post-dispatch-uncertainty".to_owned())
        );
    }

    #[test]
    fn redaction_preserves_retry_class_without_preserving_auth_material() {
        let redacted = redact_text(
            "authorization: Bearer synthetic-secret UNAUTHENTICATED\n\
             remote endpoint unavailable",
        );
        assert!(!redacted.contains("synthetic-secret"));
        let classification = classification_value(classify_failure(&redacted, false));
        assert_eq!(classification["kind"], "authentication");
        assert_eq!(classification["retryLocally"], true);
    }
}
