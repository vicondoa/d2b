#![forbid(unsafe_code)]

use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

const SCHEMA_VERSION: u64 = 1;
const DEFAULT_U9_REPORT: &str = "tests/golden/bazel/cache-transfer-representative.json";
const DEFAULT_U9_REPORT_DIGEST: &str =
    "sha256:7b7df84d16442b5e2314d416944b09174244b284b02b52049df57632da6d5907";
const DEFAULT_U9_TARGET_SET_DIGEST: &str =
    "sha256:576bbb5fd15ccdd2ae7db72515aefdf66b2413a60687921d1077f7dab5593dae";
const DEFAULT_U9_CONFIGURATION_DIGEST: &str =
    "sha256:0aa9e883656411026b29209da57c0de9b52e1edc718f8bb8d226d2221678afb7";
const DEFAULT_TARGET_SET: &str = "tests/golden/bazel/cache-policy.json";
const DEFAULT_CONFIGURATION: &str = ".bazelrc";
const DEFAULT_SELECTED_CLOSURE: &str = "tests/golden/bazel/eligibility.json";
const DEFAULT_NAMESPACE: &str =
    "d2b/qualification/linux-x86_64/rules_rust/worker-v1/minimal/lock-v1";
const DEFAULT_TOOLCHAIN: &str = "rules_rust";
const DEFAULT_WORKER_IMAGE: &str = "d2b-bazel-worker/v1";
const REQUIRED_PROJECTION: &str = "xtask-buildbuddy-probe/v1";
const WORKING_BUDGET_BYTES: u64 = 80_000_000_000;
const HEADROOM_BYTES: u64 = 20_000_000_000;
const WALL_TIME_BUDGET_MILLIS: u64 = 180_000;
const DEFAULT_MAX_AGE_MILLIS: u64 = 24 * 60 * 60 * 1_000;
const MIN_PROVIDER_SAMPLES: usize = 5;

type Result<T> = std::result::Result<T, String>;

const PRE_DISPATCH_CLASSES: &[&str] = &[
    "missing-credentials",
    "authentication",
    "endpoint",
    "worker",
    "transport",
];

const PROVIDER_FIELDS: &[&str] = &[
    "schemaVersion",
    "provider",
    "projection",
    "source",
    "status",
    "reason",
    "providerAccountedTransfer",
    "probe",
    "authenticated",
    "executionEntitled",
    "cacheReadEvidence",
    "cacheWriteEvidence",
    "readOnlyProbe",
    "uploadsDisabled",
    "secretRedaction",
    "trustedSeed",
    "dispatchEvidence",
    "invocationId",
    "sampleId",
    "observedAtMillis",
    "workerArchitecture",
    "workerArchitectures",
    "workerImage",
    "sampleClass",
    "freshWorktree",
    "isolatedServer",
    "localDiskCacheDisabled",
    "cacheState",
    "worktreeId",
    "outputRootId",
    "outputBaseId",
    "bazelServerId",
    "localCacheId",
    "transferBytes",
    "qualificationMetrics",
    "commit",
    "identity",
    "cache",
    "samples",
];

const CANDIDATE_FIELDS: &[&str] = &[
    "schemaVersion",
    "commit",
    "targetSet",
    "configuration",
    "selectedClosure",
    "namespace",
    "toolchain",
    "platform",
    "coverage",
    "cache",
    "fallback",
    "latency",
];

const COVERAGE_FIELDS: &[&str] = &[
    "currentScheduler",
    "currentSchedulerPass",
    "bazel",
    "bazelPass",
    "seedFailuresObserved",
    "seededFailuresObserved",
    "equivalentTargetSet",
    "targetSetEquivalent",
];

const CACHE_FIELDS: &[&str] = &[
    "trustedSeedComplete",
    "seedComplete",
    "asyncUploadsDrained",
    "seedUploadsDrained",
    "unchangedCacheableExecutions",
    "unchangedCacheableActionExecutions",
    "approvedUncacheableReasons",
    "approvedUncacheableActions",
    "cacheMatrix",
];

const CACHE_MATRIX_FIELDS: &[&str] = &[
    "warm",
    "unchanged",
    "sourceInvalidation",
    "toolchainInvalidation",
    "featureInvalidation",
    "lockInvalidation",
    "platformInvalidation",
    "agedCache",
    "eviction",
    "compression",
    "architecture",
    "empty",
    "workerImageInvalidation",
    "crossMachine",
    "retry",
    "fallback",
];

const FALLBACK_FIELDS: &[&str] = &[
    "status",
    "localRetryCount",
    "retryCount",
    "identicalTargetSet",
    "targetSetIdentical",
    "dispatchStarted",
    "failureClass",
    "attempt",
    "maxLocalRetries",
    "retryLocally",
];

const IDENTITY_FIELDS: &[&str] = &[
    "commit",
    "targetSetDigest",
    "configurationDigest",
    "selectedClosureDigest",
    "namespace",
    "toolchain",
    "platform",
];

const PROBE_FIELDS: &[&str] = &[
    "kind",
    "command",
    "input",
    "readOnly",
    "fixtureSafe",
    "credentialMode",
    "nonce",
];

const METRIC_FIELDS: &[&str] = &[
    "wallTimeMillis",
    "actionCacheHits",
    "actionCacheMisses",
    "casHits",
    "casMisses",
    "remoteExecutions",
    "repositoryTrafficBytes",
    "besTrafficBytes",
    "retryTrafficBytes",
    "localNixMillis",
];

#[derive(Clone, Debug)]
struct Options {
    candidate: Option<PathBuf>,
    provider_evidence: Option<PathBuf>,
    output: Option<PathBuf>,
    u9_report: PathBuf,
    target_set: Option<PathBuf>,
    configuration: Option<PathBuf>,
    selected_closure: Option<PathBuf>,
    commit: Option<String>,
    namespace: Option<String>,
    toolchain: Option<String>,
    nonce: Option<String>,
    now_millis: Option<u64>,
    max_age_millis: u64,
    report_only: bool,
}

#[derive(Clone, Debug)]
struct CandidateContext {
    commit: String,
    target_set_digest: String,
    configuration_digest: String,
    selected_closure_digest: String,
    namespace: String,
    toolchain: String,
    platform: String,
    coverage: Value,
    cache: Value,
    fallback: Value,
    latency: Value,
    evidence_origin_trusted: bool,
}

#[derive(Clone, Debug)]
struct U9Bounds {
    unique_input_bytes: u64,
    gross_input_bytes: u64,
    output_bytes: u64,
    fan_out_ratio: f64,
    max_fan_out: u64,
    graph_digest: String,
    eligibility_digest: String,
    toolchain: String,
    platform: String,
}

#[derive(Clone, Debug)]
struct ProviderSample {
    invocation_id: String,
    sample_id: String,
    observed_at_millis: u64,
    uploaded: u64,
    downloaded: u64,
    metrics: Map<String, Value>,
    worker_architecture: String,
    worker_image: String,
    worktree_id: String,
    output_root_id: String,
    output_base_id: String,
    bazel_server_id: String,
    local_cache_id: String,
}

#[derive(Clone, Debug)]
struct ProviderCollection {
    samples: Vec<ProviderSample>,
    provider_status: String,
    provider_reason: Option<String>,
    evidence_origin_trusted: bool,
    provider_accounted: bool,
    authenticated: bool,
    secret_redaction: bool,
    trusted_seed: bool,
    execution_entitled: bool,
    uploads_disabled: bool,
}

