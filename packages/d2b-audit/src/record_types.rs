//! Typed v3 authoritative audit records.

use crate::hash_chain::{AuditChainLink, AuditHash, genesis_hash, payload_hash, record_hash};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Current audit record schema version.
pub const AUDIT_SCHEMA_VERSION: u16 = 1;

/// Closed audit record classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuditRecordClass {
    /// A resource mutation.
    ResourceMutation,
    /// A provider or resource upgrade decision.
    ResourceUpgrade,
    /// An authorization policy change.
    RbacChange,
    /// A ComponentSession connection event.
    SessionConnect,
    /// A bus route admission decision.
    RouteAdmission,
    /// A cross-Zone resource share event.
    ResourceShare,
    /// A broker-mediated effect.
    BrokerEffect,
    /// A process lifecycle effect.
    ProcessEffect,
    /// A state reset.
    StateReset,
}

impl AuditRecordClass {
    /// Stable wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ResourceMutation => "resource-mutation",
            Self::ResourceUpgrade => "resource-upgrade",
            Self::RbacChange => "rbac-change",
            Self::SessionConnect => "session-connect",
            Self::RouteAdmission => "route-admission",
            Self::ResourceShare => "resource-share",
            Self::BrokerEffect => "broker-effect",
            Self::ProcessEffect => "process-effect",
            Self::StateReset => "state-reset",
        }
    }

    /// Name of the class-specific object field.
    pub const fn fields_key(self) -> &'static str {
        match self {
            Self::ResourceMutation => "resource_mutation_fields",
            Self::ResourceUpgrade => "resource_upgrade_fields",
            Self::RbacChange => "rbac_change_fields",
            Self::SessionConnect => "session_connect_fields",
            Self::RouteAdmission => "route_admission_fields",
            Self::ResourceShare => "resource_share_fields",
            Self::BrokerEffect => "broker_effect_fields",
            Self::ProcessEffect => "process_effect_fields",
            Self::StateReset => "state_reset_fields",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "resource-mutation" => Self::ResourceMutation,
            "resource-upgrade" => Self::ResourceUpgrade,
            "rbac-change" => Self::RbacChange,
            "session-connect" => Self::SessionConnect,
            "route-admission" => Self::RouteAdmission,
            "resource-share" => Self::ResourceShare,
            "broker-effect" => Self::BrokerEffect,
            "process-effect" => Self::ProcessEffect,
            "state-reset" => Self::StateReset,
            _ => return None,
        })
    }
}

/// Resource mutation fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ResourceMutationFields {
    /// Mutation verb.
    pub verb: String,
    /// Closed resource type name or `vendor`.
    pub resource_type: String,
    /// Opaque resource UID.
    pub resource_uid: String,
    /// Resulting resource generation.
    pub generation: u64,
    /// Expected revision.
    pub expected_revision: u64,
    /// Resulting revision.
    pub resulting_revision: u64,
    /// Authenticated subject digest.
    pub subject_digest: String,
    /// Authorization policy revision.
    pub policy_revision: u64,
    /// Closed result.
    pub outcome: String,
    /// Stable error code, when present.
    pub error_code: Option<String>,
}

/// Resource upgrade fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ResourceUpgradeFields {
    /// Upgrade operation.
    pub verb: String,
    /// Resource type.
    pub resource_type: String,
    /// Opaque resource UID.
    pub resource_uid: String,
    /// Closed update state.
    pub update_state: String,
    /// Closed disruption class.
    pub disruption: String,
    /// Whether state is preserved.
    pub preserve_state: bool,
    /// Closed reason codes.
    pub reasons: Vec<String>,
    /// Observed generation.
    pub observed_generation: u64,
    /// Target generation.
    pub target_generation: u64,
    /// Bounded affected-resource count.
    pub affected_owned_count: u32,
    /// Opaque operation identifier.
    pub operation_id: String,
    /// Closed result.
    pub outcome: String,
    /// Stable error code, when present.
    pub error_code: Option<String>,
}

/// RBAC change fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct RbacChangeFields {
    /// RBAC verb.
    pub verb: String,
    /// `Role` or `RoleBinding`.
    pub resource_type: String,
    /// Opaque resource UID.
    pub resource_uid: String,
    /// Resulting generation.
    pub generation: u64,
    /// Authenticated subject digest.
    pub subject_digest: String,
    /// Authorization policy revision.
    pub policy_revision: u64,
    /// Closed result.
    pub outcome: String,
}

