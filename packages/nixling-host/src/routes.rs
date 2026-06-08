//! Host-prepare routes module.
//!
//! Implements the fail-closed route preflight predicate set and the host
//! LAN CIDR derivation. Each predicate operates on a typed snapshot of the
//! host's route + hosts-file + bound-socket state so the same code
//! drives both the host-prepare path and the pre-VM-start hook.

use crate::ifname::{looks_nixling_owned, DEFAULT_PREFIX};
use crate::nftables::{evaluate_coexistence_policy, hash_inet_nixling_table, NftError};
use nixling_core::host::IfName;
use nixling_core::host_w3::{
    CoexistencePolicy, FirewallCoexistencePolicy, FirewallManager, HostsEntry,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// -------------------------------------------------------------------
// Snapshot types
// -------------------------------------------------------------------

/// Snapshot of `rtnetlink RTM_GETROUTE` table 254 / RT_TABLE_MAIN.
/// Each entry encodes only the fields the preflight inspects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RouteTableSnapshot {
    pub routes: Vec<RouteRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteRow {
    pub destination: String, // "default" or "10.0.0.0/24"
    pub via: Option<String>,
    pub device: String,
    pub scope: RouteScope,
    pub family: AddrFamily,
    pub up: bool,
    /// `true` when this is a `ptp` route with no broadcast (VPN-like).
    pub point_to_point: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RouteScope {
    Global,
    Link,
    Host,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AddrFamily {
    V4,
    V6,
}

// -------------------------------------------------------------------
// Predicate 1: default route for each env uplink bridge matches.
// -------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutePreflightError {
    NoDefaultRouteForUplink {
        uplink: String,
    },
    ForeignDefaultRouteOnUplink {
        uplink: String,
        observed: RouteRow,
    },
    Ipv6AddressPresentOnNixlingLink {
        ifname: String,
    },
    HostsBlockDrift {
        expected: String,
        observed: String,
    },
    DnsmasqNotBound {
        ifname: String,
        port: u16,
    },
    FirewallCoexistenceViolation {
        detected: FirewallManager,
        declared: CoexistencePolicy,
    },
    InetNixlingTableDrift {
        expected: String,
        observed: String,
    },
}

impl std::fmt::Display for RoutePreflightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDefaultRouteForUplink { uplink } => {
                write!(f, "route-preflight: no default route for uplink {uplink:?}")
            }
            Self::ForeignDefaultRouteOnUplink { uplink, observed } => write!(
                f,
                "route-preflight: foreign default route on uplink {uplink:?} (observed {observed:?})"
            ),
            Self::Ipv6AddressPresentOnNixlingLink { ifname } => write!(
                f,
                "route-preflight: IPv6 address present on nixling link {ifname:?}"
            ),
            Self::HostsBlockDrift { .. } => {
                write!(f, "route-preflight: /etc/hosts managed-block drift")
            }
            Self::DnsmasqNotBound { ifname, port } => write!(
                f,
                "route-preflight: dnsmasq not bound on {ifname:?}:{port}"
            ),
            Self::FirewallCoexistenceViolation { detected, declared } => write!(
                f,
                "route-preflight: firewall coexistence violation (detected={detected:?}, declared={declared:?})"
            ),
            Self::InetNixlingTableDrift { expected, observed } => write!(
                f,
                "route-preflight: inet nixling table drift (expected={expected}, observed={observed})"
            ),
        }
    }
}

impl std::error::Error for RoutePreflightError {}

/// Expected uplink → default route owner mapping. `expected_via` is
/// the IP listed in `host.json`; `None` means no specific gateway IP
/// is constrained (only the device must match).
#[derive(Debug, Clone)]
pub struct UplinkExpectation {
    pub uplink_device: String,
    pub expected_via: Option<String>,
}

