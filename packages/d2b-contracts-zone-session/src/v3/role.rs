//! Native RBAC Role contract.
//!
//! Resource and ComponentSession verbs are intentionally separate closed
//! sets.  In particular, `relay` is transport forwarding authority and can
//! never be smuggled into CRUD by treating all verbs as strings.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    ResourceName, ResourceRef, ResourceTypeName, ZoneId,
    execution_policy::{BoundedText, redacted_debug},
};

/// Canonical Role ResourceType name.
pub const ROLE_RESOURCE_TYPE: &str = "Role";
/// Maximum rules in one Role.
pub const MAX_ROLE_RULES: usize = 32;
/// Maximum ResourceTypes in one rule.
pub const MAX_ROLE_RULE_RESOURCE_TYPES: usize = 16;
/// Maximum resource verbs in one rule.
pub const MAX_ROLE_RULE_VERBS: usize = 16;
/// Maximum session verbs in one rule.
pub const MAX_ROLE_RULE_SESSION_VERBS: usize = 9;
/// Maximum subresource selectors in one rule.
pub const MAX_ROLE_RULE_SUBRESOURCES: usize = 16;
/// Maximum resource-name selectors in one rule.
pub const MAX_ROLE_RULE_RESOURCE_NAMES: usize = 64;
/// Maximum execution references in one rule.
pub const MAX_ROLE_RULE_EXECUTION_REFS: usize = 32;
/// Maximum Zone selectors in one rule.
pub const MAX_ROLE_RULE_ZONES: usize = 8;
/// Core finalizer used while RoleBindings drain.
pub const ROLE_BINDING_DRAIN_FINALIZER: &str = "core.role-binding-drain";

/// Closed resource authorization verbs.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum RoleResourceVerb {
    Get,
    List,
    Watch,
    Create,
    UpdateSpec,
    UpdateStatus,
    UpdateMetadata,
    UpdateFinalizers,
    Delete,
    UseCredential,
    AdminCredential,
}

impl RoleResourceVerb {
    /// Every resource verb in stable order.
    pub const ALL: [Self; 11] = [
        Self::Get,
        Self::List,
        Self::Watch,
        Self::Create,
        Self::UpdateSpec,
        Self::UpdateStatus,
        Self::UpdateMetadata,
        Self::UpdateFinalizers,
        Self::Delete,
        Self::UseCredential,
        Self::AdminCredential,
    ];
}

/// Closed ComponentSession authorization verbs.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum RoleSessionVerb {
    Connect,
    Invoke,
    OpenStream,
    Relay,
    Attach,
    Cancel,
    Observe,
    AuditExport,
    SupportBundle,
}

impl RoleSessionVerb {
    /// Every session verb in stable order.
    pub const ALL: [Self; 9] = [
        Self::Connect,
        Self::Invoke,
        Self::OpenStream,
        Self::Relay,
        Self::Attach,
        Self::Cancel,
        Self::Observe,
        Self::AuditExport,
        Self::SupportBundle,
    ];
}

/// Role validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleContractError {
    EmptyResourceTypes,
    EmptyVerbs,
    BoundExceeded,
    DuplicateEntry,
    InvalidResourceName,
    InvalidExecutionRef,
    InvalidCredentialScope,
    RelayScopeRequired,
    RelayHasResourceVerb,
    RelaySelectorInvalid,
    DiagnosticSelectorInvalid,
    WildcardNotAllowed,
    InvalidWildcard,
}

impl core::fmt::Display for RoleContractError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::EmptyResourceTypes => "role-resource-types-empty",
            Self::EmptyVerbs => "role-verbs-empty",
            Self::BoundExceeded => "role-bound-exceeded",
            Self::DuplicateEntry => "role-duplicate-entry",
            Self::InvalidResourceName => "role-resource-name-invalid",
            Self::InvalidExecutionRef => "role-execution-ref-invalid",
            Self::InvalidCredentialScope => "role-credential-scope-invalid",
            Self::RelayScopeRequired => "role-relay-scope-required",
            Self::RelayHasResourceVerb => "role-relay-resource-verb",
            Self::RelaySelectorInvalid => "role-relay-selector-invalid",
            Self::DiagnosticSelectorInvalid => "role-diagnostic-selector-invalid",
            Self::WildcardNotAllowed => "role-wildcard-not-allowed",
            Self::InvalidWildcard => "role-wildcard-invalid",
        })
    }
}

