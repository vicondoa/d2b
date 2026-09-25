use schemars::JsonSchema;
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

/// One typed audit export entry. Exactly one of `record` and `error` is
/// populated by the broker.
#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditExportEntry {
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<AuditExportErrorCode>,
}

impl Eq for AuditExportEntry {}

impl core::fmt::Debug for AuditExportEntry {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AuditExportEntry")
            .field("sequence", &self.sequence)
            .field("has_record", &self.record.is_some())
            .field("error", &self.error)
            .finish()
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
