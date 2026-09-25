//! Operation ResourceType contract: one committed broker operation.
//!
//! An `Operation` row is the single declaration of a broker operation: its
//! payload schema, whether it is destructive, the secret-access ceiling, the
//! audit facet (requirement, mode, retained fields, redaction keys, and the
//! target label), the audit-join facet, the authority facet (surface, domain,
//! caller authority, broker requirement), the fd contract, the bounds, the
//! payload provenance, and - for operations inherited from the wire enum -
//! the inherited wire discriminant.
//!
//! Spawn operations are materialized by the process controller from committed
//! `Command` rows: such a row names its owning command in `ownerRef` and
//! reuses the command's parameter schema as its payload schema. An inherited
//! operation carries a `wireTag` instead; the two are mutually exclusive,
//! because a materialized operation is a new operation that no wire
//! discriminant ever named.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use d2b_contracts_resource::v3::execution_policy::{BoundedText, BoundedToken, redacted_debug};
use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_resource::v3::payload_schema::PayloadSchema;

/// Canonical `Operation` ResourceType name.
pub const OPERATION_RESOURCE_TYPE: &str = "Operation";
/// Maximum retained audit fields of one operation.
pub const MAX_OPERATION_AUDIT_FIELDS: usize = 64;
/// Maximum redaction keys of one operation.
pub const MAX_OPERATION_REDACTION_KEYS: usize = 64;
/// Maximum audit-join fields of one operation.
pub const MAX_OPERATION_JOIN_FIELDS: usize = 32;
/// Maximum declared file descriptors per fd contract list.
pub const MAX_OPERATION_FDS: usize = 16;
/// Maximum payload bytes one operation admits.
pub const MAX_OPERATION_PAYLOAD_BYTES: u32 = 16 * 1024 * 1024;
/// Maximum batch entries one operation admits.
pub const MAX_OPERATION_BATCH_ENTRIES: u32 = 65_536;
/// Maximum stream credits one operation admits.
pub const MAX_OPERATION_STREAM_CREDITS: u32 = 4_096;

/// Secret exposure ceiling for one operation.
///
/// The closed seven-value class set the authorization rows already spell; an
/// operation never widens it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SecretAccess {
    /// No secret or key material is reachable.
    None,
    /// Public key material only.
    PublicKeyOnly,
    /// Redacted secret renderings only.
    RedactedOnly,
    /// Secret metadata (existence, identity) only.
    MetadataOnly,
    /// Paths that may hold secrets, never their content.
    PossiblePathsOnly,
    /// Host key metadata only.
    HostKeyMetadata,
    /// Secret material is read or written.
    ReadWrite,
}

/// Broker-use class for one operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum BrokerRequirement {
    /// No broker involvement.
    No,
    /// Broker involved, non-mutating only.
    NoMutation,
    /// Broker involvement depends on the invocation.
    Conditional,
    /// The broker is required.
    Yes,
}

/// Audit mode for one operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AuditMode {
    /// Only denied invocations are audited.
    DenyOnly,
    /// Failures and denials are audited.
    Errors,
    /// Every invocation is audited.
    Yes,
}

/// The surface one operation is invoked from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum OperationSurface {
    /// A CLI invocation.
    Cli,
    /// A broker session.
    Broker,
}

/// The execution domain one operation runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum OperationDomain {
    /// Host-side execution.
    Host,
    /// Guest-side execution.
    Guest,
}

/// Where one operation's payload comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PayloadProvenance {
    /// The caller supplies the payload.
    Request,
    /// The bundle supplies the payload.
    Bundle,
    /// The payload is derived from other committed rows.
    Derived,
}

/// The audit facet of one operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationAudit {
    required: bool,
    mode: AuditMode,
    #[serde(default)]
    retained_fields: Vec<BoundedText>,
    #[serde(default)]
    redaction_keys: Vec<BoundedText>,
    target_label: BoundedToken,
}

impl OperationAudit {
    /// Construct one audit facet after checking the field bounds.

    /// # Errors
    ///
    /// Returns `TooManyAuditFields` when the retained-field or
    /// redaction-key list exceeds its bound.

    pub fn new(
        required: bool,
        mode: AuditMode,
        retained_fields: Vec<BoundedText>,
        redaction_keys: Vec<BoundedText>,
        target_label: BoundedToken,
    ) -> Result<Self, OperationContractError> {
        if retained_fields.len() > MAX_OPERATION_AUDIT_FIELDS
            || redaction_keys.len() > MAX_OPERATION_REDACTION_KEYS
        {
            return Err(OperationContractError::TooManyAuditFields);
        }
        Ok(Self {
            required,
            mode,
            retained_fields,
            redaction_keys,
            target_label,
        })
    }

