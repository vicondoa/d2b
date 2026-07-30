//! Provider agent bootstrap through the Zone allocator.
//!
//! A v3 Provider agent does not adopt a listening socket from its service
//! manager and performs no privileged handshake of its own. The Zone
//! allocator, which already owns the agent's ComponentSession, issues a
//! bootstrap binding naming exactly which `Provider/<name>` the agent is,
//! which Zone it serves, and which session purpose that ComponentSession
//! carries. The agent entrypoint checks the binding against what it was
//! built to be and fails closed on any disagreement.
//!
//! Two properties are deliberate.
//!
//! The binding carries no file descriptor, socket path, host path, or peer
//! credential. The ComponentSession itself stays with the Zone runtime; the
//! toolkit only decides whether the identity the allocator asserts is the
//! identity this agent may run as.
//!
//! Neither the binding nor [`ProviderAgentIdentity`] authorizes anything.
//! Holding one grants no call, no route, and no effect: authorization stays
//! with the ComponentSession admission and the Zone RBAC binding. The
//! identity exists so the agent can label its own audit events and refuse to
//! serve a Zone it was not placed in. To keep bootstrap evidence
//! non-replayable the binding is consumed by value and is not `Clone`.

use d2b_contracts::v3::zone_routing::ZonePath;
use d2b_contracts::v3::{Locality, ResourceRef, SessionPurpose, TransportBinding};

use crate::error::ProviderToolkitError;

/// The `ResourceType` a Provider agent's own resource reference must name.
pub const PROVIDER_RESOURCE_TYPE: &str = "Provider";

/// The allocator-issued bootstrap binding a Provider agent process receives
/// in place of an adopted listening descriptor.
///
/// Constructed only by the Zone allocator side and consumed exactly once by
/// [`ProviderAgentBootstrap::admit`].
#[derive(Debug)]
pub struct AllocatorSessionBinding {
    zone: ZonePath,
    provider_ref: ResourceRef,
    session_purpose: SessionPurpose,
    transport: TransportBinding,
}

impl AllocatorSessionBinding {
    /// Issue a bootstrap binding.
    pub const fn new(
        zone: ZonePath,
        provider_ref: ResourceRef,
        session_purpose: SessionPurpose,
        transport: TransportBinding,
    ) -> Self {
        Self {
            zone,
            provider_ref,
            session_purpose,
            transport,
        }
    }
}

/// The validated, non-authorizing identity a Provider agent runs under.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderAgentIdentity {
    zone: ZonePath,
    provider_ref: ResourceRef,
}

impl ProviderAgentIdentity {
    /// Borrow the Zone the agent serves.
    pub const fn zone(&self) -> &ZonePath {
        &self.zone
    }

    /// Borrow the `Provider/<name>` reference the agent implements.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }
}

impl core::fmt::Debug for ProviderAgentIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ProviderAgentIdentity(<redacted>)")
    }
}

/// What a Provider agent entrypoint was built to be.
///
/// The expectation is compiled into the agent binary, never read from the
/// binding, so a binding cannot tell an agent it is a different Provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAgentBootstrap {
    expected_provider: ResourceRef,
    expected_zone: ZonePath,
    accepted_purpose: SessionPurpose,
}

impl ProviderAgentBootstrap {
    /// Declare the agent's compiled expectation.
    pub const fn new(
        expected_provider: ResourceRef,
        expected_zone: ZonePath,
        accepted_purpose: SessionPurpose,
    ) -> Self {
        Self {
            expected_provider,
            expected_zone,
            accepted_purpose,
        }
    }

