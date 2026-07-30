//! The Provider resource conformance kit.
//!
//! Every Provider binds one or more ResourceTypes. D089 requires each
//! `ResourceApiBinding` to implement the exact base spec and status schema
//! version and fingerprint the installed ResourceType contract declares, to
//! accept the canonical minimal valid base spec without a
//! `spec.provider` extension, and to refuse an optional base capability
//! only through its signed capability matrix and the provider-neutral
//! `unsupported-capability` result.
//!
//! This module is the neutral half of that check. It resolves nothing, opens
//! nothing, and mutates nothing: it compares a Provider's declared binding
//! against the installed [`ResourceSchemaContract`] and reports a typed
//! failure. It is deliberately usable by a conformance test, by a Provider's
//! own startup self-check, and by the Provider install admission path without
//! any of them sharing code.

use std::collections::BTreeSet;
use std::fmt;

use d2b_contracts::v3::execution_policy::BoundedToken;
use d2b_contracts::v3::{
    BaseSchemaBinding, ResourceSchemaContract, ResourceSpec, ResourceTypeName,
};

/// The provider-neutral result code a Provider returns when it refuses an
/// optional base capability its signed matrix declares unsupported.
///
/// A Provider never ignores, reinterprets, renames, duplicates, or weakens a
/// base field; refusing through this one code is the only permitted way to
/// decline an optional base capability.
pub const UNSUPPORTED_CAPABILITY_CODE: &str = "unsupported-capability";

/// The largest closed error-code set a Provider may declare.
///
/// The bound exists so a Provider cannot present an unbounded code
/// vocabulary to a status reader; every shipped Provider set is far below
/// it.
pub const MAX_CLOSED_CODE_SET: usize = 64;

/// The longest permitted `code` or `reason` token, frozen by D108.
pub const MAX_CODE_BYTES: usize = 64;

/// Every conformance failure this kit reports.
///
/// The set is closed and each variant renders one stable
/// `^[a-z][a-z0-9-]*$` code. A code never echoes a Provider name, a
/// ResourceType, a field name, a digest, or a schema body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ConformanceError {
    /// A declared closed code set is empty or above its frozen ceiling.
    CodeSetOutOfRange,
    /// A declared code does not match the frozen `^[a-z][a-z0-9-]*$`
    /// grammar or its byte bound.
    CodeGrammarViolation,
    /// A declared closed code set repeats a code.
    CodeSetDuplicate,
    /// A capability appears as both supported and unsupported.
    CapabilityMatrixOverlap,
    /// A capability was refused without being declared unsupported in the
    /// signed matrix.
    CapabilityUndeclared,
    /// The descriptor binds no ResourceType.
    NoResourceTypeBinding,
    /// The descriptor binds one ResourceType more than once.
    DuplicateResourceTypeBinding,
    /// The descriptor binds a ResourceType the Zone has no installed
    /// schema contract for.
    ResourceTypeNotInstalled,
    /// The advertised base spec or status version or fingerprint differs
    /// from the installed contract.
    BaseSchemaMismatch,
    /// The Provider rejected the canonical minimal valid base spec.
    MinimalBaseRejected,
}

impl ConformanceError {
    /// Return the stable lower-kebab code for this failure.
    pub const fn code(self) -> &'static str {
        match self {
            Self::CodeSetOutOfRange => "code-set-out-of-range",
            Self::CodeGrammarViolation => "code-grammar-violation",
            Self::CodeSetDuplicate => "code-set-duplicate",
            Self::CapabilityMatrixOverlap => "capability-matrix-overlap",
            Self::CapabilityUndeclared => "capability-undeclared",
            Self::NoResourceTypeBinding => "no-resource-type-binding",
            Self::DuplicateResourceTypeBinding => "duplicate-resource-type-binding",
            Self::ResourceTypeNotInstalled => "resource-type-not-installed",
            Self::BaseSchemaMismatch => "base-schema-mismatch",
            Self::MinimalBaseRejected => "minimal-base-rejected",
        }
    }

    /// The complete closed code set, for conformance assertions.
    pub const ALL: [Self; 10] = [
        Self::CodeSetOutOfRange,
        Self::CodeGrammarViolation,
        Self::CodeSetDuplicate,
        Self::CapabilityMatrixOverlap,
        Self::CapabilityUndeclared,
        Self::NoResourceTypeBinding,
        Self::DuplicateResourceTypeBinding,
        Self::ResourceTypeNotInstalled,
        Self::BaseSchemaMismatch,
        Self::MinimalBaseRejected,
    ];
}

impl fmt::Display for ConformanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for ConformanceError {}

