#![forbid(unsafe_code)]

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

const DEFAULT_POLICY: &str = "tests/golden/bazel/cache-policy.json";
const DISPATCH_MARKERS: &[&str] = &[
    "remote execution started",
    "remote action dispatched",
    "spawnexec",
    "action cache hit",
    "action cache miss",
    "completed remote",
    "remote output",
];
const PRE_DISPATCH_MARKERS: &[&str] = &[
    "pre-dispatch",
    "before dispatch",
    "before remote dispatch",
    "before remote execution",
    "remote dispatch not started",
    "remote execution not started",
    "remote action not dispatched",
    "remote action was not dispatched",
    "remote request not sent",
    "remote request was not sent",
];
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
    println!("usage: xtask bazel-evidence check-security [--policy <path>]");
    println!("       xtask bazel-evidence security-digest [--policy <path>]");
    println!("       xtask bazel-evidence classify-failure --log <path> [--dispatch-evidence]");
    println!("       xtask bazel-evidence redact-log --input <path> --output <path>");
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
    crate::repo_root()
        .expect("repository root with Cargo.toml, BUILD.bazel, and flake.nix is not discoverable")
        .to_path_buf()
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

    fn permits_local_retry(
        self,
        pre_dispatch_evidence: bool,
        post_dispatch_retryable: bool,
    ) -> bool {
        match self {
            Self::MissingCredentials | Self::Authentication | Self::Endpoint => true,
            Self::Worker => pre_dispatch_evidence,
            Self::Transport => pre_dispatch_evidence || post_dispatch_retryable,
            Self::Analysis | Self::Test | Self::Policy | Self::Build => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FailureClassification {
    kind: Option<FailureKind>,
    dispatch_evidence: bool,
    pre_dispatch_evidence: bool,
    post_dispatch_retryable: bool,
}

fn classify_failure(log: &str, explicit_dispatch_evidence: bool) -> FailureClassification {
    let normalized = log.to_ascii_lowercase();
    let dispatch_evidence =
        explicit_dispatch_evidence || contains_any_marker(&normalized, DISPATCH_MARKERS);

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
        || normalized.contains("deadline_exceeded")
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
    let pre_dispatch_evidence = contains_any_marker(&normalized, PRE_DISPATCH_MARKERS);
    let pre_dispatch_evidence = pre_dispatch_evidence
        || matches!(
            kind,
            Some(
                FailureKind::MissingCredentials
                    | FailureKind::Authentication
                    | FailureKind::Endpoint
            )
        );
    let post_dispatch_retryable = matches!(kind, Some(FailureKind::Transport))
        && (normalized.contains("deadline exceeded") || normalized.contains("deadline_exceeded"));

    FailureClassification {
        kind,
        dispatch_evidence,
        pre_dispatch_evidence,
        post_dispatch_retryable,
    }
}

fn classification_value(classification: FailureClassification) -> Value {
    let retry = classification
        .kind
        .is_some_and(|kind| {
            kind.permits_local_retry(
                classification.pre_dispatch_evidence,
                classification.post_dispatch_retryable,
            )
        })
        && (!classification.dispatch_evidence || classification.post_dispatch_retryable);
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

fn contains_any_marker(text: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| text.contains(marker))
}

fn contains_quoted_field(lower: &str, field: &str) -> bool {
    for quote in ['"', '\''] {
        let quoted = format!("{quote}{field}{quote}");
        let mut offset = 0;
        while let Some(relative) = lower[offset..].find(&quoted) {
            let index = offset + relative;
            if lower[index + quoted.len()..].trim_start().starts_with(':') {
                return true;
            }
            offset = index + quoted.len();
        }
    }
    false
}

fn contains_credential_field(lower: &str) -> bool {
    lower.contains("x-buildbuddy-api-key")
        || lower.contains("api-key=")
        || lower.contains("api_key=")
        || lower.contains("authorization:")
        || lower.contains("api-key:")
        || lower.contains("api_key:")
        || lower.contains("bearer ")
        || contains_quoted_field(lower, "authorization")
        || contains_quoted_field(lower, "api-key")
        || contains_quoted_field(lower, "api_key")
        || contains_quoted_field(lower, "x-buildbuddy-api-key")
}

fn unescaped_quote_count(line: &str, quote: char) -> usize {
    let mut count = 0;
    let mut backslashes = 0;
    for character in line.chars() {
        if character == quote && backslashes % 2 == 0 {
            count += 1;
        }
        if character == '\\' {
            backslashes += 1;
        } else {
            backslashes = 0;
        }
    }
    count
}

#[derive(Clone, Copy)]
enum RedactionContinuation {
    Unquoted,
    Quoted(char),
}

fn redaction_continuation(lower: &str) -> Option<RedactionContinuation> {
    for quote in ['"', '\''] {
        if unescaped_quote_count(lower, quote) % 2 == 1 {
            return Some(RedactionContinuation::Quoted(quote));
        }
    }
    let trimmed = lower.trim_end();
    if trimmed.ends_with(':') || trimmed.ends_with('=') || trimmed.ends_with("bearer") {
        Some(RedactionContinuation::Unquoted)
    } else {
        None
    }
}

fn redaction_hints(lower: &str) -> String {
    let mut hints = Vec::new();
    if lower.contains("unauthenticated")
        || lower.contains("permission denied")
        || lower.contains("status code 401")
        || lower.contains("status code 403")
    {
        hints.push("authentication");
    }
    for marker in DISPATCH_MARKERS.iter().chain(PRE_DISPATCH_MARKERS.iter()) {
        if lower.contains(marker) {
            hints.push(marker);
        }
    }
    if hints.is_empty() {
        String::new()
    } else {
        format!(" {}", hints.join(" "))
    }
}

fn redacted_line(lower: &str) -> String {
    format!("[REDACTED]{}", redaction_hints(lower))
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

    let mut continuation = None;
    let mut redacted_lines = Vec::new();
    for line in output.lines() {
        if let Some(state) = continuation {
            let lower = line.to_ascii_lowercase();
            redacted_lines.push(redacted_line(&lower));
            continuation = match state {
                RedactionContinuation::Unquoted => None,
                RedactionContinuation::Quoted(quote) => unescaped_quote_count(&lower, quote)
                    .is_multiple_of(2)
                    .then_some(state),
            };
            continue;
        }

        let lower = line.to_ascii_lowercase();
        if contains_credential_field(&lower) {
            continuation = redaction_continuation(&lower);
            redacted_lines.push(redacted_line(&lower));
        } else {
            redacted_lines.push(line.to_owned());
        }
    }
    redacted_lines.join("\n")
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

    #[test]
    fn redaction_preserves_dispatch_evidence_on_secret_lines() {
        let redacted = redact_text(
            "remote execution started authorization: Bearer synthetic-secret UNAUTHENTICATED",
        );
        assert!(!redacted.contains("synthetic-secret"));
        let classification = classification_value(classify_failure(&redacted, false));
        assert_eq!(classification["dispatchEvidence"], true);
        assert_eq!(classification["retryLocally"], false);
    }

    #[test]
    fn redaction_preserves_dispatch_evidence_while_removing_secret() {
        let redacted = redact_text(
            "remote execution started authorization: Bearer dispatch-token UNAUTHENTICATED",
        );
        assert!(!redacted.contains("dispatch-token"));
        let classification = classification_value(classify_failure(&redacted, false));
        assert_eq!(classification["dispatchEvidence"], true);
        assert_eq!(classification["retryLocally"], false);
    }

    #[test]
    fn redaction_covers_quoted_structured_credentials() {
        let redacted = redact_text(
            "{\"authorization\": \"quoted-secret\"}\n\
             {\"api_key\": \"structured-secret\"}",
        );
        assert!(!redacted.contains("quoted-secret"));
        assert!(!redacted.contains("structured-secret"));
    }

    #[test]
    fn redaction_covers_multiline_credential_continuations() {
        let redacted = redact_text(
            "authorization: Bearer\n\
             continuation-secret\n\
             remote execution started",
        );
        assert!(!redacted.contains("continuation-secret"));
        assert!(redacted.contains("remote execution started"));
    }
}
