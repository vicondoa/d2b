//! W3 host-prepare module: `netlink` — owned by scope s2.
//!
//! Defines the contract surface for the rtnetlink-backed bridge/TAP
//! reconcile + the IPv6-off sysctl readback per plan.md §"W3 IPv6-off
//! ordering with NetworkManager / systemd-networkd".
//!
//! The 5-step ordered sequence per plan.md is encoded by
//! [`ipv6_off_sequence`] which drives:
//!
//! 1. **Pre-create**: NM unmanaged drop + reload (handled out-of-band
//!    by the broker's `nm` op; the netlink trait exposes the link-down
//!    state precondition);
//! 2. **Create link** with `IFF_UP` bit cleared;
//! 3. **Write per-link sysctls** while link is down;
//! 4. **Bring link up** (`IFF_UP`);
//! 5. **Readback gate**: re-read every sysctl, fail closed on drift.
//!
//! `forbid(unsafe_code)` at the crate root excludes raw FFI from this
//! file. The production `rtnetlink`-backed real backend lives in the
//! broker (which has `unsafe_code = "deny"` and a quarantined `sys.rs`
//! for the netlink + SCM_RIGHTS dance). This module defines the
//! [`NetlinkBackend`] trait both backends implement, plus a
//! [`fake::FakeBackend`] used by the L1c canary tests (see plan.md
//! §"W3 pre-merge canary matrix" rows `ipv6-sysctl-drift`,
//! `bridge-port-flag-drift`).

use crate::bridge_port::{BridgePortFlagSet, BridgePortPolicyError};
use crate::ifname::{DerivedRole, IfName};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The full IPv6-off sysctl set per plan.md §"W3 IPv6-off ordering"
/// step 3, keyed by sysctl leaf name (interface-scoped path is
/// `net.ipv6.conf.<ifname>.<leaf>` or `net.ipv4.conf.<ifname>.<leaf>`).
pub const IPV6_OFF_SYSCTLS: &[Ipv6OffSysctl] = &[
    Ipv6OffSysctl {
        family: SysctlFamily::Ipv6,
        leaf: "disable_ipv6",
        expected: "1",
    },
    Ipv6OffSysctl {
        family: SysctlFamily::Ipv6,
        leaf: "accept_ra",
        expected: "0",
    },
    Ipv6OffSysctl {
        family: SysctlFamily::Ipv6,
        leaf: "autoconf",
        expected: "0",
    },
    Ipv6OffSysctl {
        family: SysctlFamily::Ipv6,
        leaf: "addr_gen_mode",
        expected: "1",
    },
    Ipv6OffSysctl {
        family: SysctlFamily::Ipv4,
        leaf: "arp_ignore",
        expected: "1",
    },
];

