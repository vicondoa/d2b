//! The `Operation` resource type's surfaces, and the one retired row shape
//! that still has a caller.
//!
//! An `Operation` row is the single externally callable contract: its
//! payload and result schemas, the authority, audit, descriptor-carriage, and
//! bounds facets, its payload provenance, and - the part that makes it
//! executable - the trusted implementation that answers it. Every one of
//! those facets is the canonical contract in
//! [`d2b_contracts_resource::v3::operation`]; this module re-exports them
//! rather than restating them, because a second copy of an operation facet
//! here could drift from the contract the graph publishes and the broker
//! dispatches, and the drift would be invisible at every call site.
//!
//! What remains local is [`OperationSpec`]: the pre-cutover row a
//! `Command` materializes, carrying an `ownerRef` and an inherited wire
//! discriminant. The canonical contract deliberately has no such row - an
//! operation's implementation names a declared `Provider` method or trusted
//! executable template ([`OperationImplementation`]) - so this shape has no
//! successor. The only caller left is the foundation seed, which still
//! publishes the system zone's pre-cutover policy rows; the cutover deletes
//! this type together with that materialization.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use d2b_contracts_resource::v3::PayloadSchema;
use d2b_contracts_resource::v3::execution_policy::redacted_debug;
pub use d2b_contracts_resource::v3::{
    AuditJoin, AuditMode, BrokerRequirement, CallableOperation, FdContract, FdKind,
    MAX_OPERATION_AUDIT_FIELDS, MAX_OPERATION_BATCH_ENTRIES, MAX_OPERATION_FDS,
    MAX_OPERATION_JOIN_FIELDS, MAX_OPERATION_PAYLOAD_BYTES, MAX_OPERATION_REDACTION_KEYS,
    MAX_OPERATION_STREAM_CREDITS, OPERATION_RESOURCE_TYPE, OperationAudit, OperationAuthority,
    OperationBounds, OperationContractError, OperationDomain, OperationFds, OperationImplementation,
    OperationSurface, PayloadProvenance, PreopenedFd, ResourceRef, SecretAccess,
};

#[cfg(test)]
use d2b_contracts_resource::v3::execution_policy::{BoundedText, BoundedToken};

/// The `Operation` desired spec.
//
// The retirement note lives in the module documentation rather than here on
// purpose: this doc comment is the schema's own `description`, and the
// committed schema is generated from it.
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
    /// Construct the retired materialized row after checking its invariants.
    ///
    /// # Errors
    ///
    /// Returns [`MaterializedOperationError::InvalidOwnerRef`] when the owner
    /// reference does not name a `Command`,
    /// [`MaterializedOperationError::InheritedWireTagOnMaterialized`] when a
    /// materialized operation carries a wire tag,
    /// [`MaterializedOperationError::InvalidAuditJoin`] when the join names
    /// an undeclared or secret payload field,
    /// [`MaterializedOperationError::SecretAccessBelowPayload`] when the
    /// payload declares secret material without secret access, and
    /// [`MaterializedOperationError::WriteOnlyRetainedField`] when the audit
    /// retains a write-only field. Every other bound is the canonical facet's
    /// own and is reported by that facet's constructor.
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
    ) -> Result<Self, MaterializedOperationError> {
            if let Some(owner_ref) = &owner_ref {
            d2b_contracts_resource::v3::execution_policy::require_resource_type(
                owner_ref,
                "Command",
            )
            .map_err(|_| MaterializedOperationError::InvalidOwnerRef)?;
            if wire_tag.is_some() {
                return Err(MaterializedOperationError::InheritedWireTagOnMaterialized);
            }
        }
        if let Some(join) = &audit_join {
            for field in join.fields() {
                let Some(name) = field.as_str().split(':').next() else {
                    return Err(MaterializedOperationError::InvalidAuditJoin);
                };
                if !payload_schema.declares(name) || payload_schema.is_write_only(name) {
                    return Err(MaterializedOperationError::InvalidAuditJoin);
                }
            }
        }
        if secret_access == SecretAccess::None
            && payload_schema
                .property_names()
                .any(|name| payload_schema.is_write_only(name))
        {
            return Err(MaterializedOperationError::SecretAccessBelowPayload);
        }
        for field in audit.retained_fields() {
            let name = field.as_str().split(':').next().unwrap_or_default();
            if payload_schema.is_write_only(name) {
                return Err(MaterializedOperationError::WriteOnlyRetainedField);
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

    /// The owning `Command` reference of a materialized operation.
    pub const fn owner_ref(&self) -> Option<&ResourceRef> {
        self.owner_ref.as_ref()
    }

    /// The payload contract.
    pub const fn payload_schema(&self) -> &PayloadSchema {
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

/// One invalid retired materialized-operation declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterializedOperationError {
    /// The owner reference does not name a `Command`.
    InvalidOwnerRef,
    /// A materialized operation carries an inherited wire discriminant.
    InheritedWireTagOnMaterialized,
    /// The audit-join facet is empty, over bound, or names an undeclared or
    /// secret payload field.
    InvalidAuditJoin,
    /// The payload declares secret material but the operation claims no
    /// secret access.
    SecretAccessBelowPayload,
    /// The audit target label names a write-only payload field.
    WriteOnlyRetainedField,
}

impl core::fmt::Display for MaterializedOperationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            Self::InvalidOwnerRef => "operation ownerRef does not name a Command",
            Self::InheritedWireTagOnMaterialized => {
                "materialized operation carries an inherited wire tag"
            }
            Self::InvalidAuditJoin => "operation auditJoin names an undeclared or secret field",
            Self::SecretAccessBelowPayload => {
                "operation payload declares secret material but secretAccess is none"
            }
            Self::WriteOnlyRetainedField => "operation audit target label is a secret field",
        };
        formatter.write_str(text)
    }
}

