//! Native RBAC RoleBinding contract.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::role::{
    MAX_ROLE_RULE_EXECUTION_REFS, MAX_ROLE_RULE_RESOURCE_NAMES, RoleContractError, RoleRule,
};
use d2b_contracts_resource::v3::{
    CanonicalJsonObject, ResourceRef, ZoneId, execution_policy::redacted_debug,
};

/// Canonical RoleBinding ResourceType name.
pub const ROLE_BINDING_RESOURCE_TYPE: &str = "RoleBinding";
/// Maximum subjects in one RoleBinding.
pub const MAX_ROLE_BINDING_SUBJECTS: usize = 128;
/// Maximum bytes in an external-principal selector.
pub const MAX_EXTERNAL_PRINCIPAL_SELECTOR_BYTES: usize = 512;
/// Maximum resource references in one RoleBinding.
pub const MAX_ROLE_BINDING_RESOURCE_REFS: usize = 64;
/// Maximum Zone references in one RoleBinding.
pub const MAX_ROLE_BINDING_ZONE_REFS: usize = 8;
/// Maximum execution references in one RoleBinding.
pub const MAX_ROLE_BINDING_EXECUTION_REFS: usize = 32;
/// The closed subject vocabulary a RoleBinding may name.
///
/// This is the one declaration of the set: the Nix authoring surface and the
/// generator that projects these rows into it both read this list instead of
/// restating the six types.
pub const BINDABLE_SUBJECT_TYPES: [&str; 6] =
    ["Zone", "User", "Provider", "Host", "Guest", "Process"];

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
    TooManyResourceRefs,
    TooManyZoneRefs,
    TooManyExecutionRefs,
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
            Self::TooManyResourceRefs => "role-binding-resource-ref-bound-exceeded",
            Self::TooManyZoneRefs => "role-binding-zone-ref-bound-exceeded",
            Self::TooManyExecutionRefs => "role-binding-execution-ref-bound-exceeded",
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
                        EmptyAllowedPolicy::Unrestricted,
                    )
                    && narrowing_names_are_subset(
                        narrowed.resource_names(),
                        allowed.resource_names(),
                    )
                    && narrowing_set_is_subset(narrowed.zones(), allowed.zones(), EmptyAllowedPolicy::Deny)
                    && narrowing_set_is_subset(
                        narrowed.execution_refs(),
                        allowed.execution_refs(),
                        EmptyAllowedPolicy::Unrestricted,
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

/// How an empty allowed set is interpreted when checking a narrowing subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmptyAllowedPolicy {
    /// An empty allowed set grants everything.

    Unrestricted,
    /// An empty allowed set grants nothing.
    Deny,
}

fn narrowing_set_is_subset<T: PartialEq>(
    narrowed: &[T],
    allowed: &[T],
    empty_allowed: EmptyAllowedPolicy,
) -> bool {
    if allowed.is_empty() {
        return match empty_allowed {
            EmptyAllowedPolicy::Unrestricted => true,
            EmptyAllowedPolicy::Deny => narrowed.is_empty(),
        };
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

/// Authority that created a relay-bearing binding.
///
/// Mirrors the authorization evaluator's `RelayGrantAuthority` vocabulary
/// (`d2b-resource-api`) as data: `none`, `core-generated`, and
/// `durable-local-admin` are the only spellings the authoring layer admits.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum RelayAuthority {
    None,
    CoreGenerated,
    DurableLocalAdmin,
}

/// Complete RoleBinding desired state.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoleBindingSpec {
    role_ref: ResourceRef,
    subjects: Vec<ResourceRef>,
    external_principal_selector: Option<ExternalPrincipalSelector>,
    scope_narrowing: Option<ScopeNarrowing>,
    resource_refs: Vec<ResourceRef>,
    zone_refs: Vec<ZoneId>,
    execution_refs: Vec<ResourceRef>,
    relay_authority: Option<RelayAuthority>,
}

