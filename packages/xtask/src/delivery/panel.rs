//! Selected-roster panel request and attestation (spec section 12.3, work
//! item `ADR046-delivery-005`).
//!
//! `panel-request` requires the candidate-bound lifecycle selection and writes
//! the selected current roles with the required provider, model, and reasoning
//! effort from [`PANEL_PROVIDER_POLICY`], [`PANEL_MODEL_POLICY`], and
//! [`PANEL_REASONING_EFFORT_POLICY`]. Existing fixed-ten request-record sets
//! on the prior unversioned model/effort pair remain readable for
//! delivery-state compatibility.
//!
//! `panel-attest` validates a directory holding exactly one strict record per
//! role named by the stored request, each bound to the same `candidate_id`,
//! `content_id`, and `snapshot_sha256` as the request, then imports the
//! accepted records into the candidate directory so [`seal`](super::seal)
//! reads them from candidate-addressed state rather than from an
//! operator-supplied path.
//!
//! `signoff` is true if and only if `recommendations` is empty, and unanimous
//! signoff across the request's exact roster is the only passing state. A
//! finding requires a content change, which creates a new snapshot and
//! invalidates every prior record for the wave, so there is deliberately no
//! override, no force flag, and no partial-pass verdict.
//!
//! A history-only rebase is the one thing that does not invalidate a panel.
//! Spec section 12.6 preserves the review because the reviewed content is
//! provably unchanged, and [`stored_request`] implements that by matching a
//! stored request on content identity rather than on the full digest triple.
//! Validator evidence takes the opposite rule; [`seal`](super::seal) explains
//! the asymmetry.
//!
//! Provider, model, and reasoning-effort fields are spec-defined record data
//! and exist only inside the external delivery-state directory. Section 12.5
//! keeps them out of Git, a pull-request body, and a release archive; that is
//! structurally enforced here because every write goes through
//! [`CandidateDir`], which refuses any destination outside the external state
//! root, and because no record content is ever rendered to stdout.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result,
    command::{CliOptions, WaveCommand, WorkflowOutput},
    model::{
        CandidateDigests, CandidateId, CandidateMaterial, ContentId,
        PANEL_ATTESTATION_ARTIFACT_KIND, PANEL_CURRENT_ROLES, PANEL_LEGACY_MODEL_POLICY,
        PANEL_LEGACY_REASONING_EFFORT_POLICY, PANEL_MODEL_POLICY, PANEL_PROVIDER_POLICY,
        PANEL_REASONING_EFFORT_POLICY, PANEL_REQUEST_ARTIFACT_KIND, PANEL_ROLES, PanelRole,
        PanelSelectionV1, SNAPSHOT_ARTIFACT_KIND, SnapshotSha256, ensure_schema, sha256_bytes,
        validate_bounded_string, validate_identifier, validate_program_wave, validate_sha256,
    },
    storage::{
        CandidateDir, MAX_JSON_BYTES, PANEL_DIR, PANEL_REQUEST_FILE, SNAPSHOT_FILE, StateRoot,
    },
};

/// Upper bound on findings carried by one record. A record is a verdict, not a
/// transcript; anything larger is a malformed artifact rather than a review.
const MAX_RECOMMENDATIONS: usize = 64;
const MAX_PANEL_RECORD_SET_BYTES: usize = MAX_JSON_BYTES;

/// Every panel record file is named after the role that produced it, so a
/// mislabeled or duplicated role is refused by name as well as by content.
pub fn record_file_name(role: PanelRole) -> String {
    format!("{}.json", role.as_str())
}

/// Reader view of the immutable candidate snapshot.
///
/// The snapshot writer is work item `ADR046-delivery-002`, so this view is
/// deliberately tolerant of fields it does not know: it reads the digests and
/// the material, and re-derives the digests from the material to prove the two
/// agree. Everything downstream binds to the derived value.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SnapshotView {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub content_id: ContentId,
    pub candidate_id: CandidateId,
    pub snapshot_sha256: SnapshotSha256,
    pub material: CandidateMaterial,
}

impl SnapshotView {
    pub fn program(&self) -> &str {
        &self.material.program
    }

    pub fn wave(&self) -> &str {
        &self.material.wave
    }

    pub fn digests(&self) -> CandidateDigests {
        CandidateDigests {
            content_id: self.content_id.clone(),
            candidate_id: self.candidate_id.clone(),
            snapshot_sha256: self.snapshot_sha256.clone(),
        }
    }

    /// The candidate's content identity, excluding commit history.
    ///
    /// `content_id` and `candidate_id` are digests over content-only material,
    /// so equality of this pair is itself the byte-identical content proof a
    /// history-only rebase needs. `snapshot_sha256` is deliberately not part
    /// of it; that value covers the base and head object IDs and is what
    /// detects the rebase.
    pub fn content_identity(&self) -> (&CandidateId, &ContentId) {
        (&self.candidate_id, &self.content_id)
    }

