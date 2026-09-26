use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{Metadata, ObjectValidation, Schema, SchemaObject, SubschemaValidation},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Opaque page position for typed broker audit export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditExportCursor {
    pub day: String,
    pub line: u64,
    /// Sequence of the last emitted entry. This keeps page sequence numbers
    /// monotonic across restarts and continuation requests.
    #[serde(default)]
    pub sequence: u64,
}

/// Closed export failure classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AuditExportErrorCode {
    HashBreak,
    RecordInvalid,
    ReadFailed,
}

/// One typed audit export entry. Its payload is exactly one of an audit record
/// or a closed export failure class, so the state the retired `record` /
/// `error` pair left representable - neither populated - cannot be built, and
/// an entry that carries both is refused at decode.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "AuditExportEntryWire", into = "AuditExportEntryWire")]
pub struct AuditExportEntry {
    /// Monotonic page sequence of this entry.
    pub sequence: u64,
    /// The exactly-one payload.
    pub payload: AuditExportEntryPayload,
}

/// The exactly-one payload of one [`AuditExportEntry`].
#[derive(Clone, PartialEq)]
pub enum AuditExportEntryPayload {
    /// One audit record, as the broker emitted it.
    Record {
        /// The emitted record.
        record: Value,
    },
    /// The closed failure class the broker reports in place of a record.
    Error {
        /// The reported class.
        error: AuditExportErrorCode,
    },
}

impl AuditExportEntry {
    /// The record this entry carries, when its payload is a record.
    pub fn record(&self) -> Option<&Value> {
        match &self.payload {
            AuditExportEntryPayload::Record { record } => Some(record),
            AuditExportEntryPayload::Error { .. } => None,
        }
    }

    /// The export failure class this entry carries, when its payload is one.
    pub fn error(&self) -> Option<AuditExportErrorCode> {
        match &self.payload {
            AuditExportEntryPayload::Error { error } => Some(*error),
            AuditExportEntryPayload::Record { .. } => None,
        }
    }
}

/// The entry's wire shape: `sequence` plus exactly one of `record` / `error`.
///
/// The emitted members are the ones the retired optional pair emitted, and the
/// strict decode is the admission gate that refuses a frame carrying neither
/// member or both.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AuditExportEntryWire {
    sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    record: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<AuditExportErrorCode>,
}

impl From<AuditExportEntry> for AuditExportEntryWire {
    fn from(entry: AuditExportEntry) -> Self {
        let (record, error) = match entry.payload {
            AuditExportEntryPayload::Record { record } => (Some(record), None),
            AuditExportEntryPayload::Error { error } => (None, Some(error)),
        };
        Self {
            sequence: entry.sequence,
            record,
            error,
        }
    }
}

impl TryFrom<AuditExportEntryWire> for AuditExportEntry {
    type Error = &'static str;

    fn try_from(wire: AuditExportEntryWire) -> Result<Self, Self::Error> {
        let payload = match (wire.record, wire.error) {
            (Some(record), None) => AuditExportEntryPayload::Record { record },
            (None, Some(error)) => AuditExportEntryPayload::Error { error },
            (None, None) => return Err("audit export entry carries neither record nor error"),
            (Some(_), Some(_)) => {
                return Err("audit export entry carries both record and error");
            }
        };
        Ok(Self {
            sequence: wire.sequence,
            payload,
        })
    }
}

impl JsonSchema for AuditExportEntry {
    fn schema_name() -> String {
        "AuditExportEntry".to_owned()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        // The wire struct's own schema describes the emitted members and its
        // strict admission; the payload enum is the Rust-side guard, so this
        // schema is the wire shape plus the exactly-one constraint the type
        // enforces.
        let mut schema = match AuditExportEntryWire::json_schema(generator) {
            Schema::Object(schema) => schema,
            schema => return schema,
        };
        schema.metadata = Some(Box::new(Metadata {
            description: Some(
                "One typed audit export entry. Exactly one of `record` and `error` is populated \
                 by the broker."
                    .to_owned(),
            ),
            ..Default::default()
        }));
        schema.subschemas = Some(Box::new(SubschemaValidation {
            one_of: Some(vec![required_member("record"), required_member("error")]),
            ..Default::default()
        }));
        Schema::Object(schema)
    }
}

