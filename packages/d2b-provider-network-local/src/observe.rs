//! Identity-free drift observation and reconcile decisions.

use crate::controller::NetworkEffectError;
use crate::ifname::IfName;
use crate::routes::RouteTuple;
use d2b_contracts_resource::v3::network::Ipv4Cidr;
use serde_json::Value;
use std::collections::BTreeMap;
use std::process::{Command, Stdio};

/// Current host network occupancy used by root-owned Network admission.
///
/// The snapshot intentionally contains every observed interface, route, and
/// IPv4 CIDR together with any ownership marker exposed by the observation
/// source. Unmarked and foreign objects therefore remain occupied and cannot
/// be adopted by a Network reconcile.
#[derive(Clone, PartialEq, Eq)]
pub struct HostNetworkOccupancy {
    interface_names: Vec<IfName>,
    interface_ownership: BTreeMap<IfName, Vec<String>>,
    route_names: Vec<String>,
    routes: Vec<RouteTuple>,
    route_ownership: BTreeMap<RouteTuple, Vec<String>>,
    cidrs: Vec<Ipv4Cidr>,
    cidr_ownership: BTreeMap<Ipv4Cidr, Vec<String>>,
}

impl HostNetworkOccupancy {
    /// Construct an occupancy snapshot from an observation adapter.
    pub fn from_parts(
        interface_names: Vec<IfName>,
        route_names: Vec<String>,
        cidrs: Vec<Ipv4Cidr>,
    ) -> Self {
        // Route names are derived identifiers from older observation
        // adapters, not kernel state. Only `from_route_tuples` may populate
        // the route occupancy used by admission.
        Self::from_route_tuples(interface_names, route_names, Vec::new(), cidrs)
    }

    /// Construct an occupancy snapshot with observable route tuples.
    ///
    /// `route_names` is retained only as a diagnostic view; route admission
    /// uses `routes`, which must come from kernel observation.
    pub fn from_route_tuples(
        interface_names: Vec<IfName>,
        route_names: Vec<String>,
        routes: Vec<RouteTuple>,
        cidrs: Vec<Ipv4Cidr>,
    ) -> Self {
        let mut interface_names = interface_names;
        interface_names.sort();
        interface_names.dedup();
        let mut route_names = route_names;
        route_names.sort();
        route_names.dedup();
        let mut routes = routes;
        routes.sort();
        routes.dedup();
        let mut cidrs = cidrs;
        cidrs.sort();
        cidrs.dedup();
        Self {
            interface_names,
            interface_ownership: BTreeMap::new(),
            route_names,
            routes,
            route_ownership: BTreeMap::new(),
            cidrs,
            cidr_ownership: BTreeMap::new(),
        }
    }

    /// Attach ownership markers observed for interfaces.
    pub fn with_interface_ownership(
        mut self,
        markers: impl IntoIterator<Item = (IfName, String)>,
    ) -> Self {
        for (ifname, marker) in markers {
            insert_marker(&mut self.interface_ownership, ifname, marker);
        }
        self
    }

    /// Attach complete ownership-marker sets observed for interfaces.
    pub fn with_interface_ownership_markers(
        mut self,
        markers: impl IntoIterator<Item = (IfName, Vec<String>)>,
    ) -> Self {
        for (ifname, values) in markers {
            for marker in values {
                insert_marker(&mut self.interface_ownership, ifname.clone(), marker);
            }
        }
        self
    }

    /// Attach ownership markers observed for route tuples.
    pub fn with_route_ownership(
        mut self,
        markers: impl IntoIterator<Item = (RouteTuple, String)>,
    ) -> Self {
        for (route, marker) in markers {
            insert_marker(&mut self.route_ownership, route, marker);
        }
        self
    }

    /// Attach complete ownership-marker sets observed for route tuples.
    pub fn with_route_ownership_markers(
        mut self,
        markers: impl IntoIterator<Item = (RouteTuple, Vec<String>)>,
    ) -> Self {
        for (route, values) in markers {
            for marker in values {
                insert_marker(&mut self.route_ownership, route.clone(), marker);
            }
        }
        self
    }

    /// Attach ownership markers observed for IPv4 CIDRs.
    pub fn with_cidr_ownership(
        mut self,
        markers: impl IntoIterator<Item = (Ipv4Cidr, String)>,
    ) -> Self {
        for (cidr, marker) in markers {
            insert_marker(&mut self.cidr_ownership, cidr, marker);
        }
        self
    }

