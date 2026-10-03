//! The `EndpointBinding` ResourceType: one exact endpoint, one consumer.
//!
//! An `EndpointBinding` names one Endpoint, one admitted consumer, one stable
//! consumer slot, the protocol or attachment kind the consumer uses, and the
//! purpose it uses it for. The endpoint's locator is resolved privately from
//! the admitted source; the binding never grants access to the directory that
//! happens to contain the socket, and an environment variable cannot redirect
//! a consumer to a neighbouring one.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    ResourceRef,
    binding::{
        BindingRowError, BindingSourceDecision, BindingConsumerKind, BindingContractError, BindingKey, BindingKind, BindingRealizationFacet,
        BindingSlot, BindingSpecFingerprint, ExecutionParentInput, RequestedRights,
    },
    execution_policy::{BoundedToken, redacted_debug, require_resource_type},
    identity::{ResourceUid, ZoneId},
};
use d2b_contracts::wire_deserialize;

/// Canonical `EndpointBinding` ResourceType name.
pub const ENDPOINT_BINDING_RESOURCE_TYPE: &str = "EndpointBinding";

/// How one consumer reaches the exact endpoint.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum EndpointAttachmentKind {
    /// The consumer opens the endpoint as a client.
    Connect,
    /// The consumer accepts on the endpoint as a server.
    Listen,
    /// The consumer attaches to the endpoint's device or stream.
    Attach,
}

impl EndpointAttachmentKind {
    /// The right this attachment kind requests.
    pub const fn requested_rights(self) -> RequestedRights {
        match self {
            Self::Listen => RequestedRights::Observe,
            Self::Connect | Self::Attach => RequestedRights::Consume,
        }
    }

    /// The realization facets this attachment kind depends on.
    ///
    /// An attachment reaches a display or stream by name, so it needs a
    /// private binding of that exact socket as well as its descriptor. A
    /// connect or listen prefers the verified descriptor and takes the private
    /// pathname only where the backend requires one.
    pub const fn required_facets(self) -> &'static [BindingRealizationFacet] {
        const DESCRIPTOR: &[BindingRealizationFacet] = &[BindingRealizationFacet::EndpointDescriptor];
        const NAMED: &[BindingRealizationFacet] = &[
            BindingRealizationFacet::EndpointDescriptor,
            BindingRealizationFacet::EndpointPathname,
        ];
        match self {
            Self::Attach => NAMED,
            Self::Connect | Self::Listen => DESCRIPTOR,
        }
    }
}

/// The desired request for one exact endpoint used by one consumer.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EndpointBindingRequest {
    source_ref: ResourceRef,
    consumer_ref: ResourceRef,
    slot: BindingSlot,
    attachment: EndpointAttachmentKind,
    purpose: BoundedToken,
}

impl EndpointBindingRequest {
    /// Construct one request from typed references.
    ///
    /// # Errors
    ///
    /// Refuses a source that is not an `Endpoint` and a consumer this binding
    /// kind does not admit.
    pub fn new(
        source: ResourceRef,
        consumer: ResourceRef,
        slot: BindingSlot,
        attachment: EndpointAttachmentKind,
        purpose: BoundedToken,
    ) -> Result<Self, BindingContractError> {
        require_resource_type(&source, BindingKind::Endpoint.source_resource_type())?;
        let consumer_kind = BindingConsumerKind::from_resource_type(consumer.resource_type().as_str())
            .ok_or(BindingContractError::WrongResourceType)?;
        if !BindingKind::Endpoint.admits_consumer(consumer_kind) {
            return Err(BindingContractError::UnsupportedConsumerKind);
        }
        Ok(Self {
            source_ref: source,
            consumer_ref: consumer,
            slot,
            attachment,
            purpose,
        })
    }

    /// The binding kind this request belongs to.
    pub const fn kind(&self) -> BindingKind {
        BindingKind::Endpoint
    }

    /// Borrow the exact source Endpoint.
    pub const fn source_ref(&self) -> &ResourceRef {
        &self.source_ref
    }

