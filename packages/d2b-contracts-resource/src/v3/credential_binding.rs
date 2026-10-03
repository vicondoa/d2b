//! The `CredentialBinding` ResourceType: one authorized delivery.
//!
//! A `CredentialBinding` names one Credential, one admitted consumer
//! component, one stable consumer slot, the audience the credential is
//! delivered for, the operation classes that consumer may perform, and the
//! lifetime bounds of that authority. It carries no token, no secret, and no
//! delivery transcript: the material exists only inside an admitted delivery
//! session, and the graph spec states only who may use it, for what, and for
//! how long.
//!
//! Every identity bound into delivery comes from this contract and from the
//! delivery session's own identity bounds. Changing the audience, the
//! operation set, or the component generation makes the earlier delivery
//! authority unusable: prior delivery cannot be reused, and new use requires a
//! fresh admission.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    ResourceRef,
    binding::{
        BindingRowError, BindingSourceDecision, BindingConsumerKind, BindingContractError, BindingKey, BindingKind, BindingRealizationFacet,
        BindingSlot, BindingSpecFingerprint, ExecutionParentInput, RequestedRights,
    },
    execution_policy::{
        BoundedToken, DurationMs, ensure_unique, redacted_debug, require_resource_type,
    },
    identity::{ResourceUid, ZoneId},
};
use d2b_contracts::wire_deserialize;

/// Canonical `CredentialBinding` ResourceType name.
pub const CREDENTIAL_BINDING_RESOURCE_TYPE: &str = "CredentialBinding";
/// Maximum operation classes one credential binding admits.
pub const MAX_CREDENTIAL_OPERATIONS: usize = 8;
/// Longest lifetime one credential binding may request.
pub const MAX_CREDENTIAL_LIFETIME_MS: u64 = 30 * 24 * 60 * 60 * 1_000;
/// Shortest lifetime one credential binding may request.
pub const MIN_CREDENTIAL_LIFETIME_MS: u64 = 1_000;

/// The operation classes one credential delivery admits.
///
/// These are the classes the existing delivery-session contract authorizes;
/// naming them here keeps the requested authority in the same vocabulary as
/// the session that realizes it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialOperation {
    /// Acquire a token for the admitted audience.
    AcquireToken,
    /// Refresh a token for the admitted audience.
    RefreshToken,
    /// Sign a challenge for the admitted audience.
    SignChallenge,
}

/// The lifetime bounds one credential binding requests.
///
/// The delivery session requires its hard deadline to fall at or before its
/// absolute expiry; the same relation holds here, on the requested values.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CredentialLifetime {
    valid_for: DurationMs,
    expires_in: DurationMs,
}

impl CredentialLifetime {
    /// Construct the lifetime bounds after checking their relation and bounds.
    pub fn new(
        valid_for: impl Into<String>,
        expires_in: impl Into<String>,
    ) -> Result<Self, BindingContractError> {
        let valid_for = DurationMs::parse(
            valid_for.into(),
            MIN_CREDENTIAL_LIFETIME_MS,
            MAX_CREDENTIAL_LIFETIME_MS,
        )?;
        let expires_in = DurationMs::parse(
            expires_in.into(),
            MIN_CREDENTIAL_LIFETIME_MS,
            MAX_CREDENTIAL_LIFETIME_MS,
        )?;
        if valid_for.as_millis() > expires_in.as_millis() {
            return Err(BindingContractError::OutOfRange);
        }
        Ok(Self { valid_for, expires_in })
    }

    /// Borrow the requested lifetime.
    pub const fn valid_for(&self) -> &DurationMs {
        &self.valid_for
    }

    /// Borrow the requested expiry window.
    pub const fn expires_in(&self) -> &DurationMs {
        &self.expires_in
    }
}

redacted_debug!(CredentialLifetime);

wire_deserialize!(
    CredentialLifetime,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        valid_for: DurationMs,
        expires_in: DurationMs,
    },
    wire,
    Self::new(wire.valid_for.as_str(), wire.expires_in.as_str()).map_err(serde::de::Error::custom)
);

/// The desired request for one credential delivered to one consumer.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CredentialBindingRequest {
    source_ref: ResourceRef,
    consumer_ref: ResourceRef,
    slot: BindingSlot,
    audience: BoundedToken,
    operations: Vec<CredentialOperation>,
    lifetime: CredentialLifetime,
}

