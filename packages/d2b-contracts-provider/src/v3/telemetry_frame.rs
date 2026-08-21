//! Shared typed telemetry frame admission and signal-specific redaction.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::telemetry_policy::{FORBIDDEN_LABEL_KEYS, FORBIDDEN_LABEL_SUFFIXES, allowed_values};

/// Maximum raw or redacted frame size accepted by shared ingress.
pub const MAX_TELEMETRY_FRAME_BYTES: usize = 64 * 1024;

/// Closed telemetry signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TelemetrySignal {
    Metric,
    Trace,
    Log,
}

/// Exact top-level frame shape shared by emitters and collectors.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryFrame {
    pub signal: TelemetrySignal,
    #[serde(default)]
    pub value: Value,
}

impl core::fmt::Debug for TelemetryFrame {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("TelemetryFrame")
            .field("signal", &self.signal)
            .field("value_kind", &json_kind(&self.value))
            .finish()
    }
}

/// Closed frame-admission failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryFrameError {
    RawOversize,
    Malformed,
    UnknownField,
    NonFiniteNumber,
    DescriptorInvalid,
    LabelInvalid,
    ResourceAttributeInvalid,
    RedactedOversize,
}

impl core::fmt::Display for TelemetryFrameError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::RawOversize => "telemetry-frame-raw-oversize",
            Self::Malformed => "telemetry-frame-malformed",
            Self::UnknownField => "telemetry-frame-unknown-field",
            Self::NonFiniteNumber => "telemetry-frame-non-finite-number",
            Self::DescriptorInvalid => "telemetry-frame-descriptor-invalid",
            Self::LabelInvalid => "telemetry-frame-label-invalid",
            Self::ResourceAttributeInvalid => "telemetry-frame-resource-attribute-invalid",
            Self::RedactedOversize => "telemetry-frame-redacted-oversize",
        })
    }
}

impl std::error::Error for TelemetryFrameError {}

/// Parse one raw frame into the shared typed representation.
pub fn parse_raw_frame(bytes: &[u8]) -> Result<TelemetryFrame, TelemetryFrameError> {
    if bytes.len() > MAX_TELEMETRY_FRAME_BYTES {
        return Err(TelemetryFrameError::RawOversize);
    }
    serde_json::from_slice::<TelemetryFrame>(bytes).map_err(|_| TelemetryFrameError::Malformed)
}

/// Validate a previously parsed shared frame.
pub fn validate_frame(frame: &TelemetryFrame) -> Result<(), TelemetryFrameError> {
    validate_value_shape(frame.signal, &frame.value)
}

/// Parse and validate one raw frame.
pub fn validate_raw_frame(bytes: &[u8]) -> Result<TelemetryFrame, TelemetryFrameError> {
    let frame = parse_raw_frame(bytes)?;
    validate_frame(&frame)?;
    Ok(frame)
}

/// Redact and serialize one previously validated shared frame.
pub fn redact_parsed_frame(mut frame: TelemetryFrame) -> Result<Vec<u8>, TelemetryFrameError> {
    redact_value(
        &mut frame.value,
        None,
        matches!(frame.signal, TelemetrySignal::Trace | TelemetrySignal::Log),
    );
    let encoded = serde_json::to_vec(&frame).map_err(|_| TelemetryFrameError::Malformed)?;
    if encoded.len() > MAX_TELEMETRY_FRAME_BYTES {
        return Err(TelemetryFrameError::RedactedOversize);
    }
    Ok(encoded)
}

/// Parse, validate, redact, and remeasure one complete frame.
pub fn redact_frame(bytes: &[u8]) -> Result<Vec<u8>, TelemetryFrameError> {
    let frame = validate_raw_frame(bytes)?;
    redact_parsed_frame(frame)
}