/// One `required: [<member>]` branch of [`AuditExportEntry`]'s exactly-one
/// constraint.
fn required_member(member: &str) -> Schema {
    SchemaObject {
        object: Some(Box::new(ObjectValidation {
            required: [member.to_owned()].into_iter().collect(),
            ..Default::default()
        })),
        ..Default::default()
    }
    .into()
}

impl Eq for AuditExportEntry {}

impl core::fmt::Debug for AuditExportEntry {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AuditExportEntry")
            .field("sequence", &self.sequence)
            .field("has_record", &self.record().is_some())
            .field("error", &self.error())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_entry() -> AuditExportEntry {
        AuditExportEntry {
            sequence: 7,
            payload: AuditExportEntryPayload::Record {
                record: serde_json::json!({ "op": "ApplyNftables" }),
            },
        }
    }

    fn error_entry() -> AuditExportEntry {
        AuditExportEntry {
            sequence: 8,
            payload: AuditExportEntryPayload::Error {
                error: AuditExportErrorCode::ReadFailed,
            },
        }
    }

    #[test]
    fn an_entry_emits_and_admits_exactly_one_payload() {
        let record = record_entry();
        let json = serde_json::to_value(&record).expect("serialize a record entry");
        assert_eq!(
            json,
            serde_json::json!({ "sequence": 7, "record": { "op": "ApplyNftables" } })
        );
        assert_eq!(
            serde_json::from_value::<AuditExportEntry>(json).expect("decode a record entry"),
            record
        );

        let error = error_entry();
        let json = serde_json::to_value(&error).expect("serialize an error entry");
        assert_eq!(
            json,
            serde_json::json!({ "sequence": 8, "error": "read-failed" })
        );
        assert_eq!(
            serde_json::from_value::<AuditExportEntry>(json).expect("decode an error entry"),
            error
        );

        assert_eq!(record.record(), Some(&serde_json::json!({ "op": "ApplyNftables" })));
        assert_eq!(record.error(), None);
        assert_eq!(error.record(), None);
        assert_eq!(error.error(), Some(AuditExportErrorCode::ReadFailed));
        assert!(!format!("{record:?}").contains("ApplyNftables"));
    }

    #[test]
    fn an_entry_payload_is_admitted_exactly_once() {
        for frame in [
            // Neither member: the state the optional pair admitted.
            serde_json::json!({ "sequence": 1 }),
            // Both members: two payloads are not one payload.
            serde_json::json!({
                "sequence": 1,
                "record": { "op": "ApplyNftables" },
                "error": "read-failed",
            }),
            // An unknown member stays refused.
            serde_json::json!({ "sequence": 1, "record": { "op": "ApplyNftables" }, "stray": 1 }),
        ] {
            assert!(
                serde_json::from_value::<AuditExportEntry>(frame.clone()).is_err(),
                "an entry payload is admitted exactly once: {frame}"
            );
        }
    }
}

/// Failure classes for [`validate_audit_page`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditPageError {
    CompleteWithCursor,
    IncompleteWithoutCursor,
}

impl core::fmt::Display for AuditPageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AuditPageError::CompleteWithCursor => {
                f.write_str("complete audit page must omit nextCursor")
            }
            AuditPageError::IncompleteWithoutCursor => {
                f.write_str("incomplete audit page requires nextCursor")
            }
        }
    }
}

impl std::error::Error for AuditPageError {}

pub fn validate_audit_page(
    complete: bool,
    next_cursor: Option<&AuditExportCursor>,
) -> Result<(), AuditPageError> {
    match (complete, next_cursor.is_some()) {
        (true, true) => Err(AuditPageError::CompleteWithCursor),
        (false, false) => Err(AuditPageError::IncompleteWithoutCursor),
        _ => Ok(()),
    }
}
