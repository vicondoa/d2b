//! Redacted route and net-VM readiness preflight.

use crate::ifname::{derive_network_route_name, derive_network_route_name_for};
use d2b_contracts_resource::v3::network::NetworkComponentPhase;
use d2b_contracts_resource::v3::{
    IfName, ResourceBundleGenerationId, ResourceGeneration, ResourceUid,
};
use std::collections::BTreeSet;

/// Kernel route occupancy tuple. Route names are not kernel state, so
/// admission compares these observable fields instead of synthetic ids.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RouteTuple {
    destination: String,
    via: Option<String>,
    device: Option<String>,
    table: String,
}

impl RouteTuple {
    /// Construct one normalized route tuple.
    pub fn new(
        destination: impl Into<String>,
        via: Option<String>,
        device: Option<String>,
        table: impl Into<String>,
    ) -> Self {
        Self {
            destination: destination.into(),
            via,
            device,
            table: normalize_table_name(&table.into()),
        }
    }

    /// Borrow the route destination/CIDR.
    pub fn destination(&self) -> &str {
        &self.destination
    }

    /// Borrow the optional next hop.
    pub fn via(&self) -> Option<&str> {
        self.via.as_deref()
    }

    /// Borrow the optional device.
    pub fn device(&self) -> Option<&str> {
        self.device.as_deref()
    }

    /// Borrow the normalized routing table.
    pub fn table(&self) -> &str {
        &self.table
    }
}

impl core::fmt::Debug for RouteTuple {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("RouteTuple(<redacted>)")
    }
}

fn normalize_table_name(table: &str) -> String {
    match table {
        "254" => "main".to_owned(),
        "253" => "default".to_owned(),
        "255" => "local".to_owned(),
        other => other.to_owned(),
    }
}

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

/// One route intent bound to immutable Network provenance.
#[derive(Clone, PartialEq, Eq)]
pub struct NetworkRouteIntent {
    zone_uid: Option<ResourceUid>,
    network_uid: ResourceUid,
    network_generation: ResourceGeneration,
    attachment_generation: Option<ResourceGeneration>,
    bundle_generation: ResourceBundleGenerationId,
    route_name: String,
    tuple: RouteTuple,
}

impl NetworkRouteIntent {
    /// Construct a route identity from the committed Network tuple.
    pub fn new(
        network_uid: ResourceUid,
        network_generation: ResourceGeneration,
        bundle_generation: ResourceBundleGenerationId,
        index: usize,
        destination: impl Into<String>,
        device: IfName,
    ) -> Self {
        Self {
            zone_uid: None,
            network_uid: network_uid.clone(),
            network_generation,
            attachment_generation: None,
            bundle_generation,
            route_name: derive_network_route_name(&network_uid, index),
            tuple: RouteTuple::new(destination, None, Some(device.as_str().to_owned()), "main"),
        }
    }

    /// Construct a route identity bound to the complete Network provenance.
    pub fn with_provenance(
        zone_uid: ResourceUid,
        network_uid: ResourceUid,
        network_generation: ResourceGeneration,
        attachment_generation: ResourceGeneration,
        bundle_generation: ResourceBundleGenerationId,
        index: usize,
        destination: impl Into<String>,
        via: Option<String>,
        device: IfName,
        table: impl Into<String>,
    ) -> Self {
        Self {
            zone_uid: Some(zone_uid.clone()),
            network_uid: network_uid.clone(),
            network_generation,
            attachment_generation: Some(attachment_generation),
            bundle_generation,
            route_name: derive_network_route_name_for(&zone_uid, &network_uid, index),
            tuple: RouteTuple::new(destination, via, Some(device.as_str().to_owned()), table),
        }
    }

    /// Borrow the optional Zone identity.
    pub const fn zone_uid(&self) -> Option<&ResourceUid> {
        self.zone_uid.as_ref()
    }

    /// Borrow the immutable Network identity.
    pub const fn network_uid(&self) -> &ResourceUid {
        &self.network_uid
    }

    /// Return the Network generation fence.
    pub const fn network_generation(&self) -> ResourceGeneration {
        self.network_generation
    }

    /// Return the optional aggregate attachment generation.
    pub const fn attachment_generation(&self) -> Option<ResourceGeneration> {
        self.attachment_generation
    }

    /// Borrow the installed bundle generation fence.
    pub const fn bundle_generation(&self) -> &ResourceBundleGenerationId {
        &self.bundle_generation
    }

    /// Borrow the derived route identity.
    pub fn route_name(&self) -> &str {
        &self.route_name
    }

    /// Borrow the desired kernel route tuple.
    pub const fn tuple(&self) -> &RouteTuple {
        &self.tuple
    }