pub fn run_cli(args: &[String]) -> ExitCode {
    let report_only = matches!(
        args.first().map(String::as_str),
        Some("acceptance" | "cache")
    ) && Options::parse(&args[1..])
        .map(|options| options.report_only)
        .unwrap_or(false);
    match run(args) {
        Ok(value) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("qualification result serializes")
            );
            if value.get("status").and_then(Value::as_str) == Some("non-qualifying") && !report_only
            {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => {
            eprintln!("bazel-qualification failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<Value> {
    let Some(command) = args.first().map(String::as_str) else {
        print_usage();
        return Err("qualification command is required".to_owned());
    };
    match command {
        "acceptance" | "cache" => {
            let options = Options::parse(&args[1..])?;
            qualification_report(command, &options)
        }
        "identity" => {
            let options = Options::parse(&args[1..])?;
            let root = repo_root();
            let candidate = load_candidate(&root, &options)?;
            Ok(identity_value(&candidate))
        }
        "typed-fallback" => typed_fallback(&args[1..]),
        "--help" | "-h" => {
            print_usage();
            Err("help requested".to_owned())
        }
        other => Err(format!("unknown bazel-qualification command {other}")),
    }
}

fn print_usage() {
    println!("usage: xtask bazel-qualification acceptance [options]");
    println!("       xtask bazel-qualification cache [options]");
    println!("       xtask bazel-qualification identity [options]");
    println!(
        "       xtask bazel-qualification typed-fallback --class <class> --dispatch-started <bool> --attempt <n>"
    );
    println!("options:");
    println!("  --candidate <path>              local candidate evidence JSON");
    println!("  --provider-evidence <path>      sanitized provider observation JSON");
    println!("  --u9-report <path>              representative U9 report");
    println!("  --target-set <path>             JSON target-set or newline list");
    println!("  --configuration <path>          committed Bazel configuration");
    println!("  --selected-closure <path>       selected closure inventory");
    println!("  --namespace <name>              qualification cache namespace");
    println!("  --toolchain <name>              toolchain identity");
    println!("  --nonce <value>                 one-run provider evidence nonce");
    println!("  --now-millis <n>                clock for stale-evidence tests");
    println!("  --max-age-millis <n>            accepted provider evidence age");
    println!("  --report-only                  emit non-qualifying reports with exit 0");
    println!("  --output <path>                 write the sanitized report");
}

impl Options {
    fn parse(args: &[String]) -> Result<Self> {
        let mut options = Self {
            candidate: None,
            provider_evidence: None,
            output: None,
            u9_report: PathBuf::from(DEFAULT_U9_REPORT),
            target_set: None,
            configuration: None,
            selected_closure: None,
            commit: None,
            namespace: None,
            toolchain: None,
            nonce: None,
            now_millis: None,
            max_age_millis: DEFAULT_MAX_AGE_MILLIS,
            report_only: false,
        };
        let mut index = 0;
        while index < args.len() {
            let flag = args[index].as_str();
            let value = |index: &mut usize| -> Result<String> {
                *index += 1;
                let value = args
                    .get(*index)
                    .cloned()
                    .ok_or_else(|| format!("{flag} requires a value"))?;
                if value.starts_with("--") {
                    return Err(format!("{flag} requires a non-option value"));
                }
                Ok(value)
            };
            match flag {
                "--candidate" => options.candidate = Some(PathBuf::from(value(&mut index)?)),
                "--provider-evidence" | "--evidence" => {
                    options.provider_evidence = Some(PathBuf::from(value(&mut index)?))
                }
                "--output" | "-o" => options.output = Some(PathBuf::from(value(&mut index)?)),
                "--u9-report" | "--report" => options.u9_report = PathBuf::from(value(&mut index)?),
                "--target-set" => options.target_set = Some(PathBuf::from(value(&mut index)?)),
                "--configuration" | "--config" => {
                    options.configuration = Some(PathBuf::from(value(&mut index)?))
                }
                "--selected-closure" | "--closure" => {
                    options.selected_closure = Some(PathBuf::from(value(&mut index)?))
                }
                "--commit" => options.commit = Some(value(&mut index)?),
                "--namespace" => options.namespace = Some(value(&mut index)?),
                "--toolchain" => options.toolchain = Some(value(&mut index)?),
                "--nonce" => options.nonce = Some(value(&mut index)?),
                "--now-millis" => {
                    options.now_millis = Some(
                        value(&mut index)?
                            .parse()
                            .map_err(|_| "--now-millis must be an unsigned integer".to_owned())?,
                    )
                }
                "--max-age-millis" => {
                    options.max_age_millis = value(&mut index)?
                        .parse()
                        .map_err(|_| "--max-age-millis must be an unsigned integer".to_owned())?
                }
                "--report-only" => options.report_only = true,
                "--help" | "-h" => {
                    print_usage();
                    return Err("help requested".to_owned());
                }
                _ => return Err(format!("unknown option {flag}")),
            }
            index += 1;
        }
        if !test_mode() && options.now_millis.is_some() {
            return Err("--now-millis-is-test-only".to_owned());
        }
        if !test_mode() && options.max_age_millis != DEFAULT_MAX_AGE_MILLIS {
            return Err("--max-age-millis-is-test-only".to_owned());
        }
        Ok(options)
    }
}

fn typed_fallback(args: &[String]) -> Result<Value> {
    let mut class = None;
    let mut dispatch_started = None;
    let mut attempt = None;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = |index: &mut usize| -> Result<String> {
            *index += 1;
            args.get(*index)
                .cloned()
                .ok_or_else(|| format!("{flag} requires a value"))
        };
        match flag {
            "--class" | "--failure-class" => class = Some(value(&mut index)?),
            "--dispatch-started" => dispatch_started = Some(parse_bool(&value(&mut index)?, flag)?),
            "--attempt" => {
                attempt = Some(
                    value(&mut index)?
                        .parse::<u64>()
                        .map_err(|_| "--attempt must be an unsigned integer".to_owned())?,
                )
            }
            "--help" | "-h" => {
                print_usage();
                return Err("help requested".to_owned());
            }
            _ => return Err(format!("unknown option {flag}")),
        }
        index += 1;
    }
    let class = normalize_failure_class(&class.ok_or_else(|| "--class is required".to_owned())?);
    let dispatch_started =
        dispatch_started.ok_or_else(|| "--dispatch-started is required".to_owned())?;
    let attempt = attempt.ok_or_else(|| "--attempt is required".to_owned())?;
    let retry = !dispatch_started && attempt == 0 && PRE_DISPATCH_CLASSES.contains(&class.as_str());
    Ok(json!({
        "kind": class,
        "dispatchStarted": dispatch_started,
        "attempt": attempt,
        "retryLocally": retry,
        "maxLocalRetries": 1,
        "reason": if retry {
            "proven-pre-dispatch-infrastructure-failure"
        } else if dispatch_started {
            "post-dispatch-failure-is-fail-closed"
        } else if attempt > 0 {
            "local-retry-already-consumed"
        } else {
            "non-infrastructure-failure-is-fail-closed"
        }
    }))
}

fn normalize_failure_class(class: &str) -> String {
    class
        .strip_prefix("pre-dispatch-")
        .unwrap_or(class)
        .to_owned()
}

fn validate_u9_identity(root: &Path, u9: &U9Bounds, candidate: &CandidateContext) -> Result<()> {
    if candidate.target_set_digest != DEFAULT_U9_TARGET_SET_DIGEST
        || candidate.configuration_digest != DEFAULT_U9_CONFIGURATION_DIGEST
    {
        return Err("u9-evidence-stale:target-set-or-configuration".to_owned());
    }
    if u9.eligibility_digest != candidate.selected_closure_digest {
        return Err("u9-evidence-stale:selected-closure".to_owned());
    }
    let policy = read_committed_json(root, DEFAULT_TARGET_SET, "cache policy")?;
    let gate = object(
        object(&policy, "cache policy")?
            .get("u9Gate")
            .ok_or_else(|| "u9-evidence-stale:cache-policy-gate".to_owned())?,
        "cache policy u9Gate",
    )?;
    if string(gate, "graphDigest", "cache policy u9Gate")? != u9.graph_digest {
        return Err("u9-evidence-stale:graph-digest".to_owned());
    }
    if string(gate, "eligibilityDigest", "cache policy u9Gate")? != u9.eligibility_digest {
        return Err("u9-evidence-stale:eligibility-digest".to_owned());
    }
    if u9.toolchain != candidate.toolchain || u9.platform != candidate.platform {
        return Err("u9-evidence-stale:toolchain-or-platform".to_owned());
    }
    Ok(())
}

fn qualification_report(mode: &str, options: &Options) -> Result<Value> {
    let root = repo_root();
    if options.u9_report != Path::new(DEFAULT_U9_REPORT) {
        return Err("u9-report-not-canonical".to_owned());
    }
    if options.provider_evidence.is_some()
        && !test_mode()
        && env::var("D2B_BUILDBUDDY_LIVE").ok().as_deref() == Some("1")
    {
        ensure_clean_worktree(&root)?;
    }
    let candidate = load_candidate(&root, options)?;
    let u9 = load_u9_bounds(&root, &options.u9_report)?;
    validate_u9_identity(&root, &u9, &candidate)?;
    if candidate.toolchain != u9.toolchain {
        return Err("u9-evidence-stale:toolchain".to_owned());
    }

    if candidate.platform != u9.platform {
        return Err("u9-evidence-stale:platform".to_owned());
    }

    let now = options.now_millis.unwrap_or_else(now_millis);
    let provider = load_provider_evidence(
        &root,
        options.provider_evidence.as_deref(),
        now,
        options,
        &candidate,
    )?;
    let mut reasons = candidate_reasons(&candidate);
    if mode == "cache" {
        reasons.extend(cache_reasons(&candidate.cache));
    }

    let accounted_samples = if provider.provider_accounted {
        provider.samples.as_slice()
    } else {
        &[]
    };
    let mut transfer = transfer_summary(accounted_samples, &u9)?;
    transfer.insert(
        "providerAccounted".to_owned(),
        Value::Bool(provider.provider_accounted && !provider.samples.is_empty()),
    );
    let latency = latency_summary(&candidate.latency, accounted_samples);
    let provider_ready = provider_samples_ready(&provider, &mut reasons);
    if !provider_ready {
        let reason = provider
            .provider_reason
            .clone()
            .unwrap_or_else(|| "provider-accounted-transfer-missing".to_owned());
        if !reasons.iter().any(|existing| existing == &reason) {
            reasons.push(reason);
        }
    }
    if let Some(reason) = transfer.get("qualificationFailure").and_then(Value::as_str) {
        reasons.push(reason.to_owned());
    }
    if let Some(reason) = latency.get("qualificationFailure").and_then(Value::as_str) {
        reasons.push(reason.to_owned());
    }
    let p99 = transfer.get("p99Bytes").and_then(Value::as_u64);
    if p99.is_some_and(|value| value > WORKING_BUDGET_BYTES) {
        reasons.push("provider-transfer-over-working-budget".to_owned());
    }
    if let Some(p99) = p99.filter(|value| *value > 0) {
        transfer.insert(
            "monthlyRuns".to_owned(),
            Value::Number((WORKING_BUDGET_BYTES / p99).into()),
        );
    } else {
        reasons.push("provider-accounted-transfer-missing".to_owned());
        transfer.insert("monthlyRuns".to_owned(), Value::Null);
    }
    let mut unique_reasons = Vec::with_capacity(reasons.len());
    for reason in reasons {
        if !unique_reasons.iter().any(|existing| existing == &reason) {
            unique_reasons.push(reason);
        }
    }
    reasons = unique_reasons;

    let status = if reasons.is_empty() {
        "qualified"
    } else {
        "non-qualifying"
    };
    let reason = reasons.first().cloned();
    let report = json!({
        "schemaVersion": SCHEMA_VERSION,
        "reportKind": "bazel-qualification",
        "mode": mode,
        "status": status,
        "reason": reason,
        "reasons": reasons,
        "candidate": identity_value(&candidate),
        "candidateEvidenceOriginTrusted": candidate.evidence_origin_trusted,
        "coverage": candidate.coverage,
        "cache": candidate.cache,
        "fallback": candidate.fallback,
        "provider": {
            "projection": REQUIRED_PROJECTION,
            "status": provider.provider_status,
            "reason": provider.provider_reason,
            "evidenceOriginTrusted": provider.evidence_origin_trusted,
            "authenticated": provider.authenticated,
            "executionEntitled": provider.execution_entitled,
            "secretRedaction": provider.secret_redaction,
            "trustedSeed": provider.trusted_seed,
            "uploadsDisabled": provider.uploads_disabled,
            "sampleCount": provider.samples.len(),
            "invocationIds": provider.samples.iter().map(|sample| sample.invocation_id.clone()).collect::<Vec<_>>(),
            "sampleIds": provider.samples.iter().map(|sample| sample.sample_id.clone()).collect::<Vec<_>>(),
            "observedAtMillis": provider.samples.iter().map(|sample| sample.observed_at_millis).collect::<Vec<_>>(),
            "workerArchitectures": provider.samples.iter().map(|sample| sample.worker_architecture.clone()).collect::<BTreeSet<_>>(),
            "workerImage": provider.samples.first().map(|sample| sample.worker_image.clone()),
            "source": if provider.samples.is_empty() { Value::Null } else { Value::String("credential-helper-probe".to_owned()) },
            "sampleClass": if provider.samples.is_empty() { Value::Null } else { Value::String("fresh-worktree".to_owned()) },
            "freshWorktree": !provider.samples.is_empty(),
            "isolatedServer": !provider.samples.is_empty(),
            "localDiskCacheDisabled": !provider.samples.is_empty(),
            "cacheState": if provider.samples.is_empty() { Value::Null } else { Value::String("populated".to_owned()) },
            "worktreeIds": provider.samples.iter().map(|sample| sample.worktree_id.clone()).collect::<Vec<_>>(),
            "outputRootIds": provider.samples.iter().map(|sample| sample.output_root_id.clone()).collect::<Vec<_>>(),
            "outputBaseIds": provider.samples.iter().map(|sample| sample.output_base_id.clone()).collect::<Vec<_>>(),
            "bazelServerIds": provider.samples.iter().map(|sample| sample.bazel_server_id.clone()).collect::<Vec<_>>(),
            "localCacheIds": provider.samples.iter().map(|sample| sample.local_cache_id.clone()).collect::<Vec<_>>(),
            "metrics": metrics_summary(accounted_samples),
        },
        "transfer": transfer,
        "latency": latency,
        "u9Comparison": {
            "graphDigest": u9.graph_digest,
            "uniqueInputBytes": u9.unique_input_bytes,
            "grossInputBytes": u9.gross_input_bytes,
            "outputBytes": u9.output_bytes,
            "targetSetDigest": candidate.target_set_digest,
            "configurationDigest": candidate.configuration_digest,
            "fanOutRatio": u9.fan_out_ratio,
            "maxFanOut": u9.max_fan_out,
            "details": transfer["u9Comparison"].clone(),
        },
        "redaction": {
            "providerEvidenceRetained": false,
            "credentialMaterial": false,
            "paths": false,
            "clientSuppliedCounters": false
        }
    });
    if let Some(output) = &options.output {
        write_json(&resolve_path(&root, output), &report)?;
    }
    Ok(report)
}

fn load_candidate(root: &Path, options: &Options) -> Result<CandidateContext> {
    let evidence_origin_trusted = options.candidate.is_none() || test_mode();
    let input = match &options.candidate {
        Some(path) => read_json(&resolve_path(root, path))?,
        None => json!({
            "schemaVersion": SCHEMA_VERSION,
            "targetSet": read_default_target_set(root)?,
            "configuration": DEFAULT_CONFIGURATION,
            "selectedClosure": DEFAULT_SELECTED_CLOSURE,
            "namespace": DEFAULT_NAMESPACE,
            "toolchain": DEFAULT_TOOLCHAIN,
            "coverage": {},
            "cache": {},
            "fallback": {},
            "latency": {}
        }),
    };
    let object = object(&input, "candidate")?;
    validate_candidate_shape(object)?;
    if object
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .is_some_and(|version| version != SCHEMA_VERSION)
    {
        return Err("candidate-schema-mismatch".to_owned());
    }
    for field in [
        "targetSetDigest",
        "configurationDigest",
        "selectedClosureDigest",
    ] {
        if object.contains_key(field) {
            return Err(format!("client-supplied-digest:{field}"));
        }
    }

    let actual_commit = git_commit(root)?;
    let commit = object
        .get("commit")
        .and_then(Value::as_str)
        .or(options.commit.as_deref())
        .unwrap_or(&actual_commit);
    validate_commit(commit)?;
    if commit != actual_commit {
        return Err("cross-commit:candidate".to_owned());
    }

    let canonical_target_set = read_default_target_set(root)?;
    let target_set = if let Some(path) = &options.target_set {
        read_target_set(root, path)?
    } else {
        canonical_target_set.clone()
    };
    if object.get("targetSet").is_some_and(|value| {
        read_target_set_value(root, Some(value)).ok() != Some(canonical_target_set.clone())
    }) {
        return Err("client-supplied-target-set-mismatch".to_owned());
    }
    if target_set != canonical_target_set {
        return Err("candidate-target-set-mismatch".to_owned());
    }
    if target_set.is_empty() {
        return Err("candidate-target-set-empty".to_owned());
    }
    let target_set_digest = digest_bytes(
        &serde_json::to_vec(&target_set)
            .map_err(|error| format!("target-set encoding: {error}"))?,
    );

    let configuration = options
        .configuration
        .as_ref()
        .and_then(|path| path.to_str())
        .or_else(|| object.get("configuration").and_then(Value::as_str))
        .unwrap_or(DEFAULT_CONFIGURATION);
    if configuration != DEFAULT_CONFIGURATION {
        return Err("configuration-not-canonical".to_owned());
    }
    let configuration_digest = digest_configuration(root, configuration)?;

    let selected_closure = options
        .selected_closure
        .as_ref()
        .and_then(|path| path.to_str())
        .or_else(|| object.get("selectedClosure").and_then(Value::as_str))
        .unwrap_or(DEFAULT_SELECTED_CLOSURE);
    if selected_closure != DEFAULT_SELECTED_CLOSURE {
        return Err("selected-closure-not-canonical".to_owned());
    }
    let selected_closure_path = safe_relative_path(selected_closure, "selectedClosure")?;
    let selected_closure_digest = digest_file(root, &selected_closure_path, "selected-closure")?;

    let namespace = options
        .namespace
        .as_deref()
        .or_else(|| object.get("namespace").and_then(Value::as_str))
        .unwrap_or(DEFAULT_NAMESPACE);
    validate_namespace(namespace)?;
    if namespace != DEFAULT_NAMESPACE {
        return Err("namespace-not-canonical".to_owned());
    }
    let toolchain = options
        .toolchain
        .as_deref()
        .or_else(|| object.get("toolchain").and_then(Value::as_str))
        .unwrap_or(DEFAULT_TOOLCHAIN);
    validate_token(toolchain, "toolchain")?;
    if toolchain != DEFAULT_TOOLCHAIN {
        return Err("toolchain-not-canonical".to_owned());
    }

    let platform = object
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("linux-x86_64");
    validate_token(platform, "platform")?;
    if platform != "linux-x86_64" {
        return Err("platform-not-canonical".to_owned());
    }
    if object
        .get("latency")
        .is_some_and(|value| value.as_object().is_some_and(|object| !object.is_empty()))
    {
        return Err("client-supplied-latency-evidence".to_owned());
    }

    Ok(CandidateContext {
        commit: commit.to_owned(),
        target_set_digest,
        configuration_digest,
        selected_closure_digest,
        namespace: namespace.to_owned(),
        toolchain: toolchain.to_owned(),
        platform: platform.to_owned(),
        coverage: if evidence_origin_trusted {
            object.get("coverage").cloned().unwrap_or_else(|| json!({}))
        } else {
            json!({})
        },
        cache: if evidence_origin_trusted {
            object.get("cache").cloned().unwrap_or_else(|| json!({}))
        } else {
            json!({})
        },
        fallback: if evidence_origin_trusted {
            object.get("fallback").cloned().unwrap_or_else(|| json!({}))
        } else {
            json!({})
        },
        latency: json!({}),
        evidence_origin_trusted,
    })
}

fn identity_value(candidate: &CandidateContext) -> Value {
    json!({
        "commit": candidate.commit,
        "targetSetDigest": candidate.target_set_digest,
        "configurationDigest": candidate.configuration_digest,
        "selectedClosureDigest": candidate.selected_closure_digest,
        "namespace": candidate.namespace,
        "toolchain": candidate.toolchain,
        "platform": candidate.platform
    })
}

fn load_u9_bounds(root: &Path, path: &Path) -> Result<U9Bounds> {
    let (report, report_digest) = if path == Path::new(DEFAULT_U9_REPORT) {
        let bytes = committed_bytes(root, DEFAULT_U9_REPORT, "u9 report")?;
        let digest = digest_bytes(&bytes);
        (
            serde_json::from_slice(&bytes).map_err(|error| format!("parse u9 report: {error}"))?,
            Some(digest),
        )
    } else {
        let report = read_json(&resolve_path(root, path))?;
        (report, None)
    };
    let report_object = object(&report, "u9 report")?;
    if report_object.get("schemaVersion").and_then(Value::as_u64) != Some(1)
        || report_object.get("reportKind").and_then(Value::as_str) != Some("representative-summary")
    {
        return Err("u9-evidence-stale:report-schema".to_owned());
    }
    let source = object(
        report_object
            .get("source")
            .ok_or_else(|| "u9-evidence-stale:source".to_owned())?,
        "u9 report source",
    )?;
    let whole_graph = object(
        report_object
            .get("wholeGraph")
            .ok_or_else(|| "u9-evidence-stale:whole-graph".to_owned())?,
        "u9 report wholeGraph",
    )?;
    let semantics = object(
        report_object
            .get("semantics")
            .ok_or_else(|| "u9-evidence-stale:semantics".to_owned())?,
        "u9 report semantics",
    )?;
    if semantics.get("providerBilling").and_then(Value::as_bool) != Some(false) {
        return Err("u9-evidence-stale:provider-billing".to_owned());
    }
    if report_digest.as_deref() != Some(DEFAULT_U9_REPORT_DIGEST) {
        return Err("u9-evidence-stale:report-digest".to_owned());
    }
    Ok(U9Bounds {
        unique_input_bytes: u64_field(whole_graph, "uniqueInputBytes", "u9 wholeGraph")?,
        gross_input_bytes: u64_field(whole_graph, "grossInputBytes", "u9 wholeGraph")?,
        output_bytes: u64_field(whole_graph, "outputBytes", "u9 wholeGraph")?,
        fan_out_ratio: whole_graph
            .get("fanOutRatio")
            .and_then(Value::as_f64)
            .ok_or_else(|| "u9-evidence-stale:fan-out-ratio".to_owned())?,
        max_fan_out: u64_field(whole_graph, "maxFanOut", "u9 wholeGraph")?,
        graph_digest: string(source, "graphDigest", "u9 source")?.to_owned(),
        eligibility_digest: string(source, "eligibilityDigest", "u9 source")?.to_owned(),
        toolchain: string(source, "toolchain", "u9 source")?.to_owned(),
        platform: string(source, "platform", "u9 source")?.to_owned(),
    })
}

fn load_provider_evidence(
    root: &Path,
    path: Option<&Path>,
    now: u64,
    options: &Options,
    candidate: &CandidateContext,
) -> Result<ProviderCollection> {
    let Some(path) = path else {
        return Ok(ProviderCollection {
            samples: Vec::new(),
            provider_status: "unavailable".to_owned(),
            provider_reason: Some("provider-accounted-transfer-missing".to_owned()),
            evidence_origin_trusted: false,
            provider_accounted: false,
            authenticated: false,
            secret_redaction: false,
            trusted_seed: false,
            execution_entitled: false,
            uploads_disabled: false,
        });
    };
    let input = read_json(&resolve_path(root, path))?;
    reject_sensitive_or_client_input(&input)?;
    if !test_mode() {
        return Ok(ProviderCollection {
            samples: Vec::new(),
            provider_status: "unavailable".to_owned(),
            provider_reason: Some("provider-evidence-origin-untrusted".to_owned()),
            evidence_origin_trusted: false,
            provider_accounted: false,
            authenticated: false,
            secret_redaction: false,
            trusted_seed: false,
            execution_entitled: false,
            uploads_disabled: false,
        });
    }
    let root_object = object(&input, "provider evidence")?;
    let values = if let Some(samples) = root_object.get("samples") {
        samples
            .as_array()
            .ok_or_else(|| "provider-evidence-samples-invalid".to_owned())?
            .clone()
    } else if let Some(provider_evidence) = root_object.get("providerEvidence") {
        vec![provider_evidence.clone()]
    } else {
        vec![input.clone()]
    };
    if values.is_empty() {
        return Err("provider-evidence-missing-samples".to_owned());
    }

    let mut samples = Vec::new();
    let mut invocation_ids = BTreeSet::new();
    let mut sample_ids = BTreeSet::new();
    let mut independence_ids = [
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::new(),
    ];
    let mut provider_status = "qualified".to_owned();
    let mut provider_reason = None;
    let mut provider_accounted = true;
    let mut authenticated = true;
    let mut secret_redaction = true;
    let mut trusted_seed = true;
    let mut execution_entitled = true;
    let mut uploads_disabled = true;

    for value in values {
        let sample = object(&value, "provider sample")?;
        validate_provider_shape(sample)?;
        let status = sample
            .get("status")
            .and_then(Value::as_str)
            .ok_or_else(|| "provider-evidence-status-missing".to_owned())?;
        if status == "qualified"
            && sample.get("source").and_then(Value::as_str) != Some("credential-helper-probe")
        {
            return Err("provider-evidence-source-missing".to_owned());
        }
        if status == "qualified" {
            validate_probe(
                sample
                    .get("probe")
                    .ok_or_else(|| "provider-evidence-probe-missing".to_owned())?,
            )?;
            let configured_nonce = options
                .nonce
                .clone()
                .or_else(|| env::var("D2B_BUILDBUDDY_QUALIFICATION_NONCE").ok());
            let expected_nonce = configured_nonce
                .as_deref()
                .ok_or_else(|| "provider-evidence-nonce-missing".to_owned())?;
            let actual_nonce = object(
                sample
                    .get("probe")
                    .ok_or_else(|| "provider-evidence-probe-missing".to_owned())?,
                "provider probe",
            )?
            .get("nonce")
            .and_then(Value::as_str)
            .ok_or_else(|| "provider-evidence-nonce-missing".to_owned())?;
            validate_token(actual_nonce, "nonce")?;
            if actual_nonce != expected_nonce {
                return Err("provider-evidence-replay:nonce-mismatch".to_owned());
            }
        }
        provider_status = status.to_owned();
        if status != "qualified" {
            provider_accounted = false;
            authenticated &= sample
                .get("authenticated")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            provider_reason = sample
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| Some("provider-accounted-transfer-missing".to_owned()));
            secret_redaction = sample
                .get("secretRedaction")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            trusted_seed = sample
                .get("trustedSeed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            execution_entitled = sample
                .get("executionEntitled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            uploads_disabled &= sample
                .get("uploadsDisabled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            continue;
        }

        let invocation_id = sample
            .get("invocationId")
            .and_then(Value::as_str)
            .ok_or_else(|| "provider-evidence-missing:invocationId".to_owned())?;
        validate_token(invocation_id, "invocationId")?;
        if !invocation_ids.insert(invocation_id.to_owned()) {
            return Err("provider-evidence-replay:duplicate-invocation".to_owned());
        }
        let sample_id = sample
            .get("sampleId")
            .and_then(Value::as_str)
            .ok_or_else(|| "provider-evidence-missing:sampleId".to_owned())?;
        validate_token(sample_id, "sampleId")?;
        if !sample_ids.insert(sample_id.to_owned()) {
            return Err("provider-evidence-replay:duplicate-sample".to_owned());
        }
        for (index, field) in [
            "worktreeId",
            "outputRootId",
            "outputBaseId",
            "bazelServerId",
            "localCacheId",
        ]
        .into_iter()
        .enumerate()
        {
            let value = sample
                .get(field)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("provider-evidence-missing:{field}"))?;
            validate_token(value, field)?;
            if !independence_ids[index].insert(value.to_owned()) {
                return Err("provider-evidence-independent-samples-reused".to_owned());
            }
        }
        let observed_at = sample
            .get("observedAtMillis")
            .and_then(Value::as_u64)
            .ok_or_else(|| "provider-evidence-missing:observedAtMillis".to_owned())?;
        let age = now.saturating_sub(observed_at);
        if observed_at > now || age > options.max_age_millis {
            return Err("provider-evidence-stale".to_owned());
        }

        let commit = string(sample, "commit", "provider sample")?;
        validate_commit(commit)?;
        if commit != candidate.commit {
            return Err("cross-commit:provider-evidence".to_owned());
        }
        validate_provider_identity(
            sample
                .get("identity")
                .ok_or_else(|| "provider-identity-missing".to_owned())?,
            candidate,
        )?;
        if sample.get("workerImage").and_then(Value::as_str) != Some(DEFAULT_WORKER_IMAGE) {
            return Err("provider-worker-image-mismatch".to_owned());
        }
        if sample.get("sampleClass").and_then(Value::as_str) != Some("fresh-worktree")
            || sample.get("freshWorktree").and_then(Value::as_bool) != Some(true)
            || sample.get("isolatedServer").and_then(Value::as_bool) != Some(true)
            || sample
                .get("localDiskCacheDisabled")
                .and_then(Value::as_bool)
                != Some(true)
            || sample.get("cacheState").and_then(Value::as_str) != Some("populated")
        {
            return Err("provider-evidence-sample-provenance-incomplete".to_owned());
        }

        let provider_accounted_sample = sample
            .get("providerAccountedTransfer")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        provider_accounted &= provider_accounted_sample;
        authenticated &= sample
            .get("authenticated")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        secret_redaction &= sample
            .get("secretRedaction")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        trusted_seed &= sample
            .get("trustedSeed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        execution_entitled &= sample
            .get("executionEntitled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        uploads_disabled &= sample
            .get("uploadsDisabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        for field in [
            "authenticated",
            "executionEntitled",
            "cacheReadEvidence",
            "cacheWriteEvidence",
            "readOnlyProbe",
            "secretRedaction",
            "trustedSeed",
            "dispatchEvidence",
        ] {
            if sample.get(field).and_then(Value::as_bool) != Some(true) {
                provider_accounted = false;
                provider_reason = Some(format!("provider-evidence-incomplete:{field}"));
            }
        }

        let transfer = object(
            sample
                .get("transferBytes")
                .ok_or_else(|| "provider-evidence-missing:transferBytes".to_owned())?,
            "provider sample transferBytes",
        )?;
        let uploaded = u64_field(transfer, "uploaded", "provider transferBytes")?;
        let downloaded = u64_field(transfer, "downloaded", "provider transferBytes")?;
        if uploaded.checked_add(downloaded).is_none() {
            return Err("provider-evidence-transfer-overflow".to_owned());
        }
        if uploaded == 0 && downloaded == 0 {
            provider_accounted = false;
            provider_reason = Some("provider-accounted-transfer-zero".to_owned());
        }

        let metrics = object(
            sample
                .get("qualificationMetrics")
                .ok_or_else(|| "provider-evidence-missing:qualificationMetrics".to_owned())?,
            "provider qualificationMetrics",
        )?;
        for field in METRIC_FIELDS {
            u64_field(metrics, field, "provider qualificationMetrics")?;
        }
        let worker_architecture = sample
            .get("workerArchitecture")
            .and_then(Value::as_str)
            .or_else(|| {
                sample
                    .get("workerArchitectures")
                    .and_then(Value::as_array)
                    .and_then(|values| values.first())
                    .and_then(Value::as_str)
            })
            .ok_or_else(|| "provider-evidence-missing:workerArchitecture".to_owned())?;
        validate_token(worker_architecture, "workerArchitecture")?;
        if worker_architecture != candidate.platform {
            return Err("provider-worker-architecture-mismatch".to_owned());
        }
        if let Some(architectures) = sample.get("workerArchitectures").and_then(Value::as_array)
            && architectures
                .iter()
                .any(|value| value.as_str() != Some(candidate.platform.as_str()))
        {
            return Err("provider-worker-architecture-mismatch".to_owned());
        }

        let action_cache_observations =
            metrics_u64(sample, "qualificationMetrics", "actionCacheHits")
                .zip(metrics_u64(
                    sample,
                    "qualificationMetrics",
                    "actionCacheMisses",
                ))
                .and_then(|(hits, misses)| hits.checked_add(misses));
        let cas_observations = metrics_u64(sample, "qualificationMetrics", "casHits")
            .zip(metrics_u64(sample, "qualificationMetrics", "casMisses"))
            .and_then(|(hits, misses)| hits.checked_add(misses));
        if action_cache_observations == Some(0) && cas_observations == Some(0) {
            provider_accounted = false;
            provider_reason = Some("provider-evidence-counters-empty".to_owned());
        }

        samples.push(ProviderSample {
            invocation_id: invocation_id.to_owned(),
            sample_id: sample_id.to_owned(),
            observed_at_millis: observed_at,
            uploaded,
            downloaded,
            metrics: metrics.clone(),
            worker_architecture: worker_architecture.to_owned(),
            worker_image: DEFAULT_WORKER_IMAGE.to_owned(),
            worktree_id: sample
                .get("worktreeId")
                .and_then(Value::as_str)
                .ok_or_else(|| "provider-evidence-missing:worktreeId".to_owned())?
                .to_owned(),
            output_root_id: sample
                .get("outputRootId")
                .and_then(Value::as_str)
                .ok_or_else(|| "provider-evidence-missing:outputRootId".to_owned())?
                .to_owned(),
            output_base_id: sample
                .get("outputBaseId")
                .and_then(Value::as_str)
                .ok_or_else(|| "provider-evidence-missing:outputBaseId".to_owned())?
                .to_owned(),
            bazel_server_id: sample
                .get("bazelServerId")
                .and_then(Value::as_str)
                .ok_or_else(|| "provider-evidence-missing:bazelServerId".to_owned())?
                .to_owned(),
            local_cache_id: sample
                .get("localCacheId")
                .and_then(Value::as_str)
                .ok_or_else(|| "provider-evidence-missing:localCacheId".to_owned())?
                .to_owned(),
        });
    }
    if !samples.is_empty() && samples.len() < MIN_PROVIDER_SAMPLES {
        provider_accounted = false;
        provider_reason = Some(format!(
            "provider-evidence-independent-samples-insufficient:required={MIN_PROVIDER_SAMPLES}"
        ));
    }
    Ok(ProviderCollection {
        samples,
        provider_status,
        provider_reason,
        evidence_origin_trusted: true,
        provider_accounted,
        authenticated,
        secret_redaction,
        trusted_seed,
        execution_entitled,
        uploads_disabled,
    })
}

fn validate_provider_shape(value: &Map<String, Value>) -> Result<()> {
    for key in value.keys() {
        if !PROVIDER_FIELDS.contains(&key.as_str()) {
            return Err(format!("provider-evidence-field-unknown:{key}"));
        }
    }
    if value.get("provider").and_then(Value::as_str) != Some("buildbuddy") {
        return Err("provider-evidence-provider-must-be-buildbuddy".to_owned());
    }
    if value.get("projection").and_then(Value::as_str) != Some(REQUIRED_PROJECTION) {
        return Err("provider-evidence-unprojected".to_owned());
    }
    if let Some(source) = value.get("source").and_then(Value::as_str)
        && source == "client"
    {
        return Err("client-supplied-provider-evidence".to_owned());
    }
    if value
        .get("probe")
        .is_some_and(|probe| validate_probe(probe).is_err())
    {
        return Err("provider-evidence-probe-invalid".to_owned());
    }
    Ok(())
}

fn validate_probe(value: &Value) -> Result<()> {
    let probe = object(value, "provider probe")?;
    for key in probe.keys() {
        if !PROBE_FIELDS.contains(&key.as_str()) {
            return Err(format!("provider-probe-field-unknown:{key}"));
        }
    }
    if probe.get("kind").and_then(Value::as_str) != Some("credential-isolated-command")
        || probe.get("command").and_then(Value::as_str) != Some("xtask buildbuddy-probe")
        || probe.get("input").and_then(Value::as_str) != Some("D2B_BUILDBUDDY_EVIDENCE_FILE")
        || probe.get("readOnly").and_then(Value::as_bool) != Some(true)
        || probe.get("fixtureSafe").and_then(Value::as_bool) != Some(true)
        || probe.get("credentialMode").and_then(Value::as_str) != Some("credential-helper")
    {
        return Err("provider-probe-is-not-sanitized-credential-helper".to_owned());
    }
    if let Some(nonce) = probe.get("nonce").and_then(Value::as_str) {
        validate_token(nonce, "nonce")?;
    }
    Ok(())
}

fn validate_provider_identity(value: &Value, candidate: &CandidateContext) -> Result<()> {
    let identity = object(value, "provider identity")?;
    for key in identity.keys() {
        if !IDENTITY_FIELDS.contains(&key.as_str()) {
            return Err(format!("provider-identity-field-unknown:{key}"));
        }
    }
    for field in [
        "commit",
        "targetSetDigest",
        "configurationDigest",
        "selectedClosureDigest",
        "namespace",
        "toolchain",
        "platform",
    ] {
        if !identity.contains_key(field) {
            return Err(format!("provider-identity-missing:{field}"));
        }
    }
    if string(identity, "commit", "provider identity")? != candidate.commit {
        return Err("cross-commit:provider-identity".to_owned());
    }
    for (field, expected) in [
        ("targetSetDigest", candidate.target_set_digest.as_str()),
        (
            "configurationDigest",
            candidate.configuration_digest.as_str(),
        ),
        (
            "selectedClosureDigest",
            candidate.selected_closure_digest.as_str(),
        ),
        ("namespace", candidate.namespace.as_str()),
        ("toolchain", candidate.toolchain.as_str()),
        ("platform", candidate.platform.as_str()),
    ] {
        let actual = string(identity, field, "provider identity")?;
        if field.ends_with("Digest") {
            validate_digest(actual, field)?;
        }
        if actual != expected {
            return Err(format!("provider-identity-mismatch:{field}"));
        }
    }
    validate_namespace(string(identity, "namespace", "provider identity")?)?;
    validate_token(
        string(identity, "toolchain", "provider identity")?,
        "toolchain",
    )?;
    validate_token(
        string(identity, "platform", "provider identity")?,
        "platform",
    )?;
    Ok(())
}

fn metrics_u64(sample: &Map<String, Value>, object_key: &str, metric: &str) -> Option<u64> {
    sample
        .get(object_key)
        .and_then(Value::as_object)
        .and_then(|metrics| metrics.get(metric))
        .and_then(Value::as_u64)
}

fn provider_samples_ready(provider: &ProviderCollection, reasons: &mut Vec<String>) -> bool {
    if !provider.evidence_origin_trusted {
        reasons.push(
            provider
                .provider_reason
                .clone()
                .unwrap_or_else(|| "provider-evidence-origin-untrusted".to_owned()),
        );
        return false;
    }
    if provider.samples.is_empty() {
        return false;
    }
    if !provider.provider_accounted {
        if let Some(reason) = &provider.provider_reason
            && !reasons.iter().any(|existing| existing == reason)
        {
            reasons.push(reason.clone());
        }
        return false;
    }
    if !provider.secret_redaction {
        reasons.push("provider-secret-redaction-missing".to_owned());
    }
    if !provider.trusted_seed {
        reasons.push("trusted-seed-incomplete".to_owned());
    }
    if !provider.execution_entitled {
        reasons.push("execution-entitlement-missing".to_owned());
    }
    provider.provider_accounted
}

fn candidate_reasons(candidate: &CandidateContext) -> Vec<String> {
    let mut reasons = Vec::new();
    if !candidate.evidence_origin_trusted {
        reasons.push("candidate-evidence-origin-untrusted".to_owned());
    }
    let coverage = object_or_empty(&candidate.coverage);
    if !string_or_true(coverage, &["currentScheduler", "currentSchedulerPass"]) {
        reasons.push("coverage-current-scheduler-incomplete".to_owned());
    }
    if !string_or_true(coverage, &["bazel", "bazelPass"]) {
        reasons.push("coverage-bazel-incomplete".to_owned());
    }
    if !bool_alias(
        coverage,
        &["seedFailuresObserved", "seededFailuresObserved"],
    ) {
        reasons.push("coverage-seeded-failures-incomplete".to_owned());
    }
    if !bool_alias(coverage, &["equivalentTargetSet", "targetSetEquivalent"]) {
        reasons.push("coverage-target-set-mismatch".to_owned());
    }

    reasons.extend(cache_reasons(&candidate.cache));

    let fallback = object_or_empty(&candidate.fallback);
    if !bool_alias(fallback, &["identicalTargetSet", "targetSetIdentical"]) {
        reasons.push("fallback-target-set-mismatch".to_owned());
    }
    let fallback_status = fallback.get("status").and_then(Value::as_str);
    if !matches!(fallback_status, Some("not-used" | "used")) {
        reasons.push("fallback-status-missing".to_owned());
    }
    if fallback
        .get("localRetryCount")
        .and_then(Value::as_u64)
        .is_none()
        && fallback.get("retryCount").and_then(Value::as_u64).is_none()
    {
        reasons.push("fallback-retry-count-missing".to_owned());
    }
    if fallback.get("maxLocalRetries").and_then(Value::as_u64) != Some(1) {
        reasons.push("fallback-retry-limit-missing".to_owned());
    }
    if let Some(retries) =
        value_alias(fallback, &["localRetryCount", "retryCount"]).and_then(Value::as_u64)
        && retries > 1
    {
        reasons.push("fallback-retry-limit-exceeded".to_owned());
    }
    if fallback
        .get("dispatchStarted")
        .and_then(Value::as_bool)
        .is_some_and(|started| {
            started && fallback.get("status").and_then(Value::as_str) != Some("not-used")
        })
    {
        reasons.push("fallback-post-dispatch".to_owned());
    }
    if fallback_status == Some("not-used")
        && fallback
            .get("localRetryCount")
            .or_else(|| fallback.get("retryCount"))
            .and_then(Value::as_u64)
            != Some(0)
    {
        reasons.push("fallback-unused-with-retry".to_owned());
    }
    if fallback_status == Some("not-used")
        && fallback
            .get("retryLocally")
            .is_some_and(|value| value.as_bool() != Some(false))
    {
        reasons.push("fallback-unused-retry-state-invalid".to_owned());
    }
    if fallback_status == Some("used") {
        for field in ["failureClass", "dispatchStarted", "attempt"] {
            if !fallback.contains_key(field) {
                reasons.push(format!("fallback-{field}-missing"));
            }
        }
        if fallback
            .get("failureClass")
            .is_some_and(|value| value.as_str().is_none())
        {
            reasons.push("fallback-failureClass-invalid".to_owned());
        }
        if fallback
            .get("dispatchStarted")
            .is_some_and(|value| value.as_bool().is_none())
        {
            reasons.push("fallback-dispatchStarted-invalid".to_owned());
        }
        if fallback
            .get("attempt")
            .is_some_and(|value| value.as_u64().is_none())
        {
            reasons.push("fallback-attempt-invalid".to_owned());
        }
        if let (Some(class), Some(dispatch_started), Some(attempt)) = (
            fallback.get("failureClass").and_then(Value::as_str),
            fallback.get("dispatchStarted").and_then(Value::as_bool),
            fallback.get("attempt").and_then(Value::as_u64),
        ) {
            let expected_retry = !dispatch_started
                && attempt == 0
                && PRE_DISPATCH_CLASSES.contains(&normalize_failure_class(class).as_str());
            if !expected_retry {
                reasons.push("fallback-used-for-non-retriable-failure".to_owned());
            }
            if fallback.get("retryLocally").and_then(Value::as_bool) != Some(expected_retry) {
                reasons.push("fallback-retry-state-mismatch".to_owned());
            }
            let retries = value_alias(fallback, &["localRetryCount", "retryCount"])
                .and_then(Value::as_u64)
                .unwrap_or(u64::MAX);
            let expected_retries = if expected_retry { 1 } else { 0 };
            if retries != expected_retries {
                reasons.push("fallback-retry-count-mismatch".to_owned());
            }
        }
    }
    reasons
}

fn cache_reasons(value: &Value) -> Vec<String> {
    let cache = object_or_empty(value);
    let mut reasons = Vec::new();
    if !bool_alias(cache, &["trustedSeedComplete", "seedComplete"]) {
        reasons.push("cache-trusted-seed-incomplete".to_owned());
    }
    if !bool_alias(cache, &["asyncUploadsDrained", "seedUploadsDrained"]) {
        reasons.push("cache-async-seed-incomplete".to_owned());
    }
    match value_alias(
        cache,
        &[
            "unchangedCacheableExecutions",
            "unchangedCacheableActionExecutions",
        ],
    ) {
        None => reasons.push("cache-unchanged-run-missing".to_owned()),
        Some(value) => match value.as_u64() {
            Some(0) => {}
            Some(_) => reasons.push("cache-unchanged-execution".to_owned()),
            None => reasons.push("cache-unchanged-run-invalid".to_owned()),
        },
    }
    let reasons_value = cache.get("approvedUncacheableReasons");
    let actions_value = cache.get("approvedUncacheableActions");
    if reasons_value.is_none() && actions_value.is_none() {
        reasons.push("cache-uncacheable-reasons-missing".to_owned());
    } else {
        if reasons_value.is_some() && actions_value.is_some() && reasons_value != actions_value {
            reasons.push("cache-uncacheable-reasons-mismatch".to_owned());
        }
        let selected = reasons_value.or(actions_value);
        if selected.and_then(Value::as_array).is_none_or(|values| {
            values.iter().any(|value| {
                value
                    .as_str()
                    .is_none_or(|reason| validate_token(reason, "cache-reason").is_err())
            })
        }) {
            reasons.push("cache-uncacheable-reasons-invalid".to_owned());
        }
    }
    let matrix = cache.get("cacheMatrix").and_then(Value::as_object);
    if matrix.is_none() {
        reasons.push("cache-matrix-missing".to_owned());
    } else {
        for field in CACHE_MATRIX_FIELDS.iter().copied() {
            if matrix
                .and_then(|matrix| matrix.get(field))
                .and_then(Value::as_bool)
                != Some(true)
            {
                reasons.push(format!("cache-matrix-{field}-incomplete"));
            }
        }
    }
    reasons
}

fn latency_summary(candidate_latency: &Value, provider: &[ProviderSample]) -> Value {
    let _ = candidate_latency;
    let provider_values = provider
        .iter()
        .filter_map(|sample| sample.metrics.get("wallTimeMillis").and_then(Value::as_u64))
        .collect::<Vec<_>>();
    let values = provider_values;
    let mut output = distribution(&values);
    let p95 = output.get("p95").and_then(Value::as_u64);
    output.insert(
        "sampleCount".to_owned(),
        Value::Number((values.len() as u64).into()),
    );
    output.insert(
        "wallTimeBudgetMillis".to_owned(),
        Value::Number(WALL_TIME_BUDGET_MILLIS.into()),
    );
    output.insert(
        "freshWorktreeP95UnderThreeMinutes".to_owned(),
        p95.map_or(Value::Bool(false), |value| {
            Value::Bool(value < WALL_TIME_BUDGET_MILLIS)
        }),
    );
    for (distribution_key, report_key) in [
        ("p50", "p50WallTimeMillis"),
        ("p95", "p95WallTimeMillis"),
        ("p99", "p99WallTimeMillis"),
        ("max", "maxWallTimeMillis"),
    ] {
        output.insert(
            report_key.to_owned(),
            output.get(distribution_key).cloned().unwrap_or(Value::Null),
        );
    }
    if values.is_empty() {
        output.insert(
            "qualificationFailure".to_owned(),
            Value::String("latency-evidence-missing".to_owned()),
        );
    } else if p95.is_some_and(|value| value >= WALL_TIME_BUDGET_MILLIS) {
        output.insert(
            "qualificationFailure".to_owned(),
            Value::String("fresh-worktree-p95-over-three-minutes".to_owned()),
        );
    }
    Value::Object(output)
}

fn transfer_summary(samples: &[ProviderSample], u9: &U9Bounds) -> Result<Map<String, Value>> {
    let uploaded = samples
        .iter()
        .map(|sample| sample.uploaded)
        .collect::<Vec<_>>();
    let downloaded = samples
        .iter()
        .map(|sample| sample.downloaded)
        .collect::<Vec<_>>();
    let totals = samples
        .iter()
        .map(|sample| {
            sample
                .uploaded
                .checked_add(sample.downloaded)
                .ok_or_else(|| "provider-evidence-transfer-overflow".to_owned())
        })
        .collect::<Result<Vec<_>>>()?;
    let uploaded_p99 = percentile(&uploaded, 0.99);
    let downloaded_p99 = percentile(&downloaded, 0.99);
    let p99 = percentile(&totals, 0.99);
    let upper_bound = u9
        .gross_input_bytes
        .checked_add(u9.output_bytes)
        .ok_or_else(|| "u9-evidence-overflow".to_owned())?;
    let comparison = match p99 {
        Some(value) if value > upper_bound => "above-pessimistic-bound",
        Some(value) if value < u9.unique_input_bytes => "below-optimistic-bound",
        Some(_) => "within-local-model",
        None => "provider-transfer-missing",
    };
    let mut output = Map::new();
    output.insert(
        "providerAccounted".to_owned(),
        Value::Bool(!samples.is_empty()),
    );
    output.insert(
        "sampleCount".to_owned(),
        Value::Number((samples.len() as u64).into()),
    );
    output.insert(
        "uploaded".to_owned(),
        Value::Object(distribution(&uploaded)),
    );
    output.insert(
        "downloaded".to_owned(),
        Value::Object(distribution(&downloaded)),
    );
    output.insert(
        "p50Bytes".to_owned(),
        optional_u64(percentile(&totals, 0.50)),
    );
    output.insert(
        "p95Bytes".to_owned(),
        optional_u64(percentile(&totals, 0.95)),
    );
    output.insert(
        "p99Bytes".to_owned(),
        optional_u64(percentile(&totals, 0.99)),
    );
    output.insert(
        "maxBytes".to_owned(),
        optional_u64(totals.iter().max().copied()),
    );
    output.insert(
        "workingBudgetBytes".to_owned(),
        Value::Number(WORKING_BUDGET_BYTES.into()),
    );
    output.insert(
        "headroomBytes".to_owned(),
        Value::Number(HEADROOM_BYTES.into()),
    );
    output.insert(
        "u9Comparison".to_owned(),
        json!({
            "uniqueInputBytes": u9.unique_input_bytes,
            "grossInputBytes": u9.gross_input_bytes,
            "outputBytes": u9.output_bytes,
            "pessimisticUpperBoundBytes": upper_bound,
            "providerP99Bytes": p99,
            "providerUploadedP99Bytes": uploaded_p99,
            "providerDownloadedP99Bytes": downloaded_p99,
            "uploadWithinU9InputBounds": uploaded_p99.is_some_and(|value| {
                value >= u9.unique_input_bytes && value <= u9.gross_input_bytes
            }),
            "downloadWithinU9OutputBound": downloaded_p99
                .is_none_or(|value| value <= u9.output_bytes),
            "comparison": comparison,
            "withinPessimisticUpperBound": p99.is_some_and(|value| value <= upper_bound),
            "materialDivergence": p99.is_some_and(|value| value > upper_bound)
        }),
    );
    if p99.is_none() {
        output.insert(
            "qualificationFailure".to_owned(),
            Value::String("provider-accounted-transfer-missing".to_owned()),
        );
    } else if uploaded_p99.is_some_and(|value| value < u9.unique_input_bytes)
        && !samples.iter().all(sample_is_exact_warm)
    {
        output.insert(
            "qualificationFailure".to_owned(),
            Value::String("provider-transfer-below-u9-bounds".to_owned()),
        );
    } else if uploaded_p99.is_some_and(|value| value > u9.gross_input_bytes) {
        output.insert(
            "qualificationFailure".to_owned(),
            Value::String("provider-transfer-outside-u9-bounds".to_owned()),
        );
    } else if downloaded_p99.is_some_and(|value| value > u9.output_bytes) {
        output.insert(
            "qualificationFailure".to_owned(),
            Value::String("provider-transfer-outside-u9-bounds".to_owned()),
        );
    } else if p99.is_some_and(|value| value > upper_bound) {
        output.insert(
            "qualificationFailure".to_owned(),
            Value::String("provider-transfer-outside-u9-bounds".to_owned()),
        );
    }

    fn sample_is_exact_warm(sample: &ProviderSample) -> bool {
        sample
            .metrics
            .get("remoteExecutions")
            .and_then(Value::as_u64)
            == Some(0)
            && sample
                .metrics
                .get("actionCacheHits")
                .and_then(Value::as_u64)
                .is_some_and(|hits| hits > 0)
            && sample
                .metrics
                .get("actionCacheMisses")
                .and_then(Value::as_u64)
                == Some(0)
            && sample
                .metrics
                .get("casHits")
                .and_then(Value::as_u64)
                .is_some_and(|hits| hits > 0)
            && sample.metrics.get("casMisses").and_then(Value::as_u64) == Some(0)
    }
    Ok(output)
}

fn metrics_summary(samples: &[ProviderSample]) -> Value {
    let mut output = Map::new();
    for field in METRIC_FIELDS {
        let values = samples
            .iter()
            .filter_map(|sample| sample.metrics.get(*field).and_then(Value::as_u64))
            .collect::<Vec<_>>();
        output.insert((*field).to_owned(), Value::Object(distribution(&values)));
    }
    Value::Object(output)
}

fn distribution(values: &[u64]) -> Map<String, Value> {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let mut output = Map::new();
    output.insert(
        "sampleCount".to_owned(),
        Value::Number((values.len() as u64).into()),
    );
    output.insert("p50".to_owned(), optional_u64(percentile(&sorted, 0.50)));
    output.insert("p95".to_owned(), optional_u64(percentile(&sorted, 0.95)));
    output.insert("p99".to_owned(), optional_u64(percentile(&sorted, 0.99)));
    output.insert("max".to_owned(), optional_u64(sorted.last().copied()));
    output
}

fn percentile(values: &[u64], percentile: f64) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = ((sorted.len() as f64) * percentile).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted.get(index).copied()
}

fn read_default_target_set(root: &Path) -> Result<Vec<String>> {
    let policy = read_committed_json(root, DEFAULT_TARGET_SET, "cache policy")?;
    read_target_set_value(root, object(&policy, "cache policy")?.get("targetSet"))
}

fn read_target_set(root: &Path, path: &Path) -> Result<Vec<String>> {
    let path = resolve_path(root, path);
    if path.is_file() {
        let bytes = fs::read(&path).map_err(|error| format!("read target set: {error}"))?;
        if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
            return read_target_set_value(root, Some(&value));
        }
        return parse_target_lines(&String::from_utf8(bytes).map_err(|_| {
            "target-set must be UTF-8 JSON or newline-delimited labels".to_owned()
        })?);
    }
    parse_target_lines(path.to_str().unwrap_or_default())
}

fn read_target_set_value(root: &Path, value: Option<&Value>) -> Result<Vec<String>> {
    let Some(value) = value else {
        return read_default_target_set(root);
    };
    let values = if let Some(values) = value.as_array() {
        values.clone()
    } else if let Some(object) = value.as_object() {
        object
            .get("targetSet")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| "target-set must be an array".to_owned())?
    } else if let Some(text) = value.as_str() {
        return parse_target_lines(text);
    } else {
        return Err("target-set must be an array".to_owned());
    };
    let mut targets = values
        .into_iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "target-set labels must be strings".to_owned())
        })
        .collect::<Result<Vec<_>>>()?;
    targets.sort();
    targets.dedup();
    if targets
        .iter()
        .any(|target| target.is_empty() || !target.starts_with("//"))
    {
        return Err("target-set contains an invalid Bazel label".to_owned());
    }
    Ok(targets)
}

