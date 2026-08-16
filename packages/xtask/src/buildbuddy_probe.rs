use std::{env, fs, path::PathBuf, process::ExitCode};

use serde_json::{Map, Value, json};

pub const EVIDENCE_ENV: &str = "D2B_BUILDBUDDY_EVIDENCE_FILE";
const PROBE_COMMAND: &str = "xtask buildbuddy-probe";
const PROJECTION_ID: &str = "xtask-buildbuddy-probe/v1";

const QUALIFICATION_METRIC_FIELDS: &[&str] = &[
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

const QUALIFICATION_FIELDS: &[&str] = &[
    "authenticated",
    "executionEntitled",
    "cacheReadEvidence",
    "cacheWriteEvidence",
    "readOnlyProbe",
    "secretRedaction",
    "trustedSeed",
    "dispatchEvidence",
];

const PARTIAL_BOOLEAN_FIELDS: &[&str] = &[
    "authenticated",
    "executionEntitled",
    "cacheReadEvidence",
    "cacheWriteEvidence",
    "readOnlyProbe",
    "secretRedaction",
    "trustedSeed",
    "dispatchEvidence",
];

const EVIDENCE_FIELDS: &[&str] = &[
    "schemaVersion",
    "provider",
    "projection",
    "status",
    "reason",
    "source",
    "providerAccountedTransfer",
    "observedAtMillis",
    "sampleId",
    "commit",
    "identity",
    "workerArchitecture",
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
    "samples",
    "probe",
    "authenticated",
    "executionEntitled",
    "cacheReadEvidence",
    "cacheWriteEvidence",
    "readOnlyProbe",
    "uploadsDisabled",
    "transferBytes",
    "qualificationMetrics",
    "workerArchitectures",
    "secretRedaction",
    "trustedSeed",
    "invocationId",
    "dispatchEvidence",
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

const TRANSFER_FIELDS: &[&str] = &["uploaded", "downloaded"];
const U7_FIELDS: &[&str] = &[
    "source",
    "providerAccountedTransfer",
    "observedAtMillis",
    "sampleId",
    "commit",
    "identity",
    "workerArchitecture",
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

pub fn run_cli(args: &[String]) -> ExitCode {
    match run(args) {
        Ok(value) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("BuildBuddy probe result serializes")
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("buildbuddy-probe failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<Value, String> {
    let evidence_path = match args {
        [] => env::var_os(EVIDENCE_ENV).map(PathBuf::from),
        [flag, path] if flag == "--evidence-file" || flag == "-f" => Some(PathBuf::from(path)),
        [flag] if flag == "--help" || flag == "-h" => {
            println!("usage: xtask buildbuddy-probe [--evidence-file <path>]");
            return Ok(default_evidence());
        }
        _ => return Err("usage: xtask buildbuddy-probe [--evidence-file <path>]".to_owned()),
    };

    let Some(path) = evidence_path else {
        return Ok(default_evidence());
    };

    let bytes = fs::read(path).map_err(|_| "evidence-file-unreadable".to_owned())?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "evidence-file-invalid-json".to_owned())?;
    project_evidence(&value)
}

fn default_evidence() -> Value {
    json!({
        "provider": "buildbuddy",
        "status": "unavailable",
        "reason": "provider-evidence-unavailable",
        "probe": {
            "kind": "credential-isolated-command",
            "command": PROBE_COMMAND,
            "input": EVIDENCE_ENV,
            "readOnly": true,
            "fixtureSafe": true,
            "credentialMode": "none"
        },
        "authenticated": false,
        "executionEntitled": false,
        "cacheReadEvidence": false,
        "cacheWriteEvidence": false,
        "readOnlyProbe": false,
        "uploadsDisabled": null,
        "transferBytes": {
            "uploaded": null,
            "downloaded": null
        },
        "qualificationMetrics": {
            "wallTimeMillis": null,
            "actionCacheHits": null,
            "actionCacheMisses": null,
            "casHits": null,
            "casMisses": null,
            "remoteExecutions": null,
            "repositoryTrafficBytes": null,
            "besTrafficBytes": null,
            "retryTrafficBytes": null,
            "localNixMillis": null
        },
        "workerArchitectures": [],
        "secretRedaction": false,
        "trustedSeed": false,
        "invocationId": null,
        "dispatchEvidence": false
    })
}

fn project_evidence(value: &Value) -> Result<Value, String> {
    let sentinels = configured_sentinels();
    project_evidence_with_sentinels(value, &sentinels)
}

fn configured_sentinels() -> Vec<String> {
    env::var("D2B_BUILDBUDDY_SENTINELS")
        .ok()
        .map(|raw| {
            raw.split('|')
                .filter(|sentinel| !sentinel.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn project_evidence_with_sentinels(value: &Value, sentinels: &[String]) -> Result<Value, String> {
    reject_credential_keys(value)?;
    reject_forbidden_auth_fields(value)?;
    reject_sentinels(value, sentinels)?;
    let input = value
        .as_object()
        .ok_or_else(|| "evidence-root-must-be-object".to_owned())?;
    if let Some(samples) = input.get("samples") {
        let samples = samples
            .as_array()
            .ok_or_else(|| "evidence-samples-invalid".to_owned())?;
        if samples.is_empty() {
            return Err("evidence-samples-empty".to_owned());
        }
        let projected = samples
            .iter()
            .map(|sample| project_evidence_with_sentinels(sample, sentinels))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(json!({ "samples": projected }));
    }
    if let Some(version) = input.get("schemaVersion")
        && version.as_u64() != Some(1)
    {
        return Err("evidence-schema-version-invalid".to_owned());
    }
    if input.get("provider").and_then(Value::as_str) != Some("buildbuddy") {
        return Err("evidence-provider-must-be-buildbuddy".to_owned());
    }
    if let Some(projection) = input.get("projection")
        && projection.as_str() != Some(PROJECTION_ID)
    {
        return Err("evidence-projection-invalid".to_owned());
    }

    let status = input
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| "evidence-status-missing".to_owned())?;
    if !matches!(status, "unavailable" | "non-qualifying" | "qualified") {
        return Err("evidence-status-invalid".to_owned());
    }
    validate_evidence_shape(input, status)?;

    let credential_mode = match input.get("probe") {
        Some(probe) => validate_probe(probe)?,
        None if status == "unavailable" => "none".to_owned(),
        None => return Err("evidence-credential-mode-missing".to_owned()),
    };
    let mut output = default_evidence();
    let output = output
        .as_object_mut()
        .expect("default BuildBuddy evidence is an object");
    output.insert("status".to_owned(), Value::String(status.to_owned()));
    output.insert(
        "projection".to_owned(),
        Value::String(PROJECTION_ID.to_owned()),
    );
    output
        .get_mut("probe")
        .and_then(Value::as_object_mut)
        .expect("default BuildBuddy probe is an object")
        .insert(
            "credentialMode".to_owned(),
            Value::String(credential_mode.clone()),
        );
    if let Some(nonce) = input
        .get("probe")
        .and_then(Value::as_object)
        .and_then(|probe| probe.get("nonce"))
    {
        output
            .get_mut("probe")
            .and_then(Value::as_object_mut)
            .expect("default BuildBuddy probe is an object")
            .insert("nonce".to_owned(), nonce.clone());
    }
    if let Some(reason) = input.get("reason") {
        if !reason.is_string() {
            return Err("evidence-reason-must-be-string".to_owned());
        }
        output.insert("reason".to_owned(), reason.clone());
    }

    if status == "qualified" {
        validate_qualified(input, &credential_mode)?;
        validate_provider_evidence(input, &credential_mode)?;
        for field in QUALIFICATION_FIELDS {
            output.insert(
                (*field).to_owned(),
                input
                    .get(*field)
                    .cloned()
                    .ok_or_else(|| format!("evidence-field-missing:{field}"))?,
            );
        }
        for field in [
            "transferBytes",
            "qualificationMetrics",
            "workerArchitectures",
            "uploadsDisabled",
        ] {
            let value = match field {
                "transferBytes" => sanitized_transfer_bytes(input)?,
                "qualificationMetrics" => sanitized_metrics(input)?,
                "workerArchitectures" => sanitized_worker_architectures(input)?,
                "uploadsDisabled" => input
                    .get(field)
                    .cloned()
                    .ok_or_else(|| "evidence-field-missing:uploadsDisabled".to_owned())?,
                _ => unreachable!("all qualified projection fields are handled"),
            };
            output.insert(field.to_owned(), value);
        }
        if input
            .keys()
            .any(|field| U7_FIELDS.contains(&field.as_str()))
        {
            validate_u7_evidence(input)?;
            for field in U7_FIELDS {
                output.insert(
                    (*field).to_owned(),
                    input
                        .get(*field)
                        .cloned()
                        .ok_or_else(|| format!("evidence-u7-field-missing:{field}"))?,
                );
            }
        }
        output.insert(
            "invocationId".to_owned(),
            input
                .get("invocationId")
                .cloned()
                .ok_or_else(|| "evidence-field-missing:invocationId".to_owned())?,
        );
    } else if status == "non-qualifying" {
        validate_provider_evidence(input, &credential_mode)?;
        for field in PARTIAL_BOOLEAN_FIELDS {
            if let Some(value) = input.get(*field) {
                if !value.is_boolean() {
                    return Err(format!("evidence-field-must-be-boolean:{field}"));
                }
                output.insert((*field).to_owned(), value.clone());
            }
        }
        output.insert(
            "uploadsDisabled".to_owned(),
            input
                .get("uploadsDisabled")
                .cloned()
                .ok_or_else(|| "evidence-field-missing:uploadsDisabled".to_owned())?,
        );
        if let Some(worker_architectures) = input.get("workerArchitectures") {
            if worker_architectures
                .as_array()
                .is_none_or(|values| values.iter().any(|value| value.as_str().is_none()))
            {
                return Err("evidence-worker-architectures-invalid".to_owned());
            }
            output.insert(
                "workerArchitectures".to_owned(),
                worker_architectures.clone(),
            );
        }
    }

    Ok(Value::Object(output.clone()))
}

fn reject_sentinels(value: &Value, sentinels: &[String]) -> Result<(), String> {
    if let Some(object) = value.as_object() {
        for value in object.values() {
            reject_sentinels(value, sentinels)?;
        }
    } else if let Some(array) = value.as_array() {
        for value in array {
            reject_sentinels(value, sentinels)?;
        }
    } else if let Some(text) = value.as_str()
        && sentinels.iter().any(|sentinel| text.contains(sentinel))
    {
        return Err("evidence-sentinel-rejected".to_owned());
    }
    Ok(())
}

fn validate_evidence_shape(input: &Map<String, Value>, status: &str) -> Result<(), String> {
    for key in input.keys() {
        if !EVIDENCE_FIELDS.contains(&key.as_str()) {
            return Err(format!("evidence-field-unknown:{key}"));
        }
    }
    if let Some(reason) = input.get("reason") {
        let reason = reason
            .as_str()
            .ok_or_else(|| "evidence-reason-must-be-string".to_owned())?;
        validate_sanitized_token(reason, "reason")?;
    }
    if let Some(probe) = input.get("probe") {
        validate_probe_shape(probe)?;
    }
    for field in PARTIAL_BOOLEAN_FIELDS
        .iter()
        .copied()
        .chain(["uploadsDisabled"])
    {
        if let Some(value) = input.get(field)
            && !value.is_boolean()
        {
            return Err(format!("evidence-field-must-be-boolean:{field}"));
        }
    }
    if let Some(value) = input.get("transferBytes") {
        validate_numeric_object(value, TRANSFER_FIELDS, "transferBytes")?;
    }
    if let Some(value) = input.get("qualificationMetrics") {
        validate_numeric_object(value, QUALIFICATION_METRIC_FIELDS, "qualificationMetrics")?;
    }
    if let Some(value) = input.get("workerArchitectures") {
        let architectures = value
            .as_array()
            .ok_or_else(|| "evidence-worker-architectures-invalid".to_owned())?;
        for architecture in architectures {
            let architecture = architecture
                .as_str()
                .ok_or_else(|| "evidence-worker-architectures-invalid".to_owned())?;
            validate_sanitized_token(architecture, "workerArchitectures")?;
        }
    }
    if let Some(value) = input.get("invocationId") {
        let invocation_id = value
            .as_str()
            .ok_or_else(|| "evidence-invocation-id-invalid".to_owned())?;
        validate_sanitized_token(invocation_id, "invocationId")?;
    }
    if let Some(value) = input.get("source") {
        value
            .as_str()
            .ok_or_else(|| "evidence-source-invalid".to_owned())?;
    }
    if let Some(value) = input.get("providerAccountedTransfer")
        && !value.is_boolean()
    {
        return Err("evidence-provider-accounted-transfer-invalid".to_owned());
    }
    for field in ["freshWorktree", "isolatedServer", "localDiskCacheDisabled"] {
        if let Some(value) = input.get(field)
            && !value.is_boolean()
        {
            return Err(format!("evidence-{field}-invalid"));
        }
    }
    if let Some(value) = input.get("observedAtMillis")
        && value.as_u64().is_none()
    {
        return Err("evidence-observed-at-invalid".to_owned());
    }
    for field in [
        "sampleId",
        "commit",
        "workerArchitecture",
        "workerImage",
        "sampleClass",
        "cacheState",
        "worktreeId",
        "outputRootId",
        "outputBaseId",
        "bazelServerId",
        "localCacheId",
    ] {
        if let Some(value) = input.get(field)
            && value.as_str().is_none()
        {
            return Err(format!("evidence-{field}-invalid"));
        }
    }
    if let Some(identity) = input.get("identity") {
        let identity = identity
            .as_object()
            .ok_or_else(|| "evidence-identity-invalid".to_owned())?;
        for key in identity.keys() {
            if !IDENTITY_FIELDS.contains(&key.as_str()) {
                return Err(format!("evidence-identity-field-unknown:{key}"));
            }
        }
    }
    if status != "unavailable" && input.get("probe").is_none() {
        return Err("evidence-credential-mode-missing".to_owned());
    }
    Ok(())
}

fn validate_probe_shape(value: &Value) -> Result<(), String> {
    let probe = value
        .as_object()
        .ok_or_else(|| "evidence-probe-must-be-object".to_owned())?;
    for key in probe.keys() {
        if !PROBE_FIELDS.contains(&key.as_str()) {
            return Err(format!("evidence-probe-field-unknown:{key}"));
        }
    }
    if probe.get("kind").and_then(Value::as_str) != Some("credential-isolated-command") {
        return Err("evidence-probe-kind-invalid".to_owned());
    }
    if probe.get("command").and_then(Value::as_str) != Some(PROBE_COMMAND) {
        return Err("evidence-probe-command-invalid".to_owned());
    }
    if probe.get("input").and_then(Value::as_str) != Some(EVIDENCE_ENV) {
        return Err("evidence-probe-input-invalid".to_owned());
    }
    if probe.get("readOnly").and_then(Value::as_bool) != Some(true) {
        return Err("evidence-probe-must-be-read-only".to_owned());
    }
    if probe.get("fixtureSafe").and_then(Value::as_bool) != Some(true) {
        return Err("evidence-probe-must-be-fixture-safe".to_owned());
    }
    if let Some(nonce) = probe.get("nonce").and_then(Value::as_str) {
        validate_sanitized_token(nonce, "nonce")?;
    }
    Ok(())
}

fn validate_numeric_object(
    value: &Value,
    allowed_fields: &[&str],
    context: &str,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("evidence-{context}-invalid"))?;
    for key in object.keys() {
        if !allowed_fields.contains(&key.as_str()) {
            return Err(format!("evidence-{context}-field-unknown:{key}"));
        }
    }
    for key in allowed_fields {
        if let Some(value) = object.get(*key)
            && !value.is_null()
            && value.as_u64().is_none()
        {
            return Err(format!("evidence-{context}-field-invalid:{key}"));
        }
    }
    Ok(())
}

fn validate_sanitized_token(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(format!("evidence-{field}-must-be-sanitized"));
    }
    Ok(())
}

fn sanitized_transfer_bytes(input: &Map<String, Value>) -> Result<Value, String> {
    let transfer_bytes = input
        .get("transferBytes")
        .and_then(Value::as_object)
        .ok_or_else(|| "qualified-evidence-transfer-bytes-invalid".to_owned())?;
    Ok(json!({
        "uploaded": transfer_bytes.get("uploaded").cloned().unwrap_or(Value::Null),
        "downloaded": transfer_bytes.get("downloaded").cloned().unwrap_or(Value::Null)
    }))
}

fn sanitized_metrics(input: &Map<String, Value>) -> Result<Value, String> {
    let metrics = input
        .get("qualificationMetrics")
        .and_then(Value::as_object)
        .ok_or_else(|| "qualified-evidence-metrics-invalid".to_owned())?;
    let mut output = Map::new();
    for field in QUALIFICATION_METRIC_FIELDS {
        output.insert(
            (*field).to_owned(),
            metrics.get(*field).cloned().unwrap_or(Value::Null),
        );
    }
    Ok(Value::Object(output))
}

fn sanitized_worker_architectures(input: &Map<String, Value>) -> Result<Value, String> {
    let architectures = input
        .get("workerArchitectures")
        .cloned()
        .ok_or_else(|| "evidence-field-missing:workerArchitectures".to_owned())?;
    if architectures
        .as_array()
        .is_none_or(|values| values.iter().any(|value| value.as_str().is_none()))
    {
        return Err("evidence-worker-architectures-invalid".to_owned());
    }
    Ok(architectures)
}

fn validate_probe(input: &Value) -> Result<String, String> {
    let probe = input
        .as_object()
        .ok_or_else(|| "evidence-probe-must-be-object".to_owned())?;
    let mode = probe
        .get("credentialMode")
        .and_then(Value::as_str)
        .ok_or_else(|| "evidence-credential-mode-missing".to_owned())?;
    match mode {
        "none" | "credential-helper" => Ok(mode.to_owned()),
        "remote_header" | "bes_header" | "remote-header" | "bes-header" => {
            Err("evidence-header-auth-rejected".to_owned())
        }
        _ => Err("evidence-credential-mode-invalid".to_owned()),
    }
}

fn validate_provider_evidence(
    input: &Map<String, Value>,
    credential_mode: &str,
) -> Result<(), String> {
    for field in ["secretRedaction", "uploadsDisabled"] {
        match input.get(field) {
            Some(value) if value.is_boolean() => {}
            Some(_) => return Err(format!("evidence-field-must-be-boolean:{field}")),
            None => return Err(format!("evidence-field-missing:{field}")),
        }
    }
    if credential_mode == "none"
        && [
            "authenticated",
            "executionEntitled",
            "cacheReadEvidence",
            "cacheWriteEvidence",
            "trustedSeed",
            "dispatchEvidence",
        ]
        .iter()
        .any(|field| input.get(*field).and_then(Value::as_bool) == Some(true))
    {
        return Err("evidence-credential-helper-required".to_owned());
    }
    Ok(())
}

fn validate_qualified(input: &Map<String, Value>, credential_mode: &str) -> Result<(), String> {
    if credential_mode != "credential-helper" {
        return Err("qualified-evidence-requires-credential-helper".to_owned());
    }
    for field in QUALIFICATION_FIELDS {
        if input.get(*field).and_then(Value::as_bool) != Some(true) {
            return Err(format!("qualified-evidence-requires:{field}"));
        }
    }

    let transfer_bytes = input
        .get("transferBytes")
        .and_then(Value::as_object)
        .ok_or_else(|| "qualified-evidence-transfer-bytes-invalid".to_owned())?;
    for field in ["uploaded", "downloaded"] {
        if transfer_bytes.get(field).and_then(Value::as_u64).is_none() {
            return Err(format!("qualified-evidence-transfer-bytes-missing:{field}"));
        }
    }

    let metrics = input
        .get("qualificationMetrics")
        .and_then(Value::as_object)
        .ok_or_else(|| "qualified-evidence-metrics-invalid".to_owned())?;
    for field in QUALIFICATION_METRIC_FIELDS {
        if metrics.get(*field).and_then(Value::as_u64).is_none() {
            return Err(format!("qualified-evidence-metric-missing:{field}"));
        }
    }

    if input
        .get("workerArchitectures")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
    {
        return Err("qualified-evidence-worker-architectures-missing".to_owned());
    }
    if input
        .get("invocationId")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err("qualified-evidence-invocation-id-missing".to_owned());
    }
    Ok(())
}

fn validate_u7_evidence(input: &Map<String, Value>) -> Result<(), String> {
    if input.get("source").and_then(Value::as_str) != Some("credential-helper-probe") {
        return Err("evidence-u7-source-invalid".to_owned());
    }
    if input
        .get("providerAccountedTransfer")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err("evidence-u7-provider-accounting-required".to_owned());
    }
    for field in [
        "observedAtMillis",
        "sampleId",
        "commit",
        "identity",
        "workerArchitecture",
        "workerImage",
    ] {
        if !input.contains_key(field) {
            return Err(format!("evidence-u7-field-missing:{field}"));
        }
    }
    if input
        .get("observedAtMillis")
        .and_then(Value::as_u64)
        .is_none()
    {
        return Err("evidence-u7-observed-at-invalid".to_owned());
    }
    for field in ["sampleId", "commit", "workerArchitecture"] {
        let value = input
            .get(field)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("evidence-u7-{field}-invalid"))?;
        validate_sanitized_token(value, field)?;
    }
    for field in [
        "worktreeId",
        "outputRootId",
        "outputBaseId",
        "bazelServerId",
        "localCacheId",
    ] {
        let value = input
            .get(field)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("evidence-u7-field-missing:{field}"))?;
        validate_sanitized_token(value, field)?;
    }
    if input.get("workerImage").and_then(Value::as_str) != Some("d2b-bazel-worker/v1") {
        return Err("evidence-u7-worker-image-invalid".to_owned());
    }
    if input.get("sampleClass").and_then(Value::as_str) != Some("fresh-worktree")
        || input.get("freshWorktree").and_then(Value::as_bool) != Some(true)
        || input.get("isolatedServer").and_then(Value::as_bool) != Some(true)
        || input.get("localDiskCacheDisabled").and_then(Value::as_bool) != Some(true)
        || input.get("cacheState").and_then(Value::as_str) != Some("populated")
    {
        return Err("evidence-u7-sample-provenance-incomplete".to_owned());
    }
    let identity = input
        .get("identity")
        .and_then(Value::as_object)
        .ok_or_else(|| "evidence-u7-identity-invalid".to_owned())?;
    for field in IDENTITY_FIELDS {
        if identity.get(*field).and_then(Value::as_str).is_none() {
            return Err(format!("evidence-u7-identity-field-missing:{field}"));
        }
    }
    Ok(())
}

fn reject_credential_keys(value: &Value) -> Result<(), String> {
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
                    || normalized == "secret"
                    || normalized.contains("token"))
            {
                return Err("evidence-credential-field-rejected".to_owned());
            }
            reject_credential_keys(value)?;
        }
    } else if let Some(array) = value.as_array() {
        for value in array {
            reject_credential_keys(value)?;
        }
    } else if let Some(value) = value.as_str() {
        let normalized = value.to_ascii_lowercase();
        if normalized.starts_with("bearer ")
            || normalized.contains("x-buildbuddy-api-key")
            || normalized.contains("authorization:")
        {
            return Err("evidence-credential-value-rejected".to_owned());
        }
    }
    Ok(())
}

