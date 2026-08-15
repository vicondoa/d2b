use std::{env, fs, path::PathBuf, process::ExitCode};

use serde_json::{Map, Value, json};

pub const EVIDENCE_ENV: &str = "D2B_BUILDBUDDY_EVIDENCE_FILE";
const PROBE_COMMAND: &str = "xtask buildbuddy-probe";

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
    reject_credential_keys(value)?;
    let input = value
        .as_object()
        .ok_or_else(|| "evidence-root-must-be-object".to_owned())?;
    if input.get("provider").and_then(Value::as_str) != Some("buildbuddy") {
        return Err("evidence-provider-must-be-buildbuddy".to_owned());
    }

    let status = input
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| "evidence-status-missing".to_owned())?;
    if !matches!(status, "unavailable" | "non-qualifying" | "qualified") {
        return Err("evidence-status-invalid".to_owned());
    }

    let mut output = default_evidence();
    let output = output
        .as_object_mut()
        .expect("default BuildBuddy evidence is an object");
    output.insert("status".to_owned(), Value::String(status.to_owned()));
    if let Some(reason) = input.get("reason") {
        if !reason.is_string() {
            return Err("evidence-reason-must-be-string".to_owned());
        }
        output.insert("reason".to_owned(), reason.clone());
    }

    if status == "qualified" {
        validate_qualified(input)?;
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
        ] {
            output.insert(
                field.to_owned(),
                input
                    .get(field)
                    .cloned()
                    .ok_or_else(|| format!("evidence-field-missing:{field}"))?,
            );
        }
        output.insert(
            "invocationId".to_owned(),
            input
                .get("invocationId")
                .cloned()
                .ok_or_else(|| "evidence-field-missing:invocationId".to_owned())?,
        );
    }

    Ok(Value::Object(output.clone()))
}

fn validate_qualified(input: &Map<String, Value>) -> Result<(), String> {
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

fn reject_credential_keys(value: &Value) -> Result<(), String> {
    if let Some(object) = value.as_object() {
        for (key, value) in object {
            let normalized = key
                .chars()
                .filter(|character| character.is_ascii_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>();
            if [
                "apikey",
                "authorization",
                "credential",
                "password",
                "privatekey",
                "secret",
                "token",
            ]
            .contains(&normalized.as_str())
            {
                return Err("evidence-credential-field-rejected".to_owned());
            }
            reject_credential_keys(value)?;
        }
    } else if let Some(array) = value.as_array() {
        for value in array {
            reject_credential_keys(value)?;
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
}