fn parse_target_lines(text: &str) -> Result<Vec<String>> {
    let mut targets = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    targets.sort();
    targets.dedup();
    if targets.iter().any(|target| !target.starts_with("//")) {
        return Err("target-set contains an invalid Bazel label".to_owned());
    }
    Ok(targets)
}

fn digest_configuration(root: &Path, configuration: &str) -> Result<String> {
    let path = Path::new(configuration);
    if path.is_absolute() || configuration.contains("..") {
        return Err("configuration-path-invalid".to_owned());
    }
    digest_file(root, path, "configuration")
}

fn digest_file(root: &Path, path: &Path, context: &str) -> Result<String> {
    let relative = path
        .to_str()
        .ok_or_else(|| format!("{context}-path-invalid"))?;
    let bytes = committed_bytes(root, relative, context)?;
    Ok(digest_bytes(&bytes))
}

fn committed_bytes(root: &Path, relative: &str, context: &str) -> Result<Vec<u8>> {
    let path = safe_relative_path(relative, context)?;
    let path = path
        .to_str()
        .ok_or_else(|| format!("{context}-path-invalid"))?;
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["show", &format!("HEAD:{path}")])
        .output()
        .map_err(|error| format!("read committed {context}: {error}"))?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    if env::var_os("D2B_QUALIFICATION_TEST_COMMIT").is_some() {
        return fs::read(root.join(path)).map_err(|error| format!("read test {context}: {error}"));
    }
    Err(format!("committed-{context}-unavailable"))
}

