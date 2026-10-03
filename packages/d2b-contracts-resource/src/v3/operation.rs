//! The `Operation` contract: one externally callable declaration.
//!
//! An `Operation` row is the whole of what a call is: its payload and result
//! contracts, where the payload comes from, whether it mutates durable state,
//! its secret-access ceiling, its audit facet and join, its authority facet,
//! its descriptor carriage, its limits, and - the part that matters most -
//! the trusted implementation that answers it.
//!
//! # The implementation is a declared identity, not a row
//!
//! [`OperationImplementation`] names either a provider component method or a
//! trusted executable template owned by a declared Provider. It cannot name a
//! `Command` row, a host path, an argv, or anything a mutable provider row
//! could introduce: a caller does not choose code, and a provider resource
//! that names an untrusted artifact cannot create a compiled privileged
//! handler. Compiling a declared implementation into a callable handler is
//! the deployment's job, not the row's.
//!
//! # Why this lives in the contract layer
//!
//! The serializable operation data is canonical contract data, so its home
//! is this module rather than inside the provider crate that drives one
//! ResourceType. The provider crate still carries its own copy of these
//! facets for the production entry point that has not switched yet; the
//! unit that converts that provider re-exports these definitions and drops
//! its copies in the same commit, and the atomic production cutover
//! removes the rest.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    ResourceRef,
    execution_policy::{BoundedText, BoundedToken, redacted_debug},
    payload_schema::PayloadSchema,
};
use d2b_contracts::wire_deserialize;

/// Canonical `Operation` ResourceType name.
pub const OPERATION_RESOURCE_TYPE: &str = "Operation";
/// The only ResourceType an operation implementation reference may name.
pub const PROVIDER_RESOURCE_TYPE: &str = "Provider";
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
/// The closed class set the authorization rows already spell; an operation
/// never widens it.
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
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

redacted_debug!(OperationAudit);

/// The audit-join facet: the payload fields one logical operation keys on.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

redacted_debug!(AuditJoin);

/// The authority facet of one operation.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationAuthority {
    surface: OperationSurface,
    domain: OperationDomain,
    caller_authority: BoundedText,
    broker_requirement: BrokerRequirement,
}

impl OperationAuthority {
    /// Construct an authority facet.
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

redacted_debug!(OperationAuthority);

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

/// One declared file-descriptor contract entry.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FdContract {
    name: BoundedToken,
    kind: FdKind,
    required: bool,
}