    /// Whether a successful invocation must be audited.
    pub const fn required(&self) -> bool {
        self.required
    }

    /// The audit mode.
    pub const fn mode(&self) -> AuditMode {
        self.mode
    }

    /// The retained payload-free field names.
    pub fn retained_fields(&self) -> &[BoundedText] {
        &self.retained_fields
    }

    /// The payload field names whose values are redacted.
    pub fn redaction_keys(&self) -> &[BoundedText] {
        &self.redaction_keys
    }

    /// The stable opaque-target category.
    pub const fn target_label(&self) -> &BoundedToken {
        &self.target_label
    }
}

/// The audit-join facet: the payload fields one logical operation keys on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditJoin {
    fields: Vec<BoundedText>,
}

impl AuditJoin {
    /// Construct one audit-join facet after checking the field bound.

    /// # Errors
    ///
    /// Returns `InvalidAuditJoin` when the field list is empty or over
    /// its bound.
    pub fn new(fields: Vec<BoundedText>) -> Result<Self, OperationContractError> {
        if fields.is_empty() || fields.len() > MAX_OPERATION_JOIN_FIELDS {
            return Err(OperationContractError::InvalidAuditJoin);
        }
        Ok(Self { fields })
    }

    /// The declared payload field names the join key is derived from.
    pub fn fields(&self) -> &[BoundedText] {
        &self.fields
    }
}

/// The authority facet of one operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationAuthority {
    surface: OperationSurface,
    domain: OperationDomain,
    caller_authority: BoundedText,
    broker_requirement: BrokerRequirement,
}

impl OperationAuthority {
    /// Construct one authority facet.
    pub fn new(
        surface: OperationSurface,
        domain: OperationDomain,
        caller_authority: BoundedText,
        broker_requirement: BrokerRequirement,
    ) -> Self {
        Self {
            surface,
            domain,
            caller_authority,
            broker_requirement,
        }
    }

    /// The invocation surface.
    pub const fn surface(&self) -> OperationSurface {
        self.surface
    }

    /// The execution domain.
    pub const fn domain(&self) -> OperationDomain {
        self.domain
    }

    /// The caller authority class the envelope admits.
    pub const fn caller_authority(&self) -> &BoundedText {
        &self.caller_authority
    }

    /// The broker requirement class.
    pub const fn broker_requirement(&self) -> BrokerRequirement {
        self.broker_requirement
    }
}

/// One declared file-descriptor contract entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FdContract {
    name: BoundedToken,
    kind: FdKind,
    required: bool,
}

impl FdContract {
    /// Construct one fd contract entry.
    pub fn new(name: BoundedToken, kind: FdKind, required: bool) -> Self {
        Self {
            name,
            kind,
            required,
        }
    }

    /// The contract entry name.
    pub const fn name(&self) -> &BoundedToken {
        &self.name
    }

    /// The descriptor kind the caller passes or receives.
    pub const fn kind(&self) -> FdKind {
        self.kind
    }

    /// Whether the descriptor is mandatory.
    pub const fn required(&self) -> bool {
        self.required
    }
}

/// The descriptor kind of one fd contract entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FdKind {
    /// A regular file.
    File,
    /// A socket.
    Socket,
    /// A pipe.
    Pipe,
    /// A directory.
    Directory,
    /// A pidfd.
    Pidfd,
    /// An anonymous memory file.
    Memfd,
}

/// One descriptor the broker constructs before dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreopenedFd {
    name: BoundedToken,
    constructor: BoundedToken,
}

impl PreopenedFd {
    /// Construct one preopened fd entry.
    pub fn new(name: BoundedToken, constructor: BoundedToken) -> Self {
        Self { name, constructor }
    }

    /// The contract entry name.
    pub const fn name(&self) -> &BoundedToken {
        &self.name
    }

    /// The construction class the broker applies; construction parameters
    /// ride the payload schema.
    pub const fn constructor(&self) -> &BoundedToken {
        &self.constructor
    }
}

/// The fd contract of one operation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationFds {
    #[serde(default)]
    request: Vec<FdContract>,
    #[serde(default)]
    response: Vec<FdContract>,
    #[serde(default)]
    preopened: Vec<PreopenedFd>,
}

impl OperationFds {
    /// Construct one fd contract after checking the list bounds.

