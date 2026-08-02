//! Native RBAC RoleBinding contract.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    CanonicalJsonObject, ResourceRef, ZoneId,
    execution_policy::redacted_debug,
    role::{
        MAX_ROLE_RULE_EXECUTION_REFS, MAX_ROLE_RULE_RESOURCE_NAMES, RoleContractError, RoleRule,
    },
};

/// Canonical RoleBinding ResourceType name.
pub const ROLE_BINDING_RESOURCE_TYPE: &str = "RoleBinding";
/// Maximum subjects in one RoleBinding.
pub const MAX_ROLE_BINDING_SUBJECTS: usize = 128;
/// Maximum bytes in an external-principal selector.
pub const MAX_EXTERNAL_PRINCIPAL_SELECTOR_BYTES: usize = 512;

/// RoleBinding schema failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleBindingContractError {
    WrongRoleRef,
    EmptySubjects,
    TooManySubjects,
    DuplicateSubject,
    UnsupportedSubjectType,
    ExternalSelectorTooLarge,
    EmptyExternalSelector,
    ScopeTooLarge,
    ScopeNotSubset,
    Role(RoleContractError),
}

impl core::fmt::Display for RoleBindingContractError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::WrongRoleRef => "role-binding-role-ref-invalid",
            Self::EmptySubjects => "role-binding-subjects-empty",
            Self::TooManySubjects => "role-binding-subject-bound-exceeded",
            Self::DuplicateSubject => "role-binding-duplicate-subject",
            Self::UnsupportedSubjectType => "role-binding-subject-type-invalid",
            Self::ExternalSelectorTooLarge => "role-binding-external-selector-too-large",
            Self::EmptyExternalSelector => "role-binding-external-selector-empty",
            Self::ScopeTooLarge => "role-binding-scope-too-large",
            Self::ScopeNotSubset => "role-binding-scope-not-subset",
            Self::Role(error) => return error.fmt(formatter),
        })
    }
}

impl std::error::Error for RoleBindingContractError {}

impl From<RoleContractError> for RoleBindingContractError {
    fn from(value: RoleContractError) -> Self {
        Self::Role(value)
    }
}

/// Opaque external enrollment selector.  Its canonical JSON contains no
/// credential bytes and is evaluated by the authenticated session layer.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct ExternalPrincipalSelector(CanonicalJsonObject);

impl ExternalPrincipalSelector {
    /// Construct a bounded non-empty selector.
    pub fn new(value: CanonicalJsonObject) -> Result<Self, RoleBindingContractError> {
        let bytes = value.to_canonical_bytes();
        if bytes.len() > MAX_EXTERNAL_PRINCIPAL_SELECTOR_BYTES {
            return Err(RoleBindingContractError::ExternalSelectorTooLarge);
        }
        if value.is_empty() {
            return Err(RoleBindingContractError::EmptyExternalSelector);
        }
        Ok(Self(value))
    }

    /// Borrow the selector for the authenticated identity adapter.
    pub const fn value(&self) -> &CanonicalJsonObject {
        &self.0
    }
}

redacted_debug!(ExternalPrincipalSelector);

