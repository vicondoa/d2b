//! Credential primitive ResourceType base spec.
//!
//! `Credential` is the opaque rotating credential and lease lifecycle
//! ResourceType. The scope, audience, consumer, allowed-operation, rotation,
//! expiry, revocation, and identity-Guest fields are Layer 2 base fields;
//! non-secret implementation-only desired settings belong to the Layer 3
//! `spec.provider` envelope on the universal `ResourceSpec`.
//!
//! The base spec is zero-secret by construction: it carries no token, key,
//! pre-shared key, cookie, claim, or other credential byte. Sensitive bytes
//! are delivered only over a dedicated end-to-end session and never enter a
//! resource, the store, status, audit, or telemetry.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    ResourceRef,
    execution_policy::{
        ExecutionDomain, PrimitiveSpecError, parsed_deserialize, redacted_debug,
        require_execution_ref, require_resource_type, string_schema,
    },
};

/// The canonical ResourceType name for this module.
pub const CREDENTIAL_RESOURCE_TYPE: &str = "Credential";
/// Maximum bytes in one audience token.
pub const MAX_AUDIENCE_BYTES: usize = 256;
/// Maximum proactive rotation window in milliseconds.
pub const MAX_PROACTIVE_WINDOW_MS: u64 = 1_800_000;
/// Maximum Provider-granted lease lifetime in milliseconds.
pub const MAX_PROVIDER_LEASE_LIFETIME_MS: u64 = 7 * 86_400_000;

/// A validated non-secret audience token.
///
/// The charset is restricted to reject secret shapes.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct AudienceToken(String);

impl AudienceToken {
    /// Parse a bounded printable-ASCII audience token.
    pub fn parse(value: impl Into<String>) -> Result<Self, PrimitiveSpecError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_AUDIENCE_BYTES
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'/' | b':')
            })
        {
            return Err(PrimitiveSpecError::InvalidText);
        }
        Ok(Self(value))
    }

    /// Borrow the audience token.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(AudienceToken);
parsed_deserialize!(AudienceToken);
string_schema!(AudienceToken, 1, MAX_AUDIENCE_BYTES);

/// The closed typed operation classes a consumer may be granted.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialOperation {
    AcquireToken,
    RefreshToken,
    RevokeToken,
    SignChallenge,
    InspectMetadata,
}

/// Placement restriction on lease acquisition.
#[derive(Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CredentialScope {
    execution_ref: Option<ResourceRef>,
    domain_filter: Option<ExecutionDomain>,
    user_ref: Option<ResourceRef>,
}

impl CredentialScope {
    /// Construct a scope, requiring a user identity for the `user` filter.
    pub fn new(
        execution_ref: Option<ResourceRef>,
        domain_filter: Option<ExecutionDomain>,
        user_ref: Option<ResourceRef>,
    ) -> Result<Self, PrimitiveSpecError> {
        if let Some(execution_ref) = &execution_ref {
            require_execution_ref(execution_ref)?;
        }
        if let Some(user_ref) = &user_ref {
            require_resource_type(user_ref, "User")?;
        }
        if domain_filter == Some(ExecutionDomain::User) && user_ref.is_none() {
            return Err(PrimitiveSpecError::MissingRequiredField);
        }
        Ok(Self {
            execution_ref,
            domain_filter,
            user_ref,
        })
    }

    /// Borrow the permitted execution context.
    pub const fn execution_ref(&self) -> Option<&ResourceRef> {
        self.execution_ref.as_ref()
    }

    /// Return the permitted process domain.
    pub const fn domain_filter(&self) -> Option<ExecutionDomain> {
        self.domain_filter
    }

    /// Borrow the permitted acquiring user identity.
    pub const fn user_ref(&self) -> Option<&ResourceRef> {
        self.user_ref.as_ref()
    }
}

redacted_debug!(CredentialScope);

