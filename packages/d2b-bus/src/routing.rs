//! Adapted Zone service routing table.

use std::collections::BTreeMap;

use d2b_contracts::v3::{identity::ZoneId, zone_session::EndpointRole};

/// The v3 Zone control service package.
pub const ZONE_SERVICE_NAME: &str = "d2b.zone.v3.ZoneService";

/// Maximum concurrently dispatched Zone service calls.
pub const MAX_DISPATCH_IN_FLIGHT: usize = 64;

/// Maximum registered Zone bindings.
pub const DEFAULT_MAX_ZONE_BINDINGS: usize = 256;

/// Maximum retained shortcut routes.
pub const DEFAULT_MAX_SHORTCUTS: usize = 256;

/// Maximum audit/mutation records retained by one service instance.
pub const DEFAULT_MAX_MUTATION_RECORDS: usize = 1024;

/// Credential custody held by a Zone session authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialCustody {
    /// Host-local and controller sessions hold no guest credential.
    None,
    /// A gateway/guest session owns the gateway credential custody.
    GatewayGuest,
}

/// A Zone-scoped session authority used by the routing service.
///
/// This is descriptive admission state, not a capability. It carries no
/// secret, socket, subject, or store handle and cannot mint one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneSessionAuthority {
    zone: ZoneId,
    peer_role: EndpointRole,
    custody: CredentialCustody,
}

impl ZoneSessionAuthority {
    /// Construct a host/controller authority with no guest credential custody.
    pub fn local_controller(zone: ZoneId) -> Result<Self, RouteServiceError> {
        Self::new(zone, EndpointRole::ZoneController, CredentialCustody::None)
    }

    /// Construct a guest-agent authority with gateway credential custody.
    pub fn gateway_peer(zone: ZoneId) -> Result<Self, RouteServiceError> {
        Self::new(
            zone,
            EndpointRole::GuestAgent,
            CredentialCustody::GatewayGuest,
        )
    }

    /// Construct an authority after validating role/custody pairing.
    pub fn new(
        zone: ZoneId,
        peer_role: EndpointRole,
        custody: CredentialCustody,
    ) -> Result<Self, RouteServiceError> {
        let expected = match peer_role {
            EndpointRole::ZoneController | EndpointRole::HostAgent => CredentialCustody::None,
            EndpointRole::GuestAgent | EndpointRole::Provider => CredentialCustody::GatewayGuest,
            EndpointRole::Component
            | EndpointRole::UserAgent
            | EndpointRole::ZoneRelay
            | EndpointRole::ZoneBootstrap => return Err(RouteServiceError::RoleNotAdmissible),
        };
        if expected != custody {
            return Err(RouteServiceError::CustodyMismatch);
        }
        Ok(Self {
            zone,
            peer_role,
            custody,
        })
    }

    /// Borrow the Zone identity.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Return the admitted endpoint role.
    pub const fn peer_role(&self) -> EndpointRole {
        self.peer_role
    }

    /// Return the credential-custody posture.
    pub const fn custody(&self) -> CredentialCustody {
        self.custody
    }
}

/// Closed Zone service routing failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteServiceError {
    /// The endpoint role cannot own a Zone service session.
    RoleNotAdmissible,
    /// Role and credential custody disagree.
    CustodyMismatch,
    /// The bounded dispatch admission is full.
    DispatchSaturated,
    /// A service shutdown is already in progress.
    ShuttingDown,
}

impl RouteServiceError {
    /// Return the stable failure code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::RoleNotAdmissible => "zone-service-role-not-admissible",
            Self::CustodyMismatch => "zone-service-custody-mismatch",
            Self::DispatchSaturated => "zone-service-dispatch-saturated",
            Self::ShuttingDown => "zone-service-shutting-down",
        }
    }
}

impl core::fmt::Display for RouteServiceError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for RouteServiceError {}

/// Closed route direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteDirection {
    /// Component-local route.
    Local,
    /// Host-side route.
    Host,
    /// Guest-side route.
    Guest,
    /// ZoneLink route.
    ZoneLink,
}

impl RouteDirection {
    /// Stable wire label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Host => "host",
            Self::Guest => "guest",
            Self::ZoneLink => "zone_link",
        }
    }
}

/// Stable routing error class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusErrorKind {
    /// The route succeeded.
    Ok,
    /// Authorization denied it.
    Denied,
    /// The service or method was absent.
    NotFound,
    /// The route failed internally.
    Error,
}

impl BusErrorKind {
    /// Stable code used in audit and metrics.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Denied => "denied",
            Self::NotFound => "not_found",
            Self::Error => "error",
        }
    }
}

/// Closed service route table.
#[derive(Debug, Default)]
pub struct RouteTable {
    services: BTreeMap<String, Vec<String>>,
}

impl RouteTable {
    /// Register a service package and its methods.
    pub fn register(
        &mut self,
        service: impl Into<String>,
        methods: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<(), RouteTableError> {
        let service = service.into();
        if service.is_empty() || self.services.contains_key(&service) {
            return Err(RouteTableError::DuplicateService);
        }
        let mut methods = methods.into_iter().map(Into::into).collect::<Vec<_>>();
        methods.sort();
        methods.dedup();
        if methods.is_empty() {
            return Err(RouteTableError::EmptyService);
        }
        self.services.insert(service, methods);
        Ok(())
    }

    /// Resolve a service/method pair without accepting caller-supplied Zone
    /// or subject identity.
    pub fn resolve(&self, service: &str, method: &str) -> Result<(), RouteTableError> {
        let methods = self
            .services
            .get(service)
            .ok_or(RouteTableError::ServiceNotFound)?;
        if methods.iter().any(|candidate| candidate == method) {
            Ok(())
        } else {
            Err(RouteTableError::MethodNotFound)
        }
    }
}

/// Closed route-table failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteTableError {
    /// Duplicate service package.
    DuplicateService,
    /// A service had no methods.
    EmptyService,
    /// Service absent.
    ServiceNotFound,
    /// Method absent.
    MethodNotFound,
    /// The authoritative route audit sink was unavailable.
    AuditUnavailable,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_table_is_closed_and_identity_free() {
        let mut table = RouteTable::default();
        table.register("d2b.resource.v3", ["Get", "List"]).unwrap();
        assert!(table.resolve("d2b.resource.v3", "Get").is_ok());
        assert_eq!(
            table.resolve("d2b.resource.v3", "Delete"),
            Err(RouteTableError::MethodNotFound)
        );
    }

    #[test]
    fn zone_authority_keeps_gateway_custody_separate() {
        let zone = d2b_contracts::v3::identity::ZoneId::parse("work").unwrap();
        let local = ZoneSessionAuthority::local_controller(zone.clone()).unwrap();
        assert_eq!(local.custody(), CredentialCustody::None);
        let gateway = ZoneSessionAuthority::gateway_peer(zone).unwrap();
        assert_eq!(gateway.custody(), CredentialCustody::GatewayGuest);
        assert!(
            ZoneSessionAuthority::new(
                d2b_contracts::v3::identity::ZoneId::parse("work").unwrap(),
                EndpointRole::ZoneController,
                CredentialCustody::GatewayGuest,
            )
            .is_err()
        );
    }
}
