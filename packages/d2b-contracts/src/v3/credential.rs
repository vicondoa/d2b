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
use sha2::{Digest, Sha256};

use super::{
    ResourceRef, Timestamp,
    execution_policy::{
        ExecutionDomain, PrimitiveSpecError, parsed_deserialize, redacted_debug,
        require_execution_ref, require_resource_type, string_schema,
    },
};

pub mod service;

pub use service::*;

/// The canonical ResourceType name for this module.
pub const CREDENTIAL_RESOURCE_TYPE: &str = "Credential";
/// Maximum bytes in one audience token.
pub const MAX_AUDIENCE_BYTES: usize = 256;
/// Maximum proactive rotation window in milliseconds.
pub const MAX_PROACTIVE_WINDOW_MS: u64 = 1_800_000;
/// Maximum Provider-granted lease lifetime in milliseconds.
pub const MAX_PROVIDER_LEASE_LIFETIME_MS: u64 = 7 * 86_400_000;
/// Maximum bytes accepted as an opaque non-secret cloud reference.
pub const MAX_AZURE_REF_BYTES: usize = 128;
/// Maximum bytes accepted as a Provider lease handle before one-way encoding.
pub const MAX_CREDENTIAL_LEASE_HANDLE_BYTES: usize = 256;
/// Maximum bytes accepted as a credential source version before one-way encoding.
pub const MAX_CREDENTIAL_SOURCE_VERSION_BYTES: usize = 64;

const OPAQUE_DIGEST_BYTES: usize = 71;
const OPAQUE_DIGEST_PREFIX: &str = "sha256:";

/// Validation failure for a Credential base contract.
///
/// The variants deliberately carry no caller-controlled data, resource identity,
/// or credential material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialContractError {
    /// An opaque value was empty, over its bound, or used a rejected character.
    InvalidOpaqueValue,
    /// Status timestamps or state fields conflict.
    InvalidStatus,
}

impl core::fmt::Display for CredentialContractError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidOpaqueValue => "credential opaque value is invalid",
            Self::InvalidStatus => "credential status is invalid",
        })
    }
}

impl std::error::Error for CredentialContractError {}

fn validate_opaque_source(value: &str, max_bytes: usize) -> Result<(), CredentialContractError> {
    if value.is_empty()
        || value.len() > max_bytes
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_'))
    {
        return Err(CredentialContractError::InvalidOpaqueValue);
    }
    Ok(())
}

fn validate_non_secret_identifier(
    value: &str,
    max_bytes: usize,
) -> Result<(), CredentialContractError> {
    if value.is_empty()
        || value.len() > max_bytes
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'/' | b':' | b'@')
        })
    {
        return Err(CredentialContractError::InvalidOpaqueValue);
    }
    Ok(())
}

fn opaque_digest(domain: &[u8], value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update([0]);
    hasher.update(value.as_bytes());
    format!("{OPAQUE_DIGEST_PREFIX}{:x}", hasher.finalize())
}

fn validate_opaque_digest(value: &str) -> Result<(), CredentialContractError> {
    let Some(hex) = value.strip_prefix(OPAQUE_DIGEST_PREFIX) else {
        return Err(CredentialContractError::InvalidOpaqueValue);
    };
    if value.len() != OPAQUE_DIGEST_BYTES
        || hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(CredentialContractError::InvalidOpaqueValue);
    }
    Ok(())
}

fn opaque_digest_schema() -> schemars::schema::Schema {
    let mut schema = schemars::schema::SchemaObject {
        instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
            schemars::schema::InstanceType::String,
        ))),
        ..Default::default()
    };
    schema.string().min_length = Some(OPAQUE_DIGEST_BYTES as u32);
    schema.string().max_length = Some(OPAQUE_DIGEST_BYTES as u32);
    schema.string().pattern = Some("^sha256:[0-9a-f]{64}$".to_owned());
    schemars::schema::Schema::Object(schema)
}