    /// Borrow the exact consumer.
    pub const fn consumer_ref(&self) -> &ResourceRef {
        &self.consumer_ref
    }

    /// Borrow the stable consumer slot.
    pub const fn slot(&self) -> &BindingSlot {
        &self.slot
    }

    /// Return the requested attachment kind.
    pub const fn attachment(&self) -> EndpointAttachmentKind {
        self.attachment
    }

    /// Borrow the bounded usage purpose.
    pub const fn purpose(&self) -> &BoundedToken {
        &self.purpose
    }

    /// The right this request asks the source to admit.
    pub const fn requested_rights(&self) -> RequestedRights {
        self.attachment.requested_rights()
    }

    /// The realization facets this request depends on.
    pub const fn required_facets(&self) -> &'static [BindingRealizationFacet] {
        self.attachment.required_facets()
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

redacted_debug!(EndpointBindingRequest);

wire_deserialize!(
    EndpointBindingRequest,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        source_ref: ResourceRef,
        consumer_ref: ResourceRef,
        slot: BindingSlot,
        attachment: EndpointAttachmentKind,
        purpose: BoundedToken,
    },
    wire,
    Self::new(
        wire.source_ref,
        wire.consumer_ref,
        wire.slot,
        wire.attachment,
        wire.purpose,
    )
    .map_err(serde::de::Error::custom)
);

/// A Host or Guest endpoint attachment input, classified.
pub type EndpointExecutionParentInput = ExecutionParentInput<EndpointBindingRequest>;

/// Strict base EndpointBinding specification.
///
/// An EndpointBinding row delivers exactly one named endpoint to one
/// consumer. The consumer is whatever the kind admits - `Guest`, `Process`,
/// or `EphemeralProcess`, never `Host`, because a provider's host-side
/// delivery is an admitted realization leg of the binding whose consumer is
/// that helper, not a binding whose consumer is the Host.
///
/// The row carries no purpose: the endpoint's own spec carries it and the
/// admission path never reads it off the request, so a row that restated it
/// would let the two drift.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EndpointBindingSpec {
    endpoint_ref: ResourceRef,
    execution_ref: ResourceRef,
    attachment: EndpointAttachmentKind,
    slot: BoundedToken,
    source: BindingSourceDecision,
}

impl EndpointBindingSpec {
    /// Construct a strict endpoint binding specification from typed references.
    pub fn new(
        endpoint_ref: ResourceRef,
        execution_ref: ResourceRef,
        attachment: EndpointAttachmentKind,
        slot: BoundedToken,
        source: BindingSourceDecision,
    ) -> Result<Self, BindingRowError> {
        super::binding::admit_binding_row_refs(
            super::binding::BindingKind::Endpoint,
            &endpoint_ref,
            &execution_ref,
        )?;
        Ok(Self {
            endpoint_ref,
            execution_ref,
            attachment,
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
            BindingKind::Endpoint,
            self.endpoint_ref.clone(),
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
        ENDPOINT_BINDING_RESOURCE_TYPE
    }

    /// Borrow the bound Endpoint.
    pub const fn endpoint_ref(&self) -> &ResourceRef {
        &self.endpoint_ref
    }

    /// Borrow the consumer that receives the exact endpoint.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Borrow the requested attachment kind.
    pub const fn attachment(&self) -> &EndpointAttachmentKind {
        &self.attachment
    }

    /// Borrow the consumer slot the delivery occupies.
    pub const fn slot(&self) -> &BoundedToken {
        &self.slot
    }
}

redacted_debug!(EndpointBindingSpec);

wire_deserialize!(
    EndpointBindingSpec,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        endpoint_ref: ResourceRef,
        execution_ref: ResourceRef,
        attachment: EndpointAttachmentKind,
        slot: String,
        source: BindingSourceDecision,
    },
    wire,
    Self::new(
        wire.endpoint_ref,
        wire.execution_ref,
        wire.attachment,
        BoundedToken::parse(wire.slot).map_err(serde::de::Error::custom)?,
        wire.source,
    )
    .map_err(serde::de::Error::custom)
);
