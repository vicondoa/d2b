//! The `DeviceBinding` ResourceType: one admitted use of a device capability.
//!
//! A `DeviceBinding` names one Device, one admitted consumer, one stable
//! consumer slot, the named function that is being claimed, the requested
//! shared or exclusive claim, and how the claim is attached. The physical
//! authority key is not authored here: it comes from the Device provider's
//! trusted inventory, so device permission is never derived from a seccomp
//! name, a launch role, or a host path.
//!
//! Arbitration belongs to the source. Two consumers requesting an exclusive
//! capability are arbitrated once, by the Device provider, and release proof
//! gates reassignment; a helper that realizes this binding uses an explicitly
//! bound leg of this reservation rather than taking a second device claim.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    ResourceRef,
    binding::{
        BindingConsumerKind, BindingRowError, BindingContractError, BindingKey, BindingKind, BindingRealizationFacet,
        BindingSlot, BindingSpecFingerprint, ExecutionParentInput, RequestedRights,
    },
    execution_policy::{BoundedToken, PrimitiveSpecError, redacted_debug, require_resource_type},
    identity::{ResourceUid, ZoneId},
};
use d2b_contracts::wire_deserialize;

/// Canonical `DeviceBinding` ResourceType name.
pub const DEVICE_BINDING_RESOURCE_TYPE: &str = "DeviceBinding";

/// The named function one device claim covers.
///
/// The Device provider resolves the name against its trusted inventory; this
/// contract never carries the physical identity, a device node path, or a
/// host permission bit that could be widened later.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceFunction(BoundedToken);

impl DeviceFunction {
    /// Parse a `^[a-z][a-z0-9-]*$` function token.
    pub fn parse(value: impl Into<String>) -> Result<Self, PrimitiveSpecError> {
        BoundedToken::parse(value).map(Self)
    }

    /// Borrow the canonical function token.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

redacted_debug!(DeviceFunction);

/// How one consumer claims the device capability.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceClaimRequest {
    /// The consumer shares the capability with its peers.
    Shared,
    /// The consumer holds the capability alone until it releases.
    Exclusive,
}

impl DeviceClaimRequest {
    /// The shared right this claim requests.
    pub const fn requested_rights(self) -> RequestedRights {
        match self {
            Self::Shared => RequestedRights::Share,
            Self::Exclusive => RequestedRights::Exclusive,
        }
    }
}

/// How an admitted device claim reaches the consumer.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceAttachmentMode {
    /// A verified descriptor passed into the consumer.
    Descriptor,
    /// A mediated attachment owned by the Device provider.
    Mediated,
}

/// The desired request for one device capability used by one consumer.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceBindingRequest {
    source_ref: ResourceRef,
    consumer_ref: ResourceRef,
    slot: BindingSlot,
    function: DeviceFunction,
    claim: DeviceClaimRequest,
    attachment: DeviceAttachmentMode,
}

impl DeviceBindingRequest {
    /// Construct one request from typed references.
    ///
    /// # Errors
    ///
    /// Refuses a source that is not a `Device`, a consumer this binding kind
    /// does not admit, and a claim whose right the kind does not admit.
    pub fn new(
        source: ResourceRef,
        consumer: ResourceRef,
        slot: BindingSlot,
        function: DeviceFunction,
        claim: DeviceClaimRequest,
        attachment: DeviceAttachmentMode,
    ) -> Result<Self, BindingContractError> {
        require_resource_type(&source, BindingKind::Device.source_resource_type())?;
        let consumer_kind = BindingConsumerKind::from_resource_type(consumer.resource_type().as_str())
            .ok_or(BindingContractError::WrongResourceType)?;
        if !BindingKind::Device.admits_consumer(consumer_kind)
            || !BindingKind::Device.admits_rights(claim.requested_rights())
        {
            return Err(BindingContractError::UnsupportedRight);
        }
        Ok(Self {
            source_ref: source,
            consumer_ref: consumer,
            slot,
            function,
            claim,
            attachment,
        })
    }

    /// The binding kind this request belongs to.
    pub const fn kind(&self) -> BindingKind {
        BindingKind::Device
    }

    /// Borrow the exact source Device.
    pub const fn source_ref(&self) -> &ResourceRef {
        &self.source_ref
    }

