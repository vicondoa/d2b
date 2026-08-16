#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest as ShaDigest, Sha256};

const SCHEMA_VERSION: u32 = 1;
const TOP_ARTIFACT_COUNT: usize = 20;

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionClass {
    Rbe,
    RemoteCacheOnly,
    FullyLocal,
}

impl ExecutionClass {
    fn is_remote(self) -> bool {
        matches!(self, Self::Rbe | Self::RemoteCacheOnly)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Semantics {
    pub gross_bytes: String,
    pub unique_bytes: String,
    pub provider_billing: bool,
    pub remote_classes: Vec<ExecutionClass>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMetadata {
    pub execution_log: String,
    pub eligibility: String,
    pub eligibility_digest: String,
    pub graph_digest: String,
    pub configuration: Value,
    pub platform: Value,
    pub toolchain: Value,
    #[serde(default)]
    pub unlisted_targets: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRef {
    pub path: String,
    pub digest: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactExposure {
    pub path: String,
    pub paths: Vec<String>,
    pub digest: String,
    pub size_bytes: u64,
    pub fan_out: u64,
    pub producer_count: u64,
    pub consumer_count: u64,
    pub responsible_targets: Vec<String>,
    pub execution_classes: Vec<ExecutionClass>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeSummary {
    pub action_count: u64,
    pub gross_input_bytes: u64,
    pub unique_input_bytes: u64,
    pub output_bytes: u64,
    pub input_artifact_count: u64,
    pub unique_input_artifact_count: u64,
    pub output_artifact_count: u64,
    pub fan_out_artifacts: u64,
    pub max_fan_out: u64,
    pub fan_out_ratio: f64,
    pub targets: Vec<String>,
    pub responsible_targets: Vec<String>,
    #[serde(default)]
    pub largest_inputs: Vec<ArtifactExposure>,
    #[serde(default)]
    pub largest_outputs: Vec<ArtifactExposure>,
    #[serde(default)]
    pub highest_exposure: Vec<ArtifactExposure>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionSummary {
    pub id: String,
    pub mnemonic: String,
    pub target_label: String,
    pub execution_class: ExecutionClass,
    pub inputs: Vec<ArtifactRef>,
    pub outputs: Vec<ArtifactRef>,
    pub gross_input_bytes: u64,
    pub unique_input_bytes: u64,
    pub output_bytes: u64,
    pub input_artifact_count: u64,
    pub unique_input_artifact_count: u64,
    pub output_artifact_count: u64,
    pub fan_out_artifacts: u64,
    pub max_fan_out: u64,
    pub fan_out_ratio: f64,
    pub responsible_targets: Vec<String>,
    pub largest_inputs: Vec<ArtifactRef>,
    pub largest_outputs: Vec<ArtifactRef>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MnemonicSummary {
    pub mnemonic: String,
    pub action_count: u64,
    pub gross_input_bytes: u64,
    pub unique_input_bytes: u64,
    pub output_bytes: u64,
    pub input_artifact_count: u64,
    pub unique_input_artifact_count: u64,
    pub output_artifact_count: u64,
    pub fan_out_artifacts: u64,
    pub max_fan_out: u64,
    pub fan_out_ratio: f64,
    pub targets: Vec<String>,
    pub responsible_targets: Vec<String>,
    pub largest_inputs: Vec<ArtifactExposure>,
    pub largest_outputs: Vec<ArtifactExposure>,
    pub highest_exposure: Vec<ArtifactExposure>,
    pub by_class: BTreeMap<ExecutionClass, ScopeSummary>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryCrossing {
    pub direction: String,
    pub from_class: ExecutionClass,
    pub to_class: ExecutionClass,
    pub path: String,
    pub digest: String,
    pub size_bytes: u64,
    pub producer_actions: Vec<String>,
    pub consumer_actions: Vec<String>,
    pub producer_targets: Vec<String>,
    pub consumer_targets: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricDelta {
    pub action_count: i64,
    pub gross_input_bytes: i64,
    pub unique_input_bytes: i64,
    pub output_bytes: i64,
    pub fan_out_artifacts: i64,
    pub fan_out_ratio: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeltaSummary {
    pub whole_graph: MetricDelta,
    pub rbe: MetricDelta,
    pub remote_cache_only: MetricDelta,
    pub fully_local: MetricDelta,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub schema_version: u32,
    pub report_kind: String,
    pub semantics: Semantics,
    pub source: SourceMetadata,
    pub whole_graph: ScopeSummary,
    pub classes: BTreeMap<ExecutionClass, ScopeSummary>,
    pub actions: Vec<ActionSummary>,
    pub mnemonics: Vec<MnemonicSummary>,
    pub largest_artifacts: LargestArtifacts,
    pub boundary_crossings: Vec<BoundaryCrossing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<DeltaSummary>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LargestArtifacts {
    pub inputs: Vec<ArtifactExposure>,
    pub outputs: Vec<ArtifactExposure>,
    pub highest_exposure: Vec<ArtifactExposure>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonReport {
    pub schema_version: u32,
    pub report_kind: String,
    pub compatible: bool,
    pub baseline_source: SourceMetadata,
    pub optimized_source: SourceMetadata,
    pub delta: DeltaSummary,
}

#[derive(Clone, Debug, Serialize)]
struct Artifact {
    path: String,
    digest: String,
    size_bytes: u64,
}

#[derive(Clone, Debug)]
struct ParsedAction {
    id: String,
    mnemonic: String,
    target_label: String,
    execution_class: ExecutionClass,
    inputs: Vec<Artifact>,
    outputs: Vec<Artifact>,
}

#[derive(Default)]
struct ArtifactInfo {
    size_bytes: Option<u64>,
    paths: BTreeSet<String>,
    input_consumers: BTreeSet<usize>,
    output_producers: BTreeSet<usize>,
    input_consumers_by_path: BTreeMap<String, BTreeSet<usize>>,
    output_producers_by_path: BTreeMap<String, BTreeSet<usize>>,
}

#[derive(Clone, Debug)]
struct EligibilityEntry {
    eligible: bool,
}

pub fn run_cli(args: &[String]) -> ExitCode {
    match run(args) {
        Ok(Some(value)) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("cache-transfer result serializes")
            );
            ExitCode::SUCCESS
        }
        Ok(None) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("bazel-cache-transfer failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<Option<Value>> {
    if matches!(args, [flag] if flag == "--help" || flag == "-h") {
        print_usage();
        return Ok(None);
    }
    if args.first().is_some_and(|arg| arg == "compare") {
        return run_compare(&args[1..]);
    }

    let options = AnalyzeOptions::parse(args)?;
    let report = analyze_paths(
        &options.execution_log,
        &options.eligibility,
        options.configuration,
        options.platform,
        options.toolchain,
    )?;
    let report = if let Some(baseline_path) = options.baseline {
        let baseline = read_report(&baseline_path)?;
        let delta = compare_identity(&baseline, &report)?;
        Report {
            delta: Some(delta),
            ..report
        }
    } else {
        report
    };
    let value = serde_json::to_value(&report).map_err(|error| error.to_string())?;
    write_or_print(options.output.as_deref(), &value)?;
    Ok(None)
}

fn run_compare(args: &[String]) -> Result<Option<Value>> {
    let options = CompareOptions::parse(args)?;
    let baseline = read_report(&options.baseline)?;
    let optimized = read_report(&options.optimized)?;
    let delta = compare_identity(&baseline, &optimized)?;
    let comparison = ComparisonReport {
        schema_version: SCHEMA_VERSION,
        report_kind: "bazel-cache-transfer-comparison".to_owned(),
        compatible: true,
        baseline_source: baseline.source,
        optimized_source: optimized.source,
        delta,
    };
    let value = serde_json::to_value(comparison).map_err(|error| error.to_string())?;
    if let Some(output) = options.output {
        write_json(&output, &value)?;
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&value).expect("comparison serializes")
        );
    }
    Ok(None)
}

fn print_usage() {
    println!(
        "usage: xtask bazel-cache-transfer --execution-log <path> --eligibility <path> [--output <path>] [--baseline <path>] [--configuration <value>] [--platform <value>] [--toolchain <value>]"
    );
    println!(
        "       xtask bazel-cache-transfer compare --baseline <path> --optimized <path> [--output <path>]"
    );
}

struct AnalyzeOptions {
    execution_log: PathBuf,
    eligibility: PathBuf,
    output: Option<PathBuf>,
    baseline: Option<PathBuf>,
    configuration: Option<Value>,
    platform: Option<Value>,
    toolchain: Option<Value>,
}

impl AnalyzeOptions {
    fn parse(args: &[String]) -> Result<Self> {
        let mut execution_log = None;
        let mut eligibility = None;
        let mut output = None;
        let mut baseline = None;
        let mut configuration = None;
        let mut platform = None;
        let mut toolchain = None;
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
                "--execution-log" | "--log" => {
                    execution_log = Some(PathBuf::from(value(&mut index)?))
                }
                "--eligibility" => eligibility = Some(PathBuf::from(value(&mut index)?)),
                "--output" | "-o" => output = Some(PathBuf::from(value(&mut index)?)),
                "--baseline" => baseline = Some(PathBuf::from(value(&mut index)?)),
                "--configuration" => {
                    configuration = Some(Value::String(value(&mut index)?));
                }
                "--platform" => platform = Some(Value::String(value(&mut index)?)),
                "--toolchain" => toolchain = Some(Value::String(value(&mut index)?)),
                "--help" | "-h" => {
                    print_usage();
                    return Err("help requested".to_owned());
                }
                _ => return Err(format!("unknown option {flag}")),
            }
            index += 1;
        }
        Ok(Self {
            execution_log: execution_log.ok_or_else(|| "--execution-log is required".to_owned())?,
            eligibility: eligibility.ok_or_else(|| "--eligibility is required".to_owned())?,
            output,
            baseline,
            configuration,
            platform,
            toolchain,
        })
    }
}

struct CompareOptions {
    baseline: PathBuf,
    optimized: PathBuf,
    output: Option<PathBuf>,
}

impl CompareOptions {
    fn parse(args: &[String]) -> Result<Self> {
        let mut baseline = None;
        let mut optimized = None;
        let mut output = None;
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
                "--baseline" => baseline = Some(PathBuf::from(value(&mut index)?)),
                "--optimized" => optimized = Some(PathBuf::from(value(&mut index)?)),
                "--output" | "-o" => output = Some(PathBuf::from(value(&mut index)?)),
                "--help" | "-h" => {
                    print_usage();
                    return Err("help requested".to_owned());
                }
                _ => return Err(format!("unknown compare option {flag}")),
            }
            index += 1;
        }
        Ok(Self {
            baseline: baseline.ok_or_else(|| "--baseline is required".to_owned())?,
            optimized: optimized.ok_or_else(|| "--optimized is required".to_owned())?,
            output,
        })
    }
}

pub fn analyze_paths(
    execution_log: &Path,
    eligibility_path: &Path,
    configuration_override: Option<Value>,
    platform_override: Option<Value>,
    toolchain_override: Option<Value>,
) -> Result<Report> {
    let execution_bytes = fs::read(execution_log)
        .map_err(|error| format!("read execution log {}: {error}", execution_log.display()))?;
    let execution_value = parse_execution_json(&execution_bytes)?;
    let eligibility_bytes = fs::read(eligibility_path)
        .map_err(|error| format!("read eligibility {}: {error}", eligibility_path.display()))?;
    let eligibility_value: Value = serde_json::from_slice(&eligibility_bytes)
        .map_err(|error| format!("parse eligibility {}: {error}", eligibility_path.display()))?;
    let eligibility = parse_eligibility(&eligibility_value)?;
    let records = execution_records(&execution_value)?;
    let mut actions = Vec::new();
    let mut seen_ids = BTreeSet::new();
    let mut seen_fingerprints = BTreeSet::new();
    let mut unlisted_targets = BTreeSet::new();
    for (index, record) in records.iter().enumerate() {
        let Some(action) = parse_action(
            record,
            index,
            &eligibility,
            &mut seen_ids,
            &mut seen_fingerprints,
            &mut unlisted_targets,
        )?
        else {
            continue;
        };
        actions.push(action);
    }
    if actions.is_empty() {
        return Err("execution log contains no SpawnExec records".to_owned());
    }
    actions.sort_by(|left, right| left.id.cmp(&right.id));

    let configuration = configuration_override
        .or_else(|| metadata_value(&execution_value, "configuration"))
        .unwrap_or(Value::Null);
    let platform = platform_override
        .or_else(|| metadata_value(&execution_value, "platform"))
        .unwrap_or(Value::Null);
    let toolchain = toolchain_override
        .or_else(|| metadata_value(&execution_value, "toolchain"))
        .unwrap_or(Value::Null);
    require_identity(&configuration, "configuration")?;
    require_identity(&platform, "platform")?;
    require_identity(&toolchain, "toolchain")?;

    let mut artifacts = BTreeMap::<String, ArtifactInfo>::new();
    for (action_index, action) in actions.iter().enumerate() {
        for artifact in &action.inputs {
            let info = artifacts.entry(artifact.digest.clone()).or_default();
            record_artifact(info, artifact)?;
            info.input_consumers.insert(action_index);
            info.input_consumers_by_path
                .entry(artifact.path.clone())
                .or_default()
                .insert(action_index);
        }
        for artifact in &action.outputs {
            let info = artifacts.entry(artifact.digest.clone()).or_default();
            record_artifact(info, artifact)?;
            info.output_producers.insert(action_index);
            info.output_producers_by_path
                .entry(artifact.path.clone())
                .or_default()
                .insert(action_index);
        }
    }

    let action_summaries = actions
        .iter()
        .map(|action| action_summary(action, &artifacts))
        .collect::<Result<Vec<_>>>()?;
    let all_indices = (0..actions.len()).collect::<Vec<_>>();
    let whole_graph = scope_summary(&all_indices, &actions, &artifacts)?;
    let mut classes = BTreeMap::new();
    for class in [
        ExecutionClass::Rbe,
        ExecutionClass::RemoteCacheOnly,
        ExecutionClass::FullyLocal,
    ] {
        let indices = actions
            .iter()
            .enumerate()
            .filter_map(|(index, action)| (action.execution_class == class).then_some(index))
            .collect::<Vec<_>>();
        classes.insert(class, scope_summary(&indices, &actions, &artifacts)?);
    }
    let mnemonics = mnemonic_summaries(&actions, &artifacts)?;
    let largest_artifacts = largest_artifacts(&actions, &artifacts);
    let boundary_crossings = boundary_crossings(&actions, &artifacts);
    let graph_digest = graph_digest(&actions)?;

    Ok(Report {
        schema_version: SCHEMA_VERSION,
        report_kind: "bazel-cache-transfer".to_owned(),
        semantics: Semantics {
            gross_bytes: "pessimistic cold-executor upper bound".to_owned(),
            unique_bytes: "optimistic warm-executor lower bound".to_owned(),
            provider_billing: false,
            remote_classes: vec![ExecutionClass::Rbe, ExecutionClass::RemoteCacheOnly],
        },
        source: SourceMetadata {
            execution_log: execution_log.display().to_string(),
            eligibility: eligibility_path.display().to_string(),
            eligibility_digest: digest_bytes(&eligibility_bytes),
            graph_digest,
            configuration,
            platform,
            toolchain,
            unlisted_targets: unlisted_targets.into_iter().collect(),
        },
        whole_graph,
        classes,
        actions: action_summaries,
        mnemonics,
        largest_artifacts,
        boundary_crossings,
        delta: None,
    })
}

fn parse_execution_json(bytes: &[u8]) -> Result<Value> {
    let mut stream = serde_json::Deserializer::from_slice(bytes).into_iter::<Value>();
    let first = stream
        .next()
        .transpose()
        .map_err(|error| format!("parse execution log JSON: {error}"))?
        .ok_or_else(|| "execution log is empty".to_owned())?;
    let mut records = vec![first];
    for value in stream {
        records.push(value.map_err(|error| format!("parse execution log JSON: {error}"))?);
    }
    if records.len() == 1 {
        Ok(records.pop().expect("one parsed execution value"))
    } else {
        Ok(Value::Array(records))
    }
}

fn execution_records(value: &Value) -> Result<Vec<Value>> {
    if let Some(array) = value.as_array() {
        return Ok(array.clone());
    }
    let object = value
        .as_object()
        .ok_or_else(|| "execution log root must be an object or array".to_owned())?;
    for key in ["records", "events", "spawns", "executionLog"] {
        if let Some(array) = object.get(key).and_then(Value::as_array) {
            return Ok(array.clone());
        }
    }
    if spawn_payload(value).is_some() {
        return Ok(vec![value.clone()]);
    }
    Err("execution log has no records array".to_owned())
}

fn parse_eligibility(value: &Value) -> Result<BTreeMap<String, EligibilityEntry>> {
    let entries = if let Some(array) = value.as_array() {
        array
    } else {
        value
            .get("entries")
            .or_else(|| value.get("targets"))
            .and_then(Value::as_array)
            .ok_or_else(|| "eligibility must contain an entries array".to_owned())?
    };
    let mut result = BTreeMap::new();
    for (index, entry) in entries.iter().enumerate() {
        let object = entry
            .as_object()
            .ok_or_else(|| format!("eligibility entry {index} must be an object"))?;
        let label = string_field(
            object,
            &["bazelLabel", "bazel_label", "label"],
            "eligibility label",
        )?;
        let eligible = object
            .get("eligible")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                format!("eligibility entry {label} must have a boolean eligible field")
            })?;
        if result
            .insert(label.to_owned(), EligibilityEntry { eligible })
            .is_some()
        {
            return Err(format!("duplicate eligibility target label {label}"));
        }
    }
    if result.is_empty() {
        return Err("eligibility contains no target entries".to_owned());
    }
    Ok(result)
}

fn parse_action(
    record: &Value,
    index: usize,
    eligibility: &BTreeMap<String, EligibilityEntry>,
    seen_ids: &mut BTreeSet<String>,
    seen_fingerprints: &mut BTreeSet<String>,
    unlisted_targets: &mut BTreeSet<String>,
) -> Result<Option<ParsedAction>> {
    let Some(payload) = spawn_payload(record) else {
        return Ok(None);
    };
    let record_object = record.as_object();
    let source_id = record_object
        .and_then(|object| value_field(object, &["id", "actionId", "action_id"]))
        .or_else(|| value_field(payload, &["id", "actionId", "action_id"]))
        .map(value_id)
        .transpose()?;
    if let Some(source_id) = &source_id {
        if !seen_ids.insert(source_id.clone()) {
            return Err(format!("duplicate SpawnExec record id {source_id}"));
        }
    }
    let record_name = source_id
        .as_deref()
        .map_or_else(|| format!("record-{}", index + 1), str::to_owned);
    let mnemonic = string_field(payload, &["mnemonic"], "SpawnExec mnemonic")?.to_owned();
    if mnemonic.is_empty() {
        return Err(format!("SpawnExec {record_name} has an empty mnemonic"));
    }
    let target_label = string_field(
        payload,
        &["targetLabel", "target_label"],
        "SpawnExec target label",
    )?
    .to_owned();
    if target_label.is_empty() {
        return Err(format!("SpawnExec {record_name} has an empty target label"));
    }
    validate_action_result(payload, &record_name)?;
    let mut inputs = parse_artifacts(
        payload,
        &["inputs"],
        &format!("SpawnExec {record_name} inputs"),
    )?;
    let mut outputs = parse_artifacts(
        payload,
        &["actualOutputs", "actual_outputs", "outputs"],
        &format!("SpawnExec {record_name} outputs"),
    )?;
    inputs.sort_by(artifact_order_raw);
    outputs.sort_by(artifact_order_raw);
    let eligible = if let Some(entry) = eligibility.get(&target_label) {
        entry.eligible
    } else {
        unlisted_targets.insert(target_label.clone());
        let remotable = bool_field(payload, &["remotable"], false, &record_name)?;
        let remote_cacheable = bool_field(
            payload,
            &["remoteCacheable", "remote_cacheable"],
            false,
            &record_name,
        )?;
        remotable || remote_cacheable
    };
    let execution_class = classify_action(payload, eligible, &record_name)?;
    if execution_class.is_remote() && !eligible {
        return Err(format!(
            "SpawnExec {record_name} is classified as {execution_class:?} but target {target_label} is ineligible"
        ));
    }
    let fingerprint =
        serde_json::to_string(&(&mnemonic, &target_label, execution_class, &inputs, &outputs))
            .map_err(|error| format!("serialize SpawnExec {record_name} fingerprint: {error}"))?;
    if !seen_fingerprints.insert(fingerprint) {
        return Err(format!(
            "duplicate SpawnExec record {record_name} has the same action payload"
        ));
    }
    let fingerprint =
        serde_json::to_vec(&(&mnemonic, &target_label, execution_class, &inputs, &outputs))
            .map_err(|error| format!("serialize SpawnExec {record_name} identity: {error}"))?;
    let id = format!(
        "action-{}",
        digest_bytes(&fingerprint).trim_start_matches("sha256:")
    );
    Ok(Some(ParsedAction {
        id,
        mnemonic,
        target_label,
        execution_class,
        inputs,
        outputs,
    }))
}

fn validate_action_result(payload: &Map<String, Value>, id: &str) -> Result<()> {
    if let Some(status) = value_field(payload, &["status"]).and_then(Value::as_str) {
        if !status.is_empty() {
            return Err(format!("SpawnExec {id} failed with status {status}"));
        }
    }
    if let Some(exit_code) = value_field(payload, &["exitCode", "exit_code"]) {
        let exit_code = match exit_code {
            Value::Number(number) => number
                .as_i64()
                .ok_or_else(|| format!("SpawnExec {id} exitCode must be an integer"))?,
            Value::String(string) => string
                .parse::<i64>()
                .map_err(|_| format!("SpawnExec {id} exitCode must be an integer"))?,
            _ => return Err(format!("SpawnExec {id} exitCode must be an integer")),
        };
        if exit_code != 0 {
            return Err(format!("SpawnExec {id} failed with exit code {exit_code}"));
        }
    }
    Ok(())
}

fn spawn_payload(value: &Value) -> Option<&Map<String, Value>> {
    let object = value.as_object()?;
    if let Some(payload) = object
        .get("spawnExec")
        .or_else(|| object.get("spawn_exec"))
        .or_else(|| object.get("SpawnExec"))
        .or_else(|| object.get("spawn"))
        .and_then(Value::as_object)
    {
        return Some(payload);
    }
    let record_type = object
        .get("type")
        .and_then(Value::as_str)
        .map(|value| value.to_ascii_lowercase());
    if record_type
        .as_deref()
        .is_some_and(|value| value != "spawnexec" && value != "spawn")
    {
        return None;
    }
    if object.contains_key("mnemonic")
        || object.contains_key("targetLabel")
        || object.contains_key("target_label")
    {
        Some(object)
    } else {
        None
    }
}

fn classify_action(
    payload: &Map<String, Value>,
    eligible: bool,
    id: &str,
) -> Result<ExecutionClass> {
    if let Some(explicit) = value_field(payload, &["executionClass", "execution_class"]) {
        let value = explicit
            .as_str()
            .ok_or_else(|| format!("SpawnExec {id} executionClass must be a string"))?;
        return match value {
            "rbe" => Ok(ExecutionClass::Rbe),
            "remote-cache-only" | "remote_cache_only" => Ok(ExecutionClass::RemoteCacheOnly),
            "fully-local" | "fully_local" | "local" => Ok(ExecutionClass::FullyLocal),
            _ => Err(format!(
                "SpawnExec {id} has unknown execution class {value}"
            )),
        };
    }
    let remotable = bool_field(payload, &["remotable"], false, id)?;
    let remote_cacheable =
        bool_field(payload, &["remoteCacheable", "remote_cacheable"], false, id)?;
    if remotable {
        return Ok(ExecutionClass::Rbe);
    }
    if remote_cacheable {
        return Ok(ExecutionClass::RemoteCacheOnly);
    }
    let has_class_signal = value_field(
        payload,
        &[
            "remotable",
            "cacheable",
            "remoteCacheable",
            "remote_cacheable",
            "runner",
        ],
    )
    .is_some();
    if !eligible || has_class_signal {
        return Ok(ExecutionClass::FullyLocal);
    }
    Err(format!(
        "SpawnExec {id} has no execution-class signal; provide remotable/cacheable/remoteCacheable or executionClass"
    ))
}

fn parse_artifacts(
    payload: &Map<String, Value>,
    keys: &[&str],
    context: &str,
) -> Result<Vec<Artifact>> {
    let Some(value) = value_field(payload, keys) else {
        return Ok(Vec::new());
    };
    let array = value
        .as_array()
        .ok_or_else(|| format!("{context} must be an array"))?;
    array
        .iter()
        .enumerate()
        .map(|(index, value)| parse_artifact(value, &format!("{context}[{index}]")))
        .collect()
}

fn parse_artifact(value: &Value, context: &str) -> Result<Artifact> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} must be an object with path and digest"))?;
    let path = string_field(object, &["path"], &format!("{context}.path"))?.to_owned();
    if path.is_empty() {
        return Err(format!("{context}.path must not be empty"));
    }
    let symlink_target =
        value_field(object, &["symlinkTargetPath", "symlink_target_path"]).and_then(Value::as_str);
    let artifact_size = optional_u64_field(object, &["sizeBytes", "size_bytes", "size"], context)?;
    let Some(digest_value) = object.get("digest") else {
        if symlink_target.is_none() && artifact_size.unwrap_or(0) != 0 {
            return Err(format!(
                "{context}.digest is missing for a non-empty artifact"
            ));
        }
        return Ok(Artifact {
            path,
            digest: synthetic_digest(symlink_target),
            size_bytes: artifact_size.unwrap_or(0),
        });
    };
    let (digest, digest_size) = match digest_value {
        Value::String(hash) if !hash.is_empty() => (hash.clone(), None),
        Value::Object(digest) => {
            let hash = value_field(digest, &["hash"])
                .and_then(Value::as_str)
                .unwrap_or_default();
            let size = optional_u64_field(
                digest,
                &["sizeBytes", "size_bytes", "size"],
                &format!("{context}.digest"),
            )?;
            if let (Some(artifact_size), Some(digest_size)) = (artifact_size, size) {
                if artifact_size != digest_size {
                    return Err(format!(
                        "{context} has conflicting digest size {digest_size} and artifact size {artifact_size}"
                    ));
                }
            }
            if hash.is_empty() {
                if size == Some(0) || symlink_target.is_some() {
                    return Ok(Artifact {
                        path,
                        digest: synthetic_digest(symlink_target),
                        size_bytes: artifact_size.or(size).unwrap_or(0),
                    });
                }
                return Err(format!("{context}.digest.hash must not be empty"));
            }
            (hash.to_owned(), size)
        }
        _ => return Err(format!("{context}.digest must contain a hash")),
    };
    let size = artifact_size
        .or(digest_size)
        .ok_or_else(|| format!("{context} is missing a digest size"))?;
    if let Some(digest_size) = digest_size {
        if digest_size != size {
            return Err(format!(
                "{context} has conflicting digest size {digest_size} and artifact size {size}"
            ));
        }
    }
    Ok(Artifact {
        path,
        digest,
        size_bytes: size,
    })
}

fn synthetic_digest(symlink_target: Option<&str>) -> String {
    match symlink_target {
        Some(target) => format!("synthetic:symlink:{}", digest_bytes(target.as_bytes())),
        None => "synthetic:empty-file".to_owned(),
    }
}

fn record_artifact(info: &mut ArtifactInfo, artifact: &Artifact) -> Result<()> {
    if let Some(size) = info.size_bytes {
        if size != artifact.size_bytes {
            return Err(format!(
                "digest {} has conflicting sizes {size} and {}",
                artifact.digest, artifact.size_bytes
            ));
        }
    } else {
        info.size_bytes = Some(artifact.size_bytes);
    }
    info.paths.insert(artifact.path.clone());
    Ok(())
}

fn scope_summary(
    indices: &[usize],
    actions: &[ParsedAction],
    artifacts: &BTreeMap<String, ArtifactInfo>,
) -> Result<ScopeSummary> {
    let mut gross_input_bytes = 0;
    let mut unique_input_digests = BTreeSet::new();
    let mut output_bytes = 0;
    let mut input_artifact_count = 0;
    let mut output_artifact_count = 0;
    let mut targets = BTreeSet::new();
    for &index in indices {
        let action = &actions[index];
        gross_input_bytes = checked_add(
            gross_input_bytes,
            action_input_bytes(action)?,
            "gross input bytes",
        )?;
        output_bytes = checked_add(output_bytes, action_output_bytes(action)?, "output bytes")?;
        input_artifact_count = checked_add(
            input_artifact_count,
            action.inputs.len() as u64,
            "input artifact count",
        )?;
        output_artifact_count = checked_add(
            output_artifact_count,
            action.outputs.len() as u64,
            "output artifact count",
        )?;
        unique_input_digests.extend(action.inputs.iter().map(|artifact| artifact.digest.clone()));
        targets.insert(action.target_label.clone());
    }
    let mut fan_out_artifacts = 0;
    let mut max_fan_out = 0;
    for digest in &unique_input_digests {
        let fan_out = selected_consumer_count(
            artifacts
                .get(digest)
                .ok_or_else(|| format!("missing artifact registry entry for digest {digest}"))?,
            indices,
        );
        if fan_out > 1 {
            fan_out_artifacts += 1;
        }
        max_fan_out = max_fan_out.max(fan_out);
    }
    let unique_input_bytes = unique_input_digests
        .iter()
        .try_fold(0_u64, |total, digest| {
            let info = artifacts
                .get(digest)
                .ok_or_else(|| format!("missing artifact registry entry for digest {digest}"))?;
            checked_add(
                total,
                info.size_bytes
                    .ok_or_else(|| format!("artifact {digest} has no size"))?,
                "unique input bytes",
            )
        })?;
    let (largest_inputs, largest_outputs, highest_exposure) =
        scope_artifacts(indices, actions, artifacts);
    let targets = targets.into_iter().collect::<Vec<_>>();
    Ok(ScopeSummary {
        action_count: indices.len() as u64,
        gross_input_bytes,
        unique_input_bytes,
        output_bytes,
        input_artifact_count,
        unique_input_artifact_count: unique_input_digests.len() as u64,
        output_artifact_count,
        fan_out_artifacts,
        max_fan_out,
        fan_out_ratio: ratio(gross_input_bytes, unique_input_bytes)?,
        targets: targets.clone(),
        responsible_targets: targets,
        largest_inputs,
        largest_outputs,
        highest_exposure,
    })
}

fn mnemonic_summaries(
    actions: &[ParsedAction],
    artifacts: &BTreeMap<String, ArtifactInfo>,
) -> Result<Vec<MnemonicSummary>> {
    let mut grouped = BTreeMap::<String, Vec<usize>>::new();
    for (index, action) in actions.iter().enumerate() {
        grouped
            .entry(action.mnemonic.clone())
            .or_default()
            .push(index);
    }
    grouped
        .into_iter()
        .map(|(mnemonic, indices)| {
            let summary = scope_summary(&indices, actions, artifacts)?;
            let mut by_class = BTreeMap::new();
            for class in [
                ExecutionClass::Rbe,
                ExecutionClass::RemoteCacheOnly,
                ExecutionClass::FullyLocal,
            ] {
                let class_indices = indices
                    .iter()
                    .copied()
                    .filter(|index| actions[*index].execution_class == class)
                    .collect::<Vec<_>>();
                by_class.insert(class, scope_summary(&class_indices, actions, artifacts)?);
            }
            Ok(MnemonicSummary {
                mnemonic,
                action_count: summary.action_count,
                gross_input_bytes: summary.gross_input_bytes,
                unique_input_bytes: summary.unique_input_bytes,
                output_bytes: summary.output_bytes,
                input_artifact_count: summary.input_artifact_count,
                unique_input_artifact_count: summary.unique_input_artifact_count,
                output_artifact_count: summary.output_artifact_count,
                fan_out_artifacts: summary.fan_out_artifacts,
                max_fan_out: summary.max_fan_out,
                fan_out_ratio: summary.fan_out_ratio,
                targets: summary.targets.clone(),
                responsible_targets: summary.responsible_targets.clone(),
                largest_inputs: summary.largest_inputs,
                largest_outputs: summary.largest_outputs,
                highest_exposure: summary.highest_exposure,
                by_class,
            })
        })
        .collect()
}

fn action_summary(
    action: &ParsedAction,
    artifacts: &BTreeMap<String, ArtifactInfo>,
) -> Result<ActionSummary> {
    let unique_input_digests = action
        .inputs
        .iter()
        .map(|artifact| artifact.digest.clone())
        .collect::<BTreeSet<_>>();
    let mut fan_out_artifacts = 0;
    let mut max_fan_out = 0;
    for digest in &unique_input_digests {
        let fan_out = artifacts
            .get(digest)
            .ok_or_else(|| format!("missing artifact registry entry for digest {digest}"))?
            .input_consumers
            .len() as u64;
        if fan_out > 1 {
            fan_out_artifacts += 1;
        }
        max_fan_out = max_fan_out.max(fan_out);
    }
    let inputs = action.inputs.iter().map(artifact_ref).collect::<Vec<_>>();
    let outputs = action.outputs.iter().map(artifact_ref).collect::<Vec<_>>();
    let mut largest_inputs = inputs.clone();
    largest_inputs.sort_by(artifact_order);
    largest_inputs.truncate(TOP_ARTIFACT_COUNT);
    let mut largest_outputs = outputs.clone();
    largest_outputs.sort_by(artifact_order);
    largest_outputs.truncate(TOP_ARTIFACT_COUNT);
    let gross_input_bytes = action_input_bytes(action)?;
    let unique_input_bytes = unique_input_digests
        .iter()
        .try_fold(0_u64, |total, digest| {
            let info = artifacts
                .get(digest)
                .ok_or_else(|| format!("missing artifact registry entry for digest {digest}"))?;
            checked_add(
                total,
                info.size_bytes
                    .ok_or_else(|| format!("artifact {digest} has no size"))?,
                "unique action input bytes",
            )
        })?;
    Ok(ActionSummary {
        id: action.id.clone(),
        mnemonic: action.mnemonic.clone(),
        target_label: action.target_label.clone(),
        execution_class: action.execution_class,
        inputs,
        outputs,
        gross_input_bytes,
        unique_input_bytes,
        output_bytes: action_output_bytes(action)?,
        input_artifact_count: action.inputs.len() as u64,
        unique_input_artifact_count: unique_input_digests.len() as u64,
        output_artifact_count: action.outputs.len() as u64,
        fan_out_artifacts,
        max_fan_out,
        fan_out_ratio: ratio(gross_input_bytes, unique_input_bytes)?,
        responsible_targets: vec![action.target_label.clone()],
        largest_inputs,
        largest_outputs,
    })
}

fn scope_artifacts(
    indices: &[usize],
    actions: &[ParsedAction],
    artifacts: &BTreeMap<String, ArtifactInfo>,
) -> (
    Vec<ArtifactExposure>,
    Vec<ArtifactExposure>,
    Vec<ArtifactExposure>,
) {
    let selected = indices.iter().copied().collect::<BTreeSet<_>>();
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    for (digest, info) in artifacts {
        let consumers = info
            .input_consumers
            .intersection(&selected)
            .copied()
            .collect::<BTreeSet<_>>();
        let producers = info
            .output_producers
            .intersection(&selected)
            .copied()
            .collect::<BTreeSet<_>>();
        if consumers.is_empty() && producers.is_empty() {
            continue;
        }
        let exposure = scoped_artifact_exposure(digest, info, actions, &consumers, &producers);
        if !consumers.is_empty() {
            inputs.push(exposure.clone());
        }
        if !producers.is_empty() {
            outputs.push(exposure);
        }
    }
    let mut highest_exposure = inputs.clone();
    inputs.sort_by(exposure_order);
    outputs.sort_by(exposure_order);
    highest_exposure.sort_by(|left, right| {
        right
            .fan_out
            .cmp(&left.fan_out)
            .then_with(|| right.size_bytes.cmp(&left.size_bytes))
            .then_with(|| left.digest.cmp(&right.digest))
    });
    inputs.truncate(TOP_ARTIFACT_COUNT);
    outputs.truncate(TOP_ARTIFACT_COUNT);
    highest_exposure.truncate(TOP_ARTIFACT_COUNT);
    (inputs, outputs, highest_exposure)
}

fn scoped_artifact_exposure(
    digest: &str,
    info: &ArtifactInfo,
    actions: &[ParsedAction],
    consumers: &BTreeSet<usize>,
    producers: &BTreeSet<usize>,
) -> ArtifactExposure {
    let mut targets = BTreeSet::new();
    let mut classes = BTreeSet::new();
    for &index in consumers.iter().chain(producers.iter()) {
        targets.insert(actions[index].target_label.clone());
        classes.insert(actions[index].execution_class);
    }
    let paths = info.paths.iter().cloned().collect::<Vec<_>>();
    ArtifactExposure {
        path: paths.first().cloned().unwrap_or_default(),
        paths,
        digest: digest.to_owned(),
        size_bytes: info.size_bytes.unwrap_or_default(),
        fan_out: consumers.len() as u64,
        producer_count: producers.len() as u64,
        consumer_count: consumers.len() as u64,
        responsible_targets: targets.into_iter().collect(),
        execution_classes: classes.into_iter().collect(),
    }
}

fn largest_artifacts(
    actions: &[ParsedAction],
    artifacts: &BTreeMap<String, ArtifactInfo>,
) -> LargestArtifacts {
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    for (digest, info) in artifacts {
        let exposure = artifact_exposure(digest, info, actions);
        if !info.input_consumers.is_empty() {
            inputs.push(exposure.clone());
        }
        if !info.output_producers.is_empty() {
            outputs.push(exposure);
        }
    }
    let mut highest_exposure = inputs.clone();
    inputs.sort_by(exposure_order);
    outputs.sort_by(exposure_order);
    highest_exposure.sort_by(|left, right| {
        right
            .fan_out
            .cmp(&left.fan_out)
            .then_with(|| right.size_bytes.cmp(&left.size_bytes))
            .then_with(|| left.digest.cmp(&right.digest))
    });
    inputs.truncate(TOP_ARTIFACT_COUNT);
    outputs.truncate(TOP_ARTIFACT_COUNT);
    highest_exposure.truncate(TOP_ARTIFACT_COUNT);
    LargestArtifacts {
        inputs,
        outputs,
        highest_exposure,
    }
}

fn artifact_exposure(
    digest: &str,
    info: &ArtifactInfo,
    actions: &[ParsedAction],
) -> ArtifactExposure {
    let mut targets = BTreeSet::new();
    let mut classes = BTreeSet::new();
    for &index in info
        .input_consumers
        .iter()
        .chain(info.output_producers.iter())
    {
        targets.insert(actions[index].target_label.clone());
        classes.insert(actions[index].execution_class);
    }
    let paths = info.paths.iter().cloned().collect::<Vec<_>>();
    ArtifactExposure {
        path: paths.first().cloned().unwrap_or_default(),
        paths,
        digest: digest.to_owned(),
        size_bytes: info.size_bytes.unwrap_or_default(),
        fan_out: info.input_consumers.len() as u64,
        producer_count: info.output_producers.len() as u64,
        consumer_count: info.input_consumers.len() as u64,
        responsible_targets: targets.into_iter().collect(),
        execution_classes: classes.into_iter().collect(),
    }
}

fn boundary_crossings(
    actions: &[ParsedAction],
    artifacts: &BTreeMap<String, ArtifactInfo>,
) -> Vec<BoundaryCrossing> {
    let mut crossings = Vec::new();
    for (digest, info) in artifacts {
        let Some(size_bytes) = info.size_bytes else {
            continue;
        };
        for (path, producers) in &info.output_producers_by_path {
            let Some(consumers) = info.input_consumers_by_path.get(path) else {
                continue;
            };
            let producer_classes = producers
                .iter()
                .map(|index| actions[*index].execution_class)
                .collect::<BTreeSet<_>>();
            let consumer_classes = consumers
                .iter()
                .map(|index| actions[*index].execution_class)
                .collect::<BTreeSet<_>>();
            for from_class in &producer_classes {
                for to_class in &consumer_classes {
                    if from_class == to_class || from_class.is_remote() == to_class.is_remote() {
                        continue;
                    }
                    let direction = if from_class.is_remote() {
                        "remote-to-local"
                    } else {
                        "local-to-remote"
                    };
                    let producer_actions = producers
                        .iter()
                        .filter(|index| actions[**index].execution_class == *from_class)
                        .map(|index| actions[*index].id.clone())
                        .collect::<Vec<_>>();
                    let consumer_actions = consumers
                        .iter()
                        .filter(|index| actions[**index].execution_class == *to_class)
                        .map(|index| actions[*index].id.clone())
                        .collect::<Vec<_>>();
                    let producer_targets = producers
                        .iter()
                        .filter(|index| actions[**index].execution_class == *from_class)
                        .map(|index| actions[*index].target_label.clone())
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    let consumer_targets = consumers
                        .iter()
                        .filter(|index| actions[**index].execution_class == *to_class)
                        .map(|index| actions[*index].target_label.clone())
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    crossings.push(BoundaryCrossing {
                        direction: direction.to_owned(),
                        from_class: *from_class,
                        to_class: *to_class,
                        path: path.clone(),
                        digest: digest.clone(),
                        size_bytes,
                        producer_actions,
                        consumer_actions,
                        producer_targets,
                        consumer_targets,
                    });
                }
            }
        }
    }
    crossings.sort_by(|left, right| {
        left.direction
            .cmp(&right.direction)
            .then_with(|| right.size_bytes.cmp(&left.size_bytes))
            .then_with(|| left.digest.cmp(&right.digest))
    });
    crossings
}

fn graph_digest(actions: &[ParsedAction]) -> Result<String> {
    let mut normalized = actions
        .iter()
        .map(|action| {
            let mut output_paths = action
                .outputs
                .iter()
                .map(|artifact| artifact.path.clone())
                .collect::<Vec<_>>();
            output_paths.sort();
            serde_json::json!({
                "mnemonic": action.mnemonic,
                "targetLabel": action.target_label,
                "executionClass": action.execution_class,
                "outputPaths": output_paths,
            })
        })
        .collect::<Vec<_>>();
    normalized.sort_by_key(|value| value.to_string());
    let bytes = serde_json::to_vec(&normalized).map_err(|error| error.to_string())?;
    Ok(digest_bytes(&bytes))
}

fn compare_identity(baseline: &Report, optimized: &Report) -> Result<DeltaSummary> {
    if baseline.schema_version != optimized.schema_version {
        return Err(format!(
            "schema version mismatch: baseline {} optimized {}",
            baseline.schema_version, optimized.schema_version
        ));
    }
    if baseline.source.graph_digest != optimized.source.graph_digest {
        return Err(format!(
            "graph digest mismatch: baseline {} optimized {}",
            baseline.source.graph_digest, optimized.source.graph_digest
        ));
    }
    if baseline.source.eligibility_digest != optimized.source.eligibility_digest {
        return Err("eligibility digest mismatch".to_owned());
    }
    for (name, baseline_value, optimized_value) in [
        (
            "configuration",
            &baseline.source.configuration,
            &optimized.source.configuration,
        ),
        (
            "platform",
            &baseline.source.platform,
            &optimized.source.platform,
        ),
        (
            "toolchain",
            &baseline.source.toolchain,
            &optimized.source.toolchain,
        ),
    ] {
        require_identity(baseline_value, name)?;
        require_identity(optimized_value, name)?;
        if baseline_value != optimized_value {
            return Err(format!("{name} mismatch"));
        }
    }
    Ok(DeltaSummary {
        whole_graph: metric_delta(&baseline.whole_graph, &optimized.whole_graph)?,
        rbe: metric_delta(
            baseline
                .classes
                .get(&ExecutionClass::Rbe)
                .ok_or_else(|| "baseline report lacks rbe summary".to_owned())?,
            optimized
                .classes
                .get(&ExecutionClass::Rbe)
                .ok_or_else(|| "optimized report lacks rbe summary".to_owned())?,
        )?,
        remote_cache_only: metric_delta(
            baseline
                .classes
                .get(&ExecutionClass::RemoteCacheOnly)
                .ok_or_else(|| "baseline report lacks remote-cache-only summary".to_owned())?,
            optimized
                .classes
                .get(&ExecutionClass::RemoteCacheOnly)
                .ok_or_else(|| "optimized report lacks remote-cache-only summary".to_owned())?,
        )?,
        fully_local: metric_delta(
            baseline
                .classes
                .get(&ExecutionClass::FullyLocal)
                .ok_or_else(|| "baseline report lacks fully-local summary".to_owned())?,
            optimized
                .classes
                .get(&ExecutionClass::FullyLocal)
                .ok_or_else(|| "optimized report lacks fully-local summary".to_owned())?,
        )?,
    })
}

fn metric_delta(baseline: &ScopeSummary, optimized: &ScopeSummary) -> Result<MetricDelta> {
    Ok(MetricDelta {
        action_count: signed_delta(
            baseline.action_count,
            optimized.action_count,
            "action count",
        )?,
        gross_input_bytes: signed_delta(
            baseline.gross_input_bytes,
            optimized.gross_input_bytes,
            "gross input bytes",
        )?,
        unique_input_bytes: signed_delta(
            baseline.unique_input_bytes,
            optimized.unique_input_bytes,
            "unique input bytes",
        )?,
        output_bytes: signed_delta(
            baseline.output_bytes,
            optimized.output_bytes,
            "output bytes",
        )?,
        fan_out_artifacts: signed_delta(
            baseline.fan_out_artifacts,
            optimized.fan_out_artifacts,
            "fan-out artifacts",
        )?,
        fan_out_ratio: finite(
            optimized.fan_out_ratio - baseline.fan_out_ratio,
            "fan-out ratio delta",
        )?,
    })
}

fn read_report(path: &Path) -> Result<Report> {
    let bytes =
        fs::read(path).map_err(|error| format!("read report {}: {error}", path.display()))?;
    let report: Report = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse report {}: {error}", path.display()))?;
    if report.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "report {} has unsupported schema version {}",
            path.display(),
            report.schema_version
        ));
    }
    Ok(report)
}