    /// Attach complete ownership-marker sets observed for IPv4 CIDRs.
    pub fn with_cidr_ownership_markers(
        mut self,
        markers: impl IntoIterator<Item = (Ipv4Cidr, Vec<String>)>,
    ) -> Self {
        for (cidr, values) in markers {
            for marker in values {
                insert_marker(&mut self.cidr_ownership, cidr.clone(), marker);
            }
        }
        self
    }

    /// Return observed interface names.
    pub fn interface_names(&self) -> &[IfName] {
        &self.interface_names
    }

    /// Return the marker observed on an interface, when it was marked.
    pub fn interface_ownership_marker(&self, ifname: &IfName) -> Option<&str> {
        self.interface_ownership_markers(ifname)
            .first()
            .map(String::as_str)
    }

    /// Return every marker observed on an interface.
    pub fn interface_ownership_markers(&self, ifname: &IfName) -> &[String] {
        self.interface_ownership
            .get(ifname)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Return observed route identities.
    pub fn route_names(&self) -> &[String] {
        &self.route_names
    }

    /// Return observed route tuples.
    pub fn routes(&self) -> &[RouteTuple] {
        &self.routes
    }

    /// Return the marker observed on a route tuple, when it was marked.
    pub fn route_ownership_marker(&self, route: &RouteTuple) -> Option<&str> {
        self.route_ownership_markers(route)
            .first()
            .map(String::as_str)
    }

    /// Return every marker observed on a route tuple.
    pub fn route_ownership_markers(&self, route: &RouteTuple) -> &[String] {
        self.route_ownership
            .get(route)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Return observed IPv4 CIDRs.
    pub fn cidrs(&self) -> &[Ipv4Cidr] {
        &self.cidrs
    }

    /// Return the marker observed on an IPv4 CIDR, when it was marked.
    pub fn cidr_ownership_marker(&self, cidr: &Ipv4Cidr) -> Option<&str> {
        self.cidr_ownership_markers(cidr)
            .first()
            .map(String::as_str)
    }

    /// Return every marker observed on an IPv4 CIDR.
    pub fn cidr_ownership_markers(&self, cidr: &Ipv4Cidr) -> &[String] {
        self.cidr_ownership
            .get(cidr)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

impl core::fmt::Debug for HostNetworkOccupancy {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("HostNetworkOccupancy")
            .field("interface_count", &self.interface_names.len())
            .field("marked_interface_count", &self.interface_ownership.len())
            .field("route_count", &self.route_names.len())
            .field("route_tuple_count", &self.routes.len())
            .field("marked_route_count", &self.route_ownership.len())
            .field("cidr_count", &self.cidrs.len())
            .field("marked_cidr_count", &self.cidr_ownership.len())
            .finish()
    }
}

/// Closed host-observation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostNetworkObservationError {
    /// The trusted `ip` helper could not be executed.
    Backend,
    /// The helper returned malformed JSON or an invalid network value.
    InvalidOutput,
}

impl core::fmt::Display for HostNetworkObservationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Backend => "host-network-observation-backend",
            Self::InvalidOutput => "host-network-observation-invalid",
        })
    }
}

impl std::error::Error for HostNetworkObservationError {}

impl From<HostNetworkObservationError> for NetworkEffectError {
    fn from(_: HostNetworkObservationError) -> Self {
        NetworkEffectError::HostNetworkObservationFailed
    }
}

/// Observe current host links, routes, and IPv4 address occupancy.
pub fn observe_host_network() -> Result<HostNetworkOccupancy, HostNetworkObservationError> {
    let links = run_ip(&["-j", "-d", "link", "show"])?;
    let addresses = run_ip(&["-j", "-4", "addr", "show"])?;
    let routes = run_ip(&["-j", "-4", "route", "show", "table", "all"])?;
    parse_host_network_observation(&links, &addresses, &routes)
}

