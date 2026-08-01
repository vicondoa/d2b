//! Ordered IPv6 suppression and bridge-port observation contracts.

use crate::bridge_port::{BridgePortFlagDrift, BridgePortFlagSet, TapRole, validate_readback};
use crate::ifname::IfName;

/// One closed per-link sysctl setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LinkSysctlSetting {
    /// Disable IPv6 on the link.
    DisableIpv6,
    /// Reject IPv6 router advertisements.
    AcceptRouterAdvertisement,
    /// Disable IPv6 address autoconfiguration.
    Autoconfiguration,
    /// Disable automatic IPv6 address generation.
    AddressGenerationMode,
    /// Ignore ARP queries outside the intended interface scope.
    ArpIgnore,
}

impl LinkSysctlSetting {
    /// Return the required value.
    pub const fn expected(self) -> &'static str {
        match self {
            Self::DisableIpv6 => "1",
            Self::AcceptRouterAdvertisement | Self::Autoconfiguration => "0",
            Self::AddressGenerationMode | Self::ArpIgnore => "1",
        }
    }
}

/// Every per-link defense-in-depth setting in application order.
pub const IPV6_OFF_SETTINGS: &[LinkSysctlSetting] = &[
    LinkSysctlSetting::DisableIpv6,
    LinkSysctlSetting::AcceptRouterAdvertisement,
    LinkSysctlSetting::Autoconfiguration,
    LinkSysctlSetting::AddressGenerationMode,
    LinkSysctlSetting::ArpIgnore,
];

/// One closed bridge-netfilter setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeNetfilterSetting {
    /// Disable bridge traversal through IPv4 iptables.
    Iptables,
    /// Disable bridge traversal through IPv6 iptables.
    Ip6tables,
    /// Disable bridge traversal through arptables.
    Arptables,
}

impl BridgeNetfilterSetting {
    /// Return the required value.
    pub const fn expected(self) -> &'static str {
        "0"
    }
}

/// Every bridge-netfilter setting in application order.
pub const BRIDGE_NETFILTER_SETTINGS: &[BridgeNetfilterSetting] = &[
    BridgeNetfilterSetting::Iptables,
    BridgeNetfilterSetting::Ip6tables,
    BridgeNetfilterSetting::Arptables,
];

/// Link type requested through the closed bridge or TAP operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// A Network fabric bridge.
    Bridge,
    /// A persistent attachment TAP.
    Tap,
}

/// A link creation intent whose diagnostics redact its interface name.
pub struct LinkSpec {
    ifname: IfName,
    kind: LinkKind,
    mtu: Option<u32>,
}

impl LinkSpec {
    /// Construct a typed link intent.
    pub const fn new(ifname: IfName, kind: LinkKind, mtu: Option<u32>) -> Self {
        Self { ifname, kind, mtu }
    }

    /// Borrow the interface name only for the core effect adapter.
    pub const fn ifname(&self) -> &IfName {
        &self.ifname
    }

    /// Return the requested link kind.
    pub const fn kind(&self) -> LinkKind {
        self.kind
    }

    /// Return the requested MTU.
    pub const fn mtu(&self) -> Option<u32> {
        self.mtu
    }
}

impl core::fmt::Debug for LinkSpec {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("LinkSpec(<redacted>)")
    }
}

/// Injected adapter for link creation and sysctl readback.
pub trait NetlinkBackend {
    /// Create the link with its up flag clear.
    fn create_link_down(&mut self, spec: &LinkSpec) -> Result<(), NetlinkError>;
    /// Write one per-link setting.
    fn write_link_sysctl(
        &mut self,
        ifname: &IfName,
        setting: LinkSysctlSetting,
        value: &str,
    ) -> Result<(), NetlinkError>;
    /// Read one per-link setting.
    fn read_link_sysctl(
        &mut self,
        ifname: &IfName,
        setting: LinkSysctlSetting,
    ) -> Result<String, NetlinkError>;
    /// Bring an existing link up.
    fn set_link_up(&mut self, ifname: &IfName) -> Result<(), NetlinkError>;
    /// Report whether bridge netfilter is loaded.
    fn bridge_netfilter_loaded(&mut self) -> Result<bool, NetlinkError>;
    /// Write one global bridge-netfilter setting.
    fn write_bridge_netfilter_sysctl(
        &mut self,
        setting: BridgeNetfilterSetting,
        value: &str,
    ) -> Result<(), NetlinkError>;
    /// Read one global bridge-netfilter setting.
    fn read_bridge_netfilter_sysctl(
        &mut self,
        setting: BridgeNetfilterSetting,
    ) -> Result<String, NetlinkError>;
    /// Read every bridge-port flag.
    fn read_bridge_port_flags(
        &mut self,
        ifname: &IfName,
    ) -> Result<BridgePortFlagSet, NetlinkError>;
}