impl<'de> Deserialize<'de> for CredentialScope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            execution_ref: Option<ResourceRef>,
            #[serde(default)]
            domain_filter: Option<ExecutionDomain>,
            #[serde(default)]
            user_ref: Option<ResourceRef>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.execution_ref, wire.domain_filter, wire.user_ref)
            .map_err(serde::de::Error::custom)
    }
}

/// Rotation policy class.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum RotationPolicyClass {
    OnExpiry,
    Proactive,
    OnDemand,
}

/// Rotation settings.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RotationSpec {
    policy: RotationPolicyClass,
    proactive_window_ms: Option<u64>,
    max_lease_lifetime_ms: u64,
}

impl RotationSpec {
    /// Construct rotation settings after checking every frozen bound.
    ///
    /// A zero `maxLeaseLifetimeMs` selects the Provider default cap.
    pub fn new(
        policy: RotationPolicyClass,
        proactive_window_ms: Option<u64>,
        max_lease_lifetime_ms: u64,
    ) -> Result<Self, PrimitiveSpecError> {
        if max_lease_lifetime_ms > MAX_PROVIDER_LEASE_LIFETIME_MS {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        match (policy, proactive_window_ms) {
            (RotationPolicyClass::Proactive, None) => {
                return Err(PrimitiveSpecError::MissingRequiredField);
            }
            (RotationPolicyClass::Proactive, Some(window)) => {
                if window == 0 || window > MAX_PROACTIVE_WINDOW_MS {
                    return Err(PrimitiveSpecError::OutOfRange);
                }
                if max_lease_lifetime_ms != 0 && window >= max_lease_lifetime_ms / 2 {
                    return Err(PrimitiveSpecError::ConflictingFields);
                }
            }
            (_, Some(_)) => return Err(PrimitiveSpecError::ConflictingFields),
            (_, None) => {}
        }
        Ok(Self {
            policy,
            proactive_window_ms,
            max_lease_lifetime_ms,
        })
    }

    /// Return the rotation policy class.
    pub const fn policy(&self) -> RotationPolicyClass {
        self.policy
    }

    /// Return the proactive rotation window.
    pub const fn proactive_window_ms(&self) -> Option<u64> {
        self.proactive_window_ms
    }

    /// Return the lease lifetime cap.
    pub const fn max_lease_lifetime_ms(&self) -> u64 {
        self.max_lease_lifetime_ms
    }
}

impl Default for RotationSpec {
    fn default() -> Self {
        Self {
            policy: RotationPolicyClass::OnExpiry,
            proactive_window_ms: None,
            max_lease_lifetime_ms: 0,
        }
    }
}

redacted_debug!(RotationSpec);

impl<'de> Deserialize<'de> for RotationSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            policy: RotationPolicyClass,
            #[serde(default)]
            proactive_window_ms: Option<u64>,
            #[serde(default)]
            max_lease_lifetime_ms: u64,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.policy,
            wire.proactive_window_ms,
            wire.max_lease_lifetime_ms,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Hard expiry settings.
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExpirySpec {
    hard_deadline_ms: u64,
}

impl ExpirySpec {
    /// Construct expiry settings; zero selects the Provider default.
    pub fn new(hard_deadline_ms: u64) -> Result<Self, PrimitiveSpecError> {
        if hard_deadline_ms > MAX_PROVIDER_LEASE_LIFETIME_MS {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        Ok(Self { hard_deadline_ms })
    }

    /// Return the hard total lifetime ceiling.
    pub const fn hard_deadline_ms(&self) -> u64 {
        self.hard_deadline_ms
    }
}

redacted_debug!(ExpirySpec);

impl<'de> Deserialize<'de> for ExpirySpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            hard_deadline_ms: u64,
        }
        Self::new(Wire::deserialize(deserializer)?.hard_deadline_ms)
            .map_err(serde::de::Error::custom)
    }
}