impl FdContract {
    /// Construct one fd contract entry.
    pub const fn new(name: BoundedToken, kind: FdKind, required: bool) -> Self {
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

redacted_debug!(FdContract);

/// One descriptor the broker constructs before dispatch.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreopenedFd {
    name: BoundedToken,
    constructor: BoundedToken,
}

impl PreopenedFd {
    /// Construct one preopened fd entry.
    pub const fn new(name: BoundedToken, constructor: BoundedToken) -> Self {
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

redacted_debug!(PreopenedFd);

/// The fd contract of one operation.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

redacted_debug!(OperationFds);

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

/// The trusted implementation that answers one operation.
///
/// Both variants name a declared `Provider` plus an identity inside that
/// provider's contract. Neither names a resource that a mutable row could
/// use to introduce code: the provider is pinned to a trusted artifact at
/// deployment, and the component and method are resolved from that
/// provider's declared implementation set.
#[derive(
    Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase", deny_unknown_fields, tag = "kind")]
pub enum OperationImplementation {
    /// A method of one declared provider component.
    ProviderMethod {
        /// The declared Provider that owns the component.
        provider: ResourceRef,
        /// The component identity inside that provider.
        component: BoundedToken,
        /// The method identity on that component.
        method: BoundedToken,
    },
    /// A trusted executable template owned by a declared provider.
    TrustedExecutableTemplate {
        /// The declared Provider that owns the template.
        provider: ResourceRef,
        /// The template identity inside that provider.
        template: BoundedToken,
    },
}

impl OperationImplementation {
    /// Bind a provider component method.
    ///
    /// # Errors
    ///
    /// Returns `UntrustedImplementation` unless the reference names a
    /// `Provider`. A `Command`, a `Process`, or any other ResourceType is
    /// refused: executable declaration is provider-owned contract data, not
    /// a resource relationship.
    pub fn provider_method(
        provider: ResourceRef,
        component: BoundedToken,
        method: BoundedToken,
    ) -> Result<Self, OperationContractError> {
        if provider.resource_type().as_str() != PROVIDER_RESOURCE_TYPE {
            return Err(OperationContractError::UntrustedImplementation);
        }
        Ok(Self::ProviderMethod {
            provider,
            component,
            method,
        })
    }

    /// Bind a trusted executable template owned by a declared provider.
    ///
    /// # Errors
    ///
    /// Returns `UntrustedImplementation` unless the reference names a
    /// `Provider`.
    pub fn trusted_executable_template(
        provider: ResourceRef,
        template: BoundedToken,
    ) -> Result<Self, OperationContractError> {
        if provider.resource_type().as_str() != PROVIDER_RESOURCE_TYPE {
            return Err(OperationContractError::UntrustedImplementation);
        }
        Ok(Self::TrustedExecutableTemplate { provider, template })
    }

    /// The declared Provider that owns this implementation.
    pub const fn provider(&self) -> &ResourceRef {
        match self {
            Self::ProviderMethod { provider, .. } | Self::TrustedExecutableTemplate { provider, .. } => provider,
        }
    }

    /// Whether this implementation runs as a provider component method.
    pub const fn is_provider_method(&self) -> bool {
        matches!(self, Self::ProviderMethod { .. })
    }
}

redacted_debug!(OperationImplementation);

/// The complete callable declaration of one operation.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CallableOperation {
    implementation: OperationImplementation,
    payload_schema: PayloadSchema,
    #[serde(skip_serializing_if = "Option::is_none")]
    result_schema: Option<PayloadSchema>,
    destructive: bool,
    secret_access: SecretAccess,
    audit: OperationAudit,
    #[serde(skip_serializing_if = "Option::is_none")]
    audit_join: Option<AuditJoin>,
    authority: OperationAuthority,
    fds: OperationFds,
    bounds: OperationBounds,
    payload_provenance: PayloadProvenance,
}

impl CallableOperation {
    /// Construct an operation after checking every facet invariant.
    /// # Errors
    ///
    /// Returns `InvalidAuditJoin` when the join names an undeclared or
    /// secret payload field, `SecretAccessBelowPayload` when the payload
    /// declares secret material without secret access, and
    /// `WriteOnlyRetainedField` when the audit retains a write-only field.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        implementation: OperationImplementation,
        payload_schema: PayloadSchema,
        result_schema: Option<PayloadSchema>,
        destructive: bool,
        secret_access: SecretAccess,
        audit: OperationAudit,
        audit_join: Option<AuditJoin>,
        authority: OperationAuthority,
        fds: OperationFds,
        bounds: OperationBounds,
        payload_provenance: PayloadProvenance,
    ) -> Result<Self, OperationContractError> {
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
            implementation,
            payload_schema,
            result_schema,
            destructive,
            secret_access,
            audit,
            audit_join,
            authority,
            fds,
            bounds,
            payload_provenance,
        })
    }

    /// The trusted implementation that answers this operation.
    pub const fn implementation(&self) -> &OperationImplementation {
        &self.implementation
    }

    /// The payload contract.
    pub const fn payload_schema(&self) -> &PayloadSchema {
        &self.payload_schema
    }

    /// The result contract, when the operation declares one.
    pub const fn result_schema(&self) -> Option<&PayloadSchema> {
        self.result_schema.as_ref()
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
    pub const fn audit(&self) -> &OperationAudit {
        &self.audit
    }

    /// The audit-join facet, when the operation has a durable logical identity.
    pub const fn audit_join(&self) -> Option<&AuditJoin> {
        self.audit_join.as_ref()
    }

    /// The authority facet.
    pub const fn authority(&self) -> &OperationAuthority {
        &self.authority
    }

    /// The fd contract.
    pub const fn fds(&self) -> &OperationFds {
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
}

redacted_debug!(CallableOperation);

wire_deserialize!(
    CallableOperation,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        implementation: OperationImplementation,
        payload_schema: PayloadSchema,
        #[serde(default)]
        result_schema: Option<PayloadSchema>,
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
    },
    wire,
    CallableOperation::new(
        wire.implementation,
        wire.payload_schema,
        wire.result_schema,
        wire.destructive,
        wire.secret_access,
        wire.audit,
        wire.audit_join,
        wire.authority,
        wire.fds,
        wire.bounds,
        wire.payload_provenance,
    )
    .map_err(serde::de::Error::custom)
);