/// Assert that a Provider's declared error vocabulary is a closed, unique,
/// grammar-conformant code set.
///
/// Every Provider and every shared conformance library in this workspace
/// declares such a set and previously repeated the same assertion in its own
/// test module. This is that check, once.
pub fn check_closed_code_set(codes: &[&str]) -> Result<(), ConformanceError> {
    if codes.is_empty() || codes.len() > MAX_CLOSED_CODE_SET {
        return Err(ConformanceError::CodeSetOutOfRange);
    }
    let mut seen = BTreeSet::new();
    for code in codes {
        if !is_conformant_code(code) {
            return Err(ConformanceError::CodeGrammarViolation);
        }
        if !seen.insert(*code) {
            return Err(ConformanceError::CodeSetDuplicate);
        }
    }
    Ok(())
}

fn is_conformant_code(code: &str) -> bool {
    if code.is_empty() || code.len() > MAX_CODE_BYTES {
        return false;
    }
    let mut bytes = code.bytes();
    let head_ok = matches!(bytes.next(), Some(b'a'..=b'z'));
    let tail_ok =
        bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    head_ok && tail_ok
}

/// What a Provider's signed capability matrix says about one optional base
/// capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CapabilityDisposition {
    /// The Provider implements the capability.
    Supported,
    /// The Provider declares the capability unsupported and may refuse it.
    Unsupported,
    /// The matrix says nothing; the Provider may neither implement nor
    /// refuse it.
    Undeclared,
}

/// A Provider's signed matrix of supported and unsupported optional base
/// capabilities for one ResourceType.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityMatrix {
    supported: BTreeSet<BoundedToken>,
    unsupported: BTreeSet<BoundedToken>,
}

impl CapabilityMatrix {
    /// Declare a matrix.
    ///
    /// A capability named on both sides is rejected: a Provider that both
    /// implements and refuses a capability has no decidable disposition.
    /// An empty matrix is valid and means the Provider implements every
    /// mandatory base field and declines nothing.
    pub fn new(
        supported: impl IntoIterator<Item = BoundedToken>,
        unsupported: impl IntoIterator<Item = BoundedToken>,
    ) -> Result<Self, ConformanceError> {
        let supported: BTreeSet<BoundedToken> = supported.into_iter().collect();
        let unsupported: BTreeSet<BoundedToken> = unsupported.into_iter().collect();
        if supported.intersection(&unsupported).next().is_some() {
            return Err(ConformanceError::CapabilityMatrixOverlap);
        }
        Ok(Self {
            supported,
            unsupported,
        })
    }

    /// Return the declared disposition of one optional base capability.
    pub fn disposition(&self, capability: &BoundedToken) -> CapabilityDisposition {
        if self.supported.contains(capability) {
            CapabilityDisposition::Supported
        } else if self.unsupported.contains(capability) {
            CapabilityDisposition::Unsupported
        } else {
            CapabilityDisposition::Undeclared
        }
    }

    /// Return the provider-neutral refusal code for a capability this
    /// matrix declares unsupported.
    ///
    /// Refusing a capability the matrix does not declare unsupported fails
    /// closed, so a Provider cannot silently decline a base capability it
    /// never advertised.
    pub fn refuse(&self, capability: &BoundedToken) -> Result<&'static str, ConformanceError> {
        match self.disposition(capability) {
            CapabilityDisposition::Unsupported => Ok(UNSUPPORTED_CAPABILITY_CODE),
            CapabilityDisposition::Supported | CapabilityDisposition::Undeclared => {
                Err(ConformanceError::CapabilityUndeclared)
            }
        }
    }
}

/// One `ResourceApiBinding` a Provider declares: the ResourceType it binds,
/// the exact base schema identity it implements, and its signed capability
/// matrix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderResourceTypeBinding {
    resource_type: ResourceTypeName,
    base_binding: BaseSchemaBinding,
    capabilities: CapabilityMatrix,
}

impl ProviderResourceTypeBinding {
    /// Declare a binding.
    pub const fn new(
        resource_type: ResourceTypeName,
        base_binding: BaseSchemaBinding,
        capabilities: CapabilityMatrix,
    ) -> Self {
        Self {
            resource_type,
            base_binding,
            capabilities,
        }
    }

    /// Borrow the bound ResourceType.
    pub const fn resource_type(&self) -> &ResourceTypeName {
        &self.resource_type
    }

    /// Borrow the advertised base schema identity.
    pub const fn base_binding(&self) -> &BaseSchemaBinding {
        &self.base_binding
    }