macro_rules! opaque_credential_value {
    ($name:ident, $max:expr, $domain:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Validate a raw identifier and retain only its domain-separated digest.
            pub fn parse(value: impl AsRef<str>) -> Result<Self, CredentialContractError> {
                let value = value.as_ref();
                validate_opaque_source(value, $max)?;
                Ok(Self(opaque_digest($domain, value)))
            }

            /// Borrow the non-reversible representation used on authorized wires.
            pub fn as_opaque_str(&self) -> &str {
                &self.0
            }

            /// Reconstruct a value from its authorized one-way wire representation.
            pub fn from_opaque_digest(
                value: impl Into<String>,
            ) -> Result<Self, CredentialContractError> {
                let value = value.into();
                validate_opaque_digest(&value)?;
                Ok(Self(value))
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(concat!(stringify!($name), "(<redacted>)"))
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(concat!(stringify!($name), "(<redacted>)"))
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                Self::from_opaque_digest(String::deserialize(deserializer)?)
                    .map_err(serde::de::Error::custom)
            }
        }

        impl schemars::JsonSchema for $name {
            fn schema_name() -> String {
                stringify!($name).to_owned()
            }

            fn json_schema(
                _gen: &mut schemars::r#gen::SchemaGenerator,
            ) -> schemars::schema::Schema {
                opaque_digest_schema()
            }
        }
    };
}

/// A bounded non-secret cloud identifier whose diagnostics are always redacted.
///
/// Providers need the validated tenant, client, or region value when calling
/// their backing service, so serialization preserves it instead of hashing it.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct OpaqueAzureRef(String);

impl OpaqueAzureRef {
    /// Validate and preserve a bare non-secret cloud identifier.
    pub fn parse(value: impl Into<String>) -> Result<Self, CredentialContractError> {
        let value = value.into();
        validate_opaque_source(&value, MAX_AZURE_REF_BYTES)?;
        Ok(Self(value))
    }

    /// Borrow the validated identifier for Provider use.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for OpaqueAzureRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OpaqueAzureRef(<redacted>)")
    }
}

impl core::fmt::Display for OpaqueAzureRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OpaqueAzureRef(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for OpaqueAzureRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for OpaqueAzureRef {
    fn schema_name() -> String {
        "OpaqueAzureRef".to_owned()
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        let mut schema = schemars::schema::SchemaObject {
            instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
                schemars::schema::InstanceType::String,
            ))),
            ..Default::default()
        };
        schema.string().min_length = Some(1);
        schema.string().max_length = Some(MAX_AZURE_REF_BYTES as u32);
        schema.string().pattern = Some("^[A-Za-z0-9._-]+$".to_owned());
        schemars::schema::Schema::Object(schema)
    }
}

opaque_credential_value!(
    CredentialLeaseHandle,
    MAX_CREDENTIAL_LEASE_HANDLE_BYTES,
    b"d2b:v3:credential-lease-handle",
    "A bounded non-authorizing lease handle represented only by a one-way digest."
);
opaque_credential_value!(
    CredentialSourceVersion,
    MAX_CREDENTIAL_SOURCE_VERSION_BYTES,
    b"d2b:v3:credential-source-version",
    "A bounded non-secret source version represented only by a one-way digest."
);

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
        if validate_non_secret_identifier(&value, MAX_AUDIENCE_BYTES).is_err() {
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
pub enum OperationClass {
    AcquireToken,
    RefreshToken,
    RevokeToken,
    SignChallenge,
    InspectMetadata,
}

/// Compatibility name for the operation enum used by the prepared base spec.
pub type CredentialOperation = OperationClass;

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
pub struct CredentialRotationPolicy {
    policy: RotationPolicyClass,
    proactive_window_ms: Option<u64>,
    max_lease_lifetime_ms: u64,
}

impl CredentialRotationPolicy {
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

impl Default for CredentialRotationPolicy {
    fn default() -> Self {
        Self {
            policy: RotationPolicyClass::OnExpiry,
            proactive_window_ms: None,
            max_lease_lifetime_ms: 0,
        }
    }
}

redacted_debug!(CredentialRotationPolicy);

impl<'de> Deserialize<'de> for CredentialRotationPolicy {
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

/// Compatibility name for the prepared Credential spec field.
pub type RotationSpec = CredentialRotationPolicy;

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
pub struct CredentialRevocationPolicy {
    pub on_owner_delete: RevocationAction,
    pub on_provider_generation: RevocationAction,
}

impl Default for CredentialRevocationPolicy {
    fn default() -> Self {
        Self {
            on_owner_delete: RevocationAction::Immediate,
            on_provider_generation: RevocationAction::Immediate,
        }
    }
}

redacted_debug!(CredentialRevocationPolicy);

/// Compatibility name for the prepared Credential spec field.
pub type RevocationSpec = CredentialRevocationPolicy;

/// Current non-secret state of a Credential lease.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum CredentialLeaseState {
    Active,
    Expired,
    Revoked,
    Unknown,
}

/// Execution placement to which the observed lease is bound.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum PlacementBinding {
    UserAgent,
    HostSystem,
    GuestAgent,
}

/// Closed Credential condition types written by its controller.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum CredentialConditionType {
    CredentialReady,
    RotationDue,
    ProviderUnavailable,
    LeaseRevoked,
}