impl std::error::Error for RoleContractError {}

/// One exact Role rule.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoleRule {
    resource_types: Vec<ResourceTypeName>,
    verbs: Vec<RoleResourceVerb>,
    subresources: Vec<BoundedText>,
    resource_names: Vec<String>,
    zones: Vec<ZoneId>,
    execution_refs: Vec<ResourceRef>,
    session_verbs: Vec<RoleSessionVerb>,
}

impl RoleRule {
    /// Construct and canonicalize one Role rule.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mut resource_types: Vec<ResourceTypeName>,
        mut verbs: Vec<RoleResourceVerb>,
        mut subresources: Vec<BoundedText>,
        mut resource_names: Vec<String>,
        mut zones: Vec<ZoneId>,
        mut execution_refs: Vec<ResourceRef>,
        mut session_verbs: Vec<RoleSessionVerb>,
    ) -> Result<Self, RoleContractError> {
        if resource_types.is_empty() {
            return Err(RoleContractError::EmptyResourceTypes);
        }
        if verbs.is_empty() && session_verbs.is_empty() {
            return Err(RoleContractError::EmptyVerbs);
        }
        if resource_types.len() > MAX_ROLE_RULE_RESOURCE_TYPES
            || verbs.len() > MAX_ROLE_RULE_VERBS
            || session_verbs.len() > MAX_ROLE_RULE_SESSION_VERBS
            || subresources.len() > MAX_ROLE_RULE_SUBRESOURCES
            || resource_names.len() > MAX_ROLE_RULE_RESOURCE_NAMES
            || zones.len() > MAX_ROLE_RULE_ZONES
            || execution_refs.len() > MAX_ROLE_RULE_EXECUTION_REFS
        {
            return Err(RoleContractError::BoundExceeded);
        }
        resource_types.sort();
        verbs.sort();
        subresources.sort();
        zones.sort();
        execution_refs.sort();
        session_verbs.sort();
        if duplicate(&resource_types)
            || duplicate(&verbs)
            || duplicate(&subresources)
            || duplicate(&zones)
            || duplicate(&execution_refs)
            || duplicate(&session_verbs)
        {
            return Err(RoleContractError::DuplicateEntry);
        }
        for name in &resource_names {
            if name == "*" {
                continue;
            }
            ResourceName::parse(name.clone())
                .map_err(|_| RoleContractError::InvalidResourceName)?;
        }
        resource_names.sort();
        if duplicate(&resource_names) {
            return Err(RoleContractError::DuplicateEntry);
        }
        if execution_refs
            .iter()
            .any(|reference| reference.resource_type().as_str() == "ZoneLink")
        {
            // ZoneLink refs are valid execution selectors only when a caller
            // explicitly binds them; the type itself remains canonical.
        }
        if execution_refs.iter().any(|reference| {
            !matches!(
                reference.resource_type().as_str(),
                "Host" | "Guest" | "Process" | "ZoneLink"
            )
        }) {
            return Err(RoleContractError::InvalidExecutionRef);
        }
        let has_relay = session_verbs.contains(&RoleSessionVerb::Relay);
        if has_relay {
            if !verbs.is_empty() || resource_names.is_empty() || zones.is_empty() {
                return Err(if verbs.is_empty() {
                    RoleContractError::RelayScopeRequired
                } else {
                    RoleContractError::RelayHasResourceVerb
                });
            }
            if resource_names.iter().any(|name| name == "*") {
                return Err(RoleContractError::RelaySelectorInvalid);
            }
        }
        let has_diagnostic = session_verbs.iter().any(|verb| {
            matches!(
                verb,
                RoleSessionVerb::AuditExport | RoleSessionVerb::SupportBundle
            )
        });
        if has_diagnostic
            && (subresources.is_empty()
                || subresources.iter().any(|selector| {
                    !matches!(
                        selector.as_str(),
                        "d2b.audit.v3.AuditService/Export"
                            | "d2b.support.v3.SupportService/GenerateBundle"
                    )
                }))
        {
            return Err(RoleContractError::DiagnosticSelectorInvalid);
        }
        if verbs.iter().any(|verb| {
            matches!(
                verb,
                RoleResourceVerb::UseCredential | RoleResourceVerb::AdminCredential
            )
        }) && (resource_types.len() != 1
            || resource_types[0].as_str() != "Credential"
            || subresources.is_empty())
        {
            return Err(RoleContractError::InvalidCredentialScope);
        }
        if verbs.contains(&RoleResourceVerb::AdminCredential)
            && (subresources
                .iter()
                .any(|selector| !matches!(selector.as_str(), "create" | "update-spec" | "delete"))
                || subresources.iter().any(|selector| {
                    let required = match selector.as_str() {
                        "create" => RoleResourceVerb::Create,
                        "update-spec" => RoleResourceVerb::UpdateSpec,
                        "delete" => RoleResourceVerb::Delete,
                        _ => return true,
                    };
                    !verbs.contains(&required)
                }))
        {
            return Err(RoleContractError::InvalidCredentialScope);
        }
        Ok(Self {
            resource_types,
            verbs,
            subresources,
            resource_names,
            zones,
            execution_refs,
            session_verbs,
        })
    }

    /// Validate provenance-dependent wildcard rules.
    pub fn validate_provenance(
        &self,
        core_controller_generated: bool,
    ) -> Result<(), RoleContractError> {
        let wildcard_count = self
            .resource_names
            .iter()
            .filter(|name| name.as_str() == "*")
            .count();
        if wildcard_count > 1 {
            return Err(RoleContractError::InvalidWildcard);
        }
        if wildcard_count != 0 && !core_controller_generated {
            return Err(RoleContractError::WildcardNotAllowed);
        }
        Ok(())
    }

    /// Borrow ResourceTypes.
    pub fn resource_types(&self) -> &[ResourceTypeName] {
        &self.resource_types
    }

    /// Borrow resource verbs.
    pub fn verbs(&self) -> &[RoleResourceVerb] {
        &self.verbs
    }

    /// Borrow service/subresource selectors.
    pub fn subresources(&self) -> &[BoundedText] {
        &self.subresources
    }

    /// Borrow exact name selectors. `"*"` is only valid for reviewed core
    /// roles and is never an implicit wildcard.
    pub fn resource_names(&self) -> &[String] {
        &self.resource_names
    }

    /// Borrow Zone selectors.
    pub fn zones(&self) -> &[ZoneId] {
        &self.zones
    }

    /// Borrow execution selectors.
    pub fn execution_refs(&self) -> &[ResourceRef] {
        &self.execution_refs
    }

    /// Borrow session verbs.
    pub fn session_verbs(&self) -> &[RoleSessionVerb] {
        &self.session_verbs
    }

    /// Whether this rule contains relay authority.
    pub fn permits_relay(&self) -> bool {
        self.session_verbs.contains(&RoleSessionVerb::Relay)
    }
}

