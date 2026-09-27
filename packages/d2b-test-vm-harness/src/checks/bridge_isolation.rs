//! The bridge port isolation check, ported from its fixture.
//!
//! It exercises the bridge shape the host configures at runtime, as root,
//! inside the guest: one non-isolated net-VM port and two isolated workload
//! ports on `br-work-lan`, each port's peer living in its own network
//! namespace. The assertions are the fixture's, in the fixture's order and
//! with the fixture's own command text and bounds: the port that carries the
//! net VM stays reachable from both workloads, and the two workload ports
//! stay isolated from each other - including after one of them changes its
//! MAC address, which is the case a MAC-address-based filter would not catch.
//!
//! The fixture declared a plain NixOS node - it never wanted the d2b daemon
//! host - so the guest it boots is declared in
//! `nix/test-support/host-integration-node.nix` beside the reusable nodes,
//! with the same two packages it declared, and the guest's activation
//! contract is `multi-user.target`, which is the unit a node that declares no
//! acceptance units falls back to.
//!
//! `start_all()` is not restated here: it is the lane's own boot of the
//! guest the check runs against.

use crate::legacy::{GuestControl, LegacyError, LegacyResult};

/// The network namespaces the fixture creates, in its own order.
const NAMESPACES: [&str; 3] = ["netvm", "vm10", "vm11"];

/// Each bridge port and the namespace its peer device lives in, in the
/// fixture's own order.
const PORTS: [(&str, &str); 3] = [
    ("work-l1", "netvm"),
    ("work-l10", "vm10"),
    ("work-l11", "vm11"),
];

/// The address each namespace's `eth0` gets, in the fixture's own order.
const ADDRESSES: [(&str, &str); 3] = [
    ("netvm", "10.20.0.1/24"),
    ("vm10", "10.20.0.10/24"),
    ("vm11", "10.20.0.11/24"),
];

/// The MAC address `vm10` changes its `eth0` to. The second isolation
/// assertion is made after this, so isolation is shown not to rest on the
/// address the port was isolated with.
const REPLACEMENT_MAC: &str = "02:20:00:00:00:11";

/// The check's assertions, in the order its fixture made them.
pub fn assertions(control: &mut GuestControl) -> LegacyResult<()> {
    // The netns mount point, the namespaces, the bridge, and the three veth
    // pairs the fixture built before it asserted anything about them.
    control.succeed(&["mkdir -p /run/netns"], None)?;
    for namespace in NAMESPACES {
        control.succeed(&[&format!("ip netns add {namespace}")], None)?;
    }
    control.succeed(&["ip link add br-work-lan type bridge"], None)?;
    control.succeed(&["ip link set br-work-lan up"], None)?;
    for (port, namespace) in PORTS {
        control.succeed(
            &[&format!(
                "ip link add {port} type veth peer name eth0 netns {namespace}"
            )],
            None,
        )?;
        control.succeed(&[&format!("ip link set {port} master br-work-lan")], None)?;
        control.succeed(&[&format!("ip link set {port} up")], None)?;
    }

    // Port isolation on the two workload ports, and not on the net-VM port.
    control.succeed(&["bridge link set dev work-l10 isolated on"], None)?;
    control.succeed(&["bridge link set dev work-l11 isolated on"], None)?;

    for namespace in NAMESPACES {
        control.succeed(&[&format!("ip netns exec {namespace} ip link set lo up")], None)?;
        control.succeed(
            &[&format!("ip netns exec {namespace} ip link set eth0 up")],
            None,
        )?;
    }
    for (namespace, address) in ADDRESSES {
        control.succeed(
            &[&format!("ip netns exec {namespace} ip addr add {address} dev eth0")],
            None,
        )?;
    }

    // The declared isolation state, read back off the live bridge: the
    // net-VM port must remain non-isolated and both workload ports isolated.
    let work_l1 = control.succeed(&["bridge -d link show dev work-l1"], None)?;
    if work_l1.contains("isolated on") {
        return Err(LegacyError::Assertion(
            "net-VM bridge port work-l1 must remain non-isolated".to_owned(),
        ));
    }
    for (port, message) in [
        ("work-l10", "workload bridge port work-l10 is not isolated"),
        ("work-l11", "workload bridge port work-l11 is not isolated"),
    ] {
        let port_state = control.succeed(&[&format!("bridge -d link show dev {port}")], None)?;
        if !port_state.contains("isolated on") {
            return Err(LegacyError::Assertion(message.to_owned()));
        }
    }

    // Both workloads reach the net VM.
    control.succeed(&["ip netns exec vm10 ping -c1 -W1 10.20.0.1 >/dev/null"], None)?;
    control.succeed(&["ip netns exec vm11 ping -c1 -W1 10.20.0.1 >/dev/null"], None)?;

    // The two workloads do not reach each other.
    control.fail(
        &["ip netns exec vm10 ping -c1 -W1 10.20.0.11 >/dev/null 2>&1"],
        None,
    )?;

    // ... and still do not after vm10's eth0 changes address, so the
    // isolation is a property of the port rather than of the address it was
    // isolated with.
    control.succeed(
        &[&format!(
            "ip netns exec vm10 ip link set dev eth0 address {REPLACEMENT_MAC}"
        )],
        None,
    )?;
    control.fail(
        &["ip netns exec vm10 ping -c1 -W1 10.20.0.11 >/dev/null 2>&1"],
        None,
    )?;

    Ok(())
}