    /// Rejects a snapshot whose recorded digests do not re-derive from its own
    /// material, so a hand-edited candidate address cannot be laundered into
    /// the panel or seal lanes.
    pub fn validate(&self, candidate: &CandidateDir) -> Result<()> {
        ensure_artifact_kind(&self.artifact_kind, SNAPSHOT_ARTIFACT_KIND, "snapshot")?;
        ensure_schema(self.schema_version, "snapshot")?;
        let derived = self.material.digests()?;
        if derived != self.digests() {
            return Err(DeliveryError::new(
                "snapshot digests do not match the snapshot's own material; the candidate \
                 snapshot is not self-consistent",
            ));
        }
        candidate.validate_artifact_address(self.wave(), &self.candidate_id, "snapshot")?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PanelFormat {
    Legacy,
    Current,
}

impl PanelFormat {
    fn panel_format_version(self) -> Option<u32> {
        match self {
            Self::Legacy => None,
            Self::Current => Some(1),
        }
    }
}

/// The candidate-bound panel request written by `panel-request`.
///
/// `panel_format_version` is optional in this in-memory view only so the
/// unchanged legacy artifacts can be represented. Reads select one strict
/// wire DTO first; [`PanelFormat`] is never inferred after a permissive
/// fallback.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PanelRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel_format_version: Option<u32>,
    pub artifact_kind: String,
    pub schema_version: u32,
    pub program: String,
    pub wave: String,
    pub candidate_id: CandidateId,
    pub content_id: ContentId,
    pub snapshot_sha256: SnapshotSha256,
    pub provider: String,
    pub model_version: String,
    pub reasoning_effort: String,
    pub roles: Vec<PanelRole>,
    pub record_artifact_kind: String,
    pub record_schema_version: u32,
    pub record_files: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CurrentPanelRequest {
    panel_format_version: u32,
    artifact_kind: String,
    schema_version: u32,
    program: String,
    wave: String,
    candidate_id: CandidateId,
    content_id: ContentId,
    snapshot_sha256: SnapshotSha256,
    provider: String,
    model_version: String,
    reasoning_effort: String,
    roles: Vec<PanelRole>,
    record_artifact_kind: String,
    record_schema_version: u32,
    record_files: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyPanelRequest {
    artifact_kind: String,
    schema_version: u32,
    program: String,
    wave: String,
    candidate_id: CandidateId,
    content_id: ContentId,
    snapshot_sha256: SnapshotSha256,
    provider: String,
    model_version: String,
    reasoning_effort: String,
    roles: Vec<PanelRole>,
    record_artifact_kind: String,
    record_schema_version: u32,
    record_files: Vec<String>,
}

impl From<CurrentPanelRequest> for PanelRequest {
    fn from(value: CurrentPanelRequest) -> Self {
        Self {
            panel_format_version: Some(value.panel_format_version),
            artifact_kind: value.artifact_kind,
            schema_version: value.schema_version,
            program: value.program,
            wave: value.wave,
            candidate_id: value.candidate_id,
            content_id: value.content_id,
            snapshot_sha256: value.snapshot_sha256,
            provider: value.provider,
            model_version: value.model_version,
            reasoning_effort: value.reasoning_effort,
            roles: value.roles,
            record_artifact_kind: value.record_artifact_kind,
            record_schema_version: value.record_schema_version,
            record_files: value.record_files,
        }
    }
}

impl From<LegacyPanelRequest> for PanelRequest {
    fn from(value: LegacyPanelRequest) -> Self {
        Self {
            panel_format_version: None,
            artifact_kind: value.artifact_kind,
            schema_version: value.schema_version,
            program: value.program,
            wave: value.wave,
            candidate_id: value.candidate_id,
            content_id: value.content_id,
            snapshot_sha256: value.snapshot_sha256,
            provider: value.provider,
            model_version: value.model_version,
            reasoning_effort: value.reasoning_effort,
            roles: value.roles,
            record_artifact_kind: value.record_artifact_kind,
            record_schema_version: value.record_schema_version,
            record_files: value.record_files,
        }
    }
}

impl<'de> Deserialize<'de> for PanelRequest {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        decode_panel_request(value).map_err(serde::de::Error::custom)
    }
}

impl PanelRequest {
    #[cfg(test)]
    pub fn for_snapshot(snapshot: &SnapshotView) -> Self {
        Self::for_snapshot_with_roles(snapshot, &PANEL_CURRENT_ROLES, PanelFormat::Current)
    }

    #[cfg(test)]
    fn legacy_for_snapshot(snapshot: &SnapshotView) -> Self {
        Self::for_snapshot_with_roles(snapshot, &PANEL_ROLES, PanelFormat::Legacy)
    }

    fn for_snapshot_with_roles(
        snapshot: &SnapshotView,
        roles: &[PanelRole],
        format: PanelFormat,
    ) -> Self {
        let (model_version, reasoning_effort) = match format {
            PanelFormat::Legacy => (
                PANEL_LEGACY_MODEL_POLICY.to_owned(),
                PANEL_LEGACY_REASONING_EFFORT_POLICY.to_owned(),
            ),
            PanelFormat::Current => (
                PANEL_MODEL_POLICY.to_owned(),
                PANEL_REASONING_EFFORT_POLICY.to_owned(),
            ),
        };
        Self {
            panel_format_version: format.panel_format_version(),
            artifact_kind: PANEL_REQUEST_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            program: snapshot.program().to_owned(),
            wave: snapshot.wave().to_owned(),
            candidate_id: snapshot.candidate_id.clone(),
            content_id: snapshot.content_id.clone(),
            snapshot_sha256: snapshot.snapshot_sha256.clone(),
            provider: PANEL_PROVIDER_POLICY.to_owned(),
            model_version,
            reasoning_effort,
            roles: roles.to_vec(),
            record_artifact_kind: PANEL_ATTESTATION_ARTIFACT_KIND.to_owned(),
            record_schema_version: DELIVERY_SCHEMA_VERSION,
            record_files: roles.iter().copied().map(record_file_name).collect(),
        }
    }

    pub(crate) fn format(&self) -> Result<PanelFormat> {
        match self.panel_format_version {
            None => Ok(PanelFormat::Legacy),
            Some(1) => Ok(PanelFormat::Current),
            Some(version) => Err(DeliveryError::new(format!(
                "panel request has unknown panel_format_version {version}"
            ))),
        }
    }

    pub fn digests(&self) -> CandidateDigests {
        CandidateDigests {
            content_id: self.content_id.clone(),
            candidate_id: self.candidate_id.clone(),
            snapshot_sha256: self.snapshot_sha256.clone(),
        }
    }

    /// The content identity this request was issued against.
    pub fn content_identity(&self) -> (&CandidateId, &ContentId) {
        (&self.candidate_id, &self.content_id)
    }

    /// Rejects a request that no longer names the selected roster or a
    /// supported binding. Current requests use the version-1 selected-roster
    /// format; legacy requests remain exact fixed-ten compatibility artifacts.
    pub fn validate(&self) -> Result<()> {
        let format = self.format()?;
        ensure_artifact_kind(
            &self.artifact_kind,
            PANEL_REQUEST_ARTIFACT_KIND,
            "panel request",
        )?;
        ensure_schema(self.schema_version, "panel request")?;
        ensure_artifact_kind(
            &self.record_artifact_kind,
            PANEL_ATTESTATION_ARTIFACT_KIND,
            "panel record",
        )?;
        ensure_schema(self.record_schema_version, "panel record")?;
        validate_program_wave(&self.program, &self.wave)?;
        ensure_panel_binding(
            format,
            &self.provider,
            &self.model_version,
            &self.reasoning_effort,
        )?;
        validate_roster_for_format(&self.roles, format, "panel request")?;
        let expected = self
            .roles
            .iter()
            .copied()
            .map(record_file_name)
            .collect::<Vec<_>>();
        if self.record_files != expected {
            return Err(DeliveryError::new(
                "panel request record_files must exactly follow its ordered roster",
            ));
        }
        Ok(())
    }
}

/// One role's strict panel record, exactly as spec section 12.3 shapes it.
///
/// The wire discriminator is omitted by legacy records and required by
/// current records. Both strict wire DTOs are selected by the bounded
/// version-first deserializer below.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PanelRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel_format_version: Option<u32>,
    pub artifact_kind: String,
    pub schema_version: u32,
    pub role: PanelRole,
    pub candidate_id: CandidateId,
    pub content_id: ContentId,
    pub snapshot_sha256: SnapshotSha256,
    pub model_version: String,
    pub provider: String,
    pub reasoning_effort: String,
    pub run_id: String,
    pub receipt_locator: String,
    pub output_sha256: String,
    pub signoff: bool,
    pub recommendations: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CurrentPanelRecord {
    panel_format_version: u32,
    artifact_kind: String,
    schema_version: u32,
    role: PanelRole,
    candidate_id: CandidateId,
    content_id: ContentId,
    snapshot_sha256: SnapshotSha256,
    model_version: String,
    provider: String,
    reasoning_effort: String,
    run_id: String,
    receipt_locator: String,
    output_sha256: String,
    signoff: bool,
    recommendations: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyPanelRecord {
    artifact_kind: String,
    schema_version: u32,
    role: PanelRole,
    candidate_id: CandidateId,
    content_id: ContentId,
    snapshot_sha256: SnapshotSha256,
    model_version: String,
    provider: String,
    reasoning_effort: String,
    run_id: String,
    receipt_locator: String,
    output_sha256: String,
    signoff: bool,
    recommendations: Vec<String>,
}

impl From<CurrentPanelRecord> for PanelRecord {
    fn from(value: CurrentPanelRecord) -> Self {
        Self {
            panel_format_version: Some(value.panel_format_version),
            artifact_kind: value.artifact_kind,
            schema_version: value.schema_version,
            role: value.role,
            candidate_id: value.candidate_id,
            content_id: value.content_id,
            snapshot_sha256: value.snapshot_sha256,
            model_version: value.model_version,
            provider: value.provider,
            reasoning_effort: value.reasoning_effort,
            run_id: value.run_id,
            receipt_locator: value.receipt_locator,
            output_sha256: value.output_sha256,
            signoff: value.signoff,
            recommendations: value.recommendations,
        }
    }
}

impl From<LegacyPanelRecord> for PanelRecord {
    fn from(value: LegacyPanelRecord) -> Self {
        Self {
            panel_format_version: None,
            artifact_kind: value.artifact_kind,
            schema_version: value.schema_version,
            role: value.role,
            candidate_id: value.candidate_id,
            content_id: value.content_id,
            snapshot_sha256: value.snapshot_sha256,
            model_version: value.model_version,
            provider: value.provider,
            reasoning_effort: value.reasoning_effort,
            run_id: value.run_id,
            receipt_locator: value.receipt_locator,
            output_sha256: value.output_sha256,
            signoff: value.signoff,
            recommendations: value.recommendations,
        }
    }
}

impl<'de> Deserialize<'de> for PanelRecord {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        decode_panel_record(value).map_err(serde::de::Error::custom)
    }
}

impl PanelRecord {
    /// Validates one record against the request it answers.
    fn validate(&self, request: &PanelRequest) -> Result<()> {
        if self.format()? != request.format()? {
            return Err(DeliveryError::new(
                "panel record and panel request use mixed panel format families",
            ));
        }
        let role = self.role.as_str();
        ensure_artifact_kind(
            &self.artifact_kind,
            PANEL_ATTESTATION_ARTIFACT_KIND,
            "panel record",
        )?;
        ensure_schema(self.schema_version, "panel record")?;
        ensure_panel_binding(
            self.format()?,
            &self.provider,
            &self.model_version,
            &self.reasoning_effort,
        )
        .map_err(|error| DeliveryError::new(format!("panel record {role}: {error}")))?;
        if self.provider != request.provider
            || self.model_version != request.model_version
            || self.reasoning_effort != request.reasoning_effort
        {
            return Err(DeliveryError::new(format!(
                "panel record {role} binding must exactly match the panel request binding"
            )));
        }
        if self.candidate_id != request.candidate_id
            || self.content_id != request.content_id
            || self.snapshot_sha256 != request.snapshot_sha256
        {
            return Err(DeliveryError::new(format!(
                "panel record {role} is bound to a different candidate than the panel request; \
                 a content change invalidates every prior record and requires a new snapshot"
            )));
        }
        if !request.roles.contains(&self.role) {
            return Err(DeliveryError::new(format!(
                "panel record {role} is outside the panel request roster"
            )));
        }
        validate_identifier(&self.run_id, "panel record run identifier")?;
        validate_receipt_locator(&self.receipt_locator, &self.provider)?;
        validate_sha256(&self.output_sha256, "panel record output digest")?;
        if self.recommendations.len() > MAX_RECOMMENDATIONS {
            return Err(DeliveryError::new(format!(
                "panel record {role} carries more than {MAX_RECOMMENDATIONS} recommendations"
            )));
        }
        for recommendation in &self.recommendations {
            validate_bounded_string(recommendation, "panel recommendation")?;
        }
        if self.signoff != self.recommendations.is_empty() {
            return Err(DeliveryError::new(format!(
                "panel record {role} is inconsistent: signoff is true if and only if \
                 recommendations is empty"
            )));
        }
        Ok(())
    }

    fn format(&self) -> Result<PanelFormat> {
        match self.panel_format_version {
            None => Ok(PanelFormat::Legacy),
            Some(1) => Ok(PanelFormat::Current),
            Some(version) => Err(DeliveryError::new(format!(
                "panel record has unknown panel_format_version {version}"
            ))),
        }
    }
}

/// One accepted record's provenance, as bound into the seal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttestedRecord {
    pub role: PanelRole,
    pub file: String,
    pub sha256: String,
    pub run_id: String,
}

/// The result of validating a complete record set.
///
/// It exists only in a passing state: [`validate_record_set`] returns an error
/// for anything short of unanimous signoff across the request roster, so
/// `unanimous` is always true on a value that exists. It is carried explicitly
/// so the sealed artifact states the property it was sealed on.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PanelAttestation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel_format_version: Option<u32>,
    pub roles: Vec<PanelRole>,
    pub records: Vec<AttestedRecord>,
    pub unanimous: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CurrentPanelAttestation {
    panel_format_version: u32,
    roles: Vec<PanelRole>,
    records: Vec<AttestedRecord>,
    unanimous: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyPanelAttestation {
    roles: Vec<PanelRole>,
    records: Vec<AttestedRecord>,
    unanimous: bool,
}

impl From<CurrentPanelAttestation> for PanelAttestation {
    fn from(value: CurrentPanelAttestation) -> Self {
        Self {
            panel_format_version: Some(value.panel_format_version),
            roles: value.roles,
            records: value.records,
            unanimous: value.unanimous,
        }
    }
}

impl From<LegacyPanelAttestation> for PanelAttestation {
    fn from(value: LegacyPanelAttestation) -> Self {
        Self {
            panel_format_version: None,
            roles: value.roles,
            records: value.records,
            unanimous: value.unanimous,
        }
    }
}

impl<'de> Deserialize<'de> for PanelAttestation {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        decode_panel_attestation(value).map_err(serde::de::Error::custom)
    }
}

impl PanelAttestation {
    /// Re-checks a deserialized attestation, for readers that did not build it
    /// themselves.
    pub fn validate(&self) -> Result<()> {
        let format = self.format()?;
        validate_roster_for_format(&self.roles, format, "panel attestation")?;
        if self.records.len() != self.roles.len() {
            return Err(DeliveryError::new(
                "panel attestation record count must match its ordered roster",
            ));
        }
        for (role, record) in self.roles.iter().zip(&self.records) {
            if record.role != *role || record.file != record_file_name(*role) {
                return Err(DeliveryError::new(
                    "panel attestation records are not in roster order",
                ));
            }
            validate_sha256(&record.sha256, "panel record digest")?;
            validate_identifier(&record.run_id, "panel record run identifier")?;
        }
        if !self.unanimous {
            return Err(DeliveryError::new(
                "panel attestation is not unanimous; every selected seat must sign off",
            ));
        }
        Ok(())
    }