fn validate_value_shape(signal: TelemetrySignal, value: &Value) -> Result<(), TelemetryFrameError> {
    validate_finite_numbers(value)?;
    let object = value.as_object().ok_or(TelemetryFrameError::Malformed)?;
    let allowed = match signal {
        TelemetrySignal::Metric => &["name", "labels", "value", "resource_attributes"][..],
        TelemetrySignal::Trace | TelemetrySignal::Log => &[
            "event",
            "name",
            "outcome",
            "handler",
            "provider",
            "domain",
            "fields",
            "trace_id",
            "span_id",
            "path",
            "argv",
            "env",
            "socket",
            "pid",
            "peer",
            "credential",
            "secret",
            "handle",
            "message",
            "text",
            "reason",
            "phase",
            "ingress",
            "error_class",
            "op",
            "operation",
            "service",
            "transport",
            "profile",
            "purpose_class",
            "kind",
            "direction",
            "record_class",
            "d2b.zone",
            "vm",
            "vm.name",
            "vm.env",
            "vm.role",
            "host.name",
        ][..],
    };
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(TelemetryFrameError::UnknownField);
    }
    if signal == TelemetrySignal::Metric {
        validate_metric_value(object)?;
    } else {
        validate_observation_value(object)?;
    }
    Ok(())
}

fn validate_observation_value(
    object: &serde_json::Map<String, Value>,
) -> Result<(), TelemetryFrameError> {
    for (key, value) in object {
        if is_sensitive_key(key) {
            if key == "trace_id" || key == "span_id" {
                if !(value.is_null()
                    || value
                        .as_str()
                        .is_some_and(d2b_contracts_resource::v3::resource_schema::is_canonical_digest))
                {
                    return Err(TelemetryFrameError::ResourceAttributeInvalid);
                }
            } else if value.is_null() {
                return Err(TelemetryFrameError::ResourceAttributeInvalid);
            }
            continue;
        }
        match key.as_str() {
            "event" => {
                let value = value
                    .as_str()
                    .ok_or(TelemetryFrameError::DescriptorInvalid)?;
                if !super::telemetry_policy::allowed_values("event")
                    .is_some_and(|values| values.contains(&value))
                {
                    return Err(TelemetryFrameError::DescriptorInvalid);
                }
            }
            "provider" | "handler" | "outcome" | "domain" | "reason" | "phase" | "ingress"
            | "error_class" | "op" | "operation" | "service" | "transport" | "profile"
            | "purpose_class" | "kind" | "direction" | "record_class" => {
                let value = value
                    .as_str()
                    .ok_or(TelemetryFrameError::DescriptorInvalid)?;
                if !super::telemetry_policy::allowed_values(key)
                    .is_some_and(|values| values.contains(&value))
                {
                    return Err(TelemetryFrameError::DescriptorInvalid);
                }
            }
            "name" => {
                let value = value
                    .as_str()
                    .ok_or(TelemetryFrameError::DescriptorInvalid)?;
                if !valid_semantic_name(value) {
                    return Err(TelemetryFrameError::DescriptorInvalid);
                }
            }
            "fields" => validate_fields_object(value)?,
            _ if value.is_string() || value.is_boolean() || value.is_number() => {}
            _ => return Err(TelemetryFrameError::Malformed),
        }
    }
    Ok(())
}

fn validate_fields_object(value: &Value) -> Result<(), TelemetryFrameError> {
    let fields = value.as_object().ok_or(TelemetryFrameError::Malformed)?;
    if fields.len() > 32 {
        return Err(TelemetryFrameError::Malformed);
    }
    for (key, value) in fields {
        if is_sensitive_key(key) {
            if value.is_null() {
                return Err(TelemetryFrameError::ResourceAttributeInvalid);
            }
            continue;
        }
        if !matches!(
            key.as_str(),
            "event"
                | "name"
                | "provider"
                | "handler"
                | "outcome"
                | "domain"
                | "reason"
                | "phase"
                | "ingress"
                | "error_class"
                | "op"
                | "operation"
                | "kind"
                | "direction"
                | "transport"
                | "service"
                | "profile"
                | "purpose_class"
        ) {
            return Err(TelemetryFrameError::UnknownField);
        }
        if value.is_object() || value.is_array() || value.is_null() {
            return Err(TelemetryFrameError::Malformed);
        }
        if key == "name" {
            let value = value
                .as_str()
                .ok_or(TelemetryFrameError::DescriptorInvalid)?;
            if !valid_semantic_name(value) {
                return Err(TelemetryFrameError::DescriptorInvalid);
            }
        } else if let Some(allowed) = super::telemetry_policy::allowed_values(key) {
            let value = value
                .as_str()
                .ok_or(TelemetryFrameError::DescriptorInvalid)?;
            if !allowed.contains(&value) {
                return Err(TelemetryFrameError::DescriptorInvalid);
            }
        }
    }
    Ok(())
}