/// Bridge-netfilter sysctls applied when `br_netfilter` is loaded.
pub const BRIDGE_NF_SYSCTLS: &[(&str, &str)] = &[
    ("net.bridge.bridge-nf-call-iptables", "0"),
    ("net.bridge.bridge-nf-call-ip6tables", "0"),
    ("net.bridge.bridge-nf-call-arptables", "0"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SysctlFamily {
    Ipv4,
    Ipv6,
}

impl SysctlFamily {
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Ipv4 => "net.ipv4.conf",
            Self::Ipv6 => "net.ipv6.conf",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ipv6OffSysctl {
    pub family: SysctlFamily,
    pub leaf: &'static str,
    pub expected: &'static str,
}

impl Ipv6OffSysctl {
    pub fn dotted_path(&self, ifname: &IfName) -> String {
        format!("{}.{}.{}", self.family.prefix(), ifname.as_str(), self.leaf)
    }
}

/// Per-link sysctl readback report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SysctlReadback {
    pub ifname: String,
    pub entries: Vec<SysctlReadbackEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SysctlReadbackEntry {
    pub key: String,
    pub expected: String,
    pub observed: String,
    pub drift: bool,
}

impl SysctlReadback {
    pub fn has_drift(&self) -> bool {
        self.entries.iter().any(|e| e.drift)
    }
}

/// Type of link created via the broker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkKind {
    Bridge,
    Tap,
}

impl From<DerivedRole> for LinkKind {
    fn from(r: DerivedRole) -> Self {
        match r {
            DerivedRole::Bridge => Self::Bridge,
            DerivedRole::Tap => Self::Tap,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkState {
    Up,
    Down,
}

/// Specification for a netlink link create operation.
#[derive(Debug, Clone)]
pub struct LinkSpec {
    pub ifname: IfName,
    pub kind: LinkKind,
    pub mtu: Option<u32>,
    pub mac: Option<[u8; 6]>,
    /// For TAPs: the uid/gid that owns the persistent character device
    /// (`TUNSETOWNER` / `TUNSETGROUP`). For bridges: ignored.
    pub tap_owner: Option<TapOwner>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TapOwner {
    pub uid: u32,
    pub gid: u32,
}

/// Error type for netlink ops — wire-stable variant tags drive the
/// audit `error_kind` field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetlinkError {
    LinkAlreadyExists { ifname: String },
    LinkNotFound { ifname: String },
    SysctlDrift(SysctlReadback),
    BridgePortFlagDrift(crate::bridge_port::BridgePortFlagDrift),
    Policy(BridgePortPolicyError),
    NotImplementedReal,
    Backend(String),
}

impl std::fmt::Display for NetlinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LinkAlreadyExists { ifname } => {
                write!(f, "netlink: link {ifname:?} already exists")
            }
            Self::LinkNotFound { ifname } => {
                write!(f, "netlink: link {ifname:?} not found")
            }
            Self::SysctlDrift(rb) => write!(
                f,
                "ipv6-sysctl-drift on {}: {} entries drifted",
                rb.ifname,
                rb.entries.iter().filter(|e| e.drift).count()
            ),
            Self::BridgePortFlagDrift(d) => write!(
                f,
                "bridge-port-flag-drift on role {:?}: {} flag(s) differ",
                d.role,
                d.differences.len()
            ),
            Self::Policy(p) => write!(f, "policy: {p}"),
            Self::NotImplementedReal => write!(
                f,
                "real rtnetlink backend not yet enabled; use NetlinkBackend trait impl"
            ),
            Self::Backend(s) => write!(f, "netlink backend: {s}"),
        }
    }
}

impl std::error::Error for NetlinkError {}

impl From<BridgePortPolicyError> for NetlinkError {
    fn from(value: BridgePortPolicyError) -> Self {
        Self::Policy(value)
    }
}

/// Abstract netlink backend. Production binaries instantiate a real
/// rtnetlink-backed impl (lives in the broker); tests instantiate
/// [`fake::FakeBackend`].
pub trait NetlinkBackend {
    fn create_link(&mut self, spec: &LinkSpec) -> Result<(), NetlinkError>;
    fn delete_link(&mut self, ifname: &IfName) -> Result<(), NetlinkError>;
    fn set_link_state(&mut self, ifname: &IfName, state: LinkState) -> Result<(), NetlinkError>;
    fn write_sysctl(&mut self, key: &str, value: &str) -> Result<(), NetlinkError>;
    fn read_sysctl(&mut self, key: &str) -> Result<String, NetlinkError>;
    fn read_bridge_port_flags(
        &mut self,
        ifname: &IfName,
    ) -> Result<BridgePortFlagSet, NetlinkError>;
    fn write_bridge_port_flags(
        &mut self,
        ifname: &IfName,
        flags: BridgePortFlagSet,
    ) -> Result<(), NetlinkError>;
    /// Mirrors `/proc/modules` lookup for `br_netfilter`.
    fn br_netfilter_loaded(&mut self) -> Result<bool, NetlinkError>;
}

/// Drives the W3 IPv6-off 5-step ordered sequence over an arbitrary
/// [`NetlinkBackend`]. Step 1 (NM unmanaged) is the broker's
/// responsibility and is therefore represented here only as a
/// precondition flag — the caller must have invoked the broker NM op
/// before calling this function.
pub fn ipv6_off_sequence<B: NetlinkBackend>(
    backend: &mut B,
    spec: &LinkSpec,
    nm_unmanaged_applied: bool,
) -> Result<SysctlReadback, NetlinkError> {
    if !nm_unmanaged_applied {
        return Err(NetlinkError::Backend(
            "ipv6-off-sequence requires NM unmanaged drop-in applied first".into(),
        ));
    }
    // Step 2: create link with IFF_UP cleared.
    backend.create_link(spec)?;
    backend.set_link_state(&spec.ifname, LinkState::Down)?;
    // Step 3: write per-link sysctls.
    for s in IPV6_OFF_SYSCTLS {
        backend.write_sysctl(&s.dotted_path(&spec.ifname), s.expected)?;
    }
    if backend.br_netfilter_loaded()? {
        for (k, v) in BRIDGE_NF_SYSCTLS {
            backend.write_sysctl(k, v)?;
        }
    }
    // Step 4: bring link up.
    backend.set_link_state(&spec.ifname, LinkState::Up)?;
    // Step 5: readback gate.
    let readback = readback_sysctls(backend, &spec.ifname)?;
    if readback.has_drift() {
        return Err(NetlinkError::SysctlDrift(readback));
    }
    Ok(readback)
}

/// Reads back every IPv6-off sysctl for `ifname`. Bridge-nf sysctls
/// are read globally and appended when `br_netfilter` is loaded.
pub fn readback_sysctls<B: NetlinkBackend>(
    backend: &mut B,
    ifname: &IfName,
) -> Result<SysctlReadback, NetlinkError> {
    let mut entries = Vec::new();
    for s in IPV6_OFF_SYSCTLS {
        let key = s.dotted_path(ifname);
        let observed = backend.read_sysctl(&key)?;
        entries.push(SysctlReadbackEntry {
            drift: observed != s.expected,
            key,
            expected: s.expected.to_owned(),
            observed,
        });
    }
    if backend.br_netfilter_loaded()? {
        for (k, v) in BRIDGE_NF_SYSCTLS {
            let observed = backend.read_sysctl(k)?;
            entries.push(SysctlReadbackEntry {
                drift: observed != *v,
                key: (*k).to_owned(),
                expected: (*v).to_owned(),
                observed,
            });
        }
    }
    Ok(SysctlReadback {
        ifname: ifname.as_str().to_owned(),
        entries,
    })
}

/// Reads back the bridge-port flag set for `ifname` and validates it
/// against the per-role defaults.
pub fn readback_bridge_port_flags<B: NetlinkBackend>(
    backend: &mut B,
    ifname: &IfName,
    role: nixling_core::host_w3::TapRoleW3,
) -> Result<BridgePortFlagSet, NetlinkError> {
    let observed = backend.read_bridge_port_flags(ifname)?;
    crate::bridge_port::validate_readback(role, observed)
        .map_err(NetlinkError::BridgePortFlagDrift)?;
    Ok(observed)
}

// -------------------------------------------------------------------
// Fake backend (used by L1c canary tests + integration tests)
// -------------------------------------------------------------------

pub mod fake {
    use super::*;
    use std::cell::RefCell;

    /// In-memory netlink simulator. Records every operation so tests
    /// can assert ordering, idempotency, and drift behaviour.
    #[derive(Debug, Default)]
    pub struct FakeBackend {
        pub links: BTreeMap<String, FakeLink>,
        pub sysctls: BTreeMap<String, String>,
        pub br_netfilter: bool,
        pub ops_log: RefCell<Vec<String>>,
        /// If set, simulates a foreign actor flipping `key` to `value`
        /// after step 3 but before step 5 readback.
        pub drift_after_write: Option<(String, String)>,
        /// If set, simulates a foreign actor returning these flags on
        /// the next bridge-port readback regardless of what was
        /// written.
        pub force_bridge_port_flags: Option<BridgePortFlagSet>,
    }

    #[derive(Debug, Clone)]
    pub struct FakeLink {
        pub spec: LinkSpec,
        pub state: LinkState,
        pub bridge_port_flags: BridgePortFlagSet,
    }

    impl FakeBackend {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn with_br_netfilter(mut self, loaded: bool) -> Self {
            self.br_netfilter = loaded;
            self
        }

        pub fn log(&self, msg: impl Into<String>) {
            self.ops_log.borrow_mut().push(msg.into());
        }

        pub fn ops(&self) -> Vec<String> {
            self.ops_log.borrow().clone()
        }
    }

    impl NetlinkBackend for FakeBackend {
        fn create_link(&mut self, spec: &LinkSpec) -> Result<(), NetlinkError> {
            self.log(format!("create_link:{}", spec.ifname.as_str()));
            let key = spec.ifname.as_str().to_owned();
            if self.links.contains_key(&key) {
                return Err(NetlinkError::LinkAlreadyExists { ifname: key });
            }
            self.links.insert(
                key,
                FakeLink {
                    spec: spec.clone(),
                    state: LinkState::Down,
                    bridge_port_flags: BridgePortFlagSet::ALL_OFF,
                },
            );
            Ok(())
        }
        fn delete_link(&mut self, ifname: &IfName) -> Result<(), NetlinkError> {
            self.log(format!("delete_link:{}", ifname.as_str()));
            if self.links.remove(ifname.as_str()).is_none() {
                return Err(NetlinkError::LinkNotFound {
                    ifname: ifname.as_str().to_owned(),
                });
            }
            Ok(())
        }
        fn set_link_state(
            &mut self,
            ifname: &IfName,
            state: LinkState,
        ) -> Result<(), NetlinkError> {
            self.log(format!("set_link_state:{}:{:?}", ifname.as_str(), state));
            let link =
                self.links
                    .get_mut(ifname.as_str())
                    .ok_or_else(|| NetlinkError::LinkNotFound {
                        ifname: ifname.as_str().to_owned(),
                    })?;
            link.state = state;
            Ok(())
        }
        fn write_sysctl(&mut self, key: &str, value: &str) -> Result<(), NetlinkError> {
            self.log(format!("write_sysctl:{key}={value}"));
            self.sysctls.insert(key.to_owned(), value.to_owned());
            // Simulate post-write foreign drift if configured.
            if let Some((dk, dv)) = self.drift_after_write.clone() {
                if dk == key {
                    self.sysctls.insert(dk, dv);
                }
            }
            Ok(())
        }
        fn read_sysctl(&mut self, key: &str) -> Result<String, NetlinkError> {
            self.log(format!("read_sysctl:{key}"));
            Ok(self.sysctls.get(key).cloned().unwrap_or_default())
        }
        fn read_bridge_port_flags(
            &mut self,
            ifname: &IfName,
        ) -> Result<BridgePortFlagSet, NetlinkError> {
            self.log(format!("read_bridge_port_flags:{}", ifname.as_str()));
            if let Some(forced) = self.force_bridge_port_flags {
                return Ok(forced);
            }
            self.links
                .get(ifname.as_str())
                .map(|l| l.bridge_port_flags)
                .ok_or_else(|| NetlinkError::LinkNotFound {
                    ifname: ifname.as_str().to_owned(),
                })
        }
        fn write_bridge_port_flags(
            &mut self,
            ifname: &IfName,
            flags: BridgePortFlagSet,
        ) -> Result<(), NetlinkError> {
            self.log(format!("write_bridge_port_flags:{}", ifname.as_str()));
            let link =
                self.links
                    .get_mut(ifname.as_str())
                    .ok_or_else(|| NetlinkError::LinkNotFound {
                        ifname: ifname.as_str().to_owned(),
                    })?;
            link.bridge_port_flags = flags;
            Ok(())
        }
        fn br_netfilter_loaded(&mut self) -> Result<bool, NetlinkError> {
            Ok(self.br_netfilter)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge_port::BridgePortFlagSet;
    use crate::ifname::{derive_from_env_vm, DerivedRole};
    use fake::FakeBackend;
    use nixling_core::host_w3::TapRoleW3;

    fn br_spec() -> LinkSpec {
        LinkSpec {
            ifname: derive_from_env_vm("e", None, DerivedRole::Bridge, None).unwrap(),
            kind: LinkKind::Bridge,
            mtu: Some(1500),
            mac: None,
            tap_owner: None,
        }
    }

    #[test]
    fn ipv6_off_sequence_runs_in_order() {
        let mut be = FakeBackend::new();
        let spec = br_spec();
        let readback = ipv6_off_sequence(&mut be, &spec, true).unwrap();
        assert!(!readback.has_drift());
        let ops = be.ops();
        let create_idx = ops
            .iter()
            .position(|o| o.starts_with("create_link:"))
            .unwrap();
        let down_idx = ops
            .iter()
            .position(|o| o.starts_with("set_link_state:") && o.ends_with("Down"))
            .unwrap();
        let first_sysctl = ops
            .iter()
            .position(|o| o.starts_with("write_sysctl:"))
            .unwrap();
        let up_idx = ops
            .iter()
            .position(|o| o.starts_with("set_link_state:") && o.ends_with("Up"))
            .unwrap();
        let readback_idx = ops
            .iter()
            .position(|o| o.starts_with("read_sysctl:"))
            .unwrap();
        assert!(
            create_idx < down_idx
                && down_idx < first_sysctl
                && first_sysctl < up_idx
                && up_idx < readback_idx,
            "ordering violated: {ops:?}"
        );
    }

    #[test]
    fn ipv6_off_sequence_fails_closed_on_nm_precondition() {
        let mut be = FakeBackend::new();
        let err = ipv6_off_sequence(&mut be, &br_spec(), false).unwrap_err();
        assert!(matches!(err, NetlinkError::Backend(_)));
    }

    #[test]
    fn drift_after_write_fails_closed() {
        let mut be = FakeBackend::new();
        let spec = br_spec();
        let key = format!("net.ipv6.conf.{}.disable_ipv6", spec.ifname.as_str());
        be.drift_after_write = Some((key, "0".into()));
        let err = ipv6_off_sequence(&mut be, &spec, true).unwrap_err();
        match err {
            NetlinkError::SysctlDrift(rb) => assert!(rb.has_drift()),
            other => panic!("expected drift, got {other:?}"),
        }
    }

    #[test]
    fn bridge_nf_sysctls_applied_when_loaded() {
        let mut be = FakeBackend::new().with_br_netfilter(true);
        let spec = br_spec();
        ipv6_off_sequence(&mut be, &spec, true).unwrap();
        for key in [
            "net.bridge.bridge-nf-call-iptables",
            "net.bridge.bridge-nf-call-ip6tables",
            "net.bridge.bridge-nf-call-arptables",
        ] {
            assert_eq!(be.sysctls.get(key).map(String::as_str), Some("0"));
        }
    }

    #[test]
    fn bridge_port_readback_drift_fails_closed() {
        let mut be = FakeBackend::new();
        be.force_bridge_port_flags = Some(BridgePortFlagSet::ALL_OFF);
        let spec = br_spec();
        be.create_link(&spec).unwrap();
        let err = readback_bridge_port_flags(&mut be, &spec.ifname, TapRoleW3::WorkloadLanIsolated)
            .unwrap_err();
        assert!(matches!(err, NetlinkError::BridgePortFlagDrift(_)));
    }

    #[test]
    fn bridge_port_readback_matches_defaults() {
        let mut be = FakeBackend::new();
        let spec = br_spec();
        be.create_link(&spec).unwrap();
        let defaults = BridgePortFlagSet::defaults_for(TapRoleW3::WorkloadLanIsolated);
        be.write_bridge_port_flags(&spec.ifname, defaults).unwrap();
        let observed =
            readback_bridge_port_flags(&mut be, &spec.ifname, TapRoleW3::WorkloadLanIsolated)
                .unwrap();
        assert_eq!(observed, defaults);
    }
}