    /// Borrow the stable consumer slot.
    pub const fn slot(&self) -> &BindingSlot {
        &self.slot
    }

    /// Borrow the exact consumer.
    ///
    /// Present on every family request, so one relation index resolves a
    /// declared consumer the same way for all five.
    pub const fn consumer_ref(&self) -> &ResourceRef {
        &self.consumer_ref
    }

    /// Borrow the named device function.
    pub const fn function(&self) -> &DeviceFunction {
        &self.function
    }

    /// Return the requested claim.
    pub const fn claim(&self) -> DeviceClaimRequest {
        self.claim
    }

    /// Return the requested attachment mode.
    pub const fn attachment(&self) -> DeviceAttachmentMode {
        self.attachment
    }

    /// The right this request asks the source to admit.
    pub const fn requested_rights(&self) -> RequestedRights {
        self.claim.requested_rights()
    }

    /// The realization facets this request depends on.
    pub const fn required_facets(&self) -> &'static [BindingRealizationFacet] {
        &[BindingRealizationFacet::DeviceAttachment]
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

redacted_debug!(DeviceBindingRequest);

wire_deserialize!(
    DeviceBindingRequest,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        source_ref: ResourceRef,
        consumer_ref: ResourceRef,
        slot: BindingSlot,
        function: DeviceFunction,
        claim: DeviceClaimRequest,
        attachment: DeviceAttachmentMode,
    },
    wire,
    Self::new(
        wire.source_ref,
        wire.consumer_ref,
        wire.slot,
        wire.function,
        wire.claim,
        wire.attachment,
    )
    .map_err(serde::de::Error::custom)
);

/// A Host or Guest device attachment input, classified.
pub type DeviceExecutionParentInput = ExecutionParentInput<DeviceBindingRequest>;

/// Strict base DeviceBinding specification.
///
/// A DeviceBinding row is the committed attachment of one named device
/// function to one consumer. The `function` is part of the row rather than
/// implied by the device: [`DeviceInventory::entry`] is keyed by the function
/// alone, so a row that did not name one would be a claim on "some function
/// of this device" - precisely the template-shaped grant this contract
/// removes. The consumer is whatever the kind admits, not a fixed `Guest`.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceBindingSpec {
    device_ref: ResourceRef,
    execution_ref: ResourceRef,
    function: DeviceFunction,
    claim: DeviceClaimRequest,
    slot: BoundedToken,
}

impl DeviceBindingSpec {
    /// Construct a strict device binding specification from typed references.
    pub fn new(
        device_ref: ResourceRef,
        execution_ref: ResourceRef,
        function: DeviceFunction,
        claim: DeviceClaimRequest,
        slot: BoundedToken,
    ) -> Result<Self, BindingRowError> {
        super::binding::admit_binding_row_refs(
            super::binding::BindingKind::Device,
            &device_ref,
            &execution_ref,
        )?;
        Ok(Self {
            device_ref,
            execution_ref,
            function,
            claim,
            slot,
        })
    }

    /// Return the standard ResourceType name.
    pub const fn resource_type() -> &'static str {
        DEVICE_BINDING_RESOURCE_TYPE
    }

    /// Borrow the bound Device.
    pub const fn device_ref(&self) -> &ResourceRef {
        &self.device_ref
    }

    /// Borrow the consumer that receives the attachment.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Borrow the device function this claim covers.
    pub const fn function(&self) -> &DeviceFunction {
        &self.function
    }

    /// Borrow the requested claim.
    pub const fn claim(&self) -> &DeviceClaimRequest {
        &self.claim
    }

    /// Borrow the consumer slot the attachment occupies.
    pub const fn slot(&self) -> &BoundedToken {
        &self.slot
    }
}

redacted_debug!(DeviceBindingSpec);

wire_deserialize!(
    DeviceBindingSpec,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        device_ref: ResourceRef,
        execution_ref: ResourceRef,
        function: DeviceFunction,
        claim: DeviceClaimRequest,
        slot: String,
    },
    wire,
    Self::new(
        wire.device_ref,
        wire.execution_ref,
        wire.function,
        wire.claim,
        BoundedToken::parse(wire.slot).map_err(serde::de::Error::custom)?,
    )
    .map_err(serde::de::Error::custom)
);