redacted_debug!(RoleRule);

impl<'de> Deserialize<'de> for RoleRule {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            resource_types: Vec<ResourceTypeName>,
            #[serde(default)]
            verbs: Vec<RoleResourceVerb>,
            #[serde(default)]
            subresources: Vec<BoundedText>,
            #[serde(default)]
            resource_names: Vec<String>,
            #[serde(default)]
            zones: Vec<ZoneId>,
            #[serde(default)]
            execution_refs: Vec<ResourceRef>,
            #[serde(default)]
            session_verbs: Vec<RoleSessionVerb>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.resource_types,
            wire.verbs,
            wire.subresources,
            wire.resource_names,
            wire.zones,
            wire.execution_refs,
            wire.session_verbs,
        )
        .map_err(serde::de::Error::custom)
    }
}

fn duplicate<T: PartialEq>(values: &[T]) -> bool {
    values.windows(2).any(|pair| pair[0] == pair[1])
}

/// The complete Role desired state.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoleSpec {
    rules: Vec<RoleRule>,
}

impl RoleSpec {
    /// Construct a bounded Role spec.
    pub fn new(rules: Vec<RoleRule>) -> Result<Self, RoleContractError> {
        if rules.is_empty() || rules.len() > MAX_ROLE_RULES {
            return Err(RoleContractError::BoundExceeded);
        }
        Ok(Self { rules })
    }