fn write_or_print(output: Option<&Path>, value: &Value) -> Result<()> {
    if let Some(output) = output {
        write_json(output, value)?;
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(value).expect("cache-transfer report serializes")
        );
    }
    Ok(())
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create output directory {}: {error}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    fs::write(path, bytes).map_err(|error| format!("write report {}: {error}", path.display()))
}

fn metadata_value(value: &Value, key: &str) -> Option<Value> {
    let object = value.as_object()?;
    object
        .get("metadata")
        .and_then(Value::as_object)
        .and_then(|metadata| value_field(metadata, &[key, &snake_case(key)]))
        .cloned()
        .or_else(|| value_field(object, &[key, &snake_case(key)]).cloned())
}

fn snake_case(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_uppercase() {
                format!("_{}", character.to_ascii_lowercase())
            } else {
                character.to_string()
            }
        })
        .collect()
}

fn value_field<'a>(object: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| object.get(*key))
}

fn string_field<'a>(
    object: &'a Map<String, Value>,
    keys: &[&str],
    context: &str,
) -> Result<&'a str> {
    value_field(object, keys)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{context} is missing or not a string"))
}

fn optional_u64_field(
    object: &Map<String, Value>,
    keys: &[&str],
    context: &str,
) -> Result<Option<u64>> {
    let Some(value) = value_field(object, keys) else {
        return Ok(None);
    };
    match value {
        Value::Number(number) => number
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("{context} size must be an unsigned integer")),
        Value::String(string) => string
            .parse::<u64>()
            .map(Some)
            .map_err(|_| format!("{context} size must be an unsigned integer")),
        _ => Err(format!("{context} size must be an unsigned integer")),
    }
}

