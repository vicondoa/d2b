//! Guest-local seed vocabulary and the resolved Guest-control Endpoint.
//!
//! The signed Guest seed schema admits the ResourceTypes below; the
//! authenticated Guest-control Endpoint identity is the only locator-free fact
//! the controller publishes. Endpoint carriage, credentials, and target-local
//! effects remain behind the authenticated session and its effect owner.

use std::fmt;

use d2b_contracts_resource::v3::{
    ResourceGeneration, ResourceRef, ResourceUid, SchemaFingerprint, ZoneId,
    activation_nixos::NIXOS_GENERATION_RESOURCE_TYPE,
};

/// Resource types admitted by the signed Guest seed schema.
pub const GUEST_SEED_RESOURCE_TYPES: &[&str] = &[
    "Process",
    "EphemeralProcess",
    "Endpoint",
    NIXOS_GENERATION_RESOURCE_TYPE,
];

/// Failures at the Guest-local session and seed boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestLocalError {
    /// Endpoint identity or generation did not match the Guest contract.
    EndpointMismatch,
}

impl GuestLocalError {
    /// Return the stable identity-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::EndpointMismatch => "guest-local-endpoint-mismatch",
        }
    }
}

impl fmt::Display for GuestLocalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for GuestLocalError {}

/// Authenticated, locator-free identity of a Guest-control Endpoint.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestControlEndpoint {
    endpoint_ref: ResourceRef,
    guest_ref: ResourceRef,
    zone: ZoneId,
    uid: ResourceUid,
    resource_generation: ResourceGeneration,
    endpoint_generation: ResourceGeneration,
    provider_generation: ResourceGeneration,
    schema_digest: SchemaFingerprint,
    ready: bool,
}

impl GuestControlEndpoint {
    /// Construct one resolved Guest-control Endpoint identity.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        endpoint_ref: ResourceRef,
        guest_ref: ResourceRef,
        zone: ZoneId,
        uid: ResourceUid,
        resource_generation: ResourceGeneration,
        endpoint_generation: ResourceGeneration,
        provider_generation: ResourceGeneration,
        schema_digest: SchemaFingerprint,
        ready: bool,
    ) -> Result<Self, GuestLocalError> {
        if endpoint_ref.resource_type().as_str() != "Endpoint"
            || guest_ref.resource_type().as_str() != "Guest"
            || zone.as_str().is_empty()
            || resource_generation.get() == 0
            || endpoint_generation.get() == 0
            || provider_generation.get() == 0
            || !ready
        {
            return Err(GuestLocalError::EndpointMismatch);
        }
        Ok(Self {
            endpoint_ref,
            guest_ref,
            zone,
            uid,
            resource_generation,
            endpoint_generation,
            provider_generation,
            schema_digest,
            ready,
        })
    }

    /// Borrow the exact Endpoint ResourceRef.
    pub const fn endpoint_ref(&self) -> &ResourceRef {
        &self.endpoint_ref
    }

    /// Borrow the producing Guest ResourceRef.
    pub const fn guest_ref(&self) -> &ResourceRef {
        &self.guest_ref
    }

    /// Borrow the exact Endpoint Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the store-assigned Endpoint UID.
    pub const fn uid(&self) -> &ResourceUid {
        &self.uid
    }

    /// Borrow the store-assigned Endpoint UID.
    pub const fn endpoint_uid(&self) -> &ResourceUid {
        &self.uid
    }

    /// Return the Endpoint Resource generation.
    pub const fn resource_generation(&self) -> ResourceGeneration {
        self.resource_generation
    }

    /// Return the producer-derived Endpoint generation.
    pub const fn endpoint_generation(&self) -> ResourceGeneration {
        self.endpoint_generation
    }

    /// Return the Provider generation observed with the Endpoint.
    pub const fn provider_generation(&self) -> ResourceGeneration {
        self.provider_generation
    }

    /// Borrow the target-local schema commitment.
    pub const fn schema_digest(&self) -> &SchemaFingerprint {
        &self.schema_digest
    }

    /// Whether the Endpoint is currently ready for an authenticated session.
    pub const fn ready(&self) -> bool {
        self.ready
    }

    /// Validate this resolution against one exact Guest and Provider contract.
    pub fn validate_for(
        &self,
        endpoint_ref: &ResourceRef,
        guest_ref: &ResourceRef,
        provider_generation: ResourceGeneration,
        schema_digest: &SchemaFingerprint,
    ) -> Result<(), GuestLocalError> {
        if !self.ready
            || &self.endpoint_ref != endpoint_ref
            || &self.guest_ref != guest_ref
            || self.provider_generation != provider_generation
            || &self.schema_digest != schema_digest
        {
            return Err(GuestLocalError::EndpointMismatch);
        }
        Ok(())
    }
}

impl fmt::Debug for GuestControlEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestControlEndpoint")
            .field("ready", &self.ready)
            .field("has_endpoint_uid", &true)
            .field("resource_generation", &self.resource_generation)
            .field("endpoint_generation", &self.endpoint_generation)
            .field("provider_generation", &self.provider_generation)
            .field("has_schema_digest", &true)
            .finish()
    }
}