/// Session connection fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SessionConnectFields {
    /// Connection lifecycle event.
    pub event: String,
    /// Noise profile.
    pub profile: String,
    /// Closed purpose class.
    pub purpose_class: String,
    /// Closed transport class.
    pub transport_class: String,
    /// Authenticated subject digest.
    pub subject_digest: String,
    /// Authorization decision.
    pub authz_decision: String,
    /// Authorization policy revision.
    pub authz_revision: u64,
    /// Opaque session generation digest.
    pub session_gen_digest: String,
    /// Closed outcome.
    pub outcome: String,
    /// Stable error code, when present.
    pub error_code: Option<String>,
}

/// Bus route admission fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct RouteAdmissionFields {
    /// Closed service package name.
    pub service: String,
    /// Method name from the service contract.
    pub method: String,
    /// Closed route direction.
    pub direction: String,
    /// Authenticated subject digest.
    pub subject_digest: String,
    /// Authorization decision.
    pub authz_decision: String,
    /// Authorization policy revision.
    pub authz_revision: u64,
    /// Closed outcome.
    pub outcome: String,
}

/// Resource share fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ResourceShareFields {
    /// Share lifecycle event.
    pub event: String,
    /// Peer Zone name.
    pub peer_zone: String,
    /// Closed capability subset.
    pub capability_subset: Vec<String>,
    /// Closed outcome.
    pub outcome: String,
}

/// Broker effect fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct BrokerEffectFields {
    /// Stable broker operation class.
    pub op_class: String,
    /// Authenticated subject digest.
    pub subject_digest: String,
    /// Opaque execution context digest.
    pub execution_context_digest: String,
    /// Opaque resource context digest.
    pub resource_context_digest: String,
    /// Closed outcome.
    pub outcome: String,
    /// Stable error code, when present.
    pub error_code: Option<String>,
}

/// Process effect fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ProcessEffectFields {
    /// Process lifecycle event.
    pub event: String,
    /// Closed Process Provider.
    pub provider: String,
    /// Process domain.
    pub domain: String,
    /// Whether the parent Host has no isolation.
    pub no_isolation: bool,
    /// Opaque execution reference digest.
    pub execution_ref_digest: String,
    /// Opaque process UID.
    pub process_uid: String,
    /// Closed outcome.
    pub outcome: String,
    /// Closed exit class.
    pub exit_class: Option<String>,
}

/// State reset fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct StateResetFields {
    /// Reset scope.
    pub scope: String,
    /// Reset trigger.
    pub trigger: String,
    /// New generation.
    pub generation: u64,
    /// Digest of the previous state.
    pub prior_digest: String,
    /// Closed result.
    pub outcome: String,
}

/// Class-specific record fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditRecordFields {
    /// Resource mutation.
    ResourceMutation(ResourceMutationFields),
    /// Resource upgrade.
    ResourceUpgrade(ResourceUpgradeFields),
    /// RBAC change.
    RbacChange(RbacChangeFields),
    /// Session connection.
    SessionConnect(SessionConnectFields),
    /// Route admission.
    RouteAdmission(RouteAdmissionFields),
    /// Resource share.
    ResourceShare(ResourceShareFields),
    /// Broker effect.
    BrokerEffect(BrokerEffectFields),
    /// Process effect.
    ProcessEffect(ProcessEffectFields),
    /// State reset.
    StateReset(StateResetFields),
}

impl AuditRecordFields {
    /// Return the class of these fields.
    pub const fn class(&self) -> AuditRecordClass {
        match self {
            Self::ResourceMutation(_) => AuditRecordClass::ResourceMutation,
            Self::ResourceUpgrade(_) => AuditRecordClass::ResourceUpgrade,
            Self::RbacChange(_) => AuditRecordClass::RbacChange,
            Self::SessionConnect(_) => AuditRecordClass::SessionConnect,
            Self::RouteAdmission(_) => AuditRecordClass::RouteAdmission,
            Self::ResourceShare(_) => AuditRecordClass::ResourceShare,
            Self::BrokerEffect(_) => AuditRecordClass::BrokerEffect,
            Self::ProcessEffect(_) => AuditRecordClass::ProcessEffect,
            Self::StateReset(_) => AuditRecordClass::StateReset,
        }
    }