impl CredentialBindingRequest {
    /// Construct one request from typed references.
    ///
    /// # Errors
    ///
    /// Refuses a source that is not a `Credential`, a consumer this binding
    /// kind does not admit, and an operation set that is empty, over bound,
    /// or duplicated.
    pub fn new(
        source: ResourceRef,
        consumer: ResourceRef,
        slot: BindingSlot,
        audience: BoundedToken,
        operations: Vec<CredentialOperation>,
        lifetime: CredentialLifetime,
    ) -> Result<Self, BindingContractError> {
        require_resource_type(&source, BindingKind::Credential.source_resource_type())?;
        let consumer_kind = BindingConsumerKind::from_resource_type(consumer.resource_type().as_str())
            .ok_or(BindingContractError::WrongResourceType)?;
        if !BindingKind::Credential.admits_consumer(consumer_kind) {
            return Err(BindingContractError::UnsupportedConsumerKind);
        }
        if operations.is_empty() || operations.len() > MAX_CREDENTIAL_OPERATIONS {
            return Err(BindingContractError::InvalidCollection);
        }
        ensure_unique(&operations).map_err(BindingContractError::from)?;
        Ok(Self {
            source_ref: source,
            consumer_ref: consumer,
            slot,
            audience,
            operations,
            lifetime,
        })
    }

    /// The binding kind this request belongs to.
    pub const fn kind(&self) -> BindingKind {
        BindingKind::Credential
    }

    /// Borrow the exact source Credential.
    pub const fn source_ref(&self) -> &ResourceRef {
        &self.source_ref
    }

    /// Borrow the exact consumer component.
    pub const fn consumer_ref(&self) -> &ResourceRef {
        &self.consumer_ref
    }

    /// Borrow the stable consumer slot.
    pub const fn slot(&self) -> &BindingSlot {
        &self.slot
    }

    /// Borrow the audience this delivery is for.
    pub const fn audience(&self) -> &BoundedToken {
        &self.audience
    }

    /// Borrow the admitted operation classes.
    pub fn operations(&self) -> &[CredentialOperation] {
        &self.operations
    }

    /// Borrow the requested lifetime bounds.
    pub const fn lifetime(&self) -> &CredentialLifetime {
        &self.lifetime
    }

    /// Whether this request admits `operation`.
    pub fn admits_operation(&self, operation: CredentialOperation) -> bool {
        self.operations.contains(&operation)
    }

    /// The right this request asks the source to admit.
    pub const fn requested_rights(&self) -> RequestedRights {
        RequestedRights::Consume
    }

    /// The realization facets this request depends on.
    pub const fn required_facets(&self) -> &'static [BindingRealizationFacet] {
        &[BindingRealizationFacet::CredentialDelivery]
    }

    /// Derive this relationship's KTD3 key from its committed identities.
    pub fn key(
        &self,
        zone: ZoneId,
        source_uid: ResourceUid,
        consumer_uid: ResourceUid,
    ) -> Result<BindingKey, BindingContractError> {
        BindingKey::new(
            zone,
            self.kind(),
            self.source_ref.clone(),
            source_uid,
            self.consumer_ref.clone(),
            consumer_uid,
            self.slot.clone(),
        )
    }

    /// The digest of this request's exact desired bytes.
    pub fn fingerprint(&self) -> BindingSpecFingerprint {
        BindingSpecFingerprint::from_request(self)
    }
}

redacted_debug!(CredentialBindingRequest);

wire_deserialize!(
    CredentialBindingRequest,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        source_ref: ResourceRef,
        consumer_ref: ResourceRef,
        slot: BindingSlot,
        audience: BoundedToken,
        operations: Vec<CredentialOperation>,
        lifetime: CredentialLifetime,
    },
    wire,
    Self::new(
        wire.source_ref,
        wire.consumer_ref,
        wire.slot,
        wire.audience,
        wire.operations,
        wire.lifetime,
    )
    .map_err(serde::de::Error::custom)
);

/// A Host or Guest credential attachment input, classified.
pub type CredentialExecutionParentInput = ExecutionParentInput<CredentialBindingRequest>;