/// One value-free netlink failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetlinkError {
    /// NetworkManager unmanaged policy was not installed first.
    NmUnmanagedRequired,
    /// The injected adapter failed without exposing raw kernel output.
    Backend,
    /// One or more IPv6 suppression settings drifted.
    SysctlDrift,
    /// Bridge-port readback disagreed with policy.
    BridgePortFlagDrift,
}

impl NetlinkError {
    /// Return the stable redacted reason code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::NmUnmanagedRequired => "nm-unmanaged-required",
            Self::Backend => "netlink-error",
            Self::SysctlDrift => "ipv6-sysctl-drift",
            Self::BridgePortFlagDrift => "bridge-port-flag-drift",
        }
    }
}

impl core::fmt::Display for NetlinkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for NetlinkError {}

/// Identity-free summary of sysctl readback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SysctlReadback {
    checked: usize,
    drifted: usize,
}

impl SysctlReadback {
    /// Return the number of checked settings.
    pub const fn checked(self) -> usize {
        self.checked
    }

    /// Return whether any setting drifted.
    pub const fn has_drift(self) -> bool {
        self.drifted != 0
    }
}

/// Create a link with IPv6 suppressed before it is brought up, then read back.
pub fn ipv6_off_sequence<B: NetlinkBackend>(
    backend: &mut B,
    spec: &LinkSpec,
    nm_unmanaged_applied: bool,
) -> Result<SysctlReadback, NetlinkError> {
    if !nm_unmanaged_applied {
        return Err(NetlinkError::NmUnmanagedRequired);
    }
    backend.create_link_down(spec)?;
    write_suppression(backend, spec.ifname())?;
    backend.set_link_up(spec.ifname())?;
    ensure_readback(backend, spec.ifname())
}

/// Re-apply IPv6 suppression during every reconcile cycle and read it back.
pub fn reapply_ipv6_suppression<B: NetlinkBackend>(
    backend: &mut B,
    ifname: &IfName,
) -> Result<SysctlReadback, NetlinkError> {
    write_suppression(backend, ifname)?;
    ensure_readback(backend, ifname)
}

fn write_suppression<B: NetlinkBackend>(
    backend: &mut B,
    ifname: &IfName,
) -> Result<(), NetlinkError> {
    for setting in IPV6_OFF_SETTINGS {
        backend.write_link_sysctl(ifname, *setting, setting.expected())?;
    }
    if backend.bridge_netfilter_loaded()? {
        for setting in BRIDGE_NETFILTER_SETTINGS {
            backend.write_bridge_netfilter_sysctl(*setting, setting.expected())?;
        }
    }
    Ok(())
}

fn ensure_readback<B: NetlinkBackend>(
    backend: &mut B,
    ifname: &IfName,
) -> Result<SysctlReadback, NetlinkError> {
    let readback = readback_sysctls(backend, ifname)?;
    if readback.has_drift() {
        Err(NetlinkError::SysctlDrift)
    } else {
        Ok(readback)
    }
}

/// Read every suppression setting without exposing its interface name.
pub fn readback_sysctls<B: NetlinkBackend>(
    backend: &mut B,
    ifname: &IfName,
) -> Result<SysctlReadback, NetlinkError> {
    let mut checked = 0;
    let mut drifted = 0;
    for setting in IPV6_OFF_SETTINGS {
        checked += 1;
        if backend.read_link_sysctl(ifname, *setting)? != setting.expected() {
            drifted += 1;
        }
    }
    if backend.bridge_netfilter_loaded()? {
        for setting in BRIDGE_NETFILTER_SETTINGS {
            checked += 1;
            if backend.read_bridge_netfilter_sysctl(*setting)? != setting.expected() {
                drifted += 1;
            }
        }
    }
    Ok(SysctlReadback { checked, drifted })
}

