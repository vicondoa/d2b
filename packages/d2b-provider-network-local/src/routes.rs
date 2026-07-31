//! Redacted route and net-VM readiness preflight.

use d2b_contracts::v3::network::NetworkComponentPhase;
use std::collections::BTreeSet;

/// Ownership classification for an observed default route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultRouteState {
    /// No default route is present for the Network uplink.
    Missing,
    /// The expected Network route is present.
    NetworkOwned,
    /// A different route occupies the expected uplink.
    Foreign,
}

/// Address-family observation for d2b-owned links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnedLinkAddressState {
    /// No IPv6 address is present.
    Ipv4Only,
    /// At least one IPv6 address is present on a d2b-owned link.
    Ipv6Present,
}

/// One host route classified without retaining an interface name in diagnostics.
pub struct RouteRow {
    destination: String,
    link: LinkClass,
    up: bool,
}

impl RouteRow {
    /// Construct one observed IPv4 link route.
    pub fn new(destination: impl Into<String>, link: LinkClass, up: bool) -> Self {
        Self {
            destination: destination.into(),
            link,
            up,
        }
    }
}

impl core::fmt::Debug for RouteRow {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RouteRow(<redacted>)")
    }
}

/// Semantic class of the link carrying a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkClass {
    /// An ordinary host LAN link.
    HostLan,
    /// A d2b-owned link, excluded from host LAN discovery.
    D2bOwned,
    /// The loopback link.
    Loopback,
    /// A known foreign virtual bridge.
    KnownForeignVirtual,
    /// A point-to-point link whose LAN interpretation is ambiguous.
    PointToPoint,
}

/// Bounded net-VM service observations used by route readiness.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NetworkServiceStatus {
    /// The dnsmasq Process phase from Network status.
    pub dnsmasq_phase: NetworkComponentPhase,
    /// The typed `dnsmasq-bound` readiness predicate.
    pub dnsmasq_bound: bool,
    /// The typed `routes-applied` readiness predicate.
    pub routes_applied: bool,
}

impl core::fmt::Debug for NetworkServiceStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("NetworkServiceStatus(<redacted>)")
    }
}

/// Value-free route preflight failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutePreflightError {
    /// No default route is present for the Network uplink.
    NoDefaultRoute,
    /// A foreign default route occupies the expected uplink.
    ForeignDefaultRoute,
    /// A d2b-owned link carries an IPv6 address.
    Ipv6AddressPresent,
    /// The dnsmasq Process is not Ready with its bound predicate set.
    DnsmasqNotBound,
    /// The net agent has not confirmed its routes.
    RoutesNotApplied,
}

impl RoutePreflightError {
    /// Return the stable redacted reason code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::NoDefaultRoute => "route-default-missing",
            Self::ForeignDefaultRoute => "route-default-foreign",
            Self::Ipv6AddressPresent => "ipv6-address-present",
            Self::DnsmasqNotBound => "dnsmasq-not-bound",
            Self::RoutesNotApplied => "routes-not-applied",
        }
    }
}

impl core::fmt::Display for RoutePreflightError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for RoutePreflightError {}

/// Fail closed unless the expected default route owns the uplink.
pub fn check_default_route(state: DefaultRouteState) -> Result<(), RoutePreflightError> {
    match state {
        DefaultRouteState::NetworkOwned => Ok(()),
        DefaultRouteState::Missing => Err(RoutePreflightError::NoDefaultRoute),
        DefaultRouteState::Foreign => Err(RoutePreflightError::ForeignDefaultRoute),
    }
}

/// Fail closed when an IPv6 address is observed on a d2b-owned link.
pub fn check_owned_link_addresses(state: OwnedLinkAddressState) -> Result<(), RoutePreflightError> {
    match state {
        OwnedLinkAddressState::Ipv4Only => Ok(()),
        OwnedLinkAddressState::Ipv6Present => Err(RoutePreflightError::Ipv6AddressPresent),
    }
}

/// Consume the Network status readiness predicate rather than host environment JSON.
pub fn check_network_services(status: NetworkServiceStatus) -> Result<(), RoutePreflightError> {
    if status.dnsmasq_phase != NetworkComponentPhase::Ready || !status.dnsmasq_bound {
        return Err(RoutePreflightError::DnsmasqNotBound);
    }
    if !status.routes_applied {
        return Err(RoutePreflightError::RoutesNotApplied);
    }
    Ok(())
}

/// Host LAN discovery result with address values redacted from diagnostics.
#[derive(PartialEq, Eq)]
pub struct HostLanCidrs {
    cidrs: Vec<String>,
    ambiguous: Vec<String>,
}

impl HostLanCidrs {
    /// Borrow unambiguous host LAN CIDRs for policy construction.
    pub fn cidrs(&self) -> &[String] {
        &self.cidrs
    }

    /// Borrow point-to-point CIDRs requiring explicit operator policy.
    pub fn ambiguous(&self) -> &[String] {
        &self.ambiguous
    }
}

impl core::fmt::Debug for HostLanCidrs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("HostLanCidrs(<redacted>)")
    }
}

/// Derive host LAN CIDRs while excluding d2b and known virtual links.
pub fn detect_host_lan_cidrs(routes: &[RouteRow]) -> HostLanCidrs {
    let mut cidrs = BTreeSet::new();
    let mut ambiguous = BTreeSet::new();
    for route in routes.iter().filter(|route| route.up) {
        match route.link {
            LinkClass::HostLan => {
                cidrs.insert(route.destination.clone());
            }
            LinkClass::PointToPoint => {
                ambiguous.insert(route.destination.clone());
            }
            LinkClass::D2bOwned | LinkClass::Loopback | LinkClass::KnownForeignVirtual => {}
        }
    }
    HostLanCidrs {
        cidrs: cidrs.into_iter().collect(),
        ambiguous: ambiguous.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_default_route_fails_closed() {
        assert_eq!(
            check_default_route(DefaultRouteState::Missing),
            Err(RoutePreflightError::NoDefaultRoute)
        );
    }

    #[test]
    fn foreign_default_route_fails_closed() {
        assert_eq!(
            check_default_route(DefaultRouteState::Foreign),
            Err(RoutePreflightError::ForeignDefaultRoute)
        );
    }

    #[test]
    fn dnsmasq_not_bound_fails_closed() {
        let error = check_network_services(NetworkServiceStatus {
            dnsmasq_phase: NetworkComponentPhase::Ready,
            dnsmasq_bound: false,
            routes_applied: true,
        })
        .unwrap_err();
        assert_eq!(error, RoutePreflightError::DnsmasqNotBound);
    }

    #[test]
    fn ipv6_address_on_owned_link_fails_closed() {
        assert_eq!(
            check_owned_link_addresses(OwnedLinkAddressState::Ipv6Present),
            Err(RoutePreflightError::Ipv6AddressPresent)
        );
    }

    #[test]
    fn host_lan_cidr_ambiguous_for_vpn() {
        let result = detect_host_lan_cidrs(&[RouteRow::new(
            "10.99.99.0/24",
            LinkClass::PointToPoint,
            true,
        )]);
        assert!(result.cidrs().is_empty());
        assert_eq!(result.ambiguous(), ["10.99.99.0/24"]);
    }
}