    /// Consume an allocator-issued binding and return the validated agent
    /// identity.
    ///
    /// Every disagreement fails closed with no partial admission: a
    /// different Provider, a different Zone, a reference that is not a
    /// `Provider` resource, a session purpose the entrypoint does not
    /// accept, or a session the allocator did not issue locally.
    pub fn admit(
        &self,
        binding: AllocatorSessionBinding,
    ) -> Result<ProviderAgentIdentity, ProviderToolkitError> {
        if binding.provider_ref.resource_type().as_str() != PROVIDER_RESOURCE_TYPE {
            return Err(ProviderToolkitError::BootstrapRefWrongType);
        }
        if binding.provider_ref != self.expected_provider {
            return Err(ProviderToolkitError::BootstrapProviderMismatch);
        }
        if binding.zone != self.expected_zone {
            return Err(ProviderToolkitError::BootstrapZoneMismatch);
        }
        if binding.session_purpose != self.accepted_purpose {
            return Err(ProviderToolkitError::BootstrapPurposeMismatch);
        }
        if binding.transport.locality() != Locality::Local {
            return Err(ProviderToolkitError::BootstrapLocalityRejected);
        }
        Ok(ProviderAgentIdentity {
            zone: binding.zone,
            provider_ref: binding.provider_ref,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::zone_routing::ZoneLabelId;
    use d2b_contracts::v3::{BindingDigest, ResourceName, ResourceTypeName};

    fn zone(label: &str) -> ZonePath {
        ZonePath::new(vec![ZoneLabelId::parse(label).expect("valid label")])
            .expect("valid zone path")
    }

    fn provider(name: &str) -> ResourceRef {
        ResourceRef::new(
            ResourceTypeName::parse(PROVIDER_RESOURCE_TYPE).expect("valid type"),
            ResourceName::parse(name).expect("valid name"),
        )
    }

    fn purpose() -> SessionPurpose {
        SessionPurpose::parse("provider-agent").expect("valid purpose")
    }

    fn transport(locality: Locality) -> TransportBinding {
        TransportBinding::new(
            locality,
            BindingDigest::parse(format!("sha256:{}", "a".repeat(64))).expect("valid digest"),
        )
    }

    fn bootstrap() -> ProviderAgentBootstrap {
        ProviderAgentBootstrap::new(provider("volume-local"), zone("work"), purpose())
    }

    // Validation obligation: v3 bootstrap via the Zone allocator, in place
    // of the ADR45 service-manager descriptor adoption.
    #[test]
    fn an_allocator_issued_binding_admits_the_expected_agent_identity() {
        let identity = bootstrap()
            .admit(AllocatorSessionBinding::new(
                zone("work"),
                provider("volume-local"),
                purpose(),
                transport(Locality::Local),
            ))
            .expect("the matching binding is admitted");
        assert_eq!(identity.zone(), &zone("work"));
        assert_eq!(identity.provider_ref(), &provider("volume-local"));
    }

    #[test]
    fn a_binding_for_another_provider_or_zone_fails_closed() {
        assert_eq!(
            bootstrap()
                .admit(AllocatorSessionBinding::new(
                    zone("work"),
                    provider("volume-virtiofs"),
                    purpose(),
                    transport(Locality::Local),
                ))
                .unwrap_err(),
            ProviderToolkitError::BootstrapProviderMismatch
        );
        assert_eq!(
            bootstrap()
                .admit(AllocatorSessionBinding::new(
                    zone("personal"),
                    provider("volume-local"),
                    purpose(),
                    transport(Locality::Local),
                ))
                .unwrap_err(),
            ProviderToolkitError::BootstrapZoneMismatch
        );
    }

    #[test]
    fn a_binding_that_is_not_a_provider_resource_fails_closed() {
        let not_a_provider = ResourceRef::new(
            ResourceTypeName::parse("Process").expect("valid type"),
            ResourceName::parse("volume-local").expect("valid name"),
        );
        assert_eq!(
            bootstrap()
                .admit(AllocatorSessionBinding::new(
                    zone("work"),
                    not_a_provider,
                    purpose(),
                    transport(Locality::Local),
                ))
                .unwrap_err(),
            ProviderToolkitError::BootstrapRefWrongType
        );
    }

    #[test]
    fn a_wrong_purpose_or_non_local_session_fails_closed() {
        assert_eq!(
            bootstrap()
                .admit(AllocatorSessionBinding::new(
                    zone("work"),
                    provider("volume-local"),
                    SessionPurpose::parse("resource-api").expect("valid purpose"),
                    transport(Locality::Local),
                ))
                .unwrap_err(),
            ProviderToolkitError::BootstrapPurposeMismatch
        );
        assert_eq!(
            bootstrap()
                .admit(AllocatorSessionBinding::new(
                    zone("work"),
                    provider("volume-local"),
                    purpose(),
                    transport(Locality::Remote),
                ))
                .unwrap_err(),
            ProviderToolkitError::BootstrapLocalityRejected
        );
    }

    #[test]
    fn the_identity_never_renders_its_principal_in_debug() {
        let identity = bootstrap()
            .admit(AllocatorSessionBinding::new(
                zone("work"),
                provider("volume-local"),
                purpose(),
                transport(Locality::Local),
            ))
            .expect("admitted");
        let rendered = format!("{identity:?}");
        assert!(!rendered.contains("work"));
        assert!(!rendered.contains("volume-local"));
    }
}