/// Non-secret lease observation nested under Credential status.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CredentialLeaseStatus {
    lease_handle: CredentialLeaseHandle,
    lease_state: CredentialLeaseState,
    rotation_generation: u64,
    source_version: CredentialSourceVersion,
    expires_at_unix_ms: u64,
    issued_at_unix_ms: u64,
    last_refreshed_at: Option<Timestamp>,
    last_rotated_at: Option<Timestamp>,
    placement_binding: PlacementBinding,
}

impl CredentialLeaseStatus {
    /// Construct bounded lease status without accepting any resource identity field.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        lease_handle: CredentialLeaseHandle,
        lease_state: CredentialLeaseState,
        rotation_generation: u64,
        source_version: CredentialSourceVersion,
        expires_at_unix_ms: u64,
        issued_at_unix_ms: u64,
        last_refreshed_at: Option<Timestamp>,
        last_rotated_at: Option<Timestamp>,
        placement_binding: PlacementBinding,
    ) -> Result<Self, CredentialContractError> {
        if rotation_generation == 0
            || issued_at_unix_ms > expires_at_unix_ms
            || (lease_state == CredentialLeaseState::Active
                && (issued_at_unix_ms == 0 || expires_at_unix_ms == 0))
        {
            return Err(CredentialContractError::InvalidStatus);
        }
        Ok(Self {
            lease_handle,
            lease_state,
            rotation_generation,
            source_version,
            expires_at_unix_ms,
            issued_at_unix_ms,
            last_refreshed_at,
            last_rotated_at,
            placement_binding,
        })
    }

    /// Borrow the opaque lease handle.
    pub const fn lease_handle(&self) -> &CredentialLeaseHandle {
        &self.lease_handle
    }

    /// Return the current lease state.
    pub const fn lease_state(&self) -> CredentialLeaseState {
        self.lease_state
    }

    /// Return the current rotation generation.
    pub const fn rotation_generation(&self) -> u64 {
        self.rotation_generation
    }

    /// Borrow the opaque source version.
    pub const fn source_version(&self) -> &CredentialSourceVersion {
        &self.source_version
    }
}

redacted_debug!(CredentialLeaseStatus);

impl<'de> Deserialize<'de> for CredentialLeaseStatus {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            lease_handle: CredentialLeaseHandle,
            lease_state: CredentialLeaseState,
            rotation_generation: u64,
            source_version: CredentialSourceVersion,
            expires_at_unix_ms: u64,
            issued_at_unix_ms: u64,
            last_refreshed_at: Option<Timestamp>,
            last_rotated_at: Option<Timestamp>,
            placement_binding: PlacementBinding,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.lease_handle,
            wire.lease_state,
            wire.rotation_generation,
            wire.source_version,
            wire.expires_at_unix_ms,
            wire.issued_at_unix_ms,
            wire.last_refreshed_at,
            wire.last_rotated_at,
            wire.placement_binding,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Current state of an optional interactive login ceremony.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum CredentialInteractionState {
    NotRequired,
    Required,
    Starting,
    AwaitingUser,
    Authenticated,
    Failed,
    Unknown,
}

/// ResourceType-common non-secret Credential status layer.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CredentialStatus {
    interaction_state: CredentialInteractionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    login_session_generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    login_deadline: Option<Timestamp>,
    credential: Option<CredentialLeaseStatus>,
}