    pub(crate) fn format(&self) -> Result<PanelFormat> {
        match self.panel_format_version {
            None => Ok(PanelFormat::Legacy),
            Some(1) => Ok(PanelFormat::Current),
            Some(version) => Err(DeliveryError::new(format!(
                "panel attestation has unknown panel_format_version {version}"
            ))),
        }
    }
}

/// Probes a bounded JSON value before selecting one strict panel DTO family.
/// Absence is the legacy family; only the integer `1` selects current.
pub(crate) fn probe_panel_format(value: &Value, label: &str) -> Result<PanelFormat> {
    let object = value
        .as_object()
        .ok_or_else(|| DeliveryError::new(format!("{label} must be a JSON object")))?;
    match object.get("panel_format_version") {
        None => Ok(PanelFormat::Legacy),
        Some(Value::Number(number)) if number.as_u64() == Some(1) => Ok(PanelFormat::Current),
        Some(_) => Err(DeliveryError::new(format!(
            "{label} has malformed or unknown panel_format_version"
        ))),
    }
}

fn decode_panel_request(value: Value) -> Result<PanelRequest> {
    match probe_panel_format(&value, "panel request")? {
        PanelFormat::Current => serde_json::from_value::<CurrentPanelRequest>(value)
            .map(Into::into)
            .map_err(|error| DeliveryError::new(format!("invalid current panel request: {error}"))),
        PanelFormat::Legacy => serde_json::from_value::<LegacyPanelRequest>(value)
            .map(Into::into)
            .map_err(|error| DeliveryError::new(format!("invalid legacy panel request: {error}"))),
    }
}

fn decode_panel_record(value: Value) -> Result<PanelRecord> {
    match probe_panel_format(&value, "panel record")? {
        PanelFormat::Current => serde_json::from_value::<CurrentPanelRecord>(value)
            .map(Into::into)
            .map_err(|error| DeliveryError::new(format!("invalid current panel record: {error}"))),
        PanelFormat::Legacy => serde_json::from_value::<LegacyPanelRecord>(value)
            .map(Into::into)
            .map_err(|error| DeliveryError::new(format!("invalid legacy panel record: {error}"))),
    }
}

fn decode_panel_attestation(value: Value) -> Result<PanelAttestation> {
    match probe_panel_format(&value, "panel attestation")? {
        PanelFormat::Current => serde_json::from_value::<CurrentPanelAttestation>(value)
            .map(Into::into)
            .map_err(|error| {
                DeliveryError::new(format!("invalid current panel attestation: {error}"))
            }),
        PanelFormat::Legacy => serde_json::from_value::<LegacyPanelAttestation>(value)
            .map(Into::into)
            .map_err(|error| {
                DeliveryError::new(format!("invalid legacy panel attestation: {error}"))
            }),
    }
}

fn validate_roster_for_format(roles: &[PanelRole], format: PanelFormat, label: &str) -> Result<()> {
    match format {
        PanelFormat::Legacy => {
            if roles != PANEL_ROLES {
                return Err(DeliveryError::new(format!(
                    "{label} must retain the exact historical ten-role roster including rust"
                )));
            }
        }
        PanelFormat::Current => {
            if roles.is_empty() {
                return Err(DeliveryError::new(format!(
                    "{label} current roster must not be empty"
                )));
            }
            let mut seen = BTreeSet::new();
            for role in roles {
                if !role.is_current() {
                    return Err(DeliveryError::new(format!(
                        "{label} current roster contains legacy or unknown seat {}",
                        role.as_str()
                    )));
                }
                if !seen.insert(*role) {
                    return Err(DeliveryError::new(format!(
                        "{label} current roster repeats seat {}",
                        role.as_str()
                    )));
                }
            }
            let canonical = PANEL_CURRENT_ROLES
                .iter()
                .copied()
                .filter(|role| seen.contains(role))
                .collect::<Vec<_>>();
            if roles != canonical {
                return Err(DeliveryError::new(format!(
                    "{label} current roster is not in selection-table order"
                )));
            }
        }
    }
    Ok(())
}

/// One record file as read from disk: its name and its exact bytes.
///
/// Bytes are kept verbatim so the digest bound into the seal is the digest of
/// the file the panel produced, not of a re-serialization.
pub(crate) type RecordFile = (String, Vec<u8>);

/// Validates a complete record set against its request.
///
/// This is the full rejection matrix. Every branch below is a distinct way a
/// record set fails; there is no path that returns a partial pass.
pub fn validate_record_set(
    candidate: &CandidateDir,
    request: &PanelRequest,
    files: &[RecordFile],
) -> Result<PanelAttestation> {
    request.validate()?;
    candidate.validate_artifact_address(
        &request.wave,
        &request.candidate_id,
        "panel request and record set",
    )?;
    if files.len() != request.roles.len() {
        return Err(DeliveryError::new(format!(
            "panel needs exactly {} records, one per role, found {}",
            request.roles.len(),
            files.len()
        )));
    }

    let mut names = BTreeSet::new();
    let mut parsed = Vec::with_capacity(files.len());
    for (name, bytes) in files {
        if !names.insert(name) {
            return Err(DeliveryError::new(format!(
                "panel record file {name:?} is duplicated"
            )));
        }
        let role = request
            .roles
            .iter()
            .copied()
            .find(|role| record_file_name(*role) == *name)
            .ok_or_else(|| {
                DeliveryError::new(format!(
                    "panel record file {name:?} is not named after a role on the request roster"
                ))
            })?;
        let record: PanelRecord = serde_json::from_slice(bytes).map_err(|error| {
            DeliveryError::new(format!(
                "panel record {name:?} is not a strict record: {error}"
            ))
        })?;
        if record.role != role {
            return Err(DeliveryError::new(format!(
                "panel record file {name:?} carries role {:?}",
                record.role.as_str()
            )));
        }
        record.validate(request)?;
        parsed.push((role, name.clone(), record, sha256_bytes(bytes)));
    }

    let parsed_roles = parsed.iter().map(|(role, ..)| *role).collect::<Vec<_>>();
    let mut expected_roles = request.roles.clone();
    expected_roles.sort();
    let mut actual_roles = parsed_roles.clone();
    actual_roles.sort();
    if actual_roles != expected_roles {
        return Err(DeliveryError::new(
            "panel records must cover every role on the request roster exactly once",
        ));
    }

    ensure_distinct(
        parsed
            .iter()
            .map(|(_, _, record, _)| record.run_id.as_str()),
        "run identifier",
    )?;
    ensure_distinct(
        parsed
            .iter()
            .map(|(_, _, record, _)| record.receipt_locator.as_str()),
        "receipt locator",
    )?;
    ensure_distinct(
        parsed
            .iter()
            .map(|(_, _, record, _)| record.output_sha256.as_str()),
        "output digest",
    )?;

    let findings = parsed
        .iter()
        .filter(|(_, _, record, _)| !record.signoff)
        .count();
    if findings > 0 {
        return Err(DeliveryError::new(format!(
            "panel is not unanimous: {findings} of {} selected roles returned findings; the wave \
             takes a content change, a new snapshot, and a fresh panel",
            request.roles.len()
        )));
    }

    let records = request
        .roles
        .iter()
        .map(|role| {
            let (_, file, record, sha256) = parsed
                .iter()
                .find(|(candidate, ..)| candidate == role)
                .expect("every roster role is present");
            AttestedRecord {
                role: *role,
                file: file.clone(),
                sha256: sha256.clone(),
                run_id: record.run_id.clone(),
            }
        })
        .collect();

    Ok(PanelAttestation {
        panel_format_version: request.panel_format_version,
        roles: request.roles.clone(),
        records,
        unanimous: true,
    })
}

/// Reads the accepted records back out of candidate-addressed state.
///
/// [`seal`](super::seal) uses this rather than re-reading the operator's
/// directory, so the seal binds the records the attestation accepted.
pub fn attested_records(
    candidate: &CandidateDir,
    request: &PanelRequest,
) -> Result<PanelAttestation> {
    let names = candidate.list(PANEL_DIR).map_err(|error| {
        DeliveryError::new(format!(
            "no attested panel records for this candidate; run panel-attest first ({error})"
        ))
    })?;
    let mut files = Vec::with_capacity(names.len());
    for name in names {
        let name = name
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| DeliveryError::new("panel record file name is not UTF-8"))?;
        let bytes = candidate.read_bytes(Path::new(PANEL_DIR).join(&name))?;
        if bytes.len() > MAX_JSON_BYTES {
            return Err(DeliveryError::new(format!(
                "panel record exceeds {MAX_JSON_BYTES} bytes"
            )));
        }
        files.push((name, bytes));
    }
    validate_record_set(candidate, request, &files)
}

/// Reads and validates the panel request stored for a candidate.
///
/// The request is matched on content identity, not on the full digest triple.
/// A panel reviews content, and spec section 12.6 lets a history-only rebase
/// preserve that review: `candidate_id` and `content_id` are digests over
/// content-only material, so their equality is the byte-identical proof the
/// reuse rests on. `snapshot_sha256` moves with the base and head object IDs
/// and is deliberately not compared here - comparing it would force a fresh
/// ten-role panel after every rebase, which is exactly what section 12.6
/// exists to avoid.
///
/// Validator evidence takes the opposite rule; see
/// [`seal`](super::seal) for why the two classes are asymmetric.
pub fn stored_request(candidate: &CandidateDir, snapshot: &SnapshotView) -> Result<PanelRequest> {
    let request: PanelRequest = candidate.read_json(PANEL_REQUEST_FILE).map_err(|error| {
        DeliveryError::new(format!(
            "no panel request for this candidate; run panel-request first ({error})"
        ))
    })?;
    request.validate()?;
    candidate.validate_artifact_address(
        &request.wave,
        &request.candidate_id,
        "stored panel request",
    )?;
    if request.content_identity() != snapshot.content_identity() {
        return Err(DeliveryError::new(
            "the stored panel request reviewed different content than this snapshot; a content \
             change requires a new snapshot and a fresh panel",
        ));
    }
    Ok(request)
}

/// Opens the candidate directory named by a snapshot artifact reference.
///
/// The candidate address (wave and candidate id) is derived from the
/// reference itself and the snapshot is read through the candidate's pinned
/// directory descriptor (see [`StateRoot::open_candidate_artifact`]), so no
/// supplied path is read and there is no separate canonicalize-and-compare. A
/// reference that does not resolve to a `<wave>/<candidate>/snapshot.json`
/// inside external delivery state fails closed. That is what keeps every later
/// stage reading candidate-addressed state instead of an arbitrary operator
/// path.
pub fn open_candidate(
    state: &StateRoot,
    snapshot_path: &Path,
) -> Result<(CandidateDir, SnapshotView)> {
    let (candidate, snapshot): (CandidateDir, SnapshotView) =
        state.open_candidate_artifact(snapshot_path, SNAPSHOT_FILE, "candidate snapshot")?;
    snapshot.validate(&candidate)?;
    Ok((candidate, snapshot))
}

/// `cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave panel-request`.
///
/// FR-049 binds the predecessor-merged condition at the panel request as well
/// as at the seal: a wave that started early under FR-048 may implement
/// against an unsealed predecessor, but it must not put a snapshot in front of
/// the panel until every prior-wave item is `Merged`. Enforcing it here rather
/// than only at `seal` is what stops a selected roster from binding to a
/// snapshot that a predecessor finding is still going to invalidate.
pub fn run_request(args: &[String]) -> Result<WorkflowOutput> {
    let mut options = CliOptions::parse(args)?;
    let snapshot_path = options.required_path("--snapshot")?;
    let selection_path = options.required_path("--selection")?;
    let (state, repository_roots) = prepare_state_with_roots(&mut options)?;
    options.finish()?;
    let snapshot_path = state.resolve_artifact_ref(&snapshot_path);
    let (candidate, snapshot) = open_candidate(&state, &snapshot_path)?;
    request_checked(
        &candidate,
        &snapshot,
        &repository_roots,
        Some(&selection_path),
    )
}

/// Applies the FR-049 predecessor-merged gate, then writes the request.
///
/// Split out from [`run_request`] so the gate is reachable from a test: the
/// CLI entrypoint builds its [`StateRoot`] through `StateRoot::prepare`, which
/// refuses a state root inside a Git working tree, and every hermetic fixture
/// lives under the ignored build tree inside this repository.
fn request_checked(
    candidate: &CandidateDir,
    snapshot: &SnapshotView,
    repository_roots: &BTreeMap<String, PathBuf>,
    selection_path: Option<&Path>,
) -> Result<WorkflowOutput> {
    super::work_item_state::require_prior_waves_merged_for_exit(
        &snapshot.material,
        repository_roots,
    )?;
    match selection_path {
        Some(path) => request_with_selection(candidate, snapshot, path),
        None => {
            #[cfg(test)]
            {
                request(candidate, snapshot)
            }
            #[cfg(not(test))]
            {
                Err(DeliveryError::new(
                    "current panel request requires an authoritative lifecycle selection",
                ))
            }
        }
    }
}

/// Writes the candidate-bound current request for internal callers and tests.
///
/// Test-only helper for delivery stages whose subject is not panel selection.
///
/// The production CLI has no path around [`request_with_selection`].
#[cfg(test)]
pub fn request(candidate: &CandidateDir, snapshot: &SnapshotView) -> Result<WorkflowOutput> {
    let request = PanelRequest::for_snapshot(snapshot);
    write_request(candidate, snapshot, request)
}

/// Reads and validates the one lifecycle selection consumed by
/// `panel-request`, then writes its exact ordered roster into the closed
/// version-1 request fields.
pub fn request_with_selection(
    candidate: &CandidateDir,
    snapshot: &SnapshotView,
    selection_path: &Path,
) -> Result<WorkflowOutput> {
    let selection: PanelSelectionV1 = read_json_file(selection_path, "panel selection")?;
    selection.validate_for_snapshot(snapshot.program(), snapshot.wave(), &snapshot.digests())?;
    let request =
        PanelRequest::for_snapshot_with_roles(snapshot, &selection.roster, PanelFormat::Current);
    write_request(candidate, snapshot, request)
}

fn write_request(
    candidate: &CandidateDir,
    snapshot: &SnapshotView,
    request: PanelRequest,
) -> Result<WorkflowOutput> {
    request.validate()?;
    candidate.validate_artifact_address(&request.wave, &request.candidate_id, "panel request")?;
    let bytes = serde_json::to_vec(&request)?;
    publish_candidate_file_no_replace(candidate, PANEL_REQUEST_FILE, &bytes, "panel request")?;
    WorkflowOutput::ok(WaveCommand::PanelRequest)
        .with_digests(&snapshot.digests())
        .with_artifact(candidate, &candidate.panel_request_path())
}

/// `cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave panel-attest`.
pub fn run_attest(args: &[String]) -> Result<WorkflowOutput> {
    let (state, snapshot_path, records_dir) = parse_attest_invocation(args)?;
    let (candidate, snapshot) = open_candidate(&state, &snapshot_path)?;
    attest(&candidate, &snapshot, &records_dir)
}