fn reject_forbidden_auth_fields(value: &Value) -> Result<(), String> {
    if let Some(object) = value.as_object() {
        for (key, value) in object {
            let normalized = key
                .chars()
                .filter(|character| character.is_ascii_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>();
            if matches!(
                normalized.as_str(),
                "remoteheader" | "besheader" | "remoteheaders" | "besheaders"
            ) {
                return Err("evidence-header-auth-rejected".to_owned());
            }
            reject_forbidden_auth_fields(value)?;
        }
    } else if let Some(array) = value.as_array() {
        for value in array {
            reject_forbidden_auth_fields(value)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_result_is_unavailable_and_credential_free() {
        let result = default_evidence();
        assert_eq!(result["status"], "unavailable");
        assert_eq!(result["probe"]["kind"], "credential-isolated-command");
        assert!(reject_credential_keys(&result).is_ok());
    }

    #[test]
    fn credential_fields_are_rejected_before_projection() {
        let result = project_evidence(&json!({
            "provider": "buildbuddy",
            "status": "non-qualifying",
            "api_key": "not-read"
        }));
        assert_eq!(result, Err("evidence-credential-field-rejected".to_owned()));
    }

    #[test]
    fn non_qualifying_projection_preserves_sanitized_capabilities() {
        let result = project_evidence(&json!({
            "provider": "buildbuddy",
            "status": "non-qualifying",
            "reason": "provider-transfer-evidence-unavailable",
            "probe": {
                "kind": "credential-isolated-command",
                "command": "xtask buildbuddy-probe",
                "input": "D2B_BUILDBUDDY_EVIDENCE_FILE",
                "readOnly": true,
                "fixtureSafe": true,
                "credentialMode": "credential-helper"
            },
            "authenticated": true,
            "executionEntitled": true,
            "cacheReadEvidence": true,
            "cacheWriteEvidence": true,
            "readOnlyProbe": true,
            "uploadsDisabled": true,
            "secretRedaction": true,
            "trustedSeed": true,
            "dispatchEvidence": true,
            "workerArchitectures": ["x86_64-linux"]
        }))
        .expect("partial provider evidence projects");

        assert_eq!(result["probe"]["credentialMode"], "credential-helper");
        assert_eq!(result["authenticated"], true);
        assert_eq!(result["executionEntitled"], true);
        assert_eq!(result["cacheReadEvidence"], true);
        assert_eq!(result["cacheWriteEvidence"], true);
        assert_eq!(result["uploadsDisabled"], true);
        assert_eq!(result["secretRedaction"], true);
        assert_eq!(result["workerArchitectures"], json!(["x86_64-linux"]));
        assert_eq!(result["transferBytes"]["uploaded"], Value::Null);
        assert_eq!(
            result["qualificationMetrics"]["remoteExecutions"],
            Value::Null
        );
        assert_eq!(result["invocationId"], Value::Null);
    }

    #[test]
    fn provider_evidence_requires_redaction_and_upload_state() {
        for field in ["secretRedaction", "uploadsDisabled"] {
            let mut evidence = json!({
                "provider": "buildbuddy",
                "status": "non-qualifying",
                "probe": {
                    "kind": "credential-isolated-command",
                    "command": "xtask buildbuddy-probe",
                    "input": "D2B_BUILDBUDDY_EVIDENCE_FILE",
                    "readOnly": true,
                    "fixtureSafe": true,
                    "credentialMode": "credential-helper"
                },
                "secretRedaction": true,
                "uploadsDisabled": true
            });
            evidence
                .as_object_mut()
                .expect("evidence object")
                .remove(field);

            assert_eq!(
                project_evidence(&evidence),
                Err(format!("evidence-field-missing:{field}"))
            );
        }
    }

    #[test]
    fn header_auth_modes_and_fields_are_rejected() {
        for mode in ["remote_header", "bes_header", "remote-header", "bes-header"] {
            for status in ["non-qualifying", "qualified"] {
                let result = project_evidence(&json!({
                    "provider": "buildbuddy",
                    "status": status,
                    "probe": {
                        "kind": "credential-isolated-command",
                        "command": "xtask buildbuddy-probe",
                        "input": "D2B_BUILDBUDDY_EVIDENCE_FILE",
                        "readOnly": true,
                        "fixtureSafe": true,
                        "credentialMode": mode
                    },
                    "secretRedaction": false,
                    "uploadsDisabled": false
                }));
                assert_eq!(result, Err("evidence-header-auth-rejected".to_owned()));
            }
        }

        for field in ["remote_header", "bes_header"] {
            let mut evidence = json!({
                "provider": "buildbuddy",
                "status": "non-qualifying",
                "probe": {
                    "kind": "credential-isolated-command",
                    "command": "xtask buildbuddy-probe",
                    "input": "D2B_BUILDBUDDY_EVIDENCE_FILE",
                    "readOnly": true,
                    "fixtureSafe": true,
                    "credentialMode": "credential-helper"
                },
                "secretRedaction": false,
                "uploadsDisabled": false
            });
            evidence
                .as_object_mut()
                .expect("evidence object")
                .insert(field.to_owned(), json!("synthetic-secret"));
            let result = project_evidence(&evidence);
            assert_eq!(result, Err("evidence-header-auth-rejected".to_owned()));
        }
    }

    #[test]
    fn credential_header_values_are_rejected() {
        let result = project_evidence(&json!({
            "provider": "buildbuddy",
            "status": "non-qualifying",
            "probe": {
                "kind": "credential-isolated-command",
                "command": "xtask buildbuddy-probe",
                "input": "D2B_BUILDBUDDY_EVIDENCE_FILE",
                "readOnly": true,
                "fixtureSafe": true,
                "credentialMode": "credential-helper"
            },
            "secretRedaction": false,
            "uploadsDisabled": false,
            "headers": {
                "x-buildbuddy-api-key": ["synthetic-secret"]
            }
        }));
        assert_eq!(result, Err("evidence-credential-field-rejected".to_owned()));
    }

    #[test]
    fn credential_values_are_rejected_even_under_unknown_fields() {
        let result = project_evidence(&json!({
            "provider": "buildbuddy",
            "status": "non-qualifying",
            "probe": {
                "kind": "credential-isolated-command",
                "command": "xtask buildbuddy-probe",
                "input": "D2B_BUILDBUDDY_EVIDENCE_FILE",
                "readOnly": true,
                "fixtureSafe": true,
                "credentialMode": "credential-helper"
            },
            "secretRedaction": false,
            "uploadsDisabled": false,
            "notes": "Bearer synthetic-secret"
        }));
        assert_eq!(result, Err("evidence-credential-value-rejected".to_owned()));
    }

    #[test]
    fn plain_encoded_and_split_sentinels_never_project_into_evidence() {
        let sentinels = [
            "plain-buildbuddy-secret".to_owned(),
            "cGxhaW4tYnVpbGRidWRkeS1zZWNyZXQ=".to_owned(),
            "x-buildbuddy-api-key=split-buildbuddy-secret".to_owned(),
        ];
        for sentinel in &sentinels {
            let evidence = json!({
                "provider": "buildbuddy",
                "status": "non-qualifying",
                "probe": {
                    "kind": "credential-isolated-command",
                    "command": "xtask buildbuddy-probe",
                    "input": "D2B_BUILDBUDDY_EVIDENCE_FILE",
                    "readOnly": true,
                    "fixtureSafe": true,
                    "credentialMode": "credential-helper"
                },
                "secretRedaction": false,
                "uploadsDisabled": false,
                "reason": sentinel
            });
            let result = project_evidence_with_sentinels(&evidence, &sentinels);
            assert!(
                result.is_err(),
                "sentinel must be rejected before evidence projection: {sentinel}"
            );
        }
    }

    #[test]
    fn qualified_evidence_requires_credential_helper_and_upload_state() {
        let mut evidence = json!({
            "provider": "buildbuddy",
            "status": "qualified",
            "probe": {
                "kind": "credential-isolated-command",
                "command": "xtask buildbuddy-probe",
                "input": "D2B_BUILDBUDDY_EVIDENCE_FILE",
                "readOnly": true,
                "fixtureSafe": true,
                "credentialMode": "none"
            },
            "authenticated": true,
            "executionEntitled": true,
            "cacheReadEvidence": true,
            "cacheWriteEvidence": true,
            "readOnlyProbe": true,
            "uploadsDisabled": true,
            "secretRedaction": true,
            "trustedSeed": true,
            "dispatchEvidence": true,
            "transferBytes": {
                "uploaded": 1,
                "downloaded": 1
            },
            "qualificationMetrics": {
                "wallTimeMillis": 1,
                "actionCacheHits": 1,
                "actionCacheMisses": 1,
                "casHits": 1,
                "casMisses": 1,
                "remoteExecutions": 1,
                "repositoryTrafficBytes": 1,
                "besTrafficBytes": 1,
                "retryTrafficBytes": 1,
                "localNixMillis": 1
            },
            "workerArchitectures": ["x86_64-linux"],
            "invocationId": "synthetic-invocation",
        });

        assert_eq!(
            project_evidence(&evidence),
            Err("qualified-evidence-requires-credential-helper".to_owned())
        );

        evidence["probe"]["credentialMode"] = json!("credential-helper");
        evidence
            .as_object_mut()
            .expect("evidence object")
            .remove("uploadsDisabled");
        assert_eq!(
            project_evidence(&evidence),
            Err("evidence-field-missing:uploadsDisabled".to_owned())
        );
    }

    #[test]
    fn qualified_u7_evidence_preserves_provider_binding_fields() {
        let evidence = json!({
            "provider": "buildbuddy",
            "status": "qualified",
            "source": "credential-helper-probe",
            "providerAccountedTransfer": true,
            "observedAtMillis": 1_700_000_000_000u64,
            "sampleId": "sample-1",
            "commit": "533681f1aabbccddee00112233445566778899aa",
            "identity": {
                "commit": "533681f1aabbccddee00112233445566778899aa",
                "targetSetDigest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
                "configurationDigest": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
                "selectedClosureDigest": "sha256:3333333333333333333333333333333333333333333333333333333333333333",
                "namespace": "d2b/qualification/linux-x86_64/rules_rust/worker-v1/minimal/lock-v1",
                "toolchain": "rules_rust",
                "platform": "linux-x86_64"
            },
            "workerArchitecture": "linux-x86_64",
            "workerImage": "d2b-bazel-worker/v1",
            "sampleClass": "fresh-worktree",
            "freshWorktree": true,
            "isolatedServer": true,
            "localDiskCacheDisabled": true,
            "cacheState": "populated",
            "worktreeId": "worktree-1",
            "outputRootId": "output-root-1",
            "outputBaseId": "output-base-1",
            "bazelServerId": "bazel-server-1",
            "localCacheId": "local-cache-1",
            "probe": {
                "kind": "credential-isolated-command",
                "command": "xtask buildbuddy-probe",
                "input": "D2B_BUILDBUDDY_EVIDENCE_FILE",
                "readOnly": true,
                "fixtureSafe": true,
                "credentialMode": "credential-helper",
                "nonce": "nonce-1"
            },
            "authenticated": true,
            "executionEntitled": true,
            "cacheReadEvidence": true,
            "cacheWriteEvidence": true,
            "readOnlyProbe": true,
            "uploadsDisabled": false,
            "secretRedaction": true,
            "trustedSeed": true,
            "dispatchEvidence": true,
            "transferBytes": {"uploaded": 1, "downloaded": 1},
            "qualificationMetrics": {
                "wallTimeMillis": 1,
                "actionCacheHits": 1,
                "actionCacheMisses": 1,
                "casHits": 1,
                "casMisses": 1,
                "remoteExecutions": 1,
                "repositoryTrafficBytes": 1,
                "besTrafficBytes": 1,
                "retryTrafficBytes": 0,
                "localNixMillis": 1
            },
            "workerArchitectures": ["linux-x86_64"],
            "invocationId": "invocation-1"
        });
        let projected = project_evidence(&evidence).expect("project U7 evidence");
        for field in U7_FIELDS {
            assert!(projected.get(*field).is_some(), "missing projected {field}");
        }
        assert_eq!(projected["source"], "credential-helper-probe");
        assert_eq!(projected["providerAccountedTransfer"], true);
        assert_eq!(projected["uploadsDisabled"], false);
        assert_eq!(projected["probe"]["nonce"], "nonce-1");

        let projected_samples = project_evidence(&json!({
            "samples": [evidence.clone(), evidence]
        }))
        .expect("project U7 sample set");
        assert_eq!(
            projected_samples["samples"].as_array().map(Vec::len),
            Some(2)
        );
        assert!(
            projected_samples["samples"][0]
                .get("providerAccountedTransfer")
                .is_some()
        );
    }
}
