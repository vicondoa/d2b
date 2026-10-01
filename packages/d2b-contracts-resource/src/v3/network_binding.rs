//! The `NetworkBinding` ResourceType: one consumer's network membership.
//!
//! A `NetworkBinding` names one Network, one admitted consumer, one stable
//! consumer slot, the membership and traffic policy that consumer asks for,
//! and how the membership is presented to that consumer. Fabric topology and
//! the shared interface realization stay with the Network provider: this
//! contract states one consumer's requirement and never authorizes a second
//! consumer's use of the same fabric.
//!
//! Per-consumer policy and leases stay distinct. Two processes on one network
//! and one execution target keep separate requirements while the provider
//! realizes the shared fabric once.

use schemars::JsonSchema;
use serde::Serialize;

use super::{
    ResourceRef,
    binding::{
        BindingRowError, BindingSourceDecision, BindingConsumerKind, BindingContractError, BindingKey, BindingKind, BindingRealizationFacet,
        BindingSlot, BindingSpecFingerprint, ExecutionParentInput, RequestedRights,
    },
    execution_policy::{BoundedToken, redacted_debug, require_resource_type},
    identity::{ResourceUid, ZoneId},
    process::{MAX_PORTS, PortSpec},
};
use d2b_contracts::wire_deserialize;

/// Canonical `NetworkBinding` ResourceType name.
pub const NETWORK_BINDING_RESOURCE_TYPE: &str = "NetworkBinding";
/// How one consumer's membership reaches it.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields, tag = "presentation")]
pub enum NetworkPresentation {
    /// An interface with the requested name inside the consumer's own
    /// network namespace.
    #[serde(rename = "namespace-interface")]
    NamespaceInterface {
        /// The requested interface name inside the consumer.
        name: BoundedToken,
    },
    /// Membership through the provider-owned shared fabric realization.
    #[serde(rename = "shared-fabric")]
    SharedFabric,
}

impl NetworkPresentation {
    /// Construct a namespace presentation after validating its name.
    pub fn namespace_interface(name: impl Into<String>) -> Result<Self, BindingContractError> {
        BoundedToken::parse(name.into())
            .map(|name| Self::NamespaceInterface { name })
            .map_err(BindingContractError::from)
    }

    /// Construct the shared-fabric presentation.
    pub const fn shared_fabric() -> Self {
        Self::SharedFabric
    }

    /// The realization facet this presentation requires.
    pub const fn required_facets(&self) -> &'static [BindingRealizationFacet] {
        match self {
            Self::NamespaceInterface { .. } => &[BindingRealizationFacet::NamespaceInterface],
            Self::SharedFabric => &[BindingRealizationFacet::SharedFabric],
        }
    }

    /// Borrow the requested interface name of a namespace presentation.
    pub const fn interface_name(&self) -> Option<&BoundedToken> {
        match self {
            Self::NamespaceInterface { name } => Some(name),
            Self::SharedFabric => None,
        }
    }
}

redacted_debug!(NetworkPresentation);

wire_deserialize!(
    NetworkPresentation,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        presentation: String,
        name: Option<String>,
    },
    wire,
    match (wire.presentation.as_str(), wire.name) {
        ("namespace-interface", Some(name)) => Self::namespace_interface(name),
        ("shared-fabric", None) => Ok(Self::SharedFabric),
        _ => Err(BindingContractError::InvalidField),
    }
    .map_err(serde::de::Error::custom)
);

/// The membership and traffic policy one consumer requests.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkMembership {
    ports: Vec<PortSpec>,
    allow_egress: bool,
}

impl NetworkMembership {
    /// Construct a membership request after checking its bounds.
    pub fn new(ports: Vec<PortSpec>, allow_egress: bool) -> Result<Self, BindingContractError> {
        if ports.len() > MAX_PORTS {
            return Err(BindingContractError::InvalidCollection);
        }
        Ok(Self { ports, allow_egress })
    }

    /// Borrow the requested inbound ports.
    pub fn ports(&self) -> &[PortSpec] {
        &self.ports
    }

    /// Whether this consumer may initiate outbound connections.
    pub const fn allow_egress(&self) -> bool {
        self.allow_egress
    }
}

redacted_debug!(NetworkMembership);

wire_deserialize!(
    NetworkMembership,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        #[serde(default)]
        ports: Vec<PortSpec>,
        #[serde(default)]
        allow_egress: bool,
    },
    wire,
    Self::new(wire.ports, wire.allow_egress).map_err(serde::de::Error::custom)
);

/// The desired request for one consumer's membership on one Network.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkBindingRequest {
    source_ref: ResourceRef,
    consumer_ref: ResourceRef,
    slot: BindingSlot,
    membership: NetworkMembership,
    presentation: NetworkPresentation,
}