/// Validates a directory of records and imports the accepted set.
pub fn attest(
    candidate: &CandidateDir,
    snapshot: &SnapshotView,
    records_dir: &Path,
) -> Result<WorkflowOutput> {
    let request = stored_request(candidate, snapshot)?;
    let files = read_record_dir(records_dir, &request.record_files)?;
    let attestation = validate_record_set(candidate, &request, &files)?;

    publish_record_set_no_replace(candidate, &files)?;
    let imported = candidate
        .list(PANEL_DIR)?
        .into_iter()
        .filter_map(|name| name.to_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let expected = attestation
        .records
        .iter()
        .map(|record| record.file.clone())
        .collect::<BTreeSet<_>>();
    if imported.iter().cloned().collect::<BTreeSet<_>>() != expected {
        return Err(DeliveryError::new(
            "the candidate's panel directory holds entries outside the attested record set; \
             remove the stale entries and re-attest",
        ));
    }

    WorkflowOutput::ok(WaveCommand::PanelAttest)
        .with_digests(&snapshot.digests())
        .with_artifact(candidate, &candidate.panel_dir())
}

/// Reads every record file from an operator-supplied directory.
///
/// The directory holds exactly the request's record names and nothing else, so
/// an unnoticed extra file cannot dilute the request's exact roster
/// requirement.
fn read_record_dir(dir: &Path, expected_files: &[String]) -> Result<Vec<RecordFile>> {
    let metadata = fs::metadata(dir).map_err(|error| {
        DeliveryError::environment(format!("cannot read panel record directory: {error}"))
    })?;
    if !metadata.is_dir() {
        return Err(DeliveryError::new("panel record path is not a directory"));
    }
    let mut names = fs::read_dir(dir)
        .map_err(|error| {
            DeliveryError::environment(format!("cannot list panel record directory: {error}"))
        })?
        .map(|entry| {
            let entry = entry.map_err(|error| {
                DeliveryError::environment(format!(
                    "cannot read panel record directory entry: {error}"
                ))
            })?;
            entry
                .file_name()
                .into_string()
                .map_err(|_| DeliveryError::new("panel record file name is not UTF-8"))
        })
        .collect::<Result<Vec<_>>>()?;
    if names.len() > expected_files.len() {
        return Err(DeliveryError::new(format!(
            "panel record directory contains more than the exact {}-entry bound",
            expected_files.len()
        )));
    }
    names.sort();
    let mut expected_names = expected_files.to_vec();
    expected_names.sort();
    if names != expected_names {
        return Err(DeliveryError::new(format!(
            "panel record directory must contain exactly {} requested record entries before \
             record bytes are read",
            expected_files.len()
        )));
    }

    let mut aggregate_bytes = 0usize;
    for name in &names {
        if !name.ends_with(".json") {
            return Err(DeliveryError::new(format!(
                "panel record directory holds {name:?}, which is not a regular record file"
            )));
        }
        let size = fs::metadata(dir.join(name))
            .map_err(|error| {
                DeliveryError::environment(format!("cannot stat panel record: {error}"))
            })?
            .len() as usize;
        aggregate_bytes = aggregate_bytes
            .checked_add(size)
            .ok_or_else(|| DeliveryError::new("panel record aggregate byte count overflowed"))?;
        if aggregate_bytes > MAX_PANEL_RECORD_SET_BYTES {
            return Err(DeliveryError::new(format!(
                "panel record directory exceeds the aggregate \
                 {MAX_PANEL_RECORD_SET_BYTES}-byte bound before record bytes are read"
            )));
        }
    }

    let mut files = Vec::with_capacity(names.len());
    for name in names {
        files.push((
            name.clone(),
            read_file_limited(&dir.join(&name), "panel record")?,
        ));
    }
    files.sort();
    Ok(files)
}

/// Parses the `--snapshot`, `--repo`, and `--state-dir` options every
/// candidate-bound stage shares.
pub(crate) fn parse_snapshot_invocation(args: &[String]) -> Result<(StateRoot, PathBuf)> {
    let mut options = CliOptions::parse(args)?;
    let snapshot_path = options.required_path("--snapshot")?;
    let state = prepare_state(&mut options)?;
    options.finish()?;
    let snapshot_path = state.resolve_artifact_ref(&snapshot_path);
    Ok((state, snapshot_path))
}

fn parse_attest_invocation(args: &[String]) -> Result<(StateRoot, PathBuf, PathBuf)> {
    let mut options = CliOptions::parse(args)?;
    let snapshot_path = options.required_path("--snapshot")?;
    let records_dir = options.required_path("--records")?;
    let state = prepare_state(&mut options)?;
    options.finish()?;
    let snapshot_path = state.resolve_artifact_ref(&snapshot_path);
    Ok((state, snapshot_path, records_dir))
}

/// Resolves the delivery state root from `--state-dir` and the `--repo`
/// checkouts delivery state must stay outside of.
pub(crate) fn prepare_state(options: &mut CliOptions) -> Result<StateRoot> {
    prepare_state_with_roots(options).map(|(state, _)| state)
}

pub(crate) fn prepare_state_with_roots(
    options: &mut CliOptions,
) -> Result<(StateRoot, BTreeMap<String, PathBuf>)> {
    let state_dir = options.optional_path("--state-dir")?;
    let roots = options.repository_roots()?;
    let checkout_paths = roots.values().cloned().collect::<Vec<_>>();
    let state = StateRoot::prepare(&checkout_paths, state_dir.as_deref())?;
    Ok((state, roots))
}

pub(crate) fn ensure_artifact_kind(found: &str, expected: &str, label: &str) -> Result<()> {
    if found != expected {
        return Err(DeliveryError::new(format!(
            "{label} artifact kind must be {expected:?}, found {found:?}"
        )));
    }
    Ok(())
}

fn ensure_panel_binding(
    format: PanelFormat,
    provider: &str,
    model: &str,
    reasoning_effort: &str,
) -> Result<()> {
    let expected = match format {
        PanelFormat::Current => (PANEL_MODEL_POLICY, PANEL_REASONING_EFFORT_POLICY),
        PanelFormat::Legacy => (
            PANEL_LEGACY_MODEL_POLICY,
            PANEL_LEGACY_REASONING_EFFORT_POLICY,
        ),
    };
    if provider != PANEL_PROVIDER_POLICY || model != expected.0 || reasoning_effort != expected.1 {
        let family = match format {
            PanelFormat::Current => "current",
            PanelFormat::Legacy => "legacy",
        };
        return Err(DeliveryError::new(format!(
            "{family} panel binding must exactly match provider {PANEL_PROVIDER_POLICY:?}, \
             model {:?}, and reasoning effort {:?} from its fixed policy",
            expected.0, expected.1
        )));
    }
    Ok(())
}

fn validate_receipt_locator(locator: &str, provider: &str) -> Result<()> {
    validate_bounded_string(locator, "panel receipt locator")?;
    let scheme = format!("{provider}://");
    if !locator.starts_with(&scheme) || locator.chars().any(char::is_control) {
        return Err(DeliveryError::new(
            "panel receipt locator must address the bound provider and hold no control \
             characters",
        ));
    }
    Ok(())
}

fn ensure_distinct<'a>(values: impl Iterator<Item = &'a str>, label: &str) -> Result<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(DeliveryError::new(format!(
                "two panel records share one {label}; each role's provenance must be distinct"
            )));
        }
    }
    Ok(())
}

/// Reads a bounded JSON artifact from an operator-supplied path.
pub(crate) fn read_json_file<T: DeserializeOwned>(path: &Path, label: &str) -> Result<T> {
    let bytes = read_file_limited(path, label)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| DeliveryError::new(format!("invalid {label}: {error}")))
}

fn read_file_limited(path: &Path, label: &str) -> Result<Vec<u8>> {
    let bytes = fs::read(path)
        .map_err(|error| DeliveryError::environment(format!("cannot read {label}: {error}")))?;
    if bytes.len() > MAX_JSON_BYTES {
        return Err(DeliveryError::new(format!(
            "{label} exceeds {MAX_JSON_BYTES} bytes"
        )));
    }
    Ok(bytes)
}

fn publish_record_set_no_replace(candidate: &CandidateDir, files: &[RecordFile]) -> Result<()> {
    let panel_dir = candidate.panel_dir();
    fs::create_dir_all(&panel_dir).map_err(|error| {
        DeliveryError::environment(format!("cannot create candidate panel directory: {error}"))
    })?;
    for (name, bytes) in files {
        write_panel_file_no_replace(&panel_dir.join(name), bytes, "panel record")?;
    }
    Ok(())
}

fn publish_candidate_file_no_replace(
    candidate: &CandidateDir,
    name: &str,
    bytes: &[u8],
    label: &str,
) -> Result<()> {
    write_panel_file_no_replace(&candidate.path().join(name), bytes, label)
}