impl<'de> Deserialize<'de> for ExternalPrincipalSelector {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(CanonicalJsonObject::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Narrowing applied to a Role for one binding.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScopeNarrowing {
    rules: Vec<RoleRule>,
}

impl ScopeNarrowing {
    /// Construct a bounded narrowing.
    pub fn new(rules: Vec<RoleRule>) -> Result<Self, RoleBindingContractError> {
        if rules.is_empty() || rules.len() > super::role::MAX_ROLE_RULES {
            return Err(RoleBindingContractError::ScopeTooLarge);
        }
        Ok(Self { rules })
    }

    /// Borrow narrowing rules.
    pub fn rules(&self) -> &[RoleRule] {
        &self.rules
    }

    /// Validate that this narrowing does not grant outside a Role.
    pub fn is_subset_of(&self, role: &super::role::RoleSpec) -> bool {
        self.rules.iter().all(|narrowed| {
            role.rules().iter().any(|allowed| {
                narrowed
                    .resource_types()
                    .iter()
                    .all(|item| allowed.resource_types().contains(item))
                    && narrowed
                        .verbs()
                        .iter()
                        .all(|item| allowed.verbs().contains(item))
                    && narrowed
                        .session_verbs()
                        .iter()
                        .all(|item| allowed.session_verbs().contains(item))
                    && narrowing_set_is_subset(
                        narrowed.subresources(),
                        allowed.subresources(),
                        true,
                    )
                    && narrowing_names_are_subset(
                        narrowed.resource_names(),
                        allowed.resource_names(),
                    )
                    && narrowing_set_is_subset(narrowed.zones(), allowed.zones(), false)
                    && narrowing_set_is_subset(
                        narrowed.execution_refs(),
                        allowed.execution_refs(),
                        true,
                    )
            })
        })
    }
}

fn narrowing_names_are_subset(narrowed: &[String], allowed: &[String]) -> bool {
    if allowed.is_empty() {
        return true;
    }
    !narrowed.is_empty()
        && narrowed
            .iter()
            .all(|item| item != "*" && allowed.contains(item))
}

fn narrowing_set_is_subset<T: PartialEq>(
    narrowed: &[T],
    allowed: &[T],
    empty_allowed_is_unrestricted: bool,
) -> bool {
    if allowed.is_empty() {
        return empty_allowed_is_unrestricted || narrowed.is_empty();
    }
    !narrowed.is_empty() && narrowed.iter().all(|item| allowed.contains(item))
}

redacted_debug!(ScopeNarrowing);

impl<'de> Deserialize<'de> for ScopeNarrowing {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            rules: Vec<RoleRule>,
        }
        Self::new(Wire::deserialize(deserializer)?.rules).map_err(serde::de::Error::custom)
    }
}

/// Complete RoleBinding desired state.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoleBindingSpec {
    role_ref: ResourceRef,
    subjects: Vec<ResourceRef>,
    external_principal_selector: Option<ExternalPrincipalSelector>,
    scope_narrowing: Option<ScopeNarrowing>,
}

impl RoleBindingSpec {
    /// Construct a RoleBinding and enforce subject/resource bounds.
    pub fn new(
        role_ref: ResourceRef,
        mut subjects: Vec<ResourceRef>,
        external_principal_selector: Option<ExternalPrincipalSelector>,
        scope_narrowing: Option<ScopeNarrowing>,
    ) -> Result<Self, RoleBindingContractError> {
        if role_ref.resource_type().as_str() != "Role" {
            return Err(RoleBindingContractError::WrongRoleRef);
        }
        if subjects.is_empty() && external_principal_selector.is_none() {
            return Err(RoleBindingContractError::EmptySubjects);
        }
        if subjects.len() > MAX_ROLE_BINDING_SUBJECTS {
            return Err(RoleBindingContractError::TooManySubjects);
        }
        if subjects.iter().any(|reference| {
            !matches!(
                reference.resource_type().as_str(),
                "Zone" | "User" | "Provider" | "Host" | "Guest" | "Process"
            )
        }) {
            return Err(RoleBindingContractError::UnsupportedSubjectType);
        }
        subjects.sort();
        if subjects.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(RoleBindingContractError::DuplicateSubject);
        }
        if scope_narrowing.as_ref().is_some_and(|scope| {
            scope.rules().iter().any(|rule| {
                rule.resource_names().len() > MAX_ROLE_RULE_RESOURCE_NAMES
                    || rule.execution_refs().len() > MAX_ROLE_RULE_EXECUTION_REFS
            })
        }) {
            return Err(RoleBindingContractError::ScopeTooLarge);
        }
        Ok(Self {
            role_ref,
            subjects,
            external_principal_selector,
            scope_narrowing,
        })
    }

    /// Borrow the immutable Role reference.
    pub const fn role_ref(&self) -> &ResourceRef {
        &self.role_ref
    }

    /// Borrow subjects.
    pub fn subjects(&self) -> &[ResourceRef] {
        &self.subjects
    }

    /// Borrow the optional trusted external selector.
    pub fn external_principal_selector(&self) -> Option<&ExternalPrincipalSelector> {
        self.external_principal_selector.as_ref()
    }

    /// Borrow optional scope narrowing.
    pub fn scope_narrowing(&self) -> Option<&ScopeNarrowing> {
        self.scope_narrowing.as_ref()
    }

    /// Validate the optional narrowing against the referenced Role's rules.
    pub fn validate_scope_against_role(
        &self,
        role: &super::role::RoleSpec,
    ) -> Result<(), RoleBindingContractError> {
        if self
            .scope_narrowing
            .as_ref()
            .is_some_and(|narrowing| !narrowing.is_subset_of(role))
        {
            Err(RoleBindingContractError::ScopeNotSubset)
        } else {
            Ok(())
        }
    }

    /// Validate that all refs resolve in one Zone and that a Zone subject is
    /// the local self-resource.
    pub fn validate_zone(&self, zone: &ZoneId) -> Result<(), RoleBindingContractError> {
        if self.subjects.iter().any(|reference| {
            reference.resource_type().as_str() == "Zone"
                && reference.name().as_str() != zone.as_str()
        }) {
            return Err(RoleBindingContractError::UnsupportedSubjectType);
        }
        Ok(())
    }
}