    /// Borrow the signed capability matrix.
    pub const fn capabilities(&self) -> &CapabilityMatrix {
        &self.capabilities
    }
}

/// Structural conformance: every declared binding resolves exactly one
/// installed ResourceType contract and advertises that contract's exact base
/// spec and status version and fingerprint.
///
/// This is the check that runs before a Provider is admitted, without
/// calling the Provider.
pub fn check_descriptor_conformance(
    bindings: &[ProviderResourceTypeBinding],
    installed: &[ResourceSchemaContract],
) -> Result<(), ConformanceError> {
    if bindings.is_empty() {
        return Err(ConformanceError::NoResourceTypeBinding);
    }
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for binding in bindings {
        if !seen.insert(binding.resource_type().as_str()) {
            return Err(ConformanceError::DuplicateResourceTypeBinding);
        }
        let contract = resolve(installed, binding.resource_type())
            .ok_or(ConformanceError::ResourceTypeNotInstalled)?;
        contract
            .verify_base_binding(binding.base_binding())
            .map_err(|_| ConformanceError::BaseSchemaMismatch)?;
    }
    Ok(())
}

/// Live conformance for one binding: the Provider advertises the installed
/// base schema identity, and the canonical minimal valid base spec is
/// accepted without any `spec.provider` extension.
pub fn check_provider_conformance(
    binding: &ProviderResourceTypeBinding,
    installed: &[ResourceSchemaContract],
    minimal_base_spec: &ResourceSpec,
) -> Result<(), ConformanceError> {
    let contract = resolve(installed, binding.resource_type())
        .ok_or(ConformanceError::ResourceTypeNotInstalled)?;
    contract
        .verify_base_binding(binding.base_binding())
        .map_err(|_| ConformanceError::BaseSchemaMismatch)?;
    contract
        .validate_minimal_base_spec(minimal_base_spec)
        .map_err(|_| ConformanceError::MinimalBaseRejected)?;
    Ok(())
}