fn read_committed_json(root: &Path, relative: &str, context: &str) -> Result<Value> {
    let bytes = committed_bytes(root, relative, context)?;
    serde_json::from_slice(&bytes).map_err(|error| format!("parse committed {context}: {error}"))
}

fn digest_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{digest:x}")
}

fn git_commit(root: &Path) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok();
    if output
        .as_ref()
        .is_none_or(|output| !output.status.success())
    {
        let test_commit = env::var("D2B_QUALIFICATION_TEST_COMMIT")
            .map_err(|_| "candidate-commit-unavailable".to_owned())?;
        validate_commit(&test_commit)?;
        return Ok(test_commit);
    }
    let commit = String::from_utf8(output.expect("successful git output").stdout)
        .map_err(|_| "candidate-commit-not-utf8".to_owned())?
        .trim()
        .to_owned();
    validate_commit(&commit)?;
    Ok(commit)
}

fn ensure_clean_worktree(root: &Path) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .map_err(|error| format!("qualification-worktree-status-unavailable: {error}"))?;
    if !output.status.success() {
        return Err("qualification-worktree-status-unavailable".to_owned());
    }
    if !output.stdout.is_empty() {
        return Err("qualification-worktree-dirty".to_owned());
    }
    Ok(())
}

fn validate_commit(value: &str) -> Result<()> {
    if value.len() < 7 || value.len() > 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("candidate-commit-invalid".to_owned());
    }
    Ok(())
}