    fn to_value(&self) -> serde_json::Value {
        match self {
            Self::ResourceMutation(value) => serde_json::to_value(value),
            Self::ResourceUpgrade(value) => serde_json::to_value(value),
            Self::RbacChange(value) => serde_json::to_value(value),
            Self::SessionConnect(value) => serde_json::to_value(value),
            Self::RouteAdmission(value) => serde_json::to_value(value),
            Self::ResourceShare(value) => serde_json::to_value(value),
            Self::BrokerEffect(value) => serde_json::to_value(value),
            Self::ProcessEffect(value) => serde_json::to_value(value),
            Self::StateReset(value) => serde_json::to_value(value),
        }
        .expect("typed audit fields serialize")
    }

    fn from_value(
        class: AuditRecordClass,
        value: serde_json::Value,
    ) -> Result<Self, serde_json::Error> {
        match class {
            AuditRecordClass::ResourceMutation => {
                serde_json::from_value(value).map(Self::ResourceMutation)
            }
            AuditRecordClass::ResourceUpgrade => {
                serde_json::from_value(value).map(Self::ResourceUpgrade)
            }
            AuditRecordClass::RbacChange => serde_json::from_value(value).map(Self::RbacChange),
            AuditRecordClass::SessionConnect => {
                serde_json::from_value(value).map(Self::SessionConnect)
            }
            AuditRecordClass::RouteAdmission => {
                serde_json::from_value(value).map(Self::RouteAdmission)
            }
            AuditRecordClass::ResourceShare => {
                serde_json::from_value(value).map(Self::ResourceShare)
            }
            AuditRecordClass::BrokerEffect => serde_json::from_value(value).map(Self::BrokerEffect),
            AuditRecordClass::ProcessEffect => {
                serde_json::from_value(value).map(Self::ProcessEffect)
            }
            AuditRecordClass::StateReset => serde_json::from_value(value).map(Self::StateReset),
        }
    }
}

/// One complete hash-chained audit record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    ts_ms: u64,
    schema_version: u16,
    zone: String,
    operation_id: String,
    correlation_id: String,
    trace_id: Option<String>,
    source: String,
    previous_hash: AuditHash,
    record_hash: AuditHash,
    fields: AuditRecordFields,
}