/// Strict base CredentialBinding specification.
///
/// A CredentialBinding row delivers one named credential to one consumer for
/// a bounded lifetime and an explicit operation set. The row never carries
/// secret material: it names what is delivered and for how long, and the
/// realization crosses the provider boundary through the family's declared
/// effect port.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CredentialBindingSpec {
    credential_ref: ResourceRef,
    execution_ref: ResourceRef,
    operations: Vec<CredentialOperation>,
    lifetime_ms: u64,
    slot: BoundedToken,
    source: BindingSourceDecision,
}

impl CredentialBindingSpec {
    /// Construct a strict credential binding specification.
    ///
    /// The operation set must be non-empty, bounded by
    /// [`MAX_CREDENTIAL_OPERATIONS`], and duplicate-free: a delivery that
    /// grants more than it names, or the same right twice, is refused.
    pub fn new(
        credential_ref: ResourceRef,
        execution_ref: ResourceRef,
        operations: Vec<CredentialOperation>,
        lifetime_ms: u64,
        slot: BoundedToken,
        source: BindingSourceDecision,
    ) -> Result<Self, BindingRowError> {
        super::binding::admit_binding_row_refs(
            super::binding::BindingKind::Credential,
            &credential_ref,
            &execution_ref,
        )?;
        if operations.is_empty() || operations.len() > MAX_CREDENTIAL_OPERATIONS {
            return Err(BindingRowError::InvalidOperations);
        }
        let mut sorted = operations.clone();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.len() != operations.len() {
            return Err(BindingRowError::DuplicateOperation);
        }
        if !(MIN_CREDENTIAL_LIFETIME_MS..=MAX_CREDENTIAL_LIFETIME_MS).contains(&lifetime_ms) {
            return Err(BindingRowError::LifetimeOutOfBounds);
        }
        Ok(Self {
            credential_ref,
            execution_ref,
            operations,
            lifetime_ms,
            slot,
            source,
        })
    }

    /// Borrow the source provider's accepted decision for this relationship.
    pub const fn source(&self) -> &BindingSourceDecision {
        &self.source
    }

    /// Derive this committed relationship's KTD3 key from its identities.
    ///
    /// The same derivation the source-side request performs, over the row's own
    /// committed references: a boundary evaluating a committed row must reach
    /// exactly the key the source admitted, or it would refuse a relationship
    /// that exists.
    pub fn key(
        &self,
        zone: ZoneId,
        source_uid: ResourceUid,
        consumer_uid: ResourceUid,
    ) -> Result<BindingKey, BindingRowError> {
        BindingKey::new(
            zone,
            BindingKind::Credential,
            self.credential_ref.clone(),
            source_uid,
            self.execution_ref.clone(),
            consumer_uid,
            BindingSlot::parse(self.slot.as_str())
                .map_err(|_| BindingRowError::WrongSourceType)?,
        )
        .map_err(|_| BindingRowError::WrongSourceType)
    }

    /// Return the standard ResourceType name.
    pub const fn resource_type() -> &'static str {
        CREDENTIAL_BINDING_RESOURCE_TYPE
    }

    /// Borrow the bound Credential.
    pub const fn credential_ref(&self) -> &ResourceRef {
        &self.credential_ref
    }

    /// Borrow the consumer the credential is delivered to.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Borrow the operations the consumer may perform.
    pub fn operations(&self) -> &[CredentialOperation] {
        &self.operations
    }

    /// Borrow the bounded delivery lifetime in milliseconds.
    pub const fn lifetime_ms(&self) -> u64 {
        self.lifetime_ms
    }

    /// Borrow the consumer slot the delivery occupies.
    pub const fn slot(&self) -> &BoundedToken {
        &self.slot
    }
}

redacted_debug!(CredentialBindingSpec);

wire_deserialize!(
    CredentialBindingSpec,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        credential_ref: ResourceRef,
        execution_ref: ResourceRef,
        operations: Vec<CredentialOperation>,
        lifetime_ms: u64,
        slot: String,
        source: BindingSourceDecision,
    },
    wire,
    Self::new(
        wire.credential_ref,
        wire.execution_ref,
        wire.operations,
        wire.lifetime_ms,
        BoundedToken::parse(wire.slot).map_err(serde::de::Error::custom)?,
        wire.source,
    )
    .map_err(serde::de::Error::custom)
);