fn require_identity(value: &Value, name: &str) -> Result<()> {
    let empty = match value {
        Value::Null => true,
        Value::String(value) => value.trim().is_empty(),
        Value::Array(value) => value.is_empty(),
        Value::Object(value) => value.is_empty(),
        _ => false,
    };
    if empty {
        return Err(format!(
            "{name} identity is missing or empty; provide --{name} or include it in execution-log metadata"
        ));
    }
    Ok(())
}

fn bool_field(
    object: &Map<String, Value>,
    keys: &[&str],
    default: bool,
    context: &str,
) -> Result<bool> {
    let Some(value) = value_field(object, keys) else {
        return Ok(default);
    };
    value
        .as_bool()
        .ok_or_else(|| format!("SpawnExec {context} execution flag must be boolean"))
}

fn value_id(value: &Value) -> Result<String> {
    match value {
        Value::String(value) if !value.is_empty() => Ok(value.clone()),
        Value::Number(value) => Ok(value.to_string()),
        _ => Err("SpawnExec id must be a non-empty string or number".to_owned()),
    }
}

fn artifact_ref(artifact: &Artifact) -> ArtifactRef {
    ArtifactRef {
        path: artifact.path.clone(),
        digest: artifact.digest.clone(),
        size_bytes: artifact.size_bytes,
    }
}