impl AuditRecord {
    /// Construct and hash a record.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ts_ms: u64,
        zone: impl Into<String>,
        operation_id: impl Into<String>,
        correlation_id: impl Into<String>,
        trace_id: Option<String>,
        source: impl Into<String>,
        previous_hash: AuditHash,
        fields: AuditRecordFields,
    ) -> Result<Self, AuditRecordError> {
        let mut record = Self {
            ts_ms,
            schema_version: AUDIT_SCHEMA_VERSION,
            zone: bounded_text(zone.into())?,
            operation_id: bounded_text(operation_id.into())?,
            correlation_id: bounded_text(correlation_id.into())?,
            trace_id: trace_id.map(bounded_text).transpose()?,
            source: bounded_text(source.into())?,
            previous_hash,
            record_hash: genesis_hash(),
            fields,
        };
        validate_fields(&record.fields)?;
        record.record_hash = record.computed_record_hash()?;
        Ok(record)
    }

    /// Current class.
    pub const fn class(&self) -> AuditRecordClass {
        self.fields.class()
    }

    /// Zone envelope value.
    pub fn zone(&self) -> &str {
        &self.zone
    }

    /// Previous hash.
    pub const fn previous_hash(&self) -> &AuditHash {
        &self.previous_hash
    }

    /// Record hash.
    pub const fn record_hash(&self) -> &AuditHash {
        &self.record_hash
    }

    /// Class fields.
    pub const fn fields(&self) -> &AuditRecordFields {
        &self.fields
    }

    /// Verify the record hash and predecessor link.
    pub fn verify(&self, expected_previous: &AuditHash) -> Result<(), AuditRecordError> {
        if &self.previous_hash != expected_previous {
            return Err(AuditRecordError::ChainMismatch);
        }
        let expected = self.computed_record_hash()?;
        if expected != self.record_hash {
            return Err(AuditRecordError::HashMismatch);
        }
        Ok(())
    }

    /// Serialize one NDJSON line.
    pub fn to_json_line(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut bytes = serde_json::to_vec(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Compute a link view for callers that need explicit chain metadata.
    pub fn chain_link(&self, sequence: u64) -> Result<AuditChainLink, AuditRecordError> {
        let payload = payload_hash(
            &serde_json::to_vec(&self.fields.to_value())
                .map_err(|_| AuditRecordError::Serialization)?,
        );
        Ok(AuditChainLink::new(
            sequence,
            self.previous_hash.clone(),
            payload,
            self.record_hash.clone(),
        ))
    }

    fn computed_record_hash(&self) -> Result<AuditHash, AuditRecordError> {
        let envelope = self.canonical_without_hash()?;
        Ok(record_hash(&self.previous_hash, &envelope))
    }

    fn canonical_without_hash(&self) -> Result<Vec<u8>, AuditRecordError> {
        serde_json::to_vec(&serde_json::json!({
            "ts_ms": self.ts_ms,
            "schema_version": self.schema_version,
            "zone": self.zone,
            "record_class": self.class().as_str(),
            "operation_id": self.operation_id,
            "correlation_id": self.correlation_id,
            "trace_id": self.trace_id,
            "source": self.source,
            "prev_hash": self.previous_hash,
            self.class().fields_key(): self.fields.to_value(),
        }))
        .map_err(|_| AuditRecordError::Serialization)
    }
}

impl Serialize for AuditRecord {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut object = serde_json::Map::new();
        object.insert("ts_ms".to_owned(), serde_json::json!(self.ts_ms));
        object.insert(
            "schema_version".to_owned(),
            serde_json::json!(self.schema_version),
        );
        object.insert("zone".to_owned(), serde_json::json!(self.zone));
        object.insert(
            "record_class".to_owned(),
            serde_json::json!(self.class().as_str()),
        );
        object.insert(
            "operation_id".to_owned(),
            serde_json::json!(self.operation_id),
        );
        object.insert(
            "correlation_id".to_owned(),
            serde_json::json!(self.correlation_id),
        );
        object.insert("trace_id".to_owned(), serde_json::json!(self.trace_id));
        object.insert("source".to_owned(), serde_json::json!(self.source));
        object.insert(
            "prev_hash".to_owned(),
            serde_json::json!(self.previous_hash),
        );
        object.insert(
            "record_hash".to_owned(),
            serde_json::json!(self.record_hash),
        );
        object.insert(self.class().fields_key().to_owned(), self.fields.to_value());
        object.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AuditRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        if contains_legacy_fields(&value) {
            return Err(serde::de::Error::custom("audit-record-legacy-field"));
        }
        let object = value
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("audit-record-not-object"))?;
        let class_name = object
            .get("record_class")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| serde::de::Error::custom("audit-record-class-missing"))?;
        let class = AuditRecordClass::parse(class_name)
            .ok_or_else(|| serde::de::Error::custom("audit-record-class-invalid"))?;
        let field_value = object
            .get(class.fields_key())
            .cloned()
            .ok_or_else(|| serde::de::Error::custom("audit-record-fields-missing"))?;
        let fields =
            AuditRecordFields::from_value(class, field_value).map_err(serde::de::Error::custom)?;
        validate_fields(&fields).map_err(serde::de::Error::custom)?;
        let get_string = |key: &str| {
            object
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| serde::de::Error::custom("audit-record-field-invalid"))
        };
        let trace_id = match object.get("trace_id") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => Some(
                value
                    .as_str()
                    .ok_or_else(|| serde::de::Error::custom("audit-record-field-invalid"))?
                    .to_owned(),
            ),
        };
        let record_hash =
            AuditHash::parse(get_string("record_hash")?).map_err(serde::de::Error::custom)?;
        let previous_hash =
            AuditHash::parse(get_string("prev_hash")?).map_err(serde::de::Error::custom)?;
        let schema_version = object
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| serde::de::Error::custom("audit-record-field-invalid"))?;
        if schema_version != u64::from(AUDIT_SCHEMA_VERSION) {
            return Err(serde::de::Error::custom("audit-record-schema-version"));
        }
        let record = Self {
            ts_ms: object
                .get("ts_ms")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| serde::de::Error::custom("audit-record-field-invalid"))?,
            schema_version: AUDIT_SCHEMA_VERSION,
            zone: get_string("zone")?,
            operation_id: get_string("operation_id")?,
            correlation_id: get_string("correlation_id")?,
            trace_id,
            source: get_string("source")?,
            previous_hash,
            record_hash,
            fields,
        };
        if record
            .computed_record_hash()
            .map_err(serde::de::Error::custom)?
            != record.record_hash
        {
            return Err(serde::de::Error::custom("audit-record-hash-mismatch"));
        }
        Ok(record)
    }
}

/// Validation failure for a typed audit record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditRecordError {
    /// A text field exceeded the bounded envelope.
    TextOutOfBounds,
    /// Canonical serialization failed.
    Serialization,
    /// The predecessor hash did not match.
    ChainMismatch,
    /// The record hash did not match.
    HashMismatch,
    /// A class field was outside its closed domain.
    FieldInvalid,
}

