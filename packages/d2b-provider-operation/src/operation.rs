//! The `Operation` resource type's surfaces.
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
#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::PayloadSchema;
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

    /// The canonical `Operation` is assembled from the contract's own facet
    /// types, so a drift between this crate and the contract is not
    /// expressible: the compiler rejects it.
    #[test]
    fn the_canonical_operation_is_built_from_the_canonical_facets() {
        let canonical = CallableOperation::new(
            OperationImplementation::provider_method(
                ResourceRef::parse("Provider/system-minijail").unwrap(),
                BoundedToken::parse("volume-binding").unwrap(),
                BoundedToken::parse("export").unwrap(),
            )
            .expect("a declared provider method"),
            payload(),
            None,
            true,
            SecretAccess::ReadWrite,
            audit(),
            Some(
                AuditJoin::new(vec![BoundedText::parse("target").unwrap()])
                    .expect("audit join facet"),
            ),
            authority(),
            OperationFds::default(),
            OperationBounds::default(),
            PayloadProvenance::Derived,
        )
        .expect("the canonical operation validates");
        assert_eq!(
            canonical.audit().mode(),
            AuditMode::Yes,
            "one audit facet, one definition"
        );
        assert_eq!(canonical.secret_access(), SecretAccess::ReadWrite);
        assert!(canonical.implementation().is_provider_method());
    }

    /// The canonical contract refuses an implementation that names anything
    /// other than a declared `Provider`: executable declaration is
    /// provider-owned contract data, not a resource relationship.
    #[test]
    fn the_canonical_operation_refuses_a_non_provider_implementation() {
        let error = OperationImplementation::provider_method(
            ResourceRef::parse("Role/worker").unwrap(),
            BoundedToken::parse("volume-binding").unwrap(),
            BoundedToken::parse("export").unwrap(),
        )
        .expect_err("a Role is not a declared provider");
        assert_eq!(error, OperationContractError::UntrustedImplementation);
        CallableOperation::new(
            OperationImplementation::trusted_executable_template(
                ResourceRef::parse("Provider/system-minijail").unwrap(),
                BoundedToken::parse("virtiofsd").unwrap(),
            )
            .expect("a declared provider template"),
            payload(),
            None,
            false,
            SecretAccess::ReadWrite,
            audit(),
            None,
            authority(),
            OperationFds::default(),
            OperationBounds::default(),
            PayloadProvenance::Derived,
        )
        .expect("a provider-owned template is the canonical shape");
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

    /// The canonical contract's row is closed in the same direction: the
    /// retired `ownerRef` and `wireTag` are refused outright rather than
    /// decoded with their authority dropped.
    #[test]
    fn the_canonical_row_refuses_the_retired_facets() {
        let mut row =
            serde_json::to_value(CallableOperation::new(
                OperationImplementation::provider_method(
                    ResourceRef::parse("Provider/system-minijail").unwrap(),
                    BoundedToken::parse("volume-binding").unwrap(),
                    BoundedToken::parse("export").unwrap(),
                )
                .expect("a declared provider method"),
                payload(),
                None,
                false,
                SecretAccess::ReadWrite,
                audit(),
                None,
                authority(),
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