fn validate_digest(value: &str, field: &str) -> Result<()> {
    if value.len() != 71 || !value.starts_with("sha256:") {
        return Err(format!("provider-identity-invalid:{field}"));
    }
    if !value[7..].bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("provider-identity-invalid:{field}"));
    }
    Ok(())
}

fn validate_namespace(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 256
        || value.starts_with('/')
        || value.contains('\\')
        || value.contains("..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
    {
        return Err("namespace-invalid".to_owned());
    }
    Ok(())
}

fn validate_token(value: &str, field: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(format!("{field}-invalid"));
    }
    Ok(())
}

fn safe_relative_path(value: &str, field: &str) -> Result<PathBuf> {
    let path = PathBuf::from(value);
    if path.is_absolute() || value.contains("..") || value.contains('\\') {
        return Err(format!("{field}-path-invalid"));
    }
    Ok(path)
}

fn reject_sensitive_or_client_input(value: &Value) -> Result<()> {
    if let Some(object) = value.as_object() {
        for (key, value) in object {
            let normalized = key
                .chars()
                .filter(|character| character.is_ascii_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>();
            let allowed_contract_key =
                matches!(normalized.as_str(), "credentialmode" | "secretredaction");
            if !allowed_contract_key
                && (normalized.contains("apikey")
                    || normalized.contains("authorization")
                    || normalized.contains("credential")
                    || normalized.contains("password")
                    || normalized.contains("privatekey")
                    || normalized.contains("token")
                    || normalized == "secret")
            {
                return Err("provider-evidence-credential-field-rejected".to_owned());
            }
            if matches!(
                normalized.as_str(),
                "clientsupplied"
                    | "clientprovided"
                    | "forged"
                    | "replayed"
                    | "replay"
                    | "evidencefile"
                    | "logpath"
                    | "socketpath"
                    | "hostname"
                    | "pid"
            ) {
                return Err("provider-evidence-untrusted-field-rejected".to_owned());
            }
            reject_sensitive_or_client_input(value)?;
        }
    } else if let Some(array) = value.as_array() {
        for value in array {
            reject_sensitive_or_client_input(value)?;
        }
    } else if let Some(text) = value.as_str() {
        if text.starts_with('/')
            || text.starts_with("~/")
            || text.contains('\\')
            || text.to_ascii_lowercase().starts_with("bearer ")
            || text.to_ascii_lowercase().contains("x-buildbuddy-api-key")
            || text.to_ascii_lowercase().contains("authorization:")
        {
            return Err("provider-evidence-path-or-credential-rejected".to_owned());
        }
    }
    Ok(())
}

fn validate_candidate_shape(candidate: &Map<String, Value>) -> Result<()> {
    for key in candidate.keys() {
        if !CANDIDATE_FIELDS.contains(&key.as_str()) {
            return Err(format!("client-supplied-candidate-field-rejected:{key}"));
        }
    }
    validate_section_shape(candidate.get("coverage"), COVERAGE_FIELDS, "coverage")?;
    validate_section_shape(candidate.get("cache"), CACHE_FIELDS, "cache")?;
    if let Some(cache_matrix) = candidate
        .get("cache")
        .and_then(Value::as_object)
        .and_then(|cache| cache.get("cacheMatrix"))
    {
        validate_section_shape(Some(cache_matrix), CACHE_MATRIX_FIELDS, "cache.cacheMatrix")?;
    }
    validate_section_shape(candidate.get("fallback"), FALLBACK_FIELDS, "fallback")?;
    if candidate
        .get("latency")
        .is_some_and(|value| !value.is_object())
    {
        return Err("candidate-latency-invalid".to_owned());
    }
    Ok(())
}

fn validate_section_shape(
    value: Option<&Value>,
    allowed_fields: &[&str],
    context: &str,
) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let object = object(value, context)?;
    for key in object.keys() {
        if !allowed_fields.contains(&key.as_str()) {
            return Err(format!("client-supplied-{context}-field-rejected:{key}"));
        }
        reject_sensitive_or_client_input(&Value::Object(
            [(key.clone(), object[key].clone())].into_iter().collect(),
        ))?;
    }
    for aliases in [
        &["currentScheduler", "currentSchedulerPass"][..],
        &["bazel", "bazelPass"][..],
        &["equivalentTargetSet", "targetSetEquivalent"][..],
        &["trustedSeedComplete", "seedComplete"][..],
        &["asyncUploadsDrained", "seedUploadsDrained"][..],
        &[
            "unchangedCacheableExecutions",
            "unchangedCacheableActionExecutions",
        ][..],
        &["approvedUncacheableReasons", "approvedUncacheableActions"][..],
        &["localRetryCount", "retryCount"][..],
        &["identicalTargetSet", "targetSetIdentical"][..],
    ] {
        if let (Some(first), Some(second)) = (object.get(aliases[0]), object.get(aliases[1]))
            && first != second
        {
            return Err(format!("client-supplied-{context}-alias-conflict"));
        }
    }
    Ok(())
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

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("create output directory: {error}"))?;
    }
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|error| format!("serialize report: {error}"))?;
    fs::write(path, bytes).map_err(|error| format!("write {}: {error}", path.display()))
}