fn validate_metric_value(
    object: &serde_json::Map<String, Value>,
) -> Result<(), TelemetryFrameError> {
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .ok_or(TelemetryFrameError::DescriptorInvalid)?;
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(TelemetryFrameError::DescriptorInvalid);
    }
    let labels = object
        .get("labels")
        .and_then(Value::as_object)
        .ok_or(TelemetryFrameError::LabelInvalid)?;
    let mut seen = BTreeSet::new();
    for (key, value) in labels {
        if !seen.insert(key)
            || FORBIDDEN_LABEL_KEYS.contains(&key.as_str())
            || FORBIDDEN_LABEL_SUFFIXES
                .iter()
                .any(|suffix| key.ends_with(suffix))
        {
            return Err(TelemetryFrameError::LabelInvalid);
        }
        let allowed = allowed_values(key).ok_or(TelemetryFrameError::LabelInvalid)?;
        let value = value.as_str().ok_or(TelemetryFrameError::LabelInvalid)?;
        if !allowed.contains(&value) {
            return Err(TelemetryFrameError::LabelInvalid);
        }
    }
    if object.get("value").and_then(Value::as_f64).is_none() {
        return Err(TelemetryFrameError::NonFiniteNumber);
    }
    if let Some(attributes) = object.get("resource_attributes") {
        let attributes = attributes
            .as_object()
            .ok_or(TelemetryFrameError::ResourceAttributeInvalid)?;
        for (key, value) in attributes {
            let Some(value) = value.as_str() else {
                return Err(TelemetryFrameError::ResourceAttributeInvalid);
            };
            if !super::telemetry_policy::OTEL_RESOURCE_ATTRIBUTES.contains(&key.as_str())
                || value.is_empty()
                || value.len() > 256
                || value
                    .bytes()
                    .any(|byte| !byte.is_ascii_graphic() || byte == b'/')
                || (matches!(
                    key.as_str(),
                    "d2b.zone"
                        | "d2b.provider"
                        | "d2b.component"
                        | "host.name"
                        | "vm.name"
                        | "vm.env"
                        | "vm.role"
                ) && !is_canonical_digest(value))
            {
                return Err(TelemetryFrameError::ResourceAttributeInvalid);
            }
        }
    }
    Ok(())
}

fn validate_finite_numbers(value: &Value) -> Result<(), TelemetryFrameError> {
    match value {
        Value::Number(number) if number.as_f64().is_none_or(|value| !value.is_finite()) => {
            Err(TelemetryFrameError::NonFiniteNumber)
        }
        Value::Object(object) => object.values().try_for_each(validate_finite_numbers),
        Value::Array(values) => values.iter().try_for_each(validate_finite_numbers),
        _ => Ok(()),
    }
}

fn redact_value(value: &mut Value, key: Option<&str>, redact_sensitive_signal: bool) {
    if key.is_some_and(is_sensitive_key) {
        if matches!(key, Some("trace_id" | "span_id")) && value.is_null() {
            return;
        }
        if let Value::String(text) = value
            && is_canonical_digest(text)
        {
            return;
        }
        let bytes = serde_json::to_vec(value).unwrap_or_default();
        *value = Value::String(d2b_contracts_resource::v3::resource_schema::canonical_digest(
            "d2b:telemetry-redaction:v1",
            &bytes,
        ));
        return;
    }
    match value {
        Value::Object(object) => {
            for (child_key, child) in object {
                redact_value(child, Some(child_key.as_str()), redact_sensitive_signal);
            }
        }
        Value::Array(values) => {
            for child in values {
                redact_value(child, key, redact_sensitive_signal);
            }
        }
        Value::String(text)
            if (redact_sensitive_signal && !is_semantic_key(key.unwrap_or_default()))
                && !is_canonical_digest(text) =>
        {
            *text = d2b_contracts_resource::v3::resource_schema::canonical_digest(
                "d2b:telemetry-redaction:v1",
                text.as_bytes(),
            );
        }
        _ => {}
    }
}

fn is_sensitive_key(key: &str) -> bool {
    matches!(
        key,
        "path"
            | "argv"
            | "env"
            | "socket"
            | "pid"
            | "peer"
            | "credential"
            | "secret"
            | "handle"
            | "message"
            | "text"
            | "d2b.zone"
            | "vm"
            | "vm.name"
            | "vm.env"
            | "vm.role"
            | "host.name"
            | "trace_id"
            | "span_id"
    ) || key.ends_with("_uid")
        || key.ends_with("_name")
        || key.ends_with("_path")
}