/// Parse link, address, and route observations without performing effects.
pub fn parse_host_network_observation(
    links: &[u8],
    addresses: &[u8],
    routes: &[u8],
) -> Result<HostNetworkOccupancy, HostNetworkObservationError> {
    let link_values = parse_array(&links)?;
    let mut interface_names = Vec::new();
    let mut interface_markers = BTreeMap::new();
    for value in &link_values {
        let Some(ifname) = value
            .get("ifname")
            .and_then(Value::as_str)
            .and_then(|value| IfName::parse(value).ok())
        else {
            continue;
        };
        interface_names.push(ifname.clone());
        if let Some(marker) = link_marker(value) {
            insert_marker(&mut interface_markers, ifname, marker);
        }
    }

    let mut cidrs = Vec::new();
    let mut cidr_markers = BTreeMap::new();
    for value in parse_array(&addresses)? {
        let Some(entries) = value.get("addr_info").and_then(Value::as_array) else {
            continue;
        };
        let interface_marker = value
            .get("ifname")
            .and_then(Value::as_str)
            .and_then(|value| IfName::parse(value).ok())
            .and_then(|ifname| interface_markers.get(&ifname));
        for entry in entries {
            let Some(local) = entry.get("local").and_then(Value::as_str) else {
                continue;
            };
            let Some(prefix) = entry.get("prefixlen").and_then(Value::as_u64) else {
                continue;
            };
            let prefix = u8::try_from(prefix).map_err(|_| HostNetworkObservationError::InvalidOutput)?;
            if let Ok(cidr) = Ipv4Cidr::parse(format!("{local}/{prefix}")) {
                if let Some(markers) = interface_marker {
                    for marker in markers {
                        insert_marker(&mut cidr_markers, cidr.clone(), marker.clone());
                    }
                }
                cidrs.push(cidr);
            }
        }
    }

    let mut route_names = Vec::new();
    let mut route_tuples = Vec::new();
    let mut route_markers = BTreeMap::new();
    for value in parse_array(&routes)? {
        let destination = value
            .get("dst")
            .and_then(Value::as_str)
            .unwrap_or("default");
        let device = value.get("dev").and_then(Value::as_str);
        let table = value
            .get("table")
            .and_then(value_to_string)
            .unwrap_or_else(|| "main".to_owned());
        let via = value
            .get("gateway")
            .and_then(value_to_string)
            .or_else(|| value.get("via").and_then(route_via_to_string));
        let route_tuple = RouteTuple::new(
            destination,
            via,
            device.map(ToOwned::to_owned),
            table,
        );
        route_names.push(format!(
            "{destination}|{}|{}",
            device.unwrap_or("-"),
            route_tuple.table()
        ));
        let route_marker = route_marker(&value);
        if let Some(marker) = route_marker.as_ref() {
            insert_marker(&mut route_markers, route_tuple.clone(), marker.clone());
        }
        route_tuples.push(route_tuple);
        if destination != "default"
            && let Ok(cidr) = Ipv4Cidr::parse(destination.to_owned())
        {
            if let Some(marker) = route_marker {
                insert_marker(&mut cidr_markers, cidr.clone(), marker);
            }
            cidrs.push(cidr);
        }
    }

    Ok(HostNetworkOccupancy::from_route_tuples(
        interface_names,
        route_names,
        route_tuples,
        cidrs,
    )
    .with_interface_ownership_markers(interface_markers)
    .with_route_ownership_markers(route_markers)
    .with_cidr_ownership_markers(cidr_markers))
}

