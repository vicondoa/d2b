//! The `ExecutionPolicy` type's driver declaration and its decode boundary.

use d2b_contracts_resource::v3::ExecutionPolicySpec;
use d2b_resource_types::{DriverDescriptor, WellKnownType, metadata_descriptor};

/// The `ExecutionPolicy` type's driver declaration.
///
/// The type realizes no target-local state: a policy row is confinement the
/// execution path reads when it admits an instance, so the shared
/// declaration-only metadata driver of `d2b-resource-runtime` is the whole
/// conversion and the only fact this crate owns is the type's identity.
pub fn execution_policy_descriptor() -> DriverDescriptor {
    metadata_descriptor(WellKnownType::EXECUTION_POLICY)
}

/// Why one committed `ExecutionPolicy` row was not read as the contract.
///
/// A refusal here is never "decoded with the field ignored": the wire mirror
/// denies unknown fields, so a row that still carries a host path, a mount, a
/// device node, or any other independently granted access stops at decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyRowError {
    /// The committed bytes are not a canonical `ExecutionPolicy` row: the
    /// row is malformed, carries a field the contract does not define, or
    /// violates one of the contract's frozen bounds.
    Rejected(String),
}

impl core::fmt::Display for PolicyRowError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Rejected(reason) => {
                write!(formatter, "the committed row is not an ExecutionPolicy row: {reason}")
            }
        }
    }
}

impl std::error::Error for PolicyRowError {}

/// Read one committed `ExecutionPolicy` row as the canonical contract.
///
/// The decode is the contract's own closed mirror, so a row that carries a
/// field this resource does not define is refused rather than decoded with
/// that field dropped. Dropping it would be the failure this contract exists
/// to prevent: a deployment that cannot honor a declared restriction is
/// refused at admission, and a row that cannot express its intent does not
/// decode at all.
///
/// # Errors
///
/// Returns [`PolicyRowError::Rejected`] when the bytes are not a canonical
/// `ExecutionPolicy` row.
pub fn decode_policy_row(bytes: &[u8]) -> Result<ExecutionPolicySpec, PolicyRowError> {
    serde_json::from_slice(bytes).map_err(|error| PolicyRowError::Rejected(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::{
        CapabilityClass, ExecutionPolicySpec, NamespaceClass, PolicyCapabilities, PolicyIdentity,
        PolicyNamespaces, PolicyRoot, PolicySeccomp, ResourceRef,
    };
    use serde_json::json;

    fn seccomp_ref() -> ResourceRef {
        ResourceRef::parse("SeccompProfile/desktop").expect("canonical reference")
    }

    /// The committed bytes read back as the contract's own spec, with every
    /// confinement facet the row declares intact.
    #[test]
    fn a_canonical_row_decodes_into_the_contract_spec() {
        let spec = ExecutionPolicySpec::new(
            PolicyNamespaces::new(vec![NamespaceClass::User, NamespaceClass::Mount])
                .expect("namespace set"),
            PolicyCapabilities::new(vec![CapabilityClass::NetworkBind]).expect("capability ceiling"),
            true,
            PolicyIdentity::new(None, false).expect("identity rules"),
            PolicyRoot::new(true, true),
            PolicySeccomp::new(Some(seccomp_ref())).expect("syscall filter"),
            Some(0o077),
        )
        .expect("policy spec");
        let bytes = serde_json::to_vec(&spec).expect("the row serializes");
        assert_eq!(decode_policy_row(&bytes).expect("the row decodes"), spec);
    }

    /// The retired access-authority fields are rejected at decode rather than
    /// ignored: a row that still grants a host path, a mount, or a device
    /// node never becomes a policy row with that grant dropped.
    #[test]
    fn a_row_carrying_access_authority_is_refused_at_decode() {
        let canonical = serde_json::to_value(policy_row()).expect("the row renders");
        for field in ["mounts", "deviceBinds", "hostPaths", "volumes", "credentials"] {
            let mut row = canonical.clone();
            row.as_object_mut()
                .expect("the row is an object")
                .insert(field.to_owned(), json!([]));
            let error = decode_policy_row(&serde_json::to_vec(&row).expect("the row serializes"))
                .expect_err("the retired field must not decode");
            assert!(
                matches!(&error, PolicyRowError::Rejected(reason) if reason.contains(field)),
                "{field}: {error}"
            );
        }
    }

    /// A policy row that narrows the ceiling is a different row: editing one
    /// restriction changes the admitted confinement, so the decode must not
    /// normalize it away.
    #[test]
    fn two_rows_differing_in_one_restriction_decode_to_different_specs() {
        let mut narrowed = policy_row();
        narrowed
            .as_object_mut()
            .expect("the row is an object")
            .insert("noNewPrivileges".to_owned(), json!(false));
        let baseline = decode_policy_row(&serde_json::to_vec(&policy_row()).expect("serializes"))
            .expect("the baseline row decodes");
        let edited = decode_policy_row(&serde_json::to_vec(&narrowed).expect("serializes"))
            .expect("the narrowed row decodes");
        assert_ne!(baseline, edited);
        assert!(baseline.no_new_privileges());
        assert!(!edited.no_new_privileges());
    }

    /// The canonical row the fixtures build.
    fn policy_row() -> serde_json::Value {
        json!({
            "namespaces": { "classes": ["user", "mount"] },
            "capabilities": { "allowed": ["network-bind"] },
            "noNewPrivileges": true,
            "identity": { "userRef": null, "requireUserNamespace": false },
            "root": { "readOnlyRoot": true, "privateRoot": true },
            "seccomp": { "profileRef": "SeccompProfile/desktop" },
            "umask": 63
        })
    }
}