fn valid_semantic_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
        && !d2b_contracts_resource::v3::resource_schema::is_canonical_digest(value)
}

fn is_semantic_key(key: &str) -> bool {
    matches!(
        key,
        "signal"
            | "event"
            | "outcome"
            | "handler"
            | "name"
            | "provider"
            | "domain"
            | "service"
            | "transport"
            | "profile"
            | "purpose_class"
            | "kind"
            | "direction"
            | "op"
            | "operation"
            | "record_class"
            | "phase"
            | "ingress"
            | "error_class"
            | "reason"
    )
}

fn is_canonical_digest(value: &str) -> bool {
    d2b_contracts_resource::v3::resource_schema::is_canonical_digest(value)
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_frame_rejects_unknown_keys_and_non_finite_values() {
        let unknown = br#"{"signal":"metric","value":{"name":"d2b_test_total","labels":{},"value":1,"extra":true}}"#;
        assert_eq!(
            validate_raw_frame(unknown),
            Err(TelemetryFrameError::UnknownField)
        );
        let finite =
            br#"{"signal":"metric","value":{"name":"d2b_test_total","labels":{},"value":1}}"#;
        assert!(validate_raw_frame(finite).is_ok());
    }

    #[test]
    fn redaction_preserves_semantic_tokens_and_is_idempotent() {
        let raw = br#"{"signal":"trace","value":{"event":"accepted","outcome":"ok","service":"store","transport":"unix","profile":"NN","kind":"single","direction":"local","record_class":"process-effect","operation":"scan","op":"vmStart","path":"/secret/path"}}"#;
        let first = redact_frame(raw).unwrap();
        let second = redact_frame(&first).unwrap();
        assert_eq!(first, second);
        let rendered = String::from_utf8(first).unwrap();
        assert!(rendered.contains("\"event\":\"accepted\""));
        assert!(rendered.contains("\"outcome\":\"ok\""));
        for semantic in [
            "\"service\":\"store\"",
            "\"transport\":\"unix\"",
            "\"profile\":\"NN\"",
            "\"kind\":\"single\"",
            "\"direction\":\"local\"",
            "\"record_class\":\"process-effect\"",
            "\"operation\":\"scan\"",
            "\"op\":\"vmStart\"",
        ] {
            assert!(
                rendered.contains(semantic),
                "missing {semantic}: {rendered}"
            );
        }
        assert!(!rendered.contains("/secret/path"));
    }

    #[test]
    fn observation_semantics_are_closed_and_sensitive_parents_are_redacted() {
        let bad_name = br#"{"signal":"trace","value":{"name":"/host/private","event":"accepted"}}"#;
        assert_eq!(
            validate_raw_frame(bad_name),
            Err(TelemetryFrameError::DescriptorInvalid)
        );
        let raw =
            br#"{"signal":"trace","value":{"event":"accepted","env":{"TOKEN":"secret"},"pid":42}}"#;
        let redacted = redact_frame(raw).unwrap();
        let rendered = String::from_utf8(redacted).unwrap();
        assert!(!rendered.contains("secret"));
        assert!(!rendered.contains("\"pid\":42"));
    }

    #[test]
    fn sensitive_numeric_and_object_metric_fields_fail_closed() {
        let frame =
            br#"{"signal":"metric","value":{"name":"d2b_test_total","labels":{},"value":1,"resource_attributes":{"d2b.zone":42}}}"#;
        assert_eq!(
            validate_raw_frame(frame),
            Err(TelemetryFrameError::ResourceAttributeInvalid)
        );
    }

    #[test]
    fn parsed_redaction_enforces_the_post_redaction_size_bound() {
        let mut object = serde_json::Map::new();
        for index in 0..1024 {
            object.insert(format!("field_{index}"), Value::String("x".to_owned()));
        }
        let frame = TelemetryFrame {
            signal: TelemetrySignal::Trace,
            value: Value::Object(object),
        };
        assert_eq!(
            redact_parsed_frame(frame),
            Err(TelemetryFrameError::RedactedOversize)
        );
    }
}