/// One invalid operation declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationContractError {
    /// The implementation reference does not name a declared `Provider`.
    UntrustedImplementation,
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
            Self::UntrustedImplementation => {
                "operation implementation must name a declared Provider"
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
            vec![BoundedText::parse("target").expect("field")],
            vec![BoundedText::parse("token").expect("field")],
            BoundedToken::parse("opaque-target").expect("token"),
        )
        .expect("audit facet")
    }

    fn authority() -> OperationAuthority {
        OperationAuthority::new(
            OperationSurface::Broker,
            OperationDomain::Host,
            BoundedText::parse("host-operator").expect("authority"),
            BrokerRequirement::Yes,
        )
    }

    fn provider_method() -> OperationImplementation {
        OperationImplementation::provider_method(
            ResourceRef::parse("Provider/volume-virtiofs").expect("provider"),
            BoundedToken::parse("controller").expect("component"),
            BoundedToken::parse("serve-view").expect("method"),
        )
        .expect("declared implementation")
    }

    fn operation(implementation: OperationImplementation) -> Result<CallableOperation, OperationContractError> {
        CallableOperation::new(
            implementation,
            payload(),
            None,
            false,
            SecretAccess::ReadWrite,
            audit(),
            Some(AuditJoin::new(vec![BoundedText::parse("target").expect("field")]).expect("join")),
            authority(),
            OperationFds::default(),
            OperationBounds::default(),
            PayloadProvenance::Request,
        )
    }

    #[test]
    fn an_operation_binds_one_declared_implementation() {
        let declared = operation(provider_method()).expect("operation validates");
        assert!(declared.implementation().is_provider_method());
        assert_eq!(
            declared.implementation().provider().to_canonical_string(),
            "Provider/volume-virtiofs"
        );
    }

    #[test]
    fn a_non_provider_row_is_not_an_implementation() {
        assert_eq!(
            OperationImplementation::provider_method(
                ResourceRef::parse("Role/worker").expect("role"),
                BoundedToken::parse("controller").expect("component"),
                BoundedToken::parse("serve-view").expect("method"),
            ),
            Err(OperationContractError::UntrustedImplementation)
        );
        assert_eq!(
            OperationImplementation::trusted_executable_template(
                ResourceRef::parse("Process/web").expect("process"),
                BoundedToken::parse("shell").expect("template"),
            ),
            Err(OperationContractError::UntrustedImplementation)
        );
    }

    #[test]
    fn a_trusted_executable_template_is_a_provider_owned_declaration() {
        let template = OperationImplementation::trusted_executable_template(
            ResourceRef::parse("Provider/activation-nixos").expect("provider"),
            BoundedToken::parse("activation-worker").expect("template"),
        )
        .expect("declared template");
        assert!(!template.is_provider_method());
        assert_eq!(
            template.provider().to_canonical_string(),
            "Provider/activation-nixos"
        );
    }

    #[test]
    fn a_row_carrying_the_retired_owner_reference_is_rejected() {
        let json = json!({
            "ownerRef": "Command/virtiofsd-worker",
            "payloadSchema": payload(),
            "destructive": false,
            "secretAccess": "read-write",
            "audit": {
                "required": true,
                "mode": "yes",
                "retainedFields": [],
                "redactionKeys": [],
                "targetLabel": "opaque-target"
            },
            "authority": {
                "surface": "broker",
                "domain": "host",
                "callerAuthority": "host-operator",
                "brokerRequirement": "yes"
            },
            "payloadProvenance": "request"
        });
        assert!(serde_json::from_value::<CallableOperation>(json).is_err());
    }

    #[test]
    fn a_row_carrying_an_inherited_wire_tag_is_rejected() {
        let mut json = serde_json::to_value(operation(provider_method()).expect("operation"))
            .expect("render");
        json.as_object_mut()
            .expect("object")
            .insert("wireTag".to_owned(), json!(7));
        assert!(serde_json::from_value::<CallableOperation>(json).is_err());
    }

    #[test]
    fn a_wire_round_trip_preserves_the_declared_implementation() {
        let declared = operation(provider_method()).expect("operation validates");
        let bytes = crate::v3::resource_schema::canonical_json_bytes(&declared)
            .expect("canonical bytes");
        let decoded: CallableOperation = serde_json::from_slice(&bytes).expect("decode");
        assert_eq!(decoded, declared);
    }
}