impl core::fmt::Display for AuditRecordError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::TextOutOfBounds => "audit-record-text-out-of-bounds",
            Self::Serialization => "audit-record-serialization-failed",
            Self::ChainMismatch => "audit-record-chain-mismatch",
            Self::HashMismatch => "audit-record-hash-mismatch",
            Self::FieldInvalid => "audit-record-field-invalid",
        })
    }
}

impl std::error::Error for AuditRecordError {}

fn bounded_text(value: String) -> Result<String, AuditRecordError> {
    if value.is_empty()
        || value.len() > 256
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b'/')
    {
        return Err(AuditRecordError::TextOutOfBounds);
    }
    Ok(value)
}

fn closed(value: &str, allowed: &[&str]) -> bool {
    allowed.contains(&value)
}

fn validate_fields(fields: &AuditRecordFields) -> Result<(), AuditRecordError> {
    let valid_resource_type = |value: &str| {
        [
            "Zone",
            "ZoneLink",
            "Provider",
            "Role",
            "RoleBinding",
            "Quota",
            "Host",
            "Guest",
            "Process",
            "EphemeralProcess",
            "Volume",
            "Network",
            "Device",
            "User",
            "Credential",
            "Endpoint",
            "ResourceExport",
            "ResourceImport",
            "vendor",
        ]
        .contains(&value)
            || value.contains(".d2bus.org.")
    };
    let valid_digestish = |value: &str| {
        !value.is_empty()
            && value.len() <= 256
            && value
                .bytes()
                .all(|byte| !byte.is_ascii_control() && byte != b'/')
    };
    match fields {
        AuditRecordFields::ResourceMutation(fields) => {
            if !closed(
                &fields.verb,
                &[
                    "create",
                    "update-spec",
                    "update-status",
                    "update-metadata",
                    "update-finalizers",
                    "delete",
                    "use-credential",
                    "admin-credential",
                ],
            ) || !valid_resource_type(&fields.resource_type)
                || !closed(
                    &fields.outcome,
                    &["ok", "conflict", "denied", "invalid", "error"],
                )
                || !valid_digestish(&fields.resource_uid)
                || !valid_digestish(&fields.subject_digest)
            {
                return Err(AuditRecordError::FieldInvalid);
            }
        }
        AuditRecordFields::ResourceUpgrade(fields) => {
            if !closed(&fields.verb, &["assess", "plan", "execute"])
                || !valid_resource_type(&fields.resource_type)
                || !closed(
                    &fields.update_state,
                    &[
                        "Current",
                        "UpdateAvailable",
                        "UpgradeRequired",
                        "Upgrading",
                        "Blocked",
                        "Unknown",
                    ],
                )
                || !closed(
                    &fields.disruption,
                    &["None", "Reload", "Restart", "Recycle", "Replace"],
                )
                || !closed(
                    &fields.outcome,
                    &["ok", "blocked", "conflict", "denied", "error"],
                )
                || !valid_digestish(&fields.resource_uid)
                || !valid_digestish(&fields.operation_id)
                || fields.reasons.len() > 16
                || fields.reasons.iter().any(|reason| !valid_digestish(reason))
            {
                return Err(AuditRecordError::FieldInvalid);
            }
        }
        AuditRecordFields::RbacChange(fields) => {
            if !closed(&fields.verb, &["create", "update-spec", "delete"])
                || !closed(&fields.resource_type, &["Role", "RoleBinding"])
                || !closed(&fields.outcome, &["ok", "denied", "error"])
                || !valid_digestish(&fields.resource_uid)
                || !valid_digestish(&fields.subject_digest)
            {
                return Err(AuditRecordError::FieldInvalid);
            }
        }
        AuditRecordFields::SessionConnect(fields) => {
            if !closed(&fields.event, &["connect", "reconnect", "close"])
                || !closed(&fields.profile, &["NN", "KK", "IKpsk2"])
                || !closed(&fields.purpose_class, &["local", "enrolled", "bootstrap"])
                || !closed(&fields.transport_class, &["unix", "vsock", "zone_link"])
                || !closed(&fields.authz_decision, &["allowed", "denied"])
                || !closed(
                    &fields.outcome,
                    &["ok", "auth", "policy", "timeout", "error"],
                )
                || !valid_digestish(&fields.subject_digest)
                || !valid_digestish(&fields.session_gen_digest)
            {
                return Err(AuditRecordError::FieldInvalid);
            }
        }
        AuditRecordFields::RouteAdmission(fields) => {
            if !valid_digestish(&fields.service)
                || !valid_digestish(&fields.method)
                || !closed(&fields.direction, &["local", "host", "guest", "zone_link"])
                || !closed(&fields.authz_decision, &["allowed", "denied"])
                || !closed(&fields.outcome, &["ok", "denied", "error"])
                || !valid_digestish(&fields.subject_digest)
            {
                return Err(AuditRecordError::FieldInvalid);
            }
        }
        AuditRecordFields::ResourceShare(fields) => {
            if !closed(
                &fields.event,
                &["advertise", "admit", "revoke", "reconnect"],
            ) || !closed(
                &fields.outcome,
                &["ok", "denied", "quota", "revoked", "degraded", "error"],
            ) || !valid_digestish(&fields.peer_zone)
                || fields.capability_subset.len() > 16
                || fields
                    .capability_subset
                    .iter()
                    .any(|capability| !valid_digestish(capability))
            {
                return Err(AuditRecordError::FieldInvalid);
            }
        }
        AuditRecordFields::BrokerEffect(fields) => {
            if !valid_digestish(&fields.op_class)
                || !valid_digestish(&fields.subject_digest)
                || !valid_digestish(&fields.execution_context_digest)
                || !valid_digestish(&fields.resource_context_digest)
                || !closed(&fields.outcome, &["ok", "denied", "error"])
            {
                return Err(AuditRecordError::FieldInvalid);
            }
        }
        AuditRecordFields::ProcessEffect(fields) => {
            if !closed(&fields.event, &["launch", "stop", "adopt", "quarantine"])
                || !closed(
                    &fields.provider,
                    &["minijail", "systemd", "system-core-user"],
                )
                || !closed(&fields.domain, &["system", "user"])
                || !closed(&fields.outcome, &["ok", "error"])
                || fields
                    .exit_class
                    .as_deref()
                    .is_some_and(|value| !closed(value, &["exited", "signaled", "killed"]))
                || fields.no_isolation
                    && (fields.domain != "user" || fields.provider != "system-core-user")
                || fields.provider == "system-core-user" && fields.domain != "user"
                || !valid_digestish(&fields.execution_ref_digest)
                || !valid_digestish(&fields.process_uid)
            {
                return Err(AuditRecordError::FieldInvalid);
            }
        }
        AuditRecordFields::StateReset(fields) => {
            if !closed(&fields.scope, &["zone", "provider", "host", "guest"])
                || !closed(
                    &fields.trigger,
                    &["operator", "upgrade", "corruption", "emergency"],
                )
                || !closed(&fields.outcome, &["ok", "error"])
                || !valid_digestish(&fields.prior_digest)
            {
                return Err(AuditRecordError::FieldInvalid);
            }
        }
    }
    Ok(())
}

