use crate::types::VmId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Store-verify request. `repair=true` requests the broker's explicit
/// repair path; builds without that path must fail closed instead of
/// returning a success-shaped repair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoreVerifyRequest {
    pub vm_id: VmId,
    #[serde(default)]
    pub repair: bool,
    #[serde(default)]
    pub tracing_span_id: Option<crate::types::TracingSpanId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StoreVerifyStatus {
    Ok,
    Drift,
    Unknown,
    Repaired,
    Failed,
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StoreVerifyUnknownReason {
    MarkerOrManifestMissing,
    MarkerOrManifestUnreadable,
    OlderHostGeneration,
    GenerationIdentityUnavailable,
}

/// Store-verify response. Field names intentionally match the public CLI
/// JSON envelope after serde's camelCase conversion on the private wire;
/// the CLI re-renders the signed snake_case envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoreVerifyResponse {
    pub vm: String,
    pub status: StoreVerifyStatus,
    pub checked: u32,
    pub drifted: u32,
    pub repaired: u32,
    #[serde(default)]
    pub unknown_reason: Option<StoreVerifyUnknownReason>,
    #[serde(default)]
    pub audit_ref: Option<String>,
    #[serde(default)]
    pub remediation: Option<String>,
}