fn write_panel_file_no_replace(path: &Path, bytes: &[u8], label: &str) -> Result<()> {
    if let Ok(existing) = fs::read(path) {
        if existing != bytes {
            return Err(DeliveryError::new(format!(
                "conflicting {label}; refusing to replace it"
            )));
        }
        return Ok(());
    }

    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = fs::read(path).map_err(|read_error| {
                DeliveryError::environment(format!("cannot read existing {label}: {read_error}"))
            })?;
            if existing != bytes {
                return Err(DeliveryError::new(format!(
                    "conflicting {label}; refusing to replace it"
                )));
            }
            return Ok(());
        }
        Err(error) => {
            return Err(DeliveryError::environment(format!(
                "cannot create {label}: {error}"
            )));
        }
    };
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| DeliveryError::environment(format!("cannot write {label}: {error}")))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::delivery::{
        DeliveryErrorKind,
        model::fixtures,
        storage::{
            SNAPSHOT_FILE,
            tests::{Scratch, assert_no_absolute_path, repo_root},
        },
    };
    pub(crate) fn snapshot() -> SnapshotView {
        snapshot_from(fixtures::material())
    }

    /// A snapshot view over a caller-supplied material, so tests can seal a
    /// wave whose expected pull-request set is not the single-slice fixture
    /// default (for example a stacked same-repository chain).
    pub(crate) fn snapshot_from(material: CandidateMaterial) -> SnapshotView {
        let digests = material.digests().expect("digests");
        SnapshotView {
            artifact_kind: SNAPSHOT_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            content_id: digests.content_id,
            candidate_id: digests.candidate_id,
            snapshot_sha256: digests.snapshot_sha256,
            material,
        }
    }

    /// Creates the candidate directory and writes the snapshot the way
    /// `wave snapshot` will.
    pub(crate) fn candidate_with_snapshot(
        scratch: &Scratch,
    ) -> (StateRoot, CandidateDir, SnapshotView) {
        candidate_with_snapshot_from(scratch, fixtures::material())
    }

    /// Like [`candidate_with_snapshot`], but binds a caller-supplied material.
    pub(crate) fn candidate_with_snapshot_from(
        scratch: &Scratch,
        material: CandidateMaterial,
    ) -> (StateRoot, CandidateDir, SnapshotView) {
        let state = StateRoot::for_tests(&scratch.path.join("state")).expect("state root");
        let snapshot = snapshot_from(material);
        let candidate = state
            .candidate(snapshot.wave(), &snapshot.candidate_id)
            .expect("candidate");
        candidate
            .write_json(SNAPSHOT_FILE, &snapshot)
            .expect("write snapshot");
        (state, candidate, snapshot)
    }

    pub(crate) fn record(role: PanelRole, snapshot: &SnapshotView) -> PanelRecord {
        PanelRecord {
            panel_format_version: Some(1),
            artifact_kind: PANEL_ATTESTATION_ARTIFACT_KIND.to_owned(),
            schema_version: DELIVERY_SCHEMA_VERSION,
            role,
            candidate_id: snapshot.candidate_id.clone(),
            content_id: snapshot.content_id.clone(),
            snapshot_sha256: snapshot.snapshot_sha256.clone(),
            model_version: PANEL_MODEL_POLICY.to_owned(),
            provider: PANEL_PROVIDER_POLICY.to_owned(),
            reasoning_effort: PANEL_REASONING_EFFORT_POLICY.to_owned(),
            run_id: format!("run-{}", role.as_str()),
            receipt_locator: format!(
                "{PANEL_PROVIDER_POLICY}://runs/run-{}/{}",
                role.as_str(),
                role.as_str()
            ),
            output_sha256: sha256_bytes(role.as_str().as_bytes()),
            signoff: true,
            recommendations: Vec::new(),
        }
    }

    pub(crate) fn record_files(snapshot: &SnapshotView) -> Vec<RecordFile> {
        record_files_for_roles(snapshot, &PANEL_CURRENT_ROLES)
    }

    fn record_files_for_roles(snapshot: &SnapshotView, roles: &[PanelRole]) -> Vec<RecordFile> {
        roles
            .iter()
            .map(|role| {
                (
                    record_file_name(*role),
                    serde_json::to_vec(&record(*role, snapshot)).expect("record"),
                )
            })
            .collect()
    }

    fn legacy_record_files(snapshot: &SnapshotView) -> Vec<RecordFile> {
        PANEL_ROLES
            .iter()
            .map(|role| {
                let mut record = record(*role, snapshot);
                record.panel_format_version = None;
                (
                    record_file_name(*role),
                    serde_json::to_vec(&record).expect("legacy record"),
                )
            })
            .collect()
    }

    fn current_selection(snapshot: &SnapshotView) -> PanelSelectionV1 {
        PanelSelectionV1 {
            artifact_kind: crate::delivery::model::PANEL_SELECTION_ARTIFACT_KIND.to_owned(),
            schema_version: crate::delivery::model::PANEL_SELECTION_SCHEMA_VERSION,
            lifecycle_id: "test-lifecycle".to_owned(),
            phase: "discovery".to_owned(),
            program: snapshot.program().to_owned(),
            wave: snapshot.wave().to_owned(),
            candidate_id: snapshot.candidate_id.clone(),
            content_id: snapshot.content_id.clone(),
            snapshot_sha256: snapshot.snapshot_sha256.clone(),
            selection_table_version: crate::delivery::model::PANEL_SELECTION_TABLE_VERSION,
            candidate_class: "code".to_owned(),
            classification_inputs: serde_json::json!({
                "changed_paths": ["src/panel.txt"],
                "signals": [],
                "candidate_class": "code",
                "ambiguous": false,
            }),
            ambiguity_widened: false,
            profiles: PANEL_CURRENT_ROLES
                .iter()
                .map(|role| (role.as_str().to_owned(), Vec::new()))
                .collect(),
            roster: PANEL_CURRENT_ROLES.to_vec(),
        }
    }

    fn write_current_selection(path: &Path, snapshot: &SnapshotView) {
        let selection = current_selection(snapshot);
        selection
            .validate_for_snapshot(snapshot.program(), snapshot.wave(), &snapshot.digests())
            .expect("selection");
        fs::write(
            path,
            serde_json::to_vec(&selection).expect("selection JSON"),
        )
        .expect("write selection");
    }

    /// Writes a record set into an operator-style directory.
    pub(crate) fn write_record_dir(scratch: &Scratch, files: &[RecordFile]) -> PathBuf {
        let dir = scratch.path.join("records");
        fs::create_dir_all(&dir).expect("records directory");
        for (name, bytes) in files {
            fs::write(dir.join(name), bytes).expect("write record");
        }
        dir
    }

    fn requested(candidate: &CandidateDir, snapshot: &SnapshotView) -> PanelRequest {
        request(candidate, snapshot).expect("panel request");
        stored_request(candidate, snapshot).expect("stored request")
    }

    #[test]
    fn a_refused_panel_read_names_the_label_not_the_path() {
        // Point a bounded read at a directory. The read refuses it, and the
        // diagnostic - which reaches operator stderr and CI logs verbatim -
        // must name the semantic label only, never the absolute path.
        let scratch = Scratch::new("panel-read-redaction");
        let decoy = scratch.path.join("panel-record");
        std::fs::create_dir_all(&decoy).expect("create the decoy directory");
        let error = read_file_limited(&decoy, "panel record")
            .expect_err("a directory must not read as a record file");
        let message = error.message();
        assert_no_absolute_path(message, &[&scratch.path, &decoy]);
        assert!(
            message.contains("panel record"),
            "the diagnostic must name the semantic label: {message}"
        );
    }

    #[test]
    fn record_directory_rejects_aggregate_bound_before_reads() {
        let scratch = Scratch::new("panel-record-aggregate-bounds");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        let request = requested(&candidate, &snapshot);
        let files = record_files(&snapshot);
        let dir = write_record_dir(&scratch, &files);

        for (name, _) in files.iter().take(2) {
            File::options()
                .write(true)
                .open(dir.join(name))
                .expect("open aggregate fixture")
                .set_len((MAX_PANEL_RECORD_SET_BYTES / 2 + 1) as u64)
                .expect("extend aggregate fixture");
        }
        let aggregate_error = read_record_dir(&dir, &request.record_files)
            .expect_err("an oversized aggregate must be rejected before record reads");
        assert!(
            aggregate_error
                .message()
                .contains("aggregate 2097152-byte bound before record bytes are read"),
            "{aggregate_error}"
        );
    }

    #[test]
    fn record_publication_compares_existing_bytes_without_replacement() {
        let scratch = Scratch::new("panel-record-no-replace");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        requested(&candidate, &snapshot);
        let files = record_files(&snapshot);
        let dir = write_record_dir(&scratch, &files);
        let first_name = &files[0].0;

        let conflicting = b"{\"foreign\":true}\n";
        candidate
            .write_bytes(Path::new(PANEL_DIR).join(first_name), conflicting)
            .expect("plant conflicting destination");
        let conflict_error = attest(&candidate, &snapshot, &dir).expect_err("replacement rejected");
        assert!(
            conflict_error.message().contains("refusing to replace"),
            "{conflict_error}"
        );
        assert_eq!(
            candidate
                .read_bytes(Path::new(PANEL_DIR).join(first_name))
                .expect("read preserved destination"),
            conflicting,
            "no-replace publication must preserve the pre-existing bytes"
        );
    }

    #[test]
    fn panel_request_publication_refuses_to_replace_existing_bytes() {
        let scratch = Scratch::new("panel-request-no-replace");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        let conflicting = b"{\"foreign\":true}\n";
        candidate
            .write_bytes(PANEL_REQUEST_FILE, conflicting)
            .expect("plant conflicting panel request");
        let error = request(&candidate, &snapshot).expect_err("request replacement rejected");
        assert!(error.message().contains("refusing to replace"), "{error}");
        assert_eq!(
            candidate
                .read_bytes(PANEL_REQUEST_FILE)
                .expect("read preserved request"),
            conflicting,
        );
    }

    #[test]
    fn a_request_binds_the_candidate_the_roster_and_the_model() {
        let scratch = Scratch::new("panel-request");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);

        let output = request(&candidate, &snapshot).expect("panel request");
        assert_eq!(output.operation.as_str(), "panel-request");
        assert_eq!(
            output.candidate_id.as_deref(),
            Some(snapshot.candidate_id.as_str())
        );
        assert_eq!(
            output.artifact.as_deref(),
            Some(format!("W0/{}/panel-request.json", snapshot.candidate_id.as_str()).as_str()),
            "the artifact must be a state-root-relative reference, not an absolute path"
        );

        let stored = stored_request(&candidate, &snapshot).expect("stored request");
        assert_eq!(stored.roles, PANEL_CURRENT_ROLES.to_vec());
        assert_eq!(stored.panel_format_version, Some(1));
        assert_eq!(stored.provider, PANEL_PROVIDER_POLICY);
        assert_eq!(stored.model_version, PANEL_MODEL_POLICY);
        assert_eq!(stored.reasoning_effort, PANEL_REASONING_EFFORT_POLICY);
        assert_eq!(stored.digests(), snapshot.digests());
    }

    #[test]
    fn a_request_stores_exactly_the_validated_variable_selection_roster() {
        let scratch = Scratch::new("panel-request-selection");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        let mut selection = current_selection(&snapshot);
        selection.roster.truncate(10);
        let selected_roles = selection.roster.clone();
        selection.profiles.retain(|role, _| {
            selected_roles
                .iter()
                .any(|selected| selected.as_str() == role)
        });
        let selection_path = scratch.path.join("selection.json");
        fs::write(
            &selection_path,
            serde_json::to_vec(&selection).expect("selection"),
        )
        .expect("selection file");

        request_with_selection(&candidate, &snapshot, &selection_path).expect("panel request");
        let request = stored_request(&candidate, &snapshot).expect("stored request");
        assert_eq!(request.panel_format_version, Some(1));
        assert_eq!(request.roles, PANEL_CURRENT_ROLES[..10].to_vec());
        assert_eq!(
            request.record_files,
            request
                .roles
                .iter()
                .copied()
                .map(record_file_name)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_selection_with_wrong_candidate_or_legacy_role_is_refused() {
        let scratch = Scratch::new("panel-request-selection-refusal");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        let mut selection = current_selection(&snapshot);
        selection.candidate_id = CandidateId::parse("d".repeat(64)).expect("digest");
        let path = scratch.path.join("selection-wrong-candidate.json");
        fs::write(&path, serde_json::to_vec(&selection).expect("selection")).expect("write");
        assert!(request_with_selection(&candidate, &snapshot, &path).is_err());

        let mut selection = current_selection(&snapshot);
        selection.roster[0] = PanelRole::Rust;
        let path = scratch.path.join("selection-rust.json");
        fs::write(&path, serde_json::to_vec(&selection).expect("selection")).expect("write");
        assert!(request_with_selection(&candidate, &snapshot, &path).is_err());

        let mut selection = current_selection(&snapshot);
        selection.schema_version = 2;
        let path = scratch.path.join("selection-schema.json");
        fs::write(&path, serde_json::to_vec(&selection).expect("selection")).expect("write");
        assert!(request_with_selection(&candidate, &snapshot, &path).is_err());

        let mut selection = current_selection(&snapshot);
        selection.selection_table_version = 1;
        let path = scratch.path.join("selection-table.json");
        fs::write(&path, serde_json::to_vec(&selection).expect("selection")).expect("write");
        assert!(request_with_selection(&candidate, &snapshot, &path).is_err());

        let mut selection = current_selection(&snapshot);
        selection.roster.swap(0, 1);
        let path = scratch.path.join("selection-order.json");
        fs::write(&path, serde_json::to_vec(&selection).expect("selection")).expect("write");
        assert!(request_with_selection(&candidate, &snapshot, &path).is_err());
    }

    #[test]
    fn a_request_never_leaves_the_external_state_directory() {
        let scratch = Scratch::new("panel-request-location");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        request(&candidate, &snapshot).expect("panel request");
        assert!(candidate.panel_request_path().starts_with(&scratch.path));

        let inside_repository = StateRoot::prepare(&[], Some(&repo_root().join("delivery-state")));
        assert!(
            inside_repository.is_err(),
            "delivery state must never resolve inside a checkout"
        );
    }

    #[test]
    fn ten_valid_records_attest_and_import() {
        let scratch = Scratch::new("panel-attest");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        requested(&candidate, &snapshot);
        let files = record_files(&snapshot);
        let dir = write_record_dir(&scratch, &files);

        let output = attest(&candidate, &snapshot, &dir).expect("attest");
        assert_eq!(output.operation.as_str(), "panel-attest");
        assert_eq!(
            candidate.list(PANEL_DIR).expect("panel dir").len(),
            PANEL_CURRENT_ROLES.len()
        );

        let request = stored_request(&candidate, &snapshot).expect("request");
        let attestation = attested_records(&candidate, &request).expect("attested");
        assert!(attestation.unanimous);
        assert_eq!(attestation.records.len(), PANEL_CURRENT_ROLES.len());
        attestation.validate().expect("round trip");
    }

    #[test]
    fn attestation_requires_a_request_first() {
        let scratch = Scratch::new("panel-attest-no-request");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        let dir = write_record_dir(&scratch, &record_files(&snapshot));
        let error = attest(&candidate, &snapshot, &dir).expect_err("no request");
        assert!(error.message().contains("panel-request"), "{error}");
    }

    /// Builds the snapshot a history-only rebase produces: the same content,
    /// so the same `candidate_id` and `content_id`, on moved commits, so a
    /// different `snapshot_sha256`.
    pub(crate) fn rebased(snapshot: &SnapshotView) -> SnapshotView {
        let mut rebased = snapshot.clone();
        rebased.material.repository_set[0].base_oid = fixtures::oid(5);
        rebased.material.repository_set[0].head_oid = fixtures::oid(6);
        rebased.material.repository_set[0].expected_pull_requests[0].head_oid = fixtures::oid(6);
        let digests = rebased.material.digests().expect("digests");
        assert_eq!(digests.candidate_id, snapshot.candidate_id);
        assert_eq!(digests.content_id, snapshot.content_id);
        assert_ne!(digests.snapshot_sha256, snapshot.snapshot_sha256);
        rebased.snapshot_sha256 = digests.snapshot_sha256;
        assert_eq!(
            rebased.material.digests().expect("rebased digests"),
            rebased.digests(),
            "rebased snapshot remains self-consistent"
        );
        rebased
    }

    #[test]
    fn a_history_only_rebase_reuses_the_stored_request_and_records() {
        let scratch = Scratch::new("panel-rebase-reuse");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        requested(&candidate, &snapshot);
        let dir = write_record_dir(&scratch, &record_files(&snapshot));
        attest(&candidate, &snapshot, &dir).expect("attest");

        let rebased = rebased(&snapshot);
        let request = stored_request(&candidate, &rebased).expect("request survives the rebase");
        let attestation = attested_records(&candidate, &request).expect("records survive");
        assert!(attestation.unanimous);
        assert_eq!(attestation.records.len(), PANEL_CURRENT_ROLES.len());
    }

    #[test]
    fn a_content_change_does_not_reuse_the_stored_request() {
        let scratch = Scratch::new("panel-content-change");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        requested(&candidate, &snapshot);

        let mut changed = snapshot.clone();
        changed.material.repository_set[0].integration_tree_oid = fixtures::oid(9);
        let digests = changed.material.digests().expect("digests");
        assert_ne!(digests.candidate_id, snapshot.candidate_id);
        changed.candidate_id = digests.candidate_id;
        changed.content_id = digests.content_id;
        changed.snapshot_sha256 = digests.snapshot_sha256;

        let error = stored_request(&candidate, &changed).expect_err("content changed");
        assert!(error.message().contains("different content"), "{error}");
    }

    fn reject(mutate: impl FnOnce(&mut Vec<RecordFile>, &SnapshotView)) -> DeliveryError {
        let scratch = Scratch::new("panel-reject");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        let request = PanelRequest::for_snapshot(&snapshot);
        let mut files = record_files(&snapshot);
        mutate(&mut files, &snapshot);
        validate_record_set(&candidate, &request, &files).expect_err("record set must be rejected")
    }

    fn rewrite(files: &mut [RecordFile], role: PanelRole, mutate: impl FnOnce(&mut PanelRecord)) {
        let name = record_file_name(role);
        let entry = files
            .iter_mut()
            .find(|(file, _)| *file == name)
            .expect("role present");
        let mut record: PanelRecord = serde_json::from_slice(&entry.1).expect("record");
        mutate(&mut record);
        entry.1 = serde_json::to_vec(&record).expect("record");
    }

    #[test]
    fn a_complete_unanimous_set_is_accepted() {
        let scratch = Scratch::new("panel-complete-set");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        let request = PanelRequest::for_snapshot(&snapshot);
        let attestation = validate_record_set(&candidate, &request, &record_files(&snapshot))
            .expect("unanimous set");
        assert_eq!(attestation.roles, PANEL_CURRENT_ROLES.to_vec());
        assert!(attestation.unanimous);
    }

    #[test]
    fn current_roster_attestation_uses_each_request_size_and_order() {
        let scratch = Scratch::new("panel-variable-rosters");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        for roles in [&PANEL_CURRENT_ROLES[..8], &PANEL_CURRENT_ROLES[..10]] {
            let request =
                PanelRequest::for_snapshot_with_roles(&snapshot, roles, PanelFormat::Current);
            let attestation = validate_record_set(
                &candidate,
                &request,
                &record_files_for_roles(&snapshot, roles),
            )
            .expect("selected roster");
            assert_eq!(attestation.panel_format_version, Some(1));
            assert_eq!(attestation.roles, roles);
            assert_eq!(
                attestation
                    .records
                    .iter()
                    .map(|record| record.file.as_str())
                    .collect::<Vec<_>>(),
                request
                    .record_files
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn a_prior_unversioned_fixed_ten_request_and_record_set_remain_accepted() {
        let scratch = Scratch::new("panel-legacy-fixed-ten");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        let mut request = PanelRequest::legacy_for_snapshot(&snapshot);
        request.model_version = PANEL_LEGACY_MODEL_POLICY.to_owned();
        request.reasoning_effort = PANEL_LEGACY_REASONING_EFFORT_POLICY.to_owned();
        let mut files = legacy_record_files(&snapshot);
        for role in PANEL_ROLES {
            rewrite(&mut files, role, |record| {
                record.model_version = PANEL_LEGACY_MODEL_POLICY.to_owned();
                record.reasoning_effort = PANEL_LEGACY_REASONING_EFFORT_POLICY.to_owned();
            });
        }

        let attestation = validate_record_set(&candidate, &request, &files)
            .expect("prior unversioned records remain compatible");
        assert!(attestation.unanimous);
    }

    #[test]
    fn current_records_reject_an_unsupported_binding() {
        let error = reject(|files, _| {
            rewrite(files, PanelRole::Security, |record| {
                record.model_version = "other-model".to_owned();
            });
        });
        assert!(
            error
                .message()
                .contains("current panel binding must exactly match"),
            "{error}"
        );
    }

    #[test]
    fn the_prior_unversioned_fixed_ten_family_rejects_an_unsupported_binding() {
        let scratch = Scratch::new("panel-legacy-current-binding");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        let mut request = PanelRequest::legacy_for_snapshot(&snapshot);
        request.model_version = "other-model".to_owned();
        let error = validate_record_set(&candidate, &request, &legacy_record_files(&snapshot))
            .expect_err("prior unversioned requests must retain their fixed binding");
        assert!(
            error
                .message()
                .contains("legacy panel binding must exactly match"),
            "{error}"
        );
    }

    #[test]
    fn a_record_binding_must_match_its_request_binding() {
        let scratch = Scratch::new("panel-record-request-binding");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        let mut request = PanelRequest::for_snapshot(&snapshot);
        request.provider = "other-provider".to_owned();
        let error = record(PanelRole::Security, &snapshot)
            .validate(&request)
            .expect_err("a record must match its request binding");
        assert!(error.message().contains("exactly match"), "{error}");
    }

    #[test]
    fn a_missing_role_is_rejected() {
        let error = reject(|files, _| {
            files.retain(|(name, _)| *name != record_file_name(PanelRole::Kernel));
        });
        assert!(error.message().contains("exactly 13 records"), "{error}");
    }

    #[test]
    fn an_extra_record_is_rejected() {
        let error = reject(|files, snapshot| {
            files.push((
                "extra.json".to_owned(),
                serde_json::to_vec(&record(PanelRole::Rust, snapshot)).expect("record"),
            ));
        });
        assert_eq!(error.kind(), DeliveryErrorKind::Invalid);
    }

    #[test]
    fn a_duplicated_role_is_rejected() {
        let error = reject(|files, snapshot| {
            let name = record_file_name(PanelRole::Docs);
            let entry = files
                .iter_mut()
                .find(|(file, _)| *file == name)
                .expect("role present");
            let mut duplicate = record(PanelRole::Rust, snapshot);
            duplicate.run_id = "run-duplicate".to_owned();
            duplicate.receipt_locator =
                format!("{PANEL_PROVIDER_POLICY}://runs/run-duplicate/rust");
            entry.1 = serde_json::to_vec(&duplicate).expect("record");
        });
        assert!(error.message().contains("carries role"), "{error}");
    }

    #[test]
    fn a_wrong_model_provider_or_reasoning_effort_is_rejected() {
        for mutate in [
            (|record: &mut PanelRecord| record.model_version = "other-model".to_owned())
                as fn(&mut PanelRecord),
            |record: &mut PanelRecord| record.provider = "other-provider".to_owned(),
            |record: &mut PanelRecord| record.reasoning_effort = "medium".to_owned(),
        ] {
            let error = reject(|files, _| rewrite(files, PanelRole::Security, mutate));
            assert!(error.message().contains("panel binding"), "{error}");
        }
    }

    #[test]
    fn a_record_bound_to_another_candidate_is_rejected() {
        let error = reject(|files, _| {
            rewrite(files, PanelRole::Security, |record| {
                record.candidate_id = CandidateId::parse("b".repeat(64)).expect("digest");
            });
        });
        assert!(error.message().contains("different candidate"), "{error}");

        let error = reject(|files, _| {
            rewrite(files, PanelRole::Security, |record| {
                record.content_id = ContentId::parse("c".repeat(64)).expect("digest");
            });
        });
        assert!(error.message().contains("different candidate"), "{error}");

        let error = reject(|files, _| {
            rewrite(files, PanelRole::Security, |record| {
                record.snapshot_sha256 = SnapshotSha256::parse("d".repeat(64)).expect("digest");
            });
        });
        assert!(error.message().contains("different candidate"), "{error}");
    }

    #[test]
    fn duplicate_run_provenance_is_rejected() {
        let error = reject(|files, _| {
            rewrite(files, PanelRole::Test, |record| {
                record.run_id = "run-software".to_owned();
            });
        });
        assert!(error.message().contains("run identifier"), "{error}");

        let error = reject(|files, _| {
            rewrite(files, PanelRole::Test, |record| {
                record.receipt_locator =
                    format!("{PANEL_PROVIDER_POLICY}://runs/run-software/software");
            });
        });
        assert!(error.message().contains("receipt locator"), "{error}");

        let error = reject(|files, _| {
            rewrite(files, PanelRole::Test, |record| {
                record.output_sha256 = sha256_bytes(PanelRole::Software.as_str().as_bytes());
            });
        });
        assert!(error.message().contains("output digest"), "{error}");
    }

    #[test]
    fn an_inconsistent_signoff_is_rejected_in_both_directions() {
        let error = reject(|files, _| {
            rewrite(files, PanelRole::Product, |record| {
                record.recommendations = vec!["operator error message is unclear".to_owned()];
            });
        });
        assert!(error.message().contains("if and only if"), "{error}");

        let error = reject(|files, _| {
            rewrite(files, PanelRole::Product, |record| {
                record.signoff = false;
            });
        });
        assert!(error.message().contains("if and only if"), "{error}");
    }

    #[test]
    fn a_finding_blocks_the_panel_even_when_the_record_is_consistent() {
        let error = reject(|files, _| {
            rewrite(files, PanelRole::Observability, |record| {
                record.signoff = false;
                record.recommendations = vec!["metric label cardinality is unbounded".to_owned()];
            });
        });
        assert!(error.message().contains("not unanimous"), "{error}");
        assert!(
            !error.message().contains("cardinality"),
            "a rejection must not echo record content: {error}"
        );
    }

    #[test]
    fn a_malformed_record_is_rejected() {
        for (label, bytes) in [
            ("not json", b"{".to_vec()),
            (
                "unknown field",
                br#"{"artifact_kind":"d2b-delivery/panel-receipt","schema_version":1,"role":"rust","extra":true}"#
                    .to_vec(),
            ),
        ] {
            let error = reject(|files, _| {
                let name = record_file_name(PanelRole::Security);
                let entry = files
                    .iter_mut()
                    .find(|(file, _)| *file == name)
                    .expect("role present");
                entry.1 = bytes;
            });
            assert_eq!(error.kind(), DeliveryErrorKind::Invalid, "{label}");
        }
    }

    #[test]
    fn a_wrong_artifact_kind_or_schema_version_is_rejected() {
        let error = reject(|files, _| {
            rewrite(files, PanelRole::Nixos, |record| {
                record.artifact_kind = "d2b-delivery/wave-snapshot".to_owned();
            });
        });
        assert!(error.message().contains("artifact kind"), "{error}");

        let error = reject(|files, _| {
            rewrite(files, PanelRole::Nixos, |record| {
                record.schema_version = DELIVERY_SCHEMA_VERSION + 1;
            });
        });
        assert!(error.message().contains("schema version"), "{error}");
    }

    #[test]
    fn a_malformed_locator_run_identifier_or_output_digest_is_rejected() {
        let error = reject(|files, _| {
            rewrite(files, PanelRole::Networking, |record| {
                record.receipt_locator = "https://example.invalid/run".to_owned();
            });
        });
        assert!(error.message().contains("receipt locator"), "{error}");

        let error = reject(|files, _| {
            rewrite(files, PanelRole::Networking, |record| {
                record.run_id = "RUN 001".to_owned();
            });
        });
        assert!(error.message().contains("run identifier"), "{error}");

        let error = reject(|files, _| {
            rewrite(files, PanelRole::Networking, |record| {
                record.output_sha256 = "not-a-digest".to_owned();
            });
        });
        assert!(error.message().contains("output digest"), "{error}");
    }

    #[derive(Deserialize)]
    struct FixtureBundle {
        request: PanelRequest,
        records: Vec<PanelRecord>,
        attestation: PanelAttestation,
        seal_panel: PanelAttestation,
    }

    #[test]
    fn the_two_compact_fixture_bundles_pin_strict_legacy_and_current_families() {
        let legacy: FixtureBundle =
            serde_json::from_str(include_str!("testdata/panel-legacy-ten.json"))
                .expect("legacy fixture");
        let current: FixtureBundle =
            serde_json::from_str(include_str!("testdata/panel-current-variable.json"))
                .expect("current fixture");

        for (bundle, expected_format, expected_len) in [
            (&legacy, PanelFormat::Legacy, PANEL_ROLES.len()),
            (&current, PanelFormat::Current, 8),
        ] {
            bundle.request.validate().expect("fixture request");
            assert_eq!(
                bundle.request.format().expect("request format"),
                expected_format
            );
            assert_eq!(bundle.request.roles.len(), expected_len);
            for record in &bundle.records {
                record.validate(&bundle.request).expect("fixture record");
            }
            bundle.attestation.validate().expect("fixture attestation");
            assert_eq!(
                bundle.attestation.format().expect("attestation format"),
                expected_format
            );
            bundle.seal_panel.validate().expect("fixture seal panel");
            assert_eq!(bundle.seal_panel, bundle.attestation);

            let request_json = serde_json::to_value(&bundle.request).expect("request JSON");
            let attestation_json =
                serde_json::to_value(&bundle.attestation).expect("attestation JSON");
            let seal_panel_json =
                serde_json::to_value(&bundle.seal_panel).expect("seal panel JSON");
            let record_json = serde_json::to_value(&bundle.records[0]).expect("record JSON");
            if expected_format == PanelFormat::Legacy {
                assert!(request_json.get("panel_format_version").is_none());
                assert!(attestation_json.get("panel_format_version").is_none());
                assert!(seal_panel_json.get("panel_format_version").is_none());
                assert!(record_json.get("panel_format_version").is_none());
                assert!(
                    bundle.request.roles.contains(&PanelRole::Rust),
                    "legacy fixture must retain rust"
                );
            } else {
                assert_eq!(request_json["panel_format_version"], 1);
                assert_eq!(attestation_json["panel_format_version"], 1);
                assert_eq!(seal_panel_json["panel_format_version"], 1);
                assert_eq!(record_json["panel_format_version"], 1);
                assert!(
                    bundle.request.roles.iter().all(|role| role.is_current()),
                    "current fixture must use current role variants"
                );
                assert!(
                    !bundle.request.roles.contains(&PanelRole::Rust),
                    "current fixture must not use rust"
                );
            }
        }
    }

    #[test]
    fn legacy_attestation_requires_strict_record_order() {
        let mut bundle: FixtureBundle =
            serde_json::from_str(include_str!("testdata/panel-legacy-ten.json"))
                .expect("legacy fixture");
        bundle.attestation.records.swap(0, 1);
        let error = bundle
            .attestation
            .validate()
            .expect_err("legacy records must stay in roster order");
        assert!(error.message().contains("not in roster order"), "{error}");

        let mut seal_panel = bundle.seal_panel;
        seal_panel.records.swap(0, 1);
        let error = seal_panel
            .validate()
            .expect_err("legacy seal records must stay in roster order");
        assert!(error.message().contains("not in roster order"), "{error}");
    }

    #[test]
    fn byte_real_legacy_request_and_seal_deserialize_and_validate() {
        let request_bytes = include_bytes!("testdata/panel-legacy-request.json");
        let request: PanelRequest =
            serde_json::from_slice(request_bytes).expect("legacy request fixture");
        request.validate().expect("legacy request");
        assert_eq!(
            request.format().expect("request format"),
            PanelFormat::Legacy
        );
        assert_eq!(request.roles, PANEL_ROLES);

        let seal_bytes = include_bytes!("testdata/panel-legacy-seal.json");
        let seal: crate::delivery::seal::SealRecord =
            serde_json::from_slice(seal_bytes).expect("legacy seal fixture");
        assert_eq!(
            seal.panel.format().expect("seal panel format"),
            PanelFormat::Legacy
        );
        assert_eq!(
            seal.panel_request_sha256,
            sha256_bytes(request_bytes),
            "the seal must bind the exact request fixture bytes"
        );
        assert_eq!(request.program, seal.program);
        assert_eq!(request.wave, seal.wave);
        assert_eq!(request.candidate_id, seal.candidate_id);
        assert_eq!(request.content_id, seal.content_id);
        assert_eq!(request.snapshot_sha256, seal.snapshot_sha256);

        let scratch = Scratch::new("panel-byte-real-legacy");
        let state = StateRoot::for_tests(&scratch.path.join("state")).expect("state root");
        let candidate = state
            .candidate(&seal.wave, &seal.candidate_id)
            .expect("legacy candidate");
        candidate
            .write_bytes(PANEL_REQUEST_FILE, request_bytes)
            .expect("write exact legacy request bytes");
        seal.validate(&candidate).expect("legacy seal");
    }

    #[test]
    fn lifecycle_fields_and_malformed_panel_families_fail_closed() {
        let legacy_request_bytes = include_bytes!("testdata/panel-legacy-request.json");
        let legacy_request: Value =
            serde_json::from_slice(legacy_request_bytes).expect("legacy request value");
        let current_bundle: Value =
            serde_json::from_str(include_str!("testdata/panel-current-variable.json"))
                .expect("current fixture bundle");

        for field in ["lifecycle_binding", "selection", "selection_bytes_sha256"] {
            let mut legacy = legacy_request.clone();
            legacy[field] = serde_json::json!({});
            assert!(
                serde_json::from_value::<PanelRequest>(legacy).is_err(),
                "legacy requests must reject unversioned {field}"
            );

            let mut current = current_bundle["request"].clone();
            current[field] = serde_json::json!({});
            assert!(
                serde_json::from_value::<PanelRequest>(current).is_err(),
                "version-1 requests must reject out-of-contract {field}"
            );

            let mut attestation = current_bundle["attestation"].clone();
            attestation[field] = serde_json::json!({});
            assert!(
                serde_json::from_value::<PanelAttestation>(attestation).is_err(),
                "version-1 attestations must reject out-of-contract {field}"
            );
        }

        let seal_value: Value =
            serde_json::from_slice(include_bytes!("testdata/panel-legacy-seal.json"))
                .expect("legacy seal value");
        for discriminator in [serde_json::json!(2), serde_json::json!("1")] {
            let mut malformed = seal_value.clone();
            malformed["panel"]["panel_format_version"] = discriminator;
            assert!(
                serde_json::from_value::<crate::delivery::seal::SealRecord>(malformed).is_err(),
                "malformed or unknown seal panel versions must not fall back to legacy"
            );
        }

        let mut unversioned_selection = seal_value.clone();
        unversioned_selection["panel"]["selection"] = serde_json::json!({});
        assert!(
            serde_json::from_value::<crate::delivery::seal::SealRecord>(unversioned_selection)
                .is_err(),
            "legacy seal panels must reject unversioned selection state"
        );

        let mut seal: crate::delivery::seal::SealRecord =
            serde_json::from_value(seal_value).expect("legacy seal");
        let mut current_request_value = legacy_request;
        current_request_value["panel_format_version"] = serde_json::json!(1);
        current_request_value["model_version"] = serde_json::json!(PANEL_MODEL_POLICY);
        current_request_value["reasoning_effort"] =
            serde_json::json!(PANEL_REASONING_EFFORT_POLICY);
        let current_roles = PANEL_CURRENT_ROLES[..10].to_vec();
        current_request_value["roles"] =
            serde_json::to_value(&current_roles).expect("current roles");
        current_request_value["record_files"] = serde_json::to_value(
            current_roles
                .iter()
                .copied()
                .map(record_file_name)
                .collect::<Vec<_>>(),
        )
        .expect("current record files");
        let current_request: PanelRequest =
            serde_json::from_value(current_request_value).expect("current request");
        current_request.validate().expect("valid current request");

        let scratch = Scratch::new("panel-mixed-seal-family");
        let state = StateRoot::for_tests(&scratch.path.join("state")).expect("state root");
        let candidate = state
            .candidate(&seal.wave, &seal.candidate_id)
            .expect("candidate");
        candidate
            .write_json(PANEL_REQUEST_FILE, &current_request)
            .expect("write current request");
        seal.panel_request_sha256 =
            sha256_bytes(&serde_json::to_vec(&current_request).expect("current request bytes"));
        let error = seal
            .validate(&candidate)
            .expect_err("a legacy seal must not validate against a current request");
        assert!(
            error.message().contains("different format or roster"),
            "{error}"
        );
    }

    #[test]
    fn malformed_unknown_and_mixed_panel_families_are_refused_before_fallback() {
        let legacy: FixtureBundle =
            serde_json::from_str(include_str!("testdata/panel-legacy-ten.json"))
                .expect("legacy fixture");
        let current: FixtureBundle =
            serde_json::from_str(include_str!("testdata/panel-current-variable.json"))
                .expect("current fixture");

        let mut unknown = serde_json::to_value(&current.request).expect("request");
        unknown["panel_format_version"] = serde_json::json!(2);
        assert!(serde_json::from_value::<PanelRequest>(unknown).is_err());

        let mut malformed = serde_json::to_value(&current.records[0]).expect("record");
        malformed["panel_format_version"] = serde_json::json!("1");
        assert!(serde_json::from_value::<PanelRecord>(malformed).is_err());

        let mut mixed = current.request.clone();
        mixed.roles = legacy.request.roles.clone();
        mixed.record_files = mixed.roles.iter().copied().map(record_file_name).collect();
        assert!(
            legacy.records[0].validate(&mixed).is_err(),
            "a current request must not accept legacy records"
        );
    }

    #[test]
    fn a_request_that_weakens_the_roster_or_binding_is_rejected() {
        let scratch = Scratch::new("panel-weakened-request");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        for mutate in [
            (|request: &mut PanelRequest| {
                request.roles.retain(|role| *role != PanelRole::Kernel);
            }) as fn(&mut PanelRequest),
            |request: &mut PanelRequest| request.model_version = "other-model".to_owned(),
            |request: &mut PanelRequest| request.provider = "other-provider".to_owned(),
            |request: &mut PanelRequest| request.reasoning_effort = "low".to_owned(),
            |request: &mut PanelRequest| request.record_files.clear(),
            |request: &mut PanelRequest| request.schema_version = DELIVERY_SCHEMA_VERSION + 1,
        ] {
            let mut request = PanelRequest::for_snapshot(&snapshot);
            mutate(&mut request);
            assert!(
                validate_record_set(&candidate, &request, &record_files(&snapshot)).is_err(),
                "a weakened request must be refused"
            );
        }
    }

    #[test]
    fn a_records_directory_holding_anything_else_is_rejected() {
        let scratch = Scratch::new("panel-records-dir");
        let (_state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        requested(&candidate, &snapshot);
        let files = record_files(&snapshot);
        let dir = write_record_dir(&scratch, &files);
        fs::create_dir(dir.join("nested")).expect("nested directory");
        let error = attest(&candidate, &snapshot, &dir).expect_err("nested entry");
        assert!(error.message().contains("exact 13-entry bound"), "{error}");
    }

    #[test]
    fn a_snapshot_with_forged_digests_is_rejected() {
        let scratch = Scratch::new("panel-forged-snapshot");
        let (_state, candidate, mut snapshot) = candidate_with_snapshot(&scratch);
        snapshot.candidate_id = CandidateId::parse("e".repeat(64)).expect("digest");
        let error = snapshot.validate(&candidate).expect_err("forged digests");
        assert!(error.message().contains("self-consistent"), "{error}");
    }

    #[test]
    fn a_snapshot_copied_to_another_candidate_address_is_rejected() {
        let scratch = Scratch::new("panel-copied-snapshot");
        let (state, _candidate, snapshot) = candidate_with_snapshot(&scratch);
        let other_id = CandidateId::parse("e".repeat(64)).expect("candidate id");
        let other = state
            .candidate(snapshot.wave(), &other_id)
            .expect("second candidate");
        other
            .write_json(SNAPSHOT_FILE, &snapshot)
            .expect("copy snapshot");

        let error = open_candidate(&state, &other.snapshot_path())
            .expect_err("copied snapshot must not change candidate identity");
        assert_eq!(error.kind(), DeliveryErrorKind::Invalid);
        assert!(
            error.message().contains("delivery-state address"),
            "{error}"
        );
    }

    #[test]
    fn a_snapshot_copied_to_another_wave_address_is_rejected() {
        let scratch = Scratch::new("panel-copied-snapshot-wave");
        let (state, _candidate, snapshot) = candidate_with_snapshot(&scratch);
        let other = state
            .candidate("W1", &snapshot.candidate_id)
            .expect("second wave");
        other
            .write_json(SNAPSHOT_FILE, &snapshot)
            .expect("copy snapshot");

        let error = open_candidate(&state, &other.snapshot_path())
            .expect_err("copied snapshot must not change wave identity");
        assert_eq!(error.kind(), DeliveryErrorKind::Invalid);
        assert!(
            error.message().contains("delivery-state address"),
            "{error}"
        );
    }

    #[test]
    fn panel_records_copied_to_another_candidate_address_are_rejected() {
        let scratch = Scratch::new("panel-copied-records");
        let (state, candidate, snapshot) = candidate_with_snapshot(&scratch);
        let request = requested(&candidate, &snapshot);
        let dir = write_record_dir(&scratch, &record_files(&snapshot));
        attest(&candidate, &snapshot, &dir).expect("attest original records");

        let other_id = CandidateId::parse("e".repeat(64)).expect("candidate id");
        let other = state
            .candidate(snapshot.wave(), &other_id)
            .expect("second candidate");
        other
            .write_json(PANEL_REQUEST_FILE, &request)
            .expect("copy panel request");
        for record in candidate.list(PANEL_DIR).expect("panel records") {
            let bytes = candidate
                .read_bytes(Path::new(PANEL_DIR).join(&record))
                .expect("read record");
            other
                .write_bytes(Path::new(PANEL_DIR).join(&record), &bytes)
                .expect("copy record");
        }

        let error = attested_records(&other, &request)
            .expect_err("copied records must not change candidate identity");
        assert_eq!(error.kind(), DeliveryErrorKind::Invalid);
        assert!(
            error.message().contains("delivery-state address"),
            "{error}"
        );
    }

    #[test]
    fn a_snapshot_outside_candidate_state_is_rejected() {
        let scratch = Scratch::new("panel-foreign-snapshot");
        let (state, _candidate, snapshot) = candidate_with_snapshot(&scratch);
        let foreign = scratch.path.join("foreign-snapshot.json");
        fs::write(&foreign, serde_json::to_vec(&snapshot).expect("snapshot")).expect("write");
        let error = open_candidate(&state, &foreign).expect_err("foreign path");
        assert!(
            error.message().contains("external delivery state"),
            "{error}"
        );
    }

    #[test]
    fn the_cli_rejects_a_missing_or_stray_option() {
        let args = |values: &[&str]| {
            values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            run_request(&args(&["--repo", "github.com/example/d2b=/checkout"]))
                .expect_err("missing --snapshot")
                .kind(),
            DeliveryErrorKind::Usage
        );
        assert_eq!(
            run_attest(&args(&[
                "--snapshot",
                "/state/snapshot.json",
                "--repo",
                "github.com/example/d2b=/checkout"
            ]))
            .expect_err("missing --records")
            .kind(),
            DeliveryErrorKind::Usage
        );
    }
    /// FR-049: the panel-request gate must refuse a wave whose predecessor
    /// still carries an unmerged work item, even though FR-048 permitted that
    /// wave to start implementing. Enforcing it only at `seal` would let ten
    /// reviewers bind to a snapshot a predecessor finding can still invalidate.
    #[test]
    fn panel_request_is_refused_while_a_prior_wave_item_is_unmerged() {
        use crate::delivery::snapshot::tests::GitFixture;

        let fixture = GitFixture::new("panel-request-prior-wave");
        fixture.write(
            "docs/specs/ADR-046-implementation-graph.json",
            "{\"nodes\":[\
             {\"id\":\"ADR046-foundation-001\",\"kind\":\"work-item\",\"wave\":\"W0\"},\
             {\"id\":\"ADR046-backend-001\",\"kind\":\"work-item\",\"wave\":\"W1\"}]}\n",
        );
        fixture.write(
            "docs/specs/ADR-046-work-items.json",
            "{\"items\":[\
             {\"workItemId\":\"ADR046-foundation-001\",\"implementationState\":\"Planned\"},\
             {\"workItemId\":\"ADR046-backend-001\",\"implementationState\":\"Merged\"}]}\n",
        );
        fixture.commit("predecessor still unmerged");

        let roots = BTreeMap::from([("github.com/example/d2b".to_owned(), fixture.repo())]);
        let scratch = Scratch::new("panel-request-prior-wave-state");
        let mut material = fixtures::material();
        "W1".clone_into(&mut material.wave);
        material.repository_set[0].integration_tree_oid = fixture.head();
        let (_state, candidate, snapshot) = candidate_with_snapshot_from(&scratch, material);

        let error = request_checked(&candidate, &snapshot, &roots, None)
            .expect_err("panel-request must refuse an unmerged predecessor");
        assert!(
            error
                .message()
                .contains("cannot request a panel for, seal, or merge W1"),
            "{}",
            error.message()
        );
    }

    /// Drives the real `wave snapshot` and `wave panel-request` entrypoints
    /// from their argument vectors, so CLI parsing, state-root preparation,
    /// artifact-reference chaining, and component wiring are all covered - not
    /// just the inner `request_checked` helper.
    ///
    /// The state root sits inside the ignored build tree, which
    /// `StateRoot::prepare` refuses in production, so the test installs the
    /// `#[cfg(test)]`-only redirection for the duration of the run. The
    /// production refusal is untouched.
    #[test]
    fn the_panel_request_entrypoint_consumes_the_javascript_selection_fixture() {
        use crate::delivery::{
            snapshot::{self, tests::GitFixture},
            storage::test_root_override,
        };

        let fixture = GitFixture::new("panel-request-cli");
        let _guard = test_root_override::install(&fixture.state());

        let snapshot_output = snapshot::run(&fixture.snapshot_args()).expect("wave snapshot");
        let snapshot_ref = snapshot_output
            .artifact
            .expect("snapshot artifact reference");
        assert!(snapshot_ref.ends_with(SNAPSHOT_FILE), "{snapshot_ref}");
        let state = StateRoot::for_tests(&fixture.state()).expect("test state");
        let snapshot_path = state.resolve_artifact_ref(Path::new(&snapshot_ref));
        let (candidate, snapshot) = open_candidate(&state, &snapshot_path).expect("open snapshot");
        let selection_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/delivery/testdata/panel-selection-js.json");
        let selection: PanelSelectionV1 =
            read_json_file(&selection_path, "JavaScript panel selection fixture")
                .expect("selection fixture");
        selection
            .validate_for_snapshot(snapshot.program(), snapshot.wave(), &snapshot.digests())
            .expect("fixture must bind the Rust snapshot");

        let args = vec![
            "--snapshot".to_owned(),
            snapshot_ref,
            "--selection".to_owned(),
            selection_path.display().to_string(),
            "--repo".to_owned(),
            format!("github.com/example/d2b={}", fixture.repo().display()),
        ];
        let output = run_request(&args).expect("wave panel-request");
        assert_eq!(output.operation.as_str(), "panel-request");
        let artifact = output.artifact.expect("panel request artifact reference");
        assert!(artifact.ends_with(PANEL_REQUEST_FILE), "{artifact}");
        let request: PanelRequest = candidate
            .read_json(PANEL_REQUEST_FILE)
            .expect("generated panel request");
        assert_eq!(request.roles, selection.roster);
        let request_json = serde_json::to_value(request).expect("request JSON");
        assert_eq!(request_json["panel_format_version"], 1);
        assert!(
            request_json.get("lifecycle_binding").is_none()
                && request_json.get("selection").is_none(),
            "selection input must not expand the closed panel request contract"
        );
    }

    /// The same entrypoint, driven the same way, must still refuse an unmerged
    /// predecessor: the gate is reached through real argument parsing, not only
    /// through the inner helper.
    #[test]
    fn the_panel_request_entrypoint_refuses_an_unmerged_prior_wave_item() {
        use crate::delivery::{
            snapshot::{self, tests::GitFixture},
            storage::test_root_override,
        };

        let fixture = GitFixture::new("panel-request-cli-prior-wave");
        fixture.write(
            "docs/specs/ADR-046-implementation-graph.json",
            "{\"nodes\":[\
             {\"id\":\"ADR046-foundation-001\",\"kind\":\"work-item\",\"wave\":\"W0\"},\
             {\"id\":\"ADR046-backend-001\",\"kind\":\"work-item\",\"wave\":\"W1\"}]}\n",
        );
        fixture.write(
            "docs/specs/ADR-046-work-items.json",
            "{\"items\":[\
             {\"workItemId\":\"ADR046-foundation-001\",\"implementationState\":\"Planned\"},\
             {\"workItemId\":\"ADR046-backend-001\",\"implementationState\":\"Merged\"}]}\n",
        );
        fixture.commit("predecessor still unmerged");
        let _guard = test_root_override::install(&fixture.state());

        let mut args = fixture.snapshot_args();
        let wave = args
            .iter()
            .position(|value| value == "--wave")
            .expect("--wave in the fixture arguments")
            + 1;
        args[wave] = "W1".to_owned();
        let snapshot_ref = snapshot::run(&args)
            .expect("wave snapshot")
            .artifact
            .expect("snapshot artifact reference");
        let state = StateRoot::for_tests(&fixture.state()).expect("test state");
        let snapshot_path = state.resolve_artifact_ref(Path::new(&snapshot_ref));
        let (_candidate, snapshot) = open_candidate(&state, &snapshot_path).expect("open snapshot");
        let selection_path = fixture.state().join("selection.json");
        write_current_selection(&selection_path, &snapshot);

        let request_args = vec![
            "--snapshot".to_owned(),
            snapshot_ref,
            "--selection".to_owned(),
            selection_path.display().to_string(),
            "--repo".to_owned(),
            format!("github.com/example/d2b={}", fixture.repo().display()),
        ];
        let error = run_request(&request_args)
            .expect_err("the entrypoint must refuse an unmerged predecessor");
        assert!(
            error
                .message()
                .contains("cannot request a panel for, seal, or merge W1"),
            "{}",
            error.message()
        );
    }
}