/// How active leases are treated on a revocation trigger.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum RevocationAction {
    Immediate,
    DrainLeases,
}

/// Revocation settings.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevocationSpec {
    pub on_owner_delete: RevocationAction,
    pub on_provider_generation: RevocationAction,
}

impl Default for RevocationSpec {
    fn default() -> Self {
        Self {
            on_owner_delete: RevocationAction::Immediate,
            on_provider_generation: RevocationAction::Immediate,
        }
    }
}

redacted_debug!(RevocationSpec);

/// The Credential ResourceType base spec.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CredentialSpec {
    scope: CredentialScope,
    audience: AudienceToken,
    consumer_ref: Option<ResourceRef>,
    allowed_operations: Vec<CredentialOperation>,
    rotation: RotationSpec,
    expiry: ExpirySpec,
    revocation: RevocationSpec,
    identity_guest_ref: Option<ResourceRef>,
    login_endpoint_ref: Option<ResourceRef>,
}

impl CredentialSpec {
    /// Construct a Credential base spec after checking every frozen bound.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scope: CredentialScope,
        audience: AudienceToken,
        consumer_ref: Option<ResourceRef>,
        mut allowed_operations: Vec<CredentialOperation>,
        rotation: RotationSpec,
        expiry: ExpirySpec,
        revocation: RevocationSpec,
        identity_guest_ref: Option<ResourceRef>,
        login_endpoint_ref: Option<ResourceRef>,
    ) -> Result<Self, PrimitiveSpecError> {
        if allowed_operations.is_empty() {
            return Err(PrimitiveSpecError::MissingRequiredField);
        }
        let declared = allowed_operations.len();
        allowed_operations.sort_unstable();
        allowed_operations.dedup();
        if allowed_operations.len() != declared {
            return Err(PrimitiveSpecError::DuplicateEntry);
        }
        if let Some(consumer_ref) = &consumer_ref {
            require_resource_type(consumer_ref, "Provider")?;
        }
        if let Some(identity_guest_ref) = &identity_guest_ref {
            require_resource_type(identity_guest_ref, "Guest")?;
        }
        if let Some(login_endpoint_ref) = &login_endpoint_ref {
            require_resource_type(login_endpoint_ref, "Endpoint")?;
        }
        if identity_guest_ref.is_some() != login_endpoint_ref.is_some() {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        if expiry.hard_deadline_ms() != 0
            && rotation.max_lease_lifetime_ms() != 0
            && expiry.hard_deadline_ms() > rotation.max_lease_lifetime_ms()
        {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        Ok(Self {
            scope,
            audience,
            consumer_ref,
            allowed_operations,
            rotation,
            expiry,
            revocation,
            identity_guest_ref,
            login_endpoint_ref,
        })
    }

    /// Construct the canonical minimal inspect-only Credential base spec.
    pub fn minimal(audience: AudienceToken) -> Self {
        Self::new(
            CredentialScope::default(),
            audience,
            None,
            vec![CredentialOperation::InspectMetadata],
            RotationSpec::default(),
            ExpirySpec::default(),
            RevocationSpec::default(),
            None,
            None,
        )
        .expect("the minimal Credential spec is always valid")
    }

    /// Borrow the placement restriction.
    pub const fn scope(&self) -> &CredentialScope {
        &self.scope
    }

    /// Borrow the non-secret audience token.
    pub const fn audience(&self) -> &AudienceToken {
        &self.audience
    }

    /// Borrow the permitted consumer Provider.
    pub const fn consumer_ref(&self) -> Option<&ResourceRef> {
        self.consumer_ref.as_ref()
    }

    /// Borrow the granted operation classes.
    pub fn allowed_operations(&self) -> &[CredentialOperation] {
        &self.allowed_operations
    }

    /// Borrow the rotation settings.
    pub const fn rotation(&self) -> &RotationSpec {
        &self.rotation
    }

    /// Borrow the identity Guest that grounds interactive login.
    pub const fn identity_guest_ref(&self) -> Option<&ResourceRef> {
        self.identity_guest_ref.as_ref()
    }
}

redacted_debug!(CredentialSpec);

impl<'de> Deserialize<'de> for CredentialSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            scope: CredentialScope,
            audience: AudienceToken,
            #[serde(default)]
            consumer_ref: Option<ResourceRef>,
            allowed_operations: Vec<CredentialOperation>,
            #[serde(default)]
            rotation: RotationSpec,
            #[serde(default)]
            expiry: ExpirySpec,
            #[serde(default)]
            revocation: RevocationSpec,
            #[serde(default)]
            identity_guest_ref: Option<ResourceRef>,
            #[serde(default)]
            login_endpoint_ref: Option<ResourceRef>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.scope,
            wire.audience,
            wire.consumer_ref,
            wire.allowed_operations,
            wire.rotation,
            wire.expiry,
            wire.revocation,
            wire.identity_guest_ref,
            wire.login_endpoint_ref,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::{execution_policy::to_base_object, resource_schema::canonical_json_bytes};

    const MINIMAL_CREDENTIAL_SPEC: &[u8] = br#"{"allowedOperations":["inspect-metadata"],"audience":"azure-resource-manager","consumerRef":null,"expiry":{"hardDeadlineMs":0},"identityGuestRef":null,"loginEndpointRef":null,"revocation":{"onOwnerDelete":"immediate","onProviderGeneration":"immediate"},"rotation":{"maxLeaseLifetimeMs":0,"policy":"on-expiry","proactiveWindowMs":null},"scope":{"domainFilter":null,"executionRef":null,"userRef":null}}"#;

    fn audience() -> AudienceToken {
        AudienceToken::parse("azure-resource-manager").unwrap()
    }

    #[test]
    fn schema_vector_pins_the_minimal_credential_base_spec() {
        let spec = CredentialSpec::minimal(audience());
        assert_eq!(
            canonical_json_bytes(&spec).unwrap(),
            MINIMAL_CREDENTIAL_SPEC
        );
        let parsed: CredentialSpec = serde_json::from_slice(MINIMAL_CREDENTIAL_SPEC).unwrap();
        assert_eq!(parsed, spec);
        let base = to_base_object(&spec).unwrap();
        for reserved in [
            "providerRef",
            "updatePolicy",
            "provider",
            "providerSettings",
        ] {
            assert!(base.get(reserved).is_none());
        }
    }

    #[test]
    fn the_base_spec_admits_no_secret_bearing_field() {
        for rejected in [
            br#"{"audience":"a","allowedOperations":["acquire-token"],"token":"x"}"#.as_slice(),
            br#"{"audience":"a","allowedOperations":["acquire-token"],"clientSecret":"x"}"#,
            br#"{"audience":"a","allowedOperations":["acquire-token"],"psk":"x"}"#,
            br#"{"audience":"a","allowedOperations":["acquire-token"],"providerSettings":{}}"#,
        ] {
            assert!(serde_json::from_slice::<CredentialSpec>(rejected).is_err());
        }
    }

    #[test]
    fn allowed_operations_are_a_closed_non_empty_unique_set() {
        assert_eq!(
            CredentialSpec::new(
                CredentialScope::default(),
                audience(),
                None,
                Vec::new(),
                RotationSpec::default(),
                ExpirySpec::default(),
                RevocationSpec::default(),
                None,
                None,
            ),
            Err(PrimitiveSpecError::MissingRequiredField)
        );
        assert_eq!(
            CredentialSpec::new(
                CredentialScope::default(),
                audience(),
                None,
                vec![
                    CredentialOperation::AcquireToken,
                    CredentialOperation::AcquireToken
                ],
                RotationSpec::default(),
                ExpirySpec::default(),
                RevocationSpec::default(),
                None,
                None,
            ),
            Err(PrimitiveSpecError::DuplicateEntry)
        );
        assert!(
            serde_json::from_slice::<CredentialSpec>(
                br#"{"audience":"a","allowedOperations":["export-key"]}"#
            )
            .is_err()
        );
    }

    #[test]
    fn scope_and_reference_types_fail_closed() {
        assert_eq!(
            CredentialScope::new(None, Some(ExecutionDomain::User), None),
            Err(PrimitiveSpecError::MissingRequiredField)
        );
        assert_eq!(
            CredentialScope::new(Some(ResourceRef::parse("Volume/x").unwrap()), None, None),
            Err(PrimitiveSpecError::WrongResourceType)
        );
        assert!(
            CredentialScope::new(
                Some(ResourceRef::parse("Guest/work-vm").unwrap()),
                Some(ExecutionDomain::User),
                Some(ResourceRef::parse("User/alice").unwrap()),
            )
            .is_ok()
        );
    }

    #[test]
    fn interactive_login_binds_a_guest_and_an_endpoint_together() {
        let with_guest_only = CredentialSpec::new(
            CredentialScope::default(),
            audience(),
            None,
            vec![CredentialOperation::AcquireToken],
            RotationSpec::default(),
            ExpirySpec::default(),
            RevocationSpec::default(),
            Some(ResourceRef::parse("Guest/identity").unwrap()),
            None,
        );
        assert_eq!(with_guest_only, Err(PrimitiveSpecError::ConflictingFields));
        assert!(
            CredentialSpec::new(
                CredentialScope::default(),
                audience(),
                None,
                vec![CredentialOperation::AcquireToken],
                RotationSpec::default(),
                ExpirySpec::default(),
                RevocationSpec::default(),
                Some(ResourceRef::parse("Guest/identity").unwrap()),
                Some(ResourceRef::parse("Endpoint/entra-login").unwrap()),
            )
            .is_ok()
        );
    }

    #[test]
    fn rotation_bounds_fail_closed() {
        assert_eq!(
            RotationSpec::new(RotationPolicyClass::Proactive, None, 3_600_000),
            Err(PrimitiveSpecError::MissingRequiredField)
        );
        assert_eq!(
            RotationSpec::new(RotationPolicyClass::Proactive, Some(2_000_000), 0),
            Err(PrimitiveSpecError::OutOfRange)
        );
        assert_eq!(
            RotationSpec::new(RotationPolicyClass::Proactive, Some(1_800_000), 3_600_000),
            Err(PrimitiveSpecError::ConflictingFields)
        );
        assert_eq!(
            RotationSpec::new(RotationPolicyClass::OnDemand, Some(1_000), 0),
            Err(PrimitiveSpecError::ConflictingFields)
        );
        assert!(
            RotationSpec::new(RotationPolicyClass::Proactive, Some(300_000), 3_600_000).is_ok()
        );
        assert_eq!(
            ExpirySpec::new(MAX_PROVIDER_LEASE_LIFETIME_MS + 1),
            Err(PrimitiveSpecError::OutOfRange)
        );
    }

    #[test]
    fn audience_rejects_control_and_whitespace_shapes() {
        assert!(AudienceToken::parse("https://management.azure.com/.default").is_ok());
        assert!(AudienceToken::parse("").is_err());
        assert!(AudienceToken::parse("token with space").is_err());
        assert!(AudienceToken::parse("a".repeat(MAX_AUDIENCE_BYTES + 1)).is_err());
    }

    #[test]
    fn diagnostics_never_echo_the_audience() {
        let marker = format!("aud-{:x}", std::process::id());
        let spec = CredentialSpec::minimal(AudienceToken::parse(marker.clone()).unwrap());
        assert!(!format!("{spec:?}").contains(&marker));
        assert!(!format!("{:?}", spec.audience()).contains(&marker));
        assert_eq!(spec.audience().as_str(), marker);
    }
}