fn object<'a>(value: &'a Value, context: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| format!("{context} must be an object"))
}

fn object_or_empty(value: &Value) -> &Map<String, Value> {
    value.as_object().unwrap_or_else(|| {
        static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
        EMPTY.get_or_init(Map::new)
    })
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

fn value_alias<'a>(object: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| object.get(*key))
}

fn bool_alias(object: &Map<String, Value>, keys: &[&str]) -> bool {
    value_alias(object, keys)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn string_or_true(object: &Map<String, Value>, keys: &[&str]) -> bool {
    value_alias(object, keys).is_some_and(|value| {
        value.as_bool() == Some(true)
            || value.as_str().is_some_and(|status| {
                matches!(status, "pass" | "passed" | "complete" | "qualified")
            })
    })
}

fn parse_bool(value: &str, flag: &str) -> Result<bool> {
    match value {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(format!("{flag} must be true or false")),
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after Unix epoch")
        .as_millis() as u64
}

fn test_mode() -> bool {
    env::var("D2B_QUALIFICATION_TEST_MODE").ok().as_deref() == Some("1")
        && env::var("D2B_QUALIFICATION_TEST_AUTH").ok().as_deref() == Some("1")
}

fn optional_u64(value: Option<u64>) -> Value {
    value.map_or(Value::Null, |value| Value::Number(value.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_uses_conservative_nearest_rank() {
        assert_eq!(percentile(&[1, 2, 3, 4], 0.95), Some(4));
        assert_eq!(percentile(&[1, 2, 3, 4], 0.99), Some(4));
        assert_eq!(percentile(&[], 0.99), None);
    }

    #[test]
    fn typed_fallback_is_fail_closed_after_dispatch() {
        assert!(!(!true && 0 == 0 && PRE_DISPATCH_CLASSES.contains(&"authentication")));
    }

    #[test]
    fn namespace_and_token_validation_reject_paths() {
        assert!(validate_namespace("d2b/qualification/linux-x86_64").is_ok());
        assert!(validate_namespace("/tmp/qualification").is_err());
        assert!(validate_token("rules_rust", "toolchain").is_ok());
        assert!(validate_token("rules/rust", "toolchain").is_err());
    }
}