    /// # Errors
    ///
    /// Returns `TooManyFds` when any of the request, response, or
    /// preopened lists exceeds its bound.
    pub fn new(
        request: Vec<FdContract>,
        response: Vec<FdContract>,
        preopened: Vec<PreopenedFd>,
    ) -> Result<Self, OperationContractError> {
        if request.len() > MAX_OPERATION_FDS
            || response.len() > MAX_OPERATION_FDS
            || preopened.len() > MAX_OPERATION_FDS
        {
            return Err(OperationContractError::TooManyFds);
        }
        Ok(Self {
            request,
            response,
            preopened,
        })
    }

    /// The descriptors the caller must pass.
    pub fn request(&self) -> &[FdContract] {
        &self.request
    }

    /// The descriptors the operation returns.
    pub fn response(&self) -> &[FdContract] {
        &self.response
    }

    /// The descriptors the broker constructs before dispatch.
    pub fn preopened(&self) -> &[PreopenedFd] {
        &self.preopened
    }
}

/// The per-operation limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationBounds {
    #[serde(default = "default_max_payload_bytes")]
    max_payload_bytes: u32,
    #[serde(default = "default_max_batch_entries")]
    max_batch_entries: u32,
    #[serde(default = "default_max_stream_credits")]
    max_stream_credits: u32,
}

impl Default for OperationBounds {
    fn default() -> Self {
        Self {
            max_payload_bytes: default_max_payload_bytes(),
            max_batch_entries: default_max_batch_entries(),
            max_stream_credits: default_max_stream_credits(),
        }
    }
}

impl OperationBounds {
    /// Construct one bounds facet.

    /// # Errors
    ///
    /// Returns `InvalidBounds` when a limit is zero or exceeds its
    /// ceiling.
    pub fn new(
        max_payload_bytes: u32,
        max_batch_entries: u32,
        max_stream_credits: u32,
    ) -> Result<Self, OperationContractError> {
        if max_payload_bytes == 0
            || max_payload_bytes > MAX_OPERATION_PAYLOAD_BYTES
            || max_batch_entries > MAX_OPERATION_BATCH_ENTRIES
            || max_stream_credits > MAX_OPERATION_STREAM_CREDITS
        {
            return Err(OperationContractError::InvalidBounds);
        }
        Ok(Self {
            max_payload_bytes,
            max_batch_entries,
            max_stream_credits,
        })
    }

    /// The payload byte ceiling.
    pub const fn max_payload_bytes(&self) -> u32 {
        self.max_payload_bytes
    }

    /// The batch entry ceiling.
    pub const fn max_batch_entries(&self) -> u32 {
        self.max_batch_entries
    }

    /// The stream credit ceiling.
    pub const fn max_stream_credits(&self) -> u32 {
        self.max_stream_credits
    }
}

const fn default_max_payload_bytes() -> u32 {
    1024 * 1024
}

const fn default_max_batch_entries() -> u32 {
    256
}

const fn default_max_stream_credits() -> u32 {
    64
}

/// The `Operation` desired spec.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OperationSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    owner_ref: Option<ResourceRef>,
    payload_schema: PayloadSchema,
    destructive: bool,
    secret_access: SecretAccess,
    audit: OperationAudit,
    #[serde(skip_serializing_if = "Option::is_none")]
    audit_join: Option<AuditJoin>,
    authority: OperationAuthority,
    fds: OperationFds,
    bounds: OperationBounds,
    payload_provenance: PayloadProvenance,
    #[serde(skip_serializing_if = "Option::is_none")]
    wire_tag: Option<u32>,
}

impl OperationSpec {
    /// Construct an operation spec after checking the facet invariants.