/// Checks predicate 1: every declared env uplink has a default route
/// pointing to it, matching `host.json`.
pub fn check_default_routes(
    snapshot: &RouteTableSnapshot,
    uplinks: &[UplinkExpectation],
) -> Result<(), RoutePreflightError> {
    for expectation in uplinks {
        let default_for_dev: Vec<&RouteRow> = snapshot
            .routes
            .iter()
            .filter(|r| {
                r.family == AddrFamily::V4
                    && r.destination == "default"
                    && r.device == expectation.uplink_device
                    && r.up
            })
            .collect();
        if default_for_dev.is_empty() {
            return Err(RoutePreflightError::NoDefaultRouteForUplink {
                uplink: expectation.uplink_device.clone(),
            });
        }
        if let Some(expected_via) = &expectation.expected_via {
            let match_via = default_for_dev
                .iter()
                .find(|r| r.via.as_deref() == Some(expected_via.as_str()));
            if match_via.is_none() {
                return Err(RoutePreflightError::ForeignDefaultRouteOnUplink {
                    uplink: expectation.uplink_device.clone(),
                    observed: (*default_for_dev[0]).clone(),
                });
            }
        }
    }
    Ok(())
}

// -------------------------------------------------------------------
// Predicate 2: no IPv6 address on any nixling link.
// -------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AddrSnapshot {
    pub rows: Vec<AddrRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddrRow {
    pub ifname: String,
    pub family: AddrFamily,
    pub address: String,
}

pub fn check_no_ipv6_on_nixling_links(
    snapshot: &AddrSnapshot,
    prefix: Option<&str>,
) -> Result<(), RoutePreflightError> {
    let prefix = prefix.unwrap_or(DEFAULT_PREFIX);
    for row in &snapshot.rows {
        if row.family == AddrFamily::V6 && looks_nixling_owned(&row.ifname, prefix) {
            return Err(RoutePreflightError::Ipv6AddressPresentOnNixlingLink {
                ifname: row.ifname.clone(),
            });
        }
    }
    Ok(())
}

// -------------------------------------------------------------------
// Predicate 3: /etc/hosts managed-block byte-equivalence.
// -------------------------------------------------------------------

pub const HOSTS_MANAGED_BEGIN: &str = "# nixling-managed begin";
pub const HOSTS_MANAGED_END: &str = "# nixling-managed end";

/// Renders the managed block exactly as the broker would write it.
/// One entry per line, address + hostname + aliases space-joined.
pub fn render_hosts_block(entries: &[HostsEntry]) -> String {
    let mut out = String::new();
    out.push_str(HOSTS_MANAGED_BEGIN);
    out.push('\n');
    for entry in entries {
        out.push_str(&entry.address);
        out.push(' ');
        out.push_str(&entry.hostname);
        for alias in &entry.aliases {
            out.push(' ');
            out.push_str(alias);
        }
        out.push('\n');
    }
    out.push_str(HOSTS_MANAGED_END);
    out.push('\n');
    out
}

/// Extracts the managed block from a `/etc/hosts` body and compares
/// it byte-for-byte against `expected`. Foreign lines outside the
/// markers are not inspected.
pub fn extract_managed_block(hosts_body: &str) -> Option<String> {
    let begin = hosts_body.find(HOSTS_MANAGED_BEGIN)?;
    let end = hosts_body[begin..].find(HOSTS_MANAGED_END)?;
    let end_full = begin + end + HOSTS_MANAGED_END.len();
    let tail = &hosts_body[end_full..];
    let after_marker_nl = tail.find('\n').map(|i| i + 1).unwrap_or(tail.len());
    Some(hosts_body[begin..end_full + after_marker_nl].to_owned())
}

pub fn check_hosts_block(
    hosts_body: &str,
    expected_entries: &[HostsEntry],
) -> Result<(), RoutePreflightError> {
    let expected = render_hosts_block(expected_entries);
    let observed = extract_managed_block(hosts_body).unwrap_or_default();
    if expected != observed {
        return Err(RoutePreflightError::HostsBlockDrift { expected, observed });
    }
    Ok(())
}

// -------------------------------------------------------------------
// Predicate 4: dnsmasq bound on declared LAN ifname/address.
// -------------------------------------------------------------------