/// Read and validate every bridge-port flag for one semantic role.
pub fn readback_bridge_port_flags<B: NetlinkBackend>(
    backend: &mut B,
    ifname: &IfName,
    role: TapRole,
) -> Result<BridgePortFlagSet, NetlinkError> {
    let observed = backend.read_bridge_port_flags(ifname)?;
    validate_readback(role, observed)
        .map_err(|_drift: BridgePortFlagDrift| NetlinkError::BridgePortFlagDrift)?;
    Ok(observed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ifname::{NetworkIfRole, derive_ifname};
    use std::collections::BTreeMap;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Operation {
        CreateDown,
        WriteLink,
        SetUp,
        ReadLink,
    }

    struct FakeBackend {
        values: BTreeMap<LinkSysctlSetting, String>,
        operations: Vec<Operation>,
        drift: Option<LinkSysctlSetting>,
    }

    impl FakeBackend {
        fn new() -> Self {
            Self {
                values: BTreeMap::new(),
                operations: Vec::new(),
                drift: None,
            }
        }
    }

    impl NetlinkBackend for FakeBackend {
        fn create_link_down(&mut self, _spec: &LinkSpec) -> Result<(), NetlinkError> {
            self.operations.push(Operation::CreateDown);
            Ok(())
        }

        fn write_link_sysctl(
            &mut self,
            _ifname: &IfName,
            setting: LinkSysctlSetting,
            value: &str,
        ) -> Result<(), NetlinkError> {
            self.operations.push(Operation::WriteLink);
            self.values.insert(setting, value.to_owned());
            Ok(())
        }

        fn read_link_sysctl(
            &mut self,
            _ifname: &IfName,
            setting: LinkSysctlSetting,
        ) -> Result<String, NetlinkError> {
            self.operations.push(Operation::ReadLink);
            if self.drift == Some(setting) {
                return Ok("drift".to_owned());
            }
            Ok(self.values.get(&setting).cloned().unwrap_or_default())
        }

        fn set_link_up(&mut self, _ifname: &IfName) -> Result<(), NetlinkError> {
            self.operations.push(Operation::SetUp);
            Ok(())
        }

        fn bridge_netfilter_loaded(&mut self) -> Result<bool, NetlinkError> {
            Ok(false)
        }

        fn write_bridge_netfilter_sysctl(
            &mut self,
            _setting: BridgeNetfilterSetting,
            _value: &str,
        ) -> Result<(), NetlinkError> {
            Ok(())
        }

        fn read_bridge_netfilter_sysctl(
            &mut self,
            _setting: BridgeNetfilterSetting,
        ) -> Result<String, NetlinkError> {
            Ok("0".to_owned())
        }

        fn read_bridge_port_flags(
            &mut self,
            _ifname: &IfName,
        ) -> Result<BridgePortFlagSet, NetlinkError> {
            Ok(BridgePortFlagSet::defaults_for(
                TapRole::WorkloadLanIsolated,
            ))
        }
    }

    fn spec() -> LinkSpec {
        LinkSpec::new(
            derive_ifname("work", NetworkIfRole::LanBridge, None, None).unwrap(),
            LinkKind::Bridge,
            Some(1500),
        )
    }

    #[test]
    fn ipv6_off_sequence_runs_in_order() {
        let mut backend = FakeBackend::new();
        let readback = ipv6_off_sequence(&mut backend, &spec(), true).unwrap();
        assert!(!readback.has_drift());
        let create = backend
            .operations
            .iter()
            .position(|op| *op == Operation::CreateDown)
            .unwrap();
        let write = backend
            .operations
            .iter()
            .position(|op| *op == Operation::WriteLink)
            .unwrap();
        let up = backend
            .operations
            .iter()
            .position(|op| *op == Operation::SetUp)
            .unwrap();
        let read = backend
            .operations
            .iter()
            .position(|op| *op == Operation::ReadLink)
            .unwrap();
        assert!(create < write && write < up && up < read);
    }

    #[test]
    fn defense_in_depth_reapply_repairs_drift() {
        let mut backend = FakeBackend::new();
        let spec = spec();
        backend
            .values
            .insert(LinkSysctlSetting::DisableIpv6, "0".to_owned());
        let readback = reapply_ipv6_suppression(&mut backend, spec.ifname()).unwrap();
        assert!(!readback.has_drift());
        assert_eq!(readback.checked(), IPV6_OFF_SETTINGS.len());
        assert!(!backend.operations.contains(&Operation::CreateDown));
        assert!(!backend.operations.contains(&Operation::SetUp));
    }

    #[test]
    fn readback_drift_fails_closed_without_identity() {
        let mut backend = FakeBackend::new();
        backend.drift = Some(LinkSysctlSetting::AcceptRouterAdvertisement);
        let error = ipv6_off_sequence(&mut backend, &spec(), true).unwrap_err();
        assert_eq!(error, NetlinkError::SysctlDrift);
        assert_eq!(format!("{error:?}"), "SysctlDrift");
    }

    #[test]
    fn bridge_port_readback_matches_defaults() {
        let mut backend = FakeBackend::new();
        let spec = spec();
        assert_eq!(
            readback_bridge_port_flags(&mut backend, spec.ifname(), TapRole::WorkloadLanIsolated,)
                .unwrap(),
            BridgePortFlagSet::defaults_for(TapRole::WorkloadLanIsolated)
        );
    }
}