    /// # Errors
    ///
    /// Returns `InvalidOwnerRef` when the owner reference does not name
    /// a `Command`, `InheritedWireTagOnMaterialized` when a materialized
    /// operation carries a wire tag, `InvalidAuditJoin` when the join
    /// names an undeclared or secret payload field,
    /// `SecretAccessBelowPayload` when the payload declares secret
    /// material without secret access, and `WriteOnlyRetainedField`
    /// when the audit retains a write-only field.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        owner_ref: Option<ResourceRef>,
        payload_schema: PayloadSchema,
        destructive: bool,
        secret_access: SecretAccess,
        audit: OperationAudit,
        audit_join: Option<AuditJoin>,
        authority: OperationAuthority,
        fds: OperationFds,
        bounds: OperationBounds,
        payload_provenance: PayloadProvenance,
        wire_tag: Option<u32>,
    ) -> Result<Self, OperationContractError> {
        if let Some(owner_ref) = &owner_ref {
            d2b_contracts_resource::v3::execution_policy::require_resource_type(owner_ref, "Command")
                .map_err(|_| OperationContractError::InvalidOwnerRef)?;
            if wire_tag.is_some() {
                return Err(OperationContractError::InheritedWireTagOnMaterialized);
            }
        }
        if let Some(join) = &audit_join {
            for field in join.fields() {
                let Some(name) = field.as_str().split(':').next() else {
                    return Err(OperationContractError::InvalidAuditJoin);
                };
                if !payload_schema.declares(name) || payload_schema.is_write_only(name) {
                    return Err(OperationContractError::InvalidAuditJoin);
                }
            }
        }
        if secret_access == SecretAccess::None
            && payload_schema
                .property_names()
                .any(|name| payload_schema.is_write_only(name))
        {
            return Err(OperationContractError::SecretAccessBelowPayload);
        }
        for field in audit.retained_fields() {
            let name = field.as_str().split(':').next().unwrap_or_default();
            if payload_schema.is_write_only(name) {
                return Err(OperationContractError::WriteOnlyRetainedField);
            }
        }
        Ok(Self {
            owner_ref,
            payload_schema,
            destructive,
            secret_access,
            audit,
            audit_join,
            authority,
            fds,
            bounds,
            payload_provenance,
            wire_tag,
        })
    }

    /// The owning `Command` reference of a materialized spawn operation.
    pub fn owner_ref(&self) -> Option<&ResourceRef> {
        self.owner_ref.as_ref()
    }

    /// The payload contract.
    pub fn payload_schema(&self) -> &PayloadSchema {
        &self.payload_schema
    }

    /// Whether the operation mutates durable state.
    pub const fn destructive(&self) -> bool {
        self.destructive
    }

    /// The secret-access ceiling.
    pub const fn secret_access(&self) -> SecretAccess {
        self.secret_access
    }

    /// The audit facet.
    pub fn audit(&self) -> &OperationAudit {
        &self.audit
    }

    /// The audit-join facet, when the operation has a durable logical identity.
    pub fn audit_join(&self) -> Option<&AuditJoin> {
        self.audit_join.as_ref()
    }

    /// The authority facet.
    pub fn authority(&self) -> &OperationAuthority {
        &self.authority
    }

    /// The fd contract.
    pub fn fds(&self) -> &OperationFds {
        &self.fds
    }

    /// The per-operation limits.
    pub const fn bounds(&self) -> &OperationBounds {
        &self.bounds
    }

    /// Where the payload comes from.
    pub const fn payload_provenance(&self) -> PayloadProvenance {
        self.payload_provenance
    }

    /// The inherited wire discriminant, when this operation came from the
    /// wire enum.
    pub const fn wire_tag(&self) -> Option<u32> {
        self.wire_tag
    }
}

redacted_debug!(OperationSpec);

impl<'de> Deserialize<'de> for OperationSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            owner_ref: Option<ResourceRef>,
            payload_schema: PayloadSchema,
            destructive: bool,
            secret_access: SecretAccess,
            audit: OperationAudit,
            #[serde(default)]
            audit_join: Option<AuditJoin>,
            authority: OperationAuthority,
            #[serde(default)]
            fds: OperationFds,
            #[serde(default)]
            bounds: OperationBounds,
            payload_provenance: PayloadProvenance,
            #[serde(default)]
            wire_tag: Option<u32>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.owner_ref,
            wire.payload_schema,
            wire.destructive,
            wire.secret_access,
            wire.audit,
            wire.audit_join,
            wire.authority,
            wire.fds,
            wire.bounds,
            wire.payload_provenance,
            wire.wire_tag,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// One invalid operation declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationContractError {
    /// The owner reference does not name a `Command`.
    InvalidOwnerRef,
    /// A materialized operation carries an inherited wire discriminant.
    InheritedWireTagOnMaterialized,
    /// The audit facet is over its field bounds.
    TooManyAuditFields,
    /// The audit-join facet is empty, over bound, or names an undeclared or
    /// secret payload field.
    InvalidAuditJoin,
    /// The fd contract is over its list bounds.
    TooManyFds,
    /// The bounds facet carries a zero or over-ceiling limit.
    InvalidBounds,
    /// The payload declares secret material but the operation claims no
    /// secret access.
    SecretAccessBelowPayload,
    /// The audit target label names a write-only payload field.
    WriteOnlyRetainedField,
}

impl core::fmt::Display for OperationContractError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            Self::InvalidOwnerRef => "operation ownerRef does not name a Command",
            Self::InheritedWireTagOnMaterialized => {
                "materialized operation carries an inherited wire tag"
            }
            Self::TooManyAuditFields => "operation audit facet is over its field bounds",
            Self::InvalidAuditJoin => "operation auditJoin names an undeclared or secret field",
            Self::TooManyFds => "operation fd contract is over its list bounds",
            Self::InvalidBounds => "operation bounds carry a zero or over-ceiling limit",
            Self::SecretAccessBelowPayload => {
                "operation payload declares secret material but secretAccess is none"
            }
            Self::WriteOnlyRetainedField => "operation audit target label is a secret field",
        };
        formatter.write_str(text)
    }
}