impl RoleBindingSpec {
    /// Construct a RoleBinding with an empty scope facet.
    pub fn new(
        role_ref: ResourceRef,
        subjects: Vec<ResourceRef>,
        external_principal_selector: Option<ExternalPrincipalSelector>,
        scope_narrowing: Option<ScopeNarrowing>,
    ) -> Result<Self, RoleBindingContractError> {
        Self::with_facets(
            role_ref,
            subjects,
            external_principal_selector,
            scope_narrowing,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        )
    }

    /// Construct a RoleBinding and enforce subject/resource bounds.
    #[allow(clippy::too_many_arguments)]
    pub fn with_facets(
        role_ref: ResourceRef,
        mut subjects: Vec<ResourceRef>,
        external_principal_selector: Option<ExternalPrincipalSelector>,
        scope_narrowing: Option<ScopeNarrowing>,
        resource_refs: Vec<ResourceRef>,
        zone_refs: Vec<ZoneId>,
        execution_refs: Vec<ResourceRef>,
        relay_authority: Option<RelayAuthority>,
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
            !BINDABLE_SUBJECT_TYPES.contains(&reference.resource_type().as_str())
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
        if resource_refs.len() > MAX_ROLE_BINDING_RESOURCE_REFS {
            return Err(RoleBindingContractError::TooManyResourceRefs);
        }
        if zone_refs.len() > MAX_ROLE_BINDING_ZONE_REFS {
            return Err(RoleBindingContractError::TooManyZoneRefs);
        }
        if execution_refs.len() > MAX_ROLE_BINDING_EXECUTION_REFS {
            return Err(RoleBindingContractError::TooManyExecutionRefs);
        }
        Ok(Self {
            role_ref,
            subjects,
            external_principal_selector,
            scope_narrowing,
            resource_refs,
            zone_refs,
            execution_refs,
            relay_authority,
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

    /// Borrow the bound resource selectors.
    pub fn resource_refs(&self) -> &[ResourceRef] {
        &self.resource_refs
    }

    /// Borrow the bound Zone selectors.
    pub fn zone_refs(&self) -> &[ZoneId] {
        &self.zone_refs
    }

    /// Borrow the bound execution selectors.
    pub fn execution_refs(&self) -> &[ResourceRef] {
        &self.execution_refs
    }

    /// Borrow the optional declared relay authority.
    pub const fn relay_authority(&self) -> Option<RelayAuthority> {
        self.relay_authority
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
            #[serde(default)]
            resource_refs: Vec<ResourceRef>,
            #[serde(default)]
            zone_refs: Vec<ZoneId>,
            #[serde(default)]
            execution_refs: Vec<ResourceRef>,
            #[serde(default)]
            relay_authority: Option<RelayAuthority>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::with_facets(
            wire.role_ref,
            wire.subjects,
            wire.external_principal_selector,
            wire.scope_narrowing,
            wire.resource_refs,
            wire.zone_refs,
            wire.execution_refs,
            wire.relay_authority,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::role::{RoleResourceVerb, RoleRule, RoleSpec};
    use d2b_contracts_resource::v3::{ResourceRef, ResourceTypeName};

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

    fn scope_refs(count: usize, resource_type: &str) -> Vec<ResourceRef> {
        (0..count)
            .map(|index| ResourceRef::parse(&format!("{resource_type}/item-{index}")).unwrap())
            .collect()
    }

    fn binding_with_facets(
        resource_refs: Vec<ResourceRef>,
        zone_refs: Vec<ZoneId>,
        execution_refs: Vec<ResourceRef>,
        relay_authority: Option<RelayAuthority>,
    ) -> Result<RoleBindingSpec, RoleBindingContractError> {
        RoleBindingSpec::with_facets(
            ResourceRef::parse("Role/operator").unwrap(),
            vec![ResourceRef::parse("User/alice").unwrap()],
            None,
            None,
            resource_refs,
            zone_refs,
            execution_refs,
            relay_authority,
        )
    }

    #[test]
    fn binding_scope_lists_are_bounded() {
        let binding = binding_with_facets(
            scope_refs(MAX_ROLE_BINDING_RESOURCE_REFS, "Process"),
            vec![ZoneId::parse("dev").unwrap()],
            scope_refs(MAX_ROLE_BINDING_EXECUTION_REFS, "Host"),
            Some(RelayAuthority::CoreGenerated),
        )
        .unwrap();
        assert_eq!(binding.resource_refs().len(), MAX_ROLE_BINDING_RESOURCE_REFS);
        assert_eq!(
            binding.zone_refs(),
            [ZoneId::parse("dev").unwrap()].as_slice()
        );
        assert_eq!(
            binding.execution_refs().len(),
            MAX_ROLE_BINDING_EXECUTION_REFS
        );
        assert_eq!(binding.relay_authority(), Some(RelayAuthority::CoreGenerated));
        assert_eq!(
            binding_with_facets(
                scope_refs(MAX_ROLE_BINDING_RESOURCE_REFS + 1, "Process"),
                Vec::new(),
                Vec::new(),
                None,
            ),
            Err(RoleBindingContractError::TooManyResourceRefs)
        );
        let zones = (0..=MAX_ROLE_BINDING_ZONE_REFS)
            .map(|index| ZoneId::parse(format!("zone-{index}")).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            binding_with_facets(Vec::new(), zones, Vec::new(), None),
            Err(RoleBindingContractError::TooManyZoneRefs)
        );
        assert_eq!(
            binding_with_facets(
                Vec::new(),
                Vec::new(),
                scope_refs(MAX_ROLE_BINDING_EXECUTION_REFS + 1, "Host"),
                None,
            ),
            Err(RoleBindingContractError::TooManyExecutionRefs)
        );
    }

    #[test]
    fn relay_authority_uses_the_authoring_spellings() {
        for (authority, spelling) in [
            (RelayAuthority::None, "\"none\""),
            (RelayAuthority::CoreGenerated, "\"core-generated\""),
            (RelayAuthority::DurableLocalAdmin, "\"durable-local-admin\""),
        ] {
            assert_eq!(serde_json::to_string(&authority).unwrap(), spelling);
            assert_eq!(
                serde_json::from_str::<RelayAuthority>(spelling).unwrap(),
                authority
            );
        }
        assert!(serde_json::from_str::<RelayAuthority>("\"self-asserted\"").is_err());
    }

    #[test]
    fn bindings_with_every_scope_facet_round_trip_through_canonical_json() {
        let json = concat!(
            r#"{"roleRef":"Role/operator","subjects":["User/alice"],"externalPrincipalSelector":null,"#,
            r#""scopeNarrowing":null,"resourceRefs":["Process/worker"],"zoneRefs":["dev"],"#,
            r#""executionRefs":["Host/local"],"relayAuthority":"durable-local-admin"}"#,
        );
        let binding: RoleBindingSpec = serde_json::from_str(json).unwrap();
        assert_eq!(binding.resource_refs().len(), 1);
        assert_eq!(
            binding.zone_refs(),
            [ZoneId::parse("dev").unwrap()].as_slice()
        );
        assert_eq!(binding.execution_refs().len(), 1);
        assert_eq!(
            binding.relay_authority(),
            Some(RelayAuthority::DurableLocalAdmin)
        );
        assert_eq!(serde_json::to_string(&binding).unwrap(), json);
        assert_eq!(
            CanonicalJsonObject::parse(&serde_json::to_vec(&binding).unwrap())
                .unwrap()
                .to_canonical_bytes(),
            CanonicalJsonObject::parse(json.as_bytes())
                .unwrap()
                .to_canonical_bytes()
        );
    }

    #[test]
    fn binding_scope_facets_default_to_empty() {
        let binding: RoleBindingSpec =
            serde_json::from_str(r#"{"roleRef":"Role/operator","subjects":["User/alice"]}"#)
                .unwrap();
        assert!(binding.resource_refs().is_empty());
        assert!(binding.zone_refs().is_empty());
        assert!(binding.execution_refs().is_empty());
        assert_eq!(binding.relay_authority(), None);
    }
}