/// Snapshot of sockets bound on the host (parsed from `/proc/net/{tcp,udp}`
/// + `/proc/net/dev` for the ifname mapping). Each row is one bound
///   `(ifname, port, proto)` triple.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BoundSocketSnapshot {
    pub rows: Vec<BoundSocketRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundSocketRow {
    pub ifname: String,
    pub port: u16,
    pub proto: SocketProto,
    pub process_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SocketProto {
    Tcp,
    Udp,
}

pub fn check_dnsmasq_bound(
    snapshot: &BoundSocketSnapshot,
    lan_ifname: &IfName,
    port: u16,
) -> Result<(), RoutePreflightError> {
    let found = snapshot.rows.iter().any(|r| {
        r.ifname == lan_ifname.as_str()
            && r.port == port
            && matches!(r.proto, SocketProto::Udp | SocketProto::Tcp)
    });
    if !found {
        return Err(RoutePreflightError::DnsmasqNotBound {
            ifname: lan_ifname.as_str().to_owned(),
            port,
        });
    }
    Ok(())
}

// -------------------------------------------------------------------
// Predicate 5: firewall coexistence policy + inet nixling integrity.
// -------------------------------------------------------------------

/// Combined firewall preflight: assert the bundle's declared
/// coexistence policy matches the live detector AND that the
/// `inet nixling` table hash matches the bundle's recorded digest.
///
/// Both `host prepare` and the pre-VM-start hook re-run this check so
/// a foreign actor
/// that mutates `inet nixling` between host prepare and VM start
/// cannot ride the stale-trust window.
pub fn check_firewall_coexistence(
    declared: &FirewallCoexistencePolicy,
    detected: FirewallManager,
    expected_table_hash: &str,
    nft_list_json: &[u8],
) -> Result<(), RoutePreflightError> {
    if declared.manager != detected {
        return Err(RoutePreflightError::FirewallCoexistenceViolation {
            detected,
            declared: declared.policy,
        });
    }
    if let Err(err) = evaluate_coexistence_policy(detected, declared.policy) {
        // Propagate as a route-preflight finding so the caller sees a
        // single error type; the NftError detail is captured in the
        // diagnostic message.
        let _ = err;
        return Err(RoutePreflightError::FirewallCoexistenceViolation {
            detected,
            declared: declared.policy,
        });
    }
    // `inet nixling` table integrity: hash the live JSON dump and
    // compare against the bundle-recorded digest.
    let observed = hash_inet_nixling_table(nft_list_json).to_string();
    if observed != expected_table_hash {
        return Err(RoutePreflightError::InetNixlingTableDrift {
            expected: expected_table_hash.to_owned(),
            observed,
        });
    }
    Ok(())
}

// Silence dead-code warnings when callers only use the bundled
// `run_route_preflight` and never call `NftError` directly here.
#[allow(dead_code)]
fn _nft_error_marker(_: NftError) {}

// -------------------------------------------------------------------
// Bundle of every preflight predicate.
// -------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RoutePreflightInput<'a> {
    pub routes: &'a RouteTableSnapshot,
    pub addrs: &'a AddrSnapshot,
    pub uplinks: &'a [UplinkExpectation],
    pub hosts_body: &'a str,
    pub hosts_expected: &'a [HostsEntry],
    pub sockets: &'a BoundSocketSnapshot,
    pub dnsmasq_ifname: &'a IfName,
    pub dnsmasq_port: u16,
    /// Bundle-declared firewall coexistence policy (per
    /// `host.json::firewallCoexistencePolicy`).
    pub firewall_coexistence: &'a FirewallCoexistencePolicy,
    /// Live detector result from
    /// [`crate::nftables::detect_firewall_manager`].
    pub nft_detector_result: FirewallManager,
    /// Bundle-recorded hash of `nft list table inet nixling -j` from
    /// the last successful apply.
    pub expected_inet_nixling_hash: &'a str,
    /// Live `nft list table inet nixling -j` output.
    pub nft_list_json: &'a [u8],
}

pub fn run_route_preflight(input: &RoutePreflightInput<'_>) -> Result<(), RoutePreflightError> {
    check_default_routes(input.routes, input.uplinks)?;
    check_no_ipv6_on_nixling_links(input.addrs, None)?;
    check_hosts_block(input.hosts_body, input.hosts_expected)?;
    check_dnsmasq_bound(input.sockets, input.dnsmasq_ifname, input.dnsmasq_port)?;
    check_firewall_coexistence(
        input.firewall_coexistence,
        input.nft_detector_result,
        input.expected_inet_nixling_hash,
        input.nft_list_json,
    )?;
    Ok(())
}