impl std::error::Error for OperationContractError {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload() -> PayloadSchema {
        PayloadSchema::parse(json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["target"],
            "properties": {
                "target": { "type": "string" },
                "token": { "type": "string", "writeOnly": true }
            }
        }))
        .expect("payload validates")
    }

    fn audit() -> OperationAudit {
        OperationAudit::new(
            true,
            AuditMode::Yes,
            vec![BoundedText::parse("target").unwrap()],
            vec![BoundedText::parse("token").unwrap()],
            BoundedToken::parse("opaque-target").unwrap(),
        )
        .expect("audit facet")
    }

    fn authority() -> OperationAuthority {
        OperationAuthority::new(
            OperationSurface::Broker,
            OperationDomain::Host,
            BoundedText::parse("host-operator").unwrap(),
            BrokerRequirement::Yes,
        )
    }

    fn spec(secret_access: SecretAccess) -> Result<OperationSpec, OperationContractError> {
        OperationSpec::new(
            Some(ResourceRef::parse("Command/virtiofsd-worker").unwrap()),
            payload(),
            false,
            secret_access,
            audit(),
            Some(AuditJoin::new(vec![BoundedText::parse("target").unwrap()]).unwrap()),
            authority(),
            OperationFds::default(),
            OperationBounds::default(),
            PayloadProvenance::Request,
            None,
        )
    }

    #[test]
    fn a_materialized_operation_reuses_the_command_payload_contract() {
        let spec = spec(SecretAccess::ReadWrite).expect("operation validates");
        assert_eq!(
            spec.owner_ref().map(ResourceRef::to_canonical_string),
            Some("Command/virtiofsd-worker".to_owned())
        );
        assert_eq!(spec.audit_join().unwrap().fields().len(), 1);
    }

    #[test]
    fn a_materialized_operation_never_carries_an_inherited_wire_tag() {
        let error = OperationSpec::new(
            Some(ResourceRef::parse("Command/x").unwrap()),
            payload(),
            false,
            SecretAccess::None,
            audit(),
            None,
            authority(),
            OperationFds::default(),
            OperationBounds::default(),
            PayloadProvenance::Request,
            Some(7),
        )
        .expect_err("wire tag on a materialized operation");
        assert_eq!(error, OperationContractError::InheritedWireTagOnMaterialized);
    }

    #[test]
    fn secret_bearing_payloads_require_a_secret_access_ceiling() {
        assert_eq!(
            spec(SecretAccess::None).expect_err("secret access below payload"),
            OperationContractError::SecretAccessBelowPayload
        );
    }

    #[test]
    fn audit_join_refuses_undeclared_or_secret_fields() {
        for field in ["missing", "token"] {
            let join = AuditJoin::new(vec![BoundedText::parse(field).unwrap()]).unwrap();
            let error = OperationSpec::new(
                None,
                payload(),
                false,
                SecretAccess::ReadWrite,
                audit(),
                Some(join),
                authority(),
                OperationFds::default(),
                OperationBounds::default(),
                PayloadProvenance::Request,
                None,
            )
            .expect_err("invalid audit join");
            assert_eq!(error, OperationContractError::InvalidAuditJoin, "{field}");
        }
    }

    #[test]
    fn zero_or_over_ceiling_bounds_are_refused() {
        assert!(OperationBounds::new(0, 1, 1).is_err());
        assert!(OperationBounds::new(MAX_OPERATION_PAYLOAD_BYTES + 1, 1, 1).is_err());
        assert!(OperationBounds::new(1, MAX_OPERATION_BATCH_ENTRIES + 1, 1).is_err());
        assert!(OperationBounds::new(1, 1, MAX_OPERATION_STREAM_CREDITS + 1).is_err());
    }

    #[test]
    fn the_wire_shape_round_trips_and_is_closed() {
        let spec = spec(SecretAccess::RedactedOnly).expect("operation validates");
        let bytes =
            d2b_contracts_resource::v3::resource_schema::canonical_json_bytes(&spec).expect("canonical");
        let restored: OperationSpec = serde_json::from_slice(&bytes).expect("parses");
        assert_eq!(restored, spec);
        let text = String::from_utf8(bytes).expect("utf8");
        assert!(text.contains("\"secretAccess\":\"redacted-only\""));
        assert!(text.contains("\"payloadProvenance\":\"request\""));
        assert!(!text.contains("wireTag"));
    }
}