fn link_marker(value: &Value) -> Option<String> {
    value
        .get("ifalias")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn insert_marker<K: Ord>(markers: &mut BTreeMap<K, Vec<String>>, key: K, marker: String) {
    let values = markers.entry(key).or_default();
    if !values.iter().any(|current| current == &marker) {
        values.push(marker);
    }
}

fn route_marker(value: &Value) -> Option<String> {
    value
        .get("ownershipMarker")
        .or_else(|| value.get("marker"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn route_via_to_string(value: &Value) -> Option<String> {
    value_to_string(value)
        .or_else(|| value.get("host").and_then(value_to_string))
        .or_else(|| value.get("addr").and_then(value_to_string))
}

fn run_ip(args: &[&str]) -> Result<Vec<u8>, HostNetworkObservationError> {
    let output = Command::new("/run/current-system/sw/bin/ip")
        .args(args)
        .env_remove("NOTIFY_SOCKET")
        .stdin(Stdio::null())
        .output()
        .map_err(|_| HostNetworkObservationError::Backend)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(HostNetworkObservationError::Backend)
    }
}

fn parse_array(bytes: &[u8]) -> Result<Vec<Value>, HostNetworkObservationError> {
    serde_json::from_slice(bytes).map_err(|_| HostNetworkObservationError::InvalidOutput)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreign_and_uidless_objects_are_retained_as_occupancy() {
        let occupancy = parse_host_network_observation(
            br#"[{"ifname":"foreign0"}]"#,
            br#"[{"addr_info":[{"local":"10.20.0.1","prefixlen":24}]}]"#,
            br#"[{"dst":"10.20.0.0/24","gateway":"192.0.2.1","dev":"foreign0","table":254}]"#,
        )
        .unwrap();
        assert_eq!(occupancy.interface_names()[0].as_str(), "foreign0");
        assert!(occupancy
            .cidrs()
            .iter()
            .any(|cidr| cidr.as_str() == "10.20.0.0/24"));
        assert_eq!(occupancy.route_names(), ["10.20.0.0/24|foreign0|main"]);
        assert_eq!(occupancy.routes().len(), 1);
        assert_eq!(occupancy.routes()[0].destination(), "10.20.0.0/24");
        assert_eq!(occupancy.routes()[0].via(), Some("192.0.2.1"));
        assert_eq!(occupancy.routes()[0].device(), Some("foreign0"));
        assert_eq!(occupancy.routes()[0].table(), "main");
    }

    #[test]
    fn marked_interfaces_addresses_and_routes_retain_provenance() {
        let bridge_marker = "d2b managed: network:bridge:lan:zone:223e4567-e89b-42d3-a456-426614174001:network:323e4567-e89b-42d3-a456-426614174002:generation:4:attachment:7:bundle:sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let route_marker = "d2b managed: network:route:route-1:zone:223e4567-e89b-42d3-a456-426614174001:network:323e4567-e89b-42d3-a456-426614174002:generation:4:attachment:7:bundle:sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let occupancy = parse_host_network_observation(
            format!(r#"[{{"ifname":"d2b-b12345678","ifalias":"{bridge_marker}"}}]"#).as_bytes(),
            br#"[{"ifname":"d2b-b12345678","addr_info":[{"local":"10.20.0.1","prefixlen":24}]}]"#,
            format!(r#"[{{"dst":"10.20.0.0/24","gateway":"192.0.2.1","dev":"d2b-b12345678","table":254,"ownershipMarker":"{route_marker}"}}]"#).as_bytes(),
        )
        .unwrap();
        let ifname = IfName::parse("d2b-b12345678").unwrap();
        let route = occupancy.routes().first().unwrap();
        let cidr = Ipv4Cidr::parse("10.20.0.1/24").unwrap();
        assert_eq!(occupancy.interface_ownership_marker(&ifname), Some(bridge_marker));
        assert_eq!(occupancy.route_ownership_marker(route), Some(route_marker));
        assert_eq!(occupancy.cidr_ownership_marker(&cidr), Some(bridge_marker));
    }
}

/// One bounded host and guest-agent observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkObservation {
    /// Projection-scoped host firewall digest matches.
    pub firewall_matches: bool,
    /// Every bridge IPv6 sysctl matches.
    pub sysctls_match: bool,
    /// Every bridge-port isolation flag matches.
    pub bridge_ports_match: bool,
    /// Peer CIDRs remain conflict free.
    pub cidrs_conflict_free: bool,
    /// The external physical-NIC authority proof remains valid.
    pub external_authority_ready: bool,
    /// The guest agent confirmed dnsmasq binding.
    pub dnsmasq_bound: bool,
    /// The guest agent confirmed firewall application.
    pub guest_firewall_applied: bool,
}

/// Closed observation result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObserveDecision {
    /// No drift was found.
    Current,
    /// Reconcile is required.
    Requeue,
    /// External authority ambiguity blocks recreation.
    Blocked,
}

/// Evaluate one observation without exposing any observed value.
pub fn evaluate_observation(
    observation: NetworkObservation,
) -> Result<ObserveDecision, NetworkEffectError> {
    if !observation.external_authority_ready {
        return Ok(ObserveDecision::Blocked);
    }
    if !observation.cidrs_conflict_free {
        return Err(NetworkEffectError::CidrConflict);
    }
    if observation.firewall_matches
        && observation.sysctls_match
        && observation.bridge_ports_match
        && observation.dnsmasq_bound
        && observation.guest_firewall_applied
    {
        Ok(ObserveDecision::Current)
    } else {
        Ok(ObserveDecision::Requeue)
    }
}