/// Per-VM pre-start hook: re-runs the full preflight against a
/// trusted bundle so a foreign actor that mutates routes / nft /
/// hosts after `host prepare --apply` cannot let the VM come up
/// against unintended state. Surfaced to the broker `Up` runtime
/// and to `nixling-priv-broker::ops::route::apply` so the same code
/// drives both call sites.
pub fn run_route_preflight_for_vm(
    vm_id: &str,
    input: &RoutePreflightInput<'_>,
) -> Result<(), RoutePreflightError> {
    // VM id is recorded by callers in the broker audit record so the
    // preflight failure is attributable; the check itself is the same
    // bundle-wide predicate set.
    let _ = vm_id;
    run_route_preflight(input)
}

// -------------------------------------------------------------------
// Host LAN CIDR derivation.
// -------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostLanCidrs {
    pub cidrs: Vec<String>,
    /// VPN-like links detected (point-to-point, no broadcast). They
    /// are surfaced as `ambiguous-host-lan` findings; the operator
    /// must override via `nixling.site.hostLanCidrs`.
    pub ambiguous: Vec<String>,
}

const KNOWN_FOREIGN_PREFIXES: &[&str] = &["docker", "virbr", "lxcbr"];

pub fn detect_host_lan_cidrs(snapshot: &RouteTableSnapshot, prefix: Option<&str>) -> HostLanCidrs {
    let prefix = prefix.unwrap_or(DEFAULT_PREFIX);
    let mut cidrs: Vec<String> = Vec::new();
    let mut ambiguous: Vec<String> = Vec::new();
    let mut seen: BTreeMap<String, ()> = BTreeMap::new();
    for r in &snapshot.routes {
        if !r.up {
            continue;
        }
        if r.family != AddrFamily::V4 {
            continue;
        }
        if r.scope != RouteScope::Link {
            continue;
        }
        if r.device == "lo" {
            continue;
        }
        if looks_nixling_owned(&r.device, prefix) {
            continue;
        }
        if KNOWN_FOREIGN_PREFIXES
            .iter()
            .any(|p| r.device.starts_with(p))
        {
            continue;
        }
        if r.point_to_point {
            ambiguous.push(r.destination.clone());
            continue;
        }
        if seen.insert(r.destination.clone(), ()).is_none() {
            cidrs.push(r.destination.clone());
        }
    }
    HostLanCidrs { cidrs, ambiguous }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ifname::{derive_from_env_vm, DerivedRole};

    fn lan_if() -> IfName {
        derive_from_env_vm("e", None, DerivedRole::Bridge, None).unwrap()
    }

    #[test]
    fn default_route_match_passes() {
        let snap = RouteTableSnapshot {
            routes: vec![RouteRow {
                destination: "default".into(),
                via: Some("192.168.1.1".into()),
                device: "wlp0".into(),
                scope: RouteScope::Global,
                family: AddrFamily::V4,
                up: true,
                point_to_point: false,
            }],
        };
        check_default_routes(
            &snap,
            &[UplinkExpectation {
                uplink_device: "wlp0".into(),
                expected_via: Some("192.168.1.1".into()),
            }],
        )
        .unwrap();
    }

    #[test]
    fn no_default_route_fails_closed() {
        let err = check_default_routes(
            &RouteTableSnapshot::default(),
            &[UplinkExpectation {
                uplink_device: "wlp0".into(),
                expected_via: None,
            }],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            RoutePreflightError::NoDefaultRouteForUplink { .. }
        ));
    }

    #[test]
    fn foreign_default_route_fails_closed() {
        let snap = RouteTableSnapshot {
            routes: vec![RouteRow {
                destination: "default".into(),
                via: Some("10.99.0.1".into()),
                device: "wlp0".into(),
                scope: RouteScope::Global,
                family: AddrFamily::V4,
                up: true,
                point_to_point: false,
            }],
        };
        let err = check_default_routes(
            &snap,
            &[UplinkExpectation {
                uplink_device: "wlp0".into(),
                expected_via: Some("192.168.1.1".into()),
            }],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            RoutePreflightError::ForeignDefaultRouteOnUplink { .. }
        ));
    }

    #[test]
    fn ipv6_address_on_nixling_link_fails_closed() {
        let nl = lan_if();
        let snap = AddrSnapshot {
            rows: vec![AddrRow {
                ifname: nl.as_str().to_owned(),
                family: AddrFamily::V6,
                address: "fe80::1".into(),
            }],
        };
        let err = check_no_ipv6_on_nixling_links(&snap, None).unwrap_err();
        assert!(matches!(
            err,
            RoutePreflightError::Ipv6AddressPresentOnNixlingLink { .. }
        ));
    }

    #[test]
    fn ipv6_address_on_foreign_link_ignored() {
        let snap = AddrSnapshot {
            rows: vec![AddrRow {
                ifname: "wlp0".into(),
                family: AddrFamily::V6,
                address: "2001:db8::1".into(),
            }],
        };
        check_no_ipv6_on_nixling_links(&snap, None).unwrap();
    }

    #[test]
    fn hosts_block_render_round_trip() {
        let entries = vec![
            HostsEntry {
                address: "10.0.0.10".into(),
                hostname: "vm-a".into(),
                aliases: vec!["a".into()],
            },
            HostsEntry {
                address: "10.0.0.11".into(),
                hostname: "vm-b".into(),
                aliases: vec![],
            },
        ];
        let rendered = render_hosts_block(&entries);
        let body = format!("127.0.0.1 localhost\n{rendered}# foreign\n");
        let extracted = extract_managed_block(&body).unwrap();
        assert_eq!(extracted, rendered);
        check_hosts_block(&body, &entries).unwrap();
    }

    #[test]
    fn hosts_block_drift_fails_closed() {
        let entries = vec![HostsEntry {
            address: "10.0.0.10".into(),
            hostname: "vm-a".into(),
            aliases: vec![],
        }];
        let body = "no-marker\n";
        let err = check_hosts_block(body, &entries).unwrap_err();
        assert!(matches!(err, RoutePreflightError::HostsBlockDrift { .. }));
    }

    #[test]
    fn dnsmasq_bound_check() {
        let lan = lan_if();
        let snap = BoundSocketSnapshot {
            rows: vec![BoundSocketRow {
                ifname: lan.as_str().to_owned(),
                port: 53,
                proto: SocketProto::Udp,
                process_name: Some("dnsmasq".into()),
            }],
        };
        check_dnsmasq_bound(&snap, &lan, 53).unwrap();
    }

    #[test]
    fn dnsmasq_not_bound_fails_closed() {
        let lan = lan_if();
        let err = check_dnsmasq_bound(&BoundSocketSnapshot::default(), &lan, 53).unwrap_err();
        assert!(matches!(err, RoutePreflightError::DnsmasqNotBound { .. }));
    }

    #[test]
    fn host_lan_cidrs_skip_nixling_and_known_foreign() {
        let nl = lan_if();
        let snap = RouteTableSnapshot {
            routes: vec![
                RouteRow {
                    destination: "192.168.1.0/24".into(),
                    via: None,
                    device: "wlp0".into(),
                    scope: RouteScope::Link,
                    family: AddrFamily::V4,
                    up: true,
                    point_to_point: false,
                },
                RouteRow {
                    destination: "172.17.0.0/16".into(),
                    via: None,
                    device: "docker0".into(),
                    scope: RouteScope::Link,
                    family: AddrFamily::V4,
                    up: true,
                    point_to_point: false,
                },
                RouteRow {
                    destination: "10.20.30.0/24".into(),
                    via: None,
                    device: nl.as_str().to_owned(),
                    scope: RouteScope::Link,
                    family: AddrFamily::V4,
                    up: true,
                    point_to_point: false,
                },
                RouteRow {
                    destination: "127.0.0.0/8".into(),
                    via: None,
                    device: "lo".into(),
                    scope: RouteScope::Link,
                    family: AddrFamily::V4,
                    up: true,
                    point_to_point: false,
                },
            ],
        };
        let res = detect_host_lan_cidrs(&snap, None);
        assert_eq!(res.cidrs, vec!["192.168.1.0/24".to_string()]);
        assert!(res.ambiguous.is_empty());
    }

    #[test]
    fn host_lan_cidr_ambiguous_for_vpn() {
        let snap = RouteTableSnapshot {
            routes: vec![RouteRow {
                destination: "10.99.99.0/24".into(),
                via: None,
                device: "tun0".into(),
                scope: RouteScope::Link,
                family: AddrFamily::V4,
                up: true,
                point_to_point: true,
            }],
        };
        let res = detect_host_lan_cidrs(&snap, None);
        assert!(res.cidrs.is_empty());
        assert_eq!(res.ambiguous, vec!["10.99.99.0/24".to_string()]);
    }

    fn empty_inet_nixling_json() -> Vec<u8> {
        // Canonical empty `nft list table inet nixling -j` output.
        br#"{"nftables":[]}"#.to_vec()
    }

    #[test]
    fn firewall_coexistence_passes_when_detector_matches_policy_and_hash() {
        let declared = FirewallCoexistencePolicy {
            manager: FirewallManager::None,
            policy: CoexistencePolicy::Coexist,
            rationale: "clean host".into(),
        };
        let json = empty_inet_nixling_json();
        let expected = hash_inet_nixling_table(&json).to_string();
        check_firewall_coexistence(&declared, FirewallManager::None, &expected, &json).unwrap();
    }

    #[test]
    fn firewall_coexistence_fails_when_detector_mismatch() {
        let declared = FirewallCoexistencePolicy {
            manager: FirewallManager::None,
            policy: CoexistencePolicy::Coexist,
            rationale: "expected no manager".into(),
        };
        let json = empty_inet_nixling_json();
        let expected = hash_inet_nixling_table(&json).to_string();
        let err =
            check_firewall_coexistence(&declared, FirewallManager::Firewalld, &expected, &json)
                .unwrap_err();
        assert!(matches!(
            err,
            RoutePreflightError::FirewallCoexistenceViolation { .. }
        ));
    }

    #[test]
    fn firewall_coexistence_fails_on_table_hash_drift() {
        let declared = FirewallCoexistencePolicy {
            manager: FirewallManager::None,
            policy: CoexistencePolicy::Coexist,
            rationale: "clean host".into(),
        };
        let json = empty_inet_nixling_json();
        let err = check_firewall_coexistence(&declared, FirewallManager::None, "deadbeef", &json)
            .unwrap_err();
        assert!(matches!(
            err,
            RoutePreflightError::InetNixlingTableDrift { .. }
        ));
    }

    #[test]
    fn run_route_preflight_for_vm_threads_through_firewall_check() {
        let lan = lan_if();
        let snap = RouteTableSnapshot {
            routes: vec![RouteRow {
                destination: "default".into(),
                via: Some("192.168.1.1".into()),
                device: "wlp0".into(),
                scope: RouteScope::Global,
                family: AddrFamily::V4,
                up: true,
                point_to_point: false,
            }],
        };
        let addrs = AddrSnapshot::default();
        let sockets = BoundSocketSnapshot {
            rows: vec![BoundSocketRow {
                ifname: lan.as_str().to_owned(),
                port: 53,
                proto: SocketProto::Udp,
                process_name: Some("dnsmasq".into()),
            }],
        };
        let entries: Vec<HostsEntry> = vec![];
        let hosts_body = render_hosts_block(&entries);
        let policy = FirewallCoexistencePolicy {
            manager: FirewallManager::None,
            policy: CoexistencePolicy::Coexist,
            rationale: "clean host".into(),
        };
        let json = empty_inet_nixling_json();
        let expected_hash = hash_inet_nixling_table(&json).to_string();
        let input = RoutePreflightInput {
            routes: &snap,
            addrs: &addrs,
            uplinks: &[UplinkExpectation {
                uplink_device: "wlp0".into(),
                expected_via: Some("192.168.1.1".into()),
            }],
            hosts_body: &hosts_body,
            hosts_expected: &entries,
            sockets: &sockets,
            dnsmasq_ifname: &lan,
            dnsmasq_port: 53,
            firewall_coexistence: &policy,
            nft_detector_result: FirewallManager::None,
            expected_inet_nixling_hash: &expected_hash,
            nft_list_json: &json,
        };
        run_route_preflight_for_vm("vm-a", &input).unwrap();
    }
}