    /// Borrow the destination used by the broker adapter.
    pub fn destination(&self) -> &str {
        self.tuple.destination()
    }

    /// Borrow the trusted route device.
    pub fn device(&self) -> Option<&str> {
        self.tuple.device()
    }
}

impl core::fmt::Debug for NetworkRouteIntent {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("NetworkRouteIntent(<redacted>)")
    }
}

/// Closed route-provenance failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteProvenanceError {
    /// The route belongs to a different Network.
    NetworkMismatch,
    /// The route's Network generation is stale.
    GenerationMismatch,
    /// The route's bundle generation is stale.
    BundleGenerationMismatch,
}

impl core::fmt::Display for RouteProvenanceError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::NetworkMismatch => "route-network-mismatch",
            Self::GenerationMismatch => "route-generation-mismatch",
            Self::BundleGenerationMismatch => "route-bundle-generation-mismatch",
        })
    }
}

impl std::error::Error for RouteProvenanceError {}

/// Validate a route before the broker can apply or remove it.
pub fn validate_network_route_intent(
    intent: &NetworkRouteIntent,
    network_uid: &ResourceUid,
    network_generation: ResourceGeneration,
    bundle_generation: &ResourceBundleGenerationId,
) -> Result<(), RouteProvenanceError> {
    if intent.network_uid() != network_uid {
        return Err(RouteProvenanceError::NetworkMismatch);
    }
    if intent.network_generation() != network_generation {
        return Err(RouteProvenanceError::GenerationMismatch);
    }
    if intent.bundle_generation() != bundle_generation {
        return Err(RouteProvenanceError::BundleGenerationMismatch);
    }
    Ok(())
}

/// Validate a route against the complete admitted provenance tuple.
#[allow(dead_code)]
pub fn validate_network_route_intent_with_provenance(
    intent: &NetworkRouteIntent,
    zone_uid: &ResourceUid,
    network_uid: &ResourceUid,
    network_generation: ResourceGeneration,
    attachment_generation: ResourceGeneration,
    bundle_generation: &ResourceBundleGenerationId,
) -> Result<(), RouteProvenanceError> {
    if intent.zone_uid() != Some(zone_uid) {
        return Err(RouteProvenanceError::NetworkMismatch);
    }
    validate_network_route_intent(intent, network_uid, network_generation, bundle_generation)?;
    if intent.attachment_generation() != Some(attachment_generation) {
        return Err(RouteProvenanceError::GenerationMismatch);
    }
    Ok(())
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

    #[test]
    fn route_provenance_rejects_swapped_network_and_stale_generation() {
        let network = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let other_network = ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap();
        let bundle = ResourceBundleGenerationId::parse(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        let route = NetworkRouteIntent::new(
            network.clone(),
            ResourceGeneration::new(4).unwrap(),
            bundle.clone(),
            0,
            "10.20.0.0/24",
            IfName::parse("d2b-b12345678").unwrap(),
        );
        assert_eq!(
            validate_network_route_intent(
                &route,
                &other_network,
                ResourceGeneration::new(4).unwrap(),
                &bundle
            ),
            Err(RouteProvenanceError::NetworkMismatch)
        );
        assert_eq!(
            validate_network_route_intent(
                &route,
                &network,
                ResourceGeneration::new(3).unwrap(),
                &bundle
            ),
            Err(RouteProvenanceError::GenerationMismatch)
        );
    }

    #[test]
    fn full_route_provenance_rejects_swapped_zone_and_attachment_generation() {
        let zone = ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").unwrap();
        let network = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let bundle = ResourceBundleGenerationId::parse(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        let route = NetworkRouteIntent::with_provenance(
            zone.clone(),
            network.clone(),
            ResourceGeneration::new(4).unwrap(),
            ResourceGeneration::new(7).unwrap(),
            bundle.clone(),
            0,
            "10.20.0.0/24",
            None,
            IfName::parse("d2b-b12345678").unwrap(),
            "main",
        );
        let other_zone = ResourceUid::parse("423e4567-e89b-42d3-a456-426614174003").unwrap();
        assert_eq!(
            validate_network_route_intent_with_provenance(
                &route,
                &other_zone,
                &network,
                ResourceGeneration::new(4).unwrap(),
                ResourceGeneration::new(7).unwrap(),
                &bundle,
            ),
            Err(RouteProvenanceError::NetworkMismatch)
        );
        assert_eq!(
            validate_network_route_intent_with_provenance(
                &route,
                &zone,
                &network,
                ResourceGeneration::new(4).unwrap(),
                ResourceGeneration::new(8).unwrap(),
                &bundle,
            ),
            Err(RouteProvenanceError::GenerationMismatch)
        );
    }
}