fn contains_legacy_fields(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(object) => object.iter().any(|(key, value)| {
            matches!(key.as_str(), "realm" | "node" | "workload_id")
                || contains_legacy_fields(value)
        }),
        serde_json::Value::Array(values) => values.iter().any(contains_legacy_fields),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(previous: AuditHash) -> AuditRecord {
        AuditRecord::new(
            1,
            "work",
            "operation-digest",
            "correlation-digest",
            None,
            "zone-runtime",
            previous,
            AuditRecordFields::ProcessEffect(ProcessEffectFields {
                event: "launch".to_owned(),
                provider: "system-core-user".to_owned(),
                domain: "user".to_owned(),
                no_isolation: true,
                execution_ref_digest: "sha256:execution".to_owned(),
                process_uid: "uid-digest".to_owned(),
                outcome: "ok".to_owned(),
                exit_class: None,
            }),
        )
        .unwrap()
    }

    #[test]
    fn v3_record_has_zone_and_no_legacy_fields() {
        let record = record(genesis_hash());
        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(
            value.get("zone").and_then(|value| value.as_str()),
            Some("work")
        );
        assert!(value.get("realm").is_none());
        assert!(value.get("node").is_none());
        assert!(value.get("workload_id").is_none());
        assert!(value.get("process_effect_fields").is_some());
        record.verify(record.previous_hash()).unwrap();
    }

    #[test]
    fn deserialization_detects_truncation_or_tampering() {
        let record = record(genesis_hash());
        let mut value = serde_json::to_value(&record).unwrap();
        value["zone"] = serde_json::json!("other");
        assert!(serde_json::from_value::<AuditRecord>(value).is_err());
    }
}