impl NetworkBindingRequest {
    /// Construct one request from typed references.
    ///
    /// # Errors
    ///
    /// Refuses a source that is not a `Network` and a consumer this binding
    /// kind does not admit.
    pub fn new(
        source: ResourceRef,
        consumer: ResourceRef,
        slot: BindingSlot,
        membership: NetworkMembership,
        presentation: NetworkPresentation,
    ) -> Result<Self, BindingContractError> {
        require_resource_type(&source, BindingKind::Network.source_resource_type())?;
        let consumer_kind = BindingConsumerKind::from_resource_type(consumer.resource_type().as_str())
            .ok_or(BindingContractError::WrongResourceType)?;
        if !BindingKind::Network.admits_consumer(consumer_kind) {
            return Err(BindingContractError::UnsupportedConsumerKind);
        }
        Ok(Self {
            source_ref: source,
            consumer_ref: consumer,
            slot,
            membership,
            presentation,
        })
    }

    /// The binding kind this request belongs to.
    pub const fn kind(&self) -> BindingKind {
        BindingKind::Network
    }

    /// Borrow the exact source Network.
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

    /// Borrow the requested membership and traffic policy.
    pub const fn membership(&self) -> &NetworkMembership {
        &self.membership
    }

    /// Borrow the consumer-side presentation.
    pub const fn presentation(&self) -> &NetworkPresentation {
        &self.presentation
    }

    /// The right this request asks the source to admit.
    pub const fn requested_rights(&self) -> RequestedRights {
        RequestedRights::Consume
    }

    /// The realization facets this request depends on.
    pub const fn required_facets(&self) -> &'static [BindingRealizationFacet] {
        self.presentation.required_facets()
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

redacted_debug!(NetworkBindingRequest);

wire_deserialize!(
    NetworkBindingRequest,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        source_ref: ResourceRef,
        consumer_ref: ResourceRef,
        slot: BindingSlot,
        membership: NetworkMembership,
        presentation: NetworkPresentation,
    },
    wire,
    Self::new(
        wire.source_ref,
        wire.consumer_ref,
        wire.slot,
        wire.membership,
        wire.presentation,
    )
    .map_err(serde::de::Error::custom)
);

/// A Host or Guest network attachment input, classified.
pub type NetworkExecutionParentInput = ExecutionParentInput<NetworkBindingRequest>;

/// Strict base NetworkBinding specification.
///
/// A NetworkBinding row is one consumer's membership in a shared fabric. It
/// carries no ports, egress rules, or other per-consumer traffic policy: the
/// firewall is the Network's single ownership slot, so a second per-consumer
/// ruleset here would fork that authority. The membership policy is the
/// source provider's admitted state and is resolved against the row, never
/// restated by it.
///
/// The consumer is whatever the kind admits, which includes `Host`: a host
/// consuming a fabric as its own parent is a distinct, live case.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkBindingSpec {
    network_ref: ResourceRef,
    execution_ref: ResourceRef,
    presentation: NetworkPresentation,
    source: BindingSourceDecision,
}

impl NetworkBindingSpec {
    /// Construct a strict network binding specification from typed references.
    pub fn new(
        network_ref: ResourceRef,
        execution_ref: ResourceRef,
        presentation: NetworkPresentation,
        source: BindingSourceDecision,
    ) -> Result<Self, BindingRowError> {
        super::binding::admit_binding_row_refs(
            super::binding::BindingKind::Network,
            &network_ref,
            &execution_ref,
        )?;
        Ok(Self {
            network_ref,
            execution_ref,
            presentation,
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
            BindingKind::Network,
            self.network_ref.clone(),
            source_uid,
            self.execution_ref.clone(),
            consumer_uid,
            BindingSlot::parse("fabric").map_err(|_| BindingRowError::WrongSourceType)?,
        )
        .map_err(|_| BindingRowError::WrongSourceType)
    }

    /// Return the standard ResourceType name.
    pub const fn resource_type() -> &'static str {
        NETWORK_BINDING_RESOURCE_TYPE
    }

    /// Borrow the bound Network.
    pub const fn network_ref(&self) -> &ResourceRef {
        &self.network_ref
    }

    /// Borrow the consumer that joins the fabric.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Borrow the requested presentation.
    pub const fn presentation(&self) -> &NetworkPresentation {
        &self.presentation
    }
}

redacted_debug!(NetworkBindingSpec);

wire_deserialize!(
    NetworkBindingSpec,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        network_ref: ResourceRef,
        execution_ref: ResourceRef,
        presentation: NetworkPresentation,
        source: BindingSourceDecision,
    },
    wire,
    Self::new(
        wire.network_ref,
        wire.execution_ref,
        wire.presentation,
        wire.source,
    )
        .map_err(serde::de::Error::custom)
);