impl std::error::Error for MaterializedOperationError {}

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

    fn spec(secret_access: SecretAccess) -> Result<OperationSpec, MaterializedOperationError> {
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

    /// The facets the retired row is built from are the canonical contract's
    /// own types, so a drift between this crate and the contract is not
    /// expressible: the compiler rejects it.
    #[test]
    fn the_retired_row_is_built_from_the_canonical_facets() {
        let spec = spec(SecretAccess::ReadWrite).expect("the retired row validates");
        let canonical = CallableOperation::new(
            OperationImplementation::provider_method(
                ResourceRef::parse("Provider/system-minijail").unwrap(),
                BoundedToken::parse("volume-binding").unwrap(),
                BoundedToken::parse("export").unwrap(),
            )
            .expect("a declared provider method"),
            spec.payload_schema().clone(),
            None,
            spec.destructive(),
            spec.secret_access(),
            spec.audit().clone(),
            spec.audit_join().cloned(),
            spec.authority().clone(),
            spec.fds().clone(),
            *spec.bounds(),
            spec.payload_provenance(),
        )
        .expect("the canonical operation validates");
        assert_eq!(
            canonical.audit().mode(),
            AuditMode::Yes,
            "one audit facet, one definition"
        );
        assert_eq!(canonical.secret_access(), SecretAccess::ReadWrite);
        assert!(canonical.implementation().is_provider_method());
        assert_eq!(
            spec.owner_ref().map(ResourceRef::to_canonical_string),
            Some("Command/virtiofsd-worker".to_owned())
        );
    }

    /// The canonical contract refuses an implementation that names anything
    /// other than a declared `Provider`: executable declaration is
    /// provider-owned contract data, not a resource relationship.
    #[test]
    fn the_canonical_operation_refuses_a_non_provider_implementation() {
        let spec = spec(SecretAccess::ReadWrite).expect("the retired row validates");
        let error = OperationImplementation::provider_method(
            ResourceRef::parse("Command/virtiofsd-worker").unwrap(),
            BoundedToken::parse("volume-binding").unwrap(),
            BoundedToken::parse("export").unwrap(),
        )
        .expect_err("a Command is not a declared provider");
        assert_eq!(error, OperationContractError::UntrustedImplementation);
        CallableOperation::new(
            OperationImplementation::trusted_executable_template(
                ResourceRef::parse("Provider/system-minijail").unwrap(),
                BoundedToken::parse("virtiofsd").unwrap(),
            )
            .expect("a declared provider template"),
            spec.payload_schema().clone(),
            None,
            false,
            spec.secret_access(),
            spec.audit().clone(),
            None,
            spec.authority().clone(),
            OperationFds::default(),
            OperationBounds::default(),
            PayloadProvenance::Derived,
        )
        .expect("a provider-owned template is the canonical shape");
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
        assert_eq!(error, MaterializedOperationError::InheritedWireTagOnMaterialized);
    }

    #[test]
    fn secret_bearing_payloads_require_a_secret_access_ceiling() {
        assert_eq!(
            spec(SecretAccess::None).expect_err("secret access below payload"),
            MaterializedOperationError::SecretAccessBelowPayload
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
            assert_eq!(error, MaterializedOperationError::InvalidAuditJoin, "{field}");
        }
    }

    /// The facet bounds the canonical contract owns are still enforced, by the
    /// canonical constructors: a bound that is not this crate's is not
    /// restated here.
    #[test]
    fn the_canonical_facet_bounds_are_still_enforced() {
        assert!(OperationBounds::new(0, 1, 1).is_err());
        assert!(OperationBounds::new(MAX_OPERATION_PAYLOAD_BYTES + 1, 1, 1).is_err());
        assert!(OperationBounds::new(1, MAX_OPERATION_BATCH_ENTRIES + 1, 1).is_err());
        assert!(OperationBounds::new(1, 1, MAX_OPERATION_STREAM_CREDITS + 1).is_err());
        let fields = (0..=MAX_OPERATION_AUDIT_FIELDS)
            .map(|index| BoundedText::parse(format!("field-{index}")).unwrap())
            .collect();
        assert!(
            OperationAudit::new(
                true,
                AuditMode::Yes,
                fields,
                Vec::new(),
                BoundedToken::parse("opaque-target").unwrap(),
            )
            .is_err()
        );
        let contracts: Vec<FdContract> = (0..=MAX_OPERATION_FDS)
            .map(|index| {
                FdContract::new(
                    BoundedToken::parse(format!("fd-{index}")).unwrap(),
                    FdKind::File,
                    true,
                )
            })
            .collect();
        assert!(OperationFds::new(contracts, Vec::new(), Vec::new()).is_err());
        let preopened: Vec<PreopenedFd> = (0..=MAX_OPERATION_FDS)
            .map(|index| {
                PreopenedFd::new(
                    BoundedToken::parse(format!("fd-{index}")).unwrap(),
                    BoundedToken::parse("open").unwrap(),
                )
            })
            .collect();
        assert!(OperationFds::new(Vec::new(), Vec::new(), preopened).is_err());
        assert!(AuditJoin::new(Vec::new()).is_err());
        let over = (0..=MAX_OPERATION_JOIN_FIELDS)
            .map(|index| BoundedText::parse(format!("field-{index}")).unwrap())
            .collect();
        assert!(AuditJoin::new(over).is_err());
    }

    /// The retired row's wire shape is unchanged, so the pre-cutover rows the
    /// foundation seed still publishes keep decoding byte-for-byte.
    #[test]
    fn the_retired_rows_wire_shape_round_trips_and_stays_closed() {
        let spec = spec(SecretAccess::RedactedOnly).expect("the retired row validates");
        let bytes =
            d2b_contracts_resource::v3::resource_schema::canonical_json_bytes(&spec).expect("canonical");
        let restored: OperationSpec = serde_json::from_slice(&bytes).expect("parses");
        assert_eq!(restored, spec);
        let text = String::from_utf8(bytes).expect("utf8");
        assert!(text.contains("\"secretAccess\":\"redacted-only\""));
        assert!(text.contains("\"payloadProvenance\":\"request\""));
        assert!(!text.contains("wireTag"));
        assert!(!text.contains("implementation"));
    }

    /// The canonical contract's row is closed in the same direction: the
    /// retired `ownerRef` and `wireTag` are refused outright rather than
    /// decoded with their authority dropped.
    #[test]
    fn the_canonical_row_refuses_the_retired_facets() {
        let spec = spec(SecretAccess::ReadWrite).expect("the retired row validates");
        let mut row =
            serde_json::to_value(CallableOperation::new(
                OperationImplementation::provider_method(
                    ResourceRef::parse("Provider/system-minijail").unwrap(),
                    BoundedToken::parse("volume-binding").unwrap(),
                    BoundedToken::parse("export").unwrap(),
                )
                .expect("a declared provider method"),
                spec.payload_schema().clone(),
                None,
                false,
                SecretAccess::ReadWrite,
                spec.audit().clone(),
                None,
                spec.authority().clone(),
                OperationFds::default(),
                OperationBounds::default(),
                PayloadProvenance::Request,
            )
            .expect("the canonical operation validates"))
            .expect("the row renders");
        for field in ["ownerRef", "wireTag", "commandRefs"] {
            let mut retired = row.clone();
            retired
                .as_object_mut()
                .expect("the row is an object")
                .insert(field.to_owned(), json!("Command/virtiofsd-worker"));
            serde_json::from_value::<CallableOperation>(retired)
                .expect_err("the retired facet must not decode");
        }
        row.as_object_mut()
            .expect("the row is an object")
            .insert("ownerRef".to_owned(), json!("Command/virtiofsd-worker"));
        serde_json::from_value::<CallableOperation>(row)
            .expect_err("a canonical operation carries no command owner reference");
    }
}