impl CredentialStatus {
    /// Construct the common status layer without accepting resource identity.
    pub fn new(
        interaction_state: CredentialInteractionState,
        login_session_generation: Option<u64>,
        login_deadline: Option<Timestamp>,
        credential: Option<CredentialLeaseStatus>,
    ) -> Result<Self, CredentialContractError> {
        if login_session_generation == Some(0)
            || (credential.is_none()
                && matches!(interaction_state, CredentialInteractionState::Authenticated))
        {
            return Err(CredentialContractError::InvalidStatus);
        }
        Ok(Self {
            interaction_state,
            login_session_generation,
            login_deadline,
            credential,
        })
    }

    /// Return the projected interactive-login state.
    pub const fn interaction_state(&self) -> CredentialInteractionState {
        self.interaction_state
    }

    /// Return the current interactive-login generation, if any.
    pub const fn login_session_generation(&self) -> Option<u64> {
        self.login_session_generation
    }

    /// Borrow the current interactive-login deadline, if any.
    pub const fn login_deadline(&self) -> Option<&Timestamp> {
        self.login_deadline.as_ref()
    }

    /// Borrow the optional lease observation.
    pub const fn credential(&self) -> Option<&CredentialLeaseStatus> {
        self.credential.as_ref()
    }
}

redacted_debug!(CredentialStatus);