redacted_debug!(RoleBindingSpec);

impl<'de> Deserialize<'de> for RoleBindingSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            role_ref: ResourceRef,
            #[serde(default)]
            subjects: Vec<ResourceRef>,
            #[serde(default)]
            external_principal_selector: Option<ExternalPrincipalSelector>,
            #[serde(default)]
            scope_narrowing: Option<ScopeNarrowing>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.role_ref,
            wire.subjects,
            wire.external_principal_selector,
            wire.scope_narrowing,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Closed RoleBinding condition names.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum RoleBindingConditionType {
    RoleResolved,
    SubjectNotFound,
    SubjectIdentityChanged,
    IndexBuilt,
    ExternalPrincipalResolved,
    Revoked,
}

/// Identity-free RoleBinding status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleBindingStatusResource {
    role_resolved: bool,
    subject_count: u32,
    unresolved_subject_count: u32,
    revoked: bool,
}

impl RoleBindingStatusResource {
    /// Construct the status projection.
    pub const fn new(
        role_resolved: bool,
        subject_count: u32,
        unresolved_subject_count: u32,
        revoked: bool,
    ) -> Self {
        Self {
            role_resolved,
            subject_count,
            unresolved_subject_count,
            revoked,
        }
    }

    /// Whether the role currently resolves.
    pub const fn role_resolved(&self) -> bool {
        self.role_resolved
    }

    /// Number of authored subjects.
    pub const fn subject_count(&self) -> u32 {
        self.subject_count
    }

    /// Number of unresolved subjects.
    pub const fn unresolved_subject_count(&self) -> u32 {
        self.unresolved_subject_count
    }

    /// Whether the binding is revoked.
    pub const fn revoked(&self) -> bool {
        self.revoked
    }
}

/// Alias used by generic status adapters.
pub type RoleBindingStatus = RoleBindingStatusResource;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::{
        ResourceTypeName,
        role::{RoleResourceVerb, RoleRule, RoleSpec},
    };

    #[test]
    fn binding_requires_role_and_bounded_subjects() {
        let role = ResourceRef::parse("Role/operator").unwrap();
        let subject = ResourceRef::parse("User/alice").unwrap();
        let binding = RoleBindingSpec::new(role, vec![subject], None, None).unwrap();
        assert_eq!(binding.subjects().len(), 1);
        assert!(
            RoleBindingSpec::new(
                ResourceRef::parse("Provider/system-core").unwrap(),
                Vec::new(),
                None,
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn duplicate_subjects_are_rejected() {
        let role = ResourceRef::parse("Role/operator").unwrap();
        let subject = ResourceRef::parse("User/alice").unwrap();
        assert_eq!(
            RoleBindingSpec::new(role, vec![subject.clone(), subject], None, None),
            Err(RoleBindingContractError::DuplicateSubject)
        );
    }

    #[test]
    fn scope_narrowing_is_a_subset_operation() {
        let allowed = RoleSpec::new(vec![
            RoleRule::new(
                vec![ResourceTypeName::parse("Process").unwrap()],
                vec![RoleResourceVerb::Get],
                vec![],
                vec!["worker".into()],
                vec![],
                vec![],
                vec![],
            )
            .unwrap(),
        ])
        .unwrap();
        let narrower = ScopeNarrowing::new(vec![
            RoleRule::new(
                vec![ResourceTypeName::parse("Process").unwrap()],
                vec![RoleResourceVerb::Get],
                vec![],
                vec!["worker".into()],
                vec![],
                vec![],
                vec![],
            )
            .unwrap(),
        ])
        .unwrap();
        assert!(narrower.is_subset_of(&allowed));
    }
}