    /// Borrow rules.
    pub fn rules(&self) -> &[RoleRule] {
        &self.rules
    }
}

redacted_debug!(RoleSpec);

impl<'de> Deserialize<'de> for RoleSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            rules: Vec<RoleRule>,
        }
        Self::new(Wire::deserialize(deserializer)?.rules).map_err(serde::de::Error::custom)
    }
}

/// Closed Role condition names.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum RoleConditionType {
    RuleSetValid,
    IndexBuilt,
    ActiveBindings,
    PendingBindingDrain,
}

/// Identity-free Role status projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleStatusResource {
    active_binding_count: u32,
    last_validated_at: Option<super::Timestamp>,
}

impl RoleStatusResource {
    /// Construct Role status.
    pub const fn new(
        active_binding_count: u32,
        last_validated_at: Option<super::Timestamp>,
    ) -> Self {
        Self {
            active_binding_count,
            last_validated_at,
        }
    }

    /// Return active binding count.
    pub const fn active_binding_count(&self) -> u32 {
        self.active_binding_count
    }

    /// Borrow last validation time.
    pub const fn last_validated_at(&self) -> Option<&super::Timestamp> {
        self.last_validated_at.as_ref()
    }
}

/// Alias used by generic status adapters.
pub type RoleStatus = RoleStatusResource;

/// Validate a Role owner reference.
pub fn validate_role_owner(owner: Option<&ResourceRef>) -> Result<(), RoleContractError> {
    if owner.is_some_and(|reference| {
        !matches!(reference.resource_type().as_str(), "Provider" | "ZoneLink")
    }) {
        Err(RoleContractError::InvalidExecutionRef)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_name() -> ResourceTypeName {
        ResourceTypeName::parse("Process").unwrap()
    }

    #[test]
    fn resource_and_session_verbs_are_separate() {
        let rule = RoleRule::new(
            vec![type_name()],
            vec![RoleResourceVerb::Get],
            Vec::new(),
            vec![String::from("worker")],
            vec![ZoneId::parse("dev").unwrap()],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        assert!(!rule.permits_relay());
        assert_eq!(rule.verbs(), &[RoleResourceVerb::Get]);
    }

    #[test]
    fn relay_requires_bounded_nonempty_scope_and_never_crud() {
        assert!(
            RoleRule::new(
                vec![type_name()],
                vec![],
                vec![],
                vec!["worker".to_owned()],
                vec![ZoneId::parse("dev").unwrap()],
                vec![],
                vec![RoleSessionVerb::Relay],
            )
            .unwrap()
            .permits_relay()
        );
        assert!(
            RoleRule::new(
                vec![type_name()],
                vec![RoleResourceVerb::Get],
                vec![],
                vec!["worker".to_owned()],
                vec![ZoneId::parse("dev").unwrap()],
                vec![],
                vec![RoleSessionVerb::Relay],
            )
            .is_err()
        );
    }

    #[test]
    fn wildcard_is_explicit_and_provenance_bound() {
        let rule = RoleRule::new(
            vec![type_name()],
            vec![RoleResourceVerb::Get],
            vec![],
            vec!["*".to_owned()],
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        assert!(rule.validate_provenance(true).is_ok());
        assert_eq!(
            rule.validate_provenance(false),
            Err(RoleContractError::WildcardNotAllowed)
        );
    }
}