impl<'de> Deserialize<'de> for CredentialStatus {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            interaction_state: CredentialInteractionState,
            login_session_generation: Option<u64>,
            login_deadline: Option<Timestamp>,
            credential: Option<CredentialLeaseStatus>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.interaction_state,
            wire.login_session_generation,
            wire.login_deadline,
            wire.credential,
        )
        .map_err(serde::de::Error::custom)
    }
}

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

    #[test]
    fn opaque_cloud_reference_preserves_the_non_secret_value_with_redacted_diagnostics() {
        let marker = format!("cloud-ref-{:x}", std::process::id());
        let reference = OpaqueAzureRef::parse(&marker).unwrap();
        let encoded = serde_json::to_string(&reference).unwrap();
        assert_eq!(reference.as_str(), marker);
        assert_eq!(encoded, format!("\"{marker}\""));
        assert!(!format!("{reference:?}").contains(&marker));
        assert!(!reference.to_string().contains(&marker));
        assert_eq!(
            serde_json::from_str::<OpaqueAzureRef>(&encoded).unwrap(),
            reference
        );
        assert!(OpaqueAzureRef::parse("SharedAccessKey=abc/def+ghi==").is_err());
        assert!(OpaqueAzureRef::parse("x".repeat(MAX_AZURE_REF_BYTES + 1)).is_err());
        let schema = schemars::schema_for!(OpaqueAzureRef);
        let schema_json = serde_json::to_string(&schema).unwrap();
        assert!(schema_json.contains("^[A-Za-z0-9._-]+$"));
    }

    #[test]
    fn lease_handle_and_source_version_are_one_way_opaque_newtypes() {
        let nonce = format!("{:x}", std::process::id());
        let lease_marker = format!("lease-{nonce}");
        let source_marker = format!("source-{nonce}");
        let lease = CredentialLeaseHandle::parse(&lease_marker).unwrap();
        let source = CredentialSourceVersion::parse(&source_marker).unwrap();
        for rendered in [
            format!("{lease:?}"),
            lease.to_string(),
            serde_json::to_string(&lease).unwrap(),
            format!("{source:?}"),
            source.to_string(),
            serde_json::to_string(&source).unwrap(),
        ] {
            assert!(!rendered.contains(&lease_marker));
            assert!(!rendered.contains(&source_marker));
        }
        assert_ne!(lease.as_opaque_str(), source.as_opaque_str());
        let schema = schemars::schema_for!(CredentialLeaseHandle);
        let schema_json = serde_json::to_string(&schema).unwrap();
        assert!(schema_json.contains("^sha256:[0-9a-f]{64}$"));
    }

    fn status_with_markers() -> (CredentialStatus, String, String) {
        let nonce = format!("{:x}", std::process::id());
        let lease_marker = format!("lease-status-{nonce}");
        let source_marker = format!("source-status-{nonce}");
        let credential = CredentialLeaseStatus::new(
            CredentialLeaseHandle::parse(&lease_marker).unwrap(),
            CredentialLeaseState::Active,
            7,
            CredentialSourceVersion::parse(&source_marker).unwrap(),
            2_000,
            1_000,
            Some(Timestamp::parse("2026-07-22T00:00:01.000Z").unwrap()),
            None,
            PlacementBinding::UserAgent,
        )
        .unwrap();
        let status = CredentialStatus::new(
            CredentialInteractionState::NotRequired,
            None,
            None,
            Some(credential),
        )
        .unwrap();
        (status, lease_marker, source_marker)
    }

    #[test]
    fn credential_status_golden_vector_is_strict_and_identity_free() {
        let (status, lease_marker, source_marker) = status_with_markers();
        let credential = status.credential().unwrap();
        let expected = format!(
            "{{\"credential\":{{\"expiresAtUnixMs\":2000,\"issuedAtUnixMs\":1000,\"lastRefreshedAt\":\"2026-07-22T00:00:01.000Z\",\"lastRotatedAt\":null,\"leaseHandle\":\"{}\",\"leaseState\":\"Active\",\"placementBinding\":\"user-agent\",\"rotationGeneration\":7,\"sourceVersion\":\"{}\"}},\"interactionState\":\"NotRequired\"}}",
            credential.lease_handle().as_opaque_str(),
            credential.source_version().as_opaque_str()
        );
        let rendered = String::from_utf8(canonical_json_bytes(&status).unwrap()).unwrap();
        assert_eq!(rendered, expected);
        assert!(!rendered.contains(&lease_marker));
        assert!(!rendered.contains(&source_marker));
        assert!(!rendered.contains("credentialRef"));
        assert!(!rendered.contains("credentialUid"));
        assert!(!rendered.contains("resourceNameDigest"));
        assert_eq!(
            serde_json::from_str::<CredentialStatus>(&rendered).unwrap(),
            status
        );
        let with_unknown = rendered.replacen('{', "{\"credentialRef\":\"Credential/private\",", 1);
        assert!(serde_json::from_str::<CredentialStatus>(&with_unknown).is_err());
    }

    #[test]
    fn process_unique_redaction_canaries_never_reach_rendered_contract_surfaces() {
        let nonce = format!("{:x}", std::process::id());
        let credential_name = format!("credential-name-{nonce}");
        let credential_ref = format!("Credential/{credential_name}");
        let credential_uid = format!(
            "123e4567-e89b-4{:0>3}-a456-{:0>12}",
            &nonce[..nonce.len().min(3)],
            nonce
        );
        let credential_digest = format!("credential-digest-{nonce}");
        let (status, lease_marker, source_marker) = status_with_markers();
        let error = CredentialContractError::InvalidOpaqueValue;
        let status_json = serde_json::to_string(&status).unwrap();
        let injected = status_json.replacen(
            '{',
            &format!(
                "{{\"credentialName\":\"{credential_name}\",\"credentialRef\":\"{credential_ref}\",\"credentialUid\":\"{credential_uid}\",\"credentialDigest\":\"{credential_digest}\","
            ),
            1,
        );
        let rejection = serde_json::from_str::<CredentialStatus>(&injected).unwrap_err();
        let credential = status.credential().unwrap();
        let surfaces = [
            format!("{status:?}"),
            status_json,
            format!("{error:?}"),
            error.to_string(),
            format!("{rejection:?}"),
            rejection.to_string(),
            format!("{:?}", credential.lease_handle()),
            credential.lease_handle().to_string(),
            format!("{:?}", credential.source_version()),
            credential.source_version().to_string(),
        ];
        for surface in surfaces {
            for marker in [
                credential_name.as_str(),
                credential_ref.as_str(),
                credential_uid.as_str(),
                credential_digest.as_str(),
                lease_marker.as_str(),
                source_marker.as_str(),
            ] {
                assert!(
                    !surface.contains(marker),
                    "redaction canary reached a rendered surface"
                );
            }
        }
    }
}