fn artifact_order(left: &ArtifactRef, right: &ArtifactRef) -> std::cmp::Ordering {
    right
        .size_bytes
        .cmp(&left.size_bytes)
        .then_with(|| left.digest.cmp(&right.digest))
        .then_with(|| left.path.cmp(&right.path))
}

fn artifact_order_raw(left: &Artifact, right: &Artifact) -> std::cmp::Ordering {
    right
        .size_bytes
        .cmp(&left.size_bytes)
        .then_with(|| left.digest.cmp(&right.digest))
        .then_with(|| left.path.cmp(&right.path))
}

fn exposure_order(left: &ArtifactExposure, right: &ArtifactExposure) -> std::cmp::Ordering {
    right
        .size_bytes
        .cmp(&left.size_bytes)
        .then_with(|| left.digest.cmp(&right.digest))
        .then_with(|| left.path.cmp(&right.path))
}

fn action_input_bytes(action: &ParsedAction) -> Result<u64> {
    action.inputs.iter().try_fold(0_u64, |total, artifact| {
        checked_add(total, artifact.size_bytes, "action input bytes")
    })
}

fn action_output_bytes(action: &ParsedAction) -> Result<u64> {
    action.outputs.iter().try_fold(0_u64, |total, artifact| {
        checked_add(total, artifact.size_bytes, "action output bytes")
    })
}