fn resolve<'a>(
    installed: &'a [ResourceSchemaContract],
    resource_type: &ResourceTypeName,
) -> Option<&'a ResourceSchemaContract> {
    installed
        .iter()
        .find(|contract| contract.resource_type().as_str() == resource_type.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::{
        BaseSchemaIdentity, ObjectFieldSchema, ResourceSpec, SchemaFingerprint, SchemaVersion,
    };

    fn fingerprint(fill: &str) -> SchemaFingerprint {
        SchemaFingerprint::parse(format!("sha256:{}", fill.repeat(64))).expect("valid fingerprint")
    }

    fn identity(fill: &str) -> BaseSchemaIdentity {
        BaseSchemaIdentity {
            version: SchemaVersion::new(1, 0).expect("valid version"),
            fingerprint: fingerprint(fill),
        }
    }

    fn binding_identity() -> BaseSchemaBinding {
        BaseSchemaBinding {
            spec: identity("1"),
            status: identity("2"),
        }
    }

    fn resource_type() -> ResourceTypeName {
        ResourceTypeName::parse("volume-local.d2bus.org.VolumeExport").expect("valid type")
    }

    fn contract() -> ResourceSchemaContract {
        ResourceSchemaContract::new(
            resource_type(),
            binding_identity(),
            ObjectFieldSchema::new(
                ["providerRef".to_owned(), "capacity".to_owned()],
                ["capacity".to_owned()],
            )
            .expect("valid base spec schema"),
            ObjectFieldSchema::empty(),
            [],
        )
        .expect("valid contract")
    }

    fn token(value: &str) -> BoundedToken {
        BoundedToken::parse(value).expect("valid token")
    }

    fn minimal_spec() -> ResourceSpec {
        let base = d2b_contracts::v3::CanonicalJsonObject::parse(br#"{"capacity":1}"#)
            .expect("valid canonical object");
        ResourceSpec::new(None, None, base, None).expect("valid minimal spec")
    }

    // Validation obligation: conformance test for a new Provider
    // ResourceType schema.
    #[test]
    fn a_new_provider_resource_type_binding_passes_descriptor_and_live_conformance() {
        let installed = [contract()];
        let binding = ProviderResourceTypeBinding::new(
            resource_type(),
            binding_identity(),
            CapabilityMatrix::new([token("snapshot")], [token("shared-write")])
                .expect("valid matrix"),
        );

        assert_eq!(
            check_descriptor_conformance(std::slice::from_ref(&binding), &installed),
            Ok(())
        );
        assert_eq!(
            check_provider_conformance(&binding, &installed, &minimal_spec()),
            Ok(())
        );
    }

    #[test]
    fn a_binding_for_an_uninstalled_resource_type_fails_closed() {
        let other = ResourceTypeName::parse("volume-local.d2bus.org.Other").expect("valid type");
        let binding = ProviderResourceTypeBinding::new(
            other,
            binding_identity(),
            CapabilityMatrix::new([], []).expect("valid matrix"),
        );
        assert_eq!(
            check_descriptor_conformance(&[binding], &[contract()]),
            Err(ConformanceError::ResourceTypeNotInstalled)
        );
    }

    #[test]
    fn a_divergent_base_fingerprint_fails_closed() {
        let binding = ProviderResourceTypeBinding::new(
            resource_type(),
            BaseSchemaBinding {
                spec: identity("3"),
                status: identity("2"),
            },
            CapabilityMatrix::new([], []).expect("valid matrix"),
        );
        assert_eq!(
            check_descriptor_conformance(&[binding], &[contract()]),
            Err(ConformanceError::BaseSchemaMismatch)
        );
    }

    #[test]
    fn a_descriptor_with_no_binding_or_a_repeated_binding_fails_closed() {
        assert_eq!(
            check_descriptor_conformance(&[], &[contract()]),
            Err(ConformanceError::NoResourceTypeBinding)
        );
        let binding = ProviderResourceTypeBinding::new(
            resource_type(),
            binding_identity(),
            CapabilityMatrix::new([], []).expect("valid matrix"),
        );
        assert_eq!(
            check_descriptor_conformance(&[binding.clone(), binding], &[contract()]),
            Err(ConformanceError::DuplicateResourceTypeBinding)
        );
    }

    #[test]
    fn a_provider_extension_is_not_a_minimal_base_spec() {
        let installed = [contract()];
        let binding = ProviderResourceTypeBinding::new(
            resource_type(),
            binding_identity(),
            CapabilityMatrix::new([], []).expect("valid matrix"),
        );
        let base = d2b_contracts::v3::CanonicalJsonObject::parse(br#"{"unknown":1}"#)
            .expect("valid canonical object");
        let spec = ResourceSpec::new(None, None, base, None).expect("valid spec");
        assert_eq!(
            check_provider_conformance(&binding, &installed, &spec),
            Err(ConformanceError::MinimalBaseRejected)
        );
    }

    #[test]
    fn a_capability_is_refusable_only_when_the_matrix_declares_it_unsupported() {
        let matrix = CapabilityMatrix::new([token("snapshot")], [token("shared-write")])
            .expect("valid matrix");
        assert_eq!(
            matrix.disposition(&token("snapshot")),
            CapabilityDisposition::Supported
        );
        assert_eq!(
            matrix.disposition(&token("quota")),
            CapabilityDisposition::Undeclared
        );
        assert_eq!(
            matrix.refuse(&token("shared-write")),
            Ok(UNSUPPORTED_CAPABILITY_CODE)
        );
        assert_eq!(
            matrix.refuse(&token("snapshot")),
            Err(ConformanceError::CapabilityUndeclared)
        );
        assert_eq!(
            matrix.refuse(&token("quota")),
            Err(ConformanceError::CapabilityUndeclared)
        );
    }

    #[test]
    fn a_capability_named_on_both_sides_is_rejected() {
        assert_eq!(
            CapabilityMatrix::new([token("snapshot")], [token("snapshot")]),
            Err(ConformanceError::CapabilityMatrixOverlap)
        );
    }

    #[test]
    fn the_closed_code_set_check_rejects_empty_duplicate_and_malformed_sets() {
        assert_eq!(check_closed_code_set(&["invalid-ticket"]), Ok(()));
        assert_eq!(
            check_closed_code_set(&[]),
            Err(ConformanceError::CodeSetOutOfRange)
        );
        assert_eq!(
            check_closed_code_set(&["a", "a"]),
            Err(ConformanceError::CodeSetDuplicate)
        );
        assert_eq!(
            check_closed_code_set(&["Invalid"]),
            Err(ConformanceError::CodeGrammarViolation)
        );
        assert_eq!(
            check_closed_code_set(&["-leading"]),
            Err(ConformanceError::CodeGrammarViolation)
        );
        assert_eq!(
            check_closed_code_set(&["a".repeat(MAX_CODE_BYTES + 1).as_str()]),
            Err(ConformanceError::CodeGrammarViolation)
        );
    }

    #[test]
    fn every_conformance_code_is_unique_and_matches_the_frozen_grammar() {
        let codes: Vec<&str> = ConformanceError::ALL
            .iter()
            .map(|error| error.code())
            .collect();
        assert_eq!(check_closed_code_set(&codes), Ok(()));
    }
}
