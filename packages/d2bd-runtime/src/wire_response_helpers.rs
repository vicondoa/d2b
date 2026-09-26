//! Provider-neutral public mutating-response construction and projection.

use d2b_contracts_control::public_wire::{MutatingVerbOutcome, MutatingVerbResponse};
use serde_json::Value;

use crate::wire::mutating_verb_response;

pub fn broker_failure_response(
    verb: &str,
    summary: String,
    remediation: String,
    target_wave: Option<String>,
) -> Value {
    mutating_verb_response(MutatingVerbResponse {
        verb: verb.to_owned(),
        outcome: MutatingVerbOutcome::BrokerError,
        target_wave,
        summary: Some(summary),
        remediation: Some(remediation),
        api_ready: None,
    })
}

pub fn invalid_request_response(verb: &str, remediation: String) -> Value {
    invalid_request_response_with_summary(verb, String::new(), remediation)
}

pub fn invalid_request_response_with_summary(
    verb: &str,
    summary: String,
    remediation: String,
) -> Value {
    mutating_verb_response(MutatingVerbResponse {
        verb: verb.to_owned(),
        outcome: MutatingVerbOutcome::InvalidRequest,
        target_wave: None,
        summary: if summary.is_empty() {
            None
        } else {
            Some(summary)
        },
        remediation: Some(remediation),
        api_ready: None,
    })
}

pub fn daemon_failure_response(verb: &str, summary: String) -> Value {
    broker_failure_response(
        verb,
        summary,
        "Admin: inspect `journalctl -u d2bd` for the daemon-side diagnostic.".to_owned(),
        None,
    )
}

pub fn applied_response(verb: &str, summary: String) -> Value {
    mutating_verb_response(MutatingVerbResponse {
        verb: verb.to_owned(),
        outcome: MutatingVerbOutcome::Applied,
        target_wave: None,
        summary: Some(summary),
        remediation: None,
        api_ready: None,
    })
}

pub fn append_response_summary(response: &mut Value, suffix: &str) {
    let Some(summary) = response
        .get("summary")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return;
    };
    let combined = format!("{summary}; {suffix}");
    if let Some(object) = response.as_object_mut() {
        object.insert("summary".to_owned(), Value::String(combined));
    }
}

pub fn api_ready_timeout_response(verb: &str, summary: String) -> Value {
    mutating_verb_response(MutatingVerbResponse {
        verb: verb.to_owned(),
        outcome: MutatingVerbOutcome::ApiReadyTimeout,
        target_wave: None,
        summary: Some(summary),
        remediation: None,
        api_ready: Some("timeout".to_owned()),
    })
}

/// Parse the wire `outcome` field into the typed [`MutatingVerbOutcome`]
/// vocabulary. Unknown or malformed outcome strings yield `None`.
pub fn response_outcome_typed(value: &Value) -> Option<MutatingVerbOutcome> {
    value
        .get("outcome")
        .cloned()
        .and_then(|outcome| serde_json::from_value(outcome).ok())
}

/// The wire string for a typed mutating-verb outcome.
fn mutating_verb_outcome_wire(outcome: MutatingVerbOutcome) -> &'static str {
    match outcome {
        MutatingVerbOutcome::DryRunPlanned => "dry-run-planned",
        MutatingVerbOutcome::Applied => "applied",
        MutatingVerbOutcome::ApiReadyTimeout => "api-ready-timeout",
        MutatingVerbOutcome::NotYetImplemented => "not-yet-implemented",
        MutatingVerbOutcome::BrokerError => "broker-error",
        MutatingVerbOutcome::InvalidRequest => "invalid-request",
    }
}

/// The wire `outcome` string, re-derived from the typed outcome vocabulary
/// rather than read verbatim from the JSON.
pub fn response_outcome(value: &Value) -> Option<&str> {
    response_outcome_typed(value).map(mutating_verb_outcome_wire)
}

pub fn response_summary(value: &Value) -> Option<&str> {
    value.get("summary").and_then(Value::as_str)
}

pub fn response_remediation(value: &Value) -> Option<&str> {
    value.get("remediation").and_then(Value::as_str)
}

pub fn response_target_wave(value: &Value) -> Option<String> {
    value
        .get("targetWave")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub fn retarget_mutating_response(value: &Value, verb: &str) -> Value {
    match response_outcome_typed(value) {
        Some(MutatingVerbOutcome::Applied) => {
            applied_response(verb, response_summary(value).unwrap_or_default().to_owned())
        }
        Some(MutatingVerbOutcome::BrokerError) => broker_failure_response(
            verb,
            response_summary(value).unwrap_or_default().to_owned(),
            response_remediation(value).unwrap_or_default().to_owned(),
            response_target_wave(value),
        ),
        Some(MutatingVerbOutcome::ApiReadyTimeout) => {
            let mut retargeted = value.clone();
            if let Some(object) = retargeted.as_object_mut() {
                object.insert("verb".to_owned(), Value::String(verb.to_owned()));
            }
            retargeted
        }
        _ => value.clone(),
    }
}