fn selected_consumer_count(info: &ArtifactInfo, indices: &[usize]) -> u64 {
    let selected = indices.iter().copied().collect::<BTreeSet<_>>();
    info.input_consumers.intersection(&selected).count() as u64
}

fn checked_add(left: u64, right: u64, context: &str) -> Result<u64> {
    left.checked_add(right)
        .ok_or_else(|| format!("{context} overflow"))
}

fn signed_delta(baseline: u64, optimized: u64, context: &str) -> Result<i64> {
    let baseline = i128::from(baseline);
    let optimized = i128::from(optimized);
    let delta = optimized - baseline;
    i64::try_from(delta).map_err(|_| format!("{context} delta does not fit in signed 64 bits"))
}

fn ratio(gross: u64, unique: u64) -> Result<f64> {
    if unique == 0 {
        return Ok(0.0);
    }
    finite(gross as f64 / unique as f64, "fan-out ratio")
}

fn finite(value: f64, context: &str) -> Result<f64> {
    value
        .is_finite()
        .then_some(value)
        .ok_or_else(|| format!("{context} is not finite"))
}

fn digest_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{digest:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_unique_bytes_has_a_finite_zero_ratio() {
        assert_eq!(ratio(10, 0).expect("zero ratio"), 0.0);
    }

    #[test]
    fn non_finite_ratios_are_rejected() {
        assert!(finite(f64::NAN, "test ratio").is_err());
        assert!(finite(f64::INFINITY, "test ratio").is_err());
    }
}
