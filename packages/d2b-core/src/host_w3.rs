//! W3 host-prepare data transfer objects.
//!
//! These types are the wire-stable contract between the public manifest
//! (and Nix emitters under `nixos-modules/`) and the d2b-host
//! reconcile crate. They land in the W3 integrator API/contract prep
//! commit so the parallel scope agents s1-s5 consume a frozen DTO
//! surface and only write to their disjoint file scopes.
//!
//! Every type is `#[serde(deny_unknown_fields)]` (per AGENTS.md
//! "Manifest bundle" version policy for security-sensitive types) and
//! derives `JsonSchema` so the W3 schema regeneration (xtask
//! `gen-schemas`) picks them up once scope agents wire them into
//! `HostJson` / `ProcessesJson`.
//!
//! The integrator prep commit deliberately does NOT extend `HostJson`
//! with these fields yet. Scope agents (s2 owns `IfNameMapping`,
//! `BridgePortFlags`, `RouteIntent`, `SysctlIntent`, `HostsEntry`,
//! `NmUnmanagedEntry`, `FirewallCoexistencePolicy`; s4 owns
//! `KernelModuleEntry`) wire the optional fields in their commits,
//! together with the matching `nixos-modules/host-json.nix` and
//! `schemaVersion`/`bundleVersion` bumps. See plan.md §"W3 schema/
//! version bump rules".

use crate::host::IfName;
#[cfg(test)]
use crate::host::TapRole;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Mapping between an environment/VM name and the derived bridge/TAP
/// interface names. Scope s2 fills in the hash-derivation algorithm
/// (see plan.md §"W3 IPv6-off ordering" and the naming-conventions
/// ADR).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IfNameMapping {
    /// Environment name (matches the public manifest `env` field).
    pub env: String,
    /// Optional VM name; absent for env-wide bridges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vm: Option<String>,
    /// Derived bridge interface name (IFNAMSIZ-compliant).
    pub bridge: IfName,
    /// Derived TAP interface name; absent for bridge-only mappings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tap: Option<IfName>,
    /// Role this TAP plays in the bridge port flag matrix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<TapRoleW3>,
}

/// W3-extended TAP role taxonomy. The W2 [`crate::host::TapRole`] enum
/// (`NetVmLan`, `WorkloadLan`, `Uplink`) stays wire-stable; this
/// extended enum disambiguates east-west vs isolated workload TAPs
/// and uplink p2p variants for the W3 bridge port flag contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TapRoleW3 {
    /// Net-VM-facing LAN port.
    NetVmLan,
    /// Workload LAN port with east-west isolation enforced.
    WorkloadLanIsolated,
    /// Workload LAN port with intra-env east-west allowed.
    WorkloadLanEastWest,
    /// Uplink point-to-point port (router/firewall).
    UplinkP2P,
}

/// Bridge port flag row matching plan.md "W3 broker variant additions"
/// `SetBridgePortFlags` audit fields. Scope s2 fills in the reconcile
/// algorithm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BridgePortFlagsW3 {
    /// TAP role this row applies to.
    pub role: TapRoleW3,
    /// `IFF_BR_ISOLATED` desired state.
    pub isolated: bool,
    /// `IFLA_BRPORT_NEIGH_SUPPRESS` desired state.
    pub neigh_suppress: bool,
    /// `IFLA_BRPORT_LEARNING` desired state.
    pub learning: bool,
    /// `IFLA_BRPORT_UNICAST_FLOOD` desired state.
    pub unicast_flood: bool,
    /// Operator-facing rationale; appears in audit + docs.
    pub rule: String,
}

/// Kernel module matrix row owned by scope s4. Mirrors W2's
/// `KernelModulesEntry` shape with the W3 `matrix_entry_id` (used by
/// `ModprobeIfAllowed` audit) and the `modules_disabled` fail-closed
/// check exposed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KernelModuleEntry {
    /// Module name as passed to `modprobe`.
    pub module: String,
    /// Stable matrix entry identifier referenced from audit events.
    pub matrix_entry_id: String,
    /// Feature gate that activates the module requirement.
    pub feature: String,
    /// Required, optional, alternatives, or deferred.
    pub requirement: ModuleRequirementW3,
    /// `kernel.modules_disabled=1` host-check disposition.
    /// Required modules always fail closed; this flag only softens the
    /// outcome for optional / alternatives / deferred rows.
    pub fail_if_modules_disabled: bool,
}

/// Module requirement class for [`KernelModuleEntry`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ModuleRequirementW3 {
    Required,
    Optional,
    Alternatives,
    Deferred,
}

/// Single route reconcile intent for `ApplyRoute` (scope s2). Scope s2
/// fills in the fail-closed route preflight + idempotency oracle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteIntent {
    /// Destination CIDR (or `default`).
    pub destination: String,
    /// Optional via-gateway IP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    /// Optional egress device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<IfName>,
    /// Routing table identifier; defaults to `main`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    /// Whether this route is owned (and managed) by d2b.
    #[serde(default)]
    pub owned: bool,
}

/// Per-link or global sysctl reconcile intent for `ApplySysctl`
/// (scope s2). Plan.md §"W3 IPv6-off ordering" lists the exact set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SysctlIntent {
    /// Sysctl key in `dotted.path` form (e.g. `net.ipv6.conf.<if>.disable_ipv6`).
    pub key: String,
    /// Desired value as a string (sysctl values are text).
    pub value: String,
    /// Optional interface scope; absent means global.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_name: Option<IfName>,
}

/// `/etc/hosts` managed-block entry for `UpdateHostsFile` (scope s2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostsEntry {
    /// IP address (v4 or v6) the hostname resolves to.
    pub address: String,
    /// Hostname.
    pub hostname: String,
    /// Optional aliases.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

/// NetworkManager unmanaged entry for `ApplyNmUnmanaged` (scope s2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NmUnmanagedEntry {
    /// Interface name pattern matched in NM's unmanaged config.
    pub if_name: IfName,
    /// Stable identifier used by the broker audit + drop-in file name.
    pub marker_id: String,
}

/// Per-host firewall coexistence policy row emitted in host.json.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FirewallCoexistencePolicy {
    /// Detected firewall manager (or `None`/`Unknown`).
    pub manager: FirewallManager,
    /// What d2b does when this manager is detected.
    pub policy: CoexistencePolicy,
    /// Operator-facing rationale; appears in docs anchors.
    pub rationale: String,
}

/// Detected host firewall manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FirewallManager {
    Firewalld,
    Ufw,
    Docker,
    Libvirt,
    IptablesNft,
    Unknown,
    None,
}

/// What d2b does when a given firewall manager is present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CoexistencePolicy {
    /// Coexist: install `inet d2b` table alongside the manager's
    /// rules without flushing.
    Coexist,
    /// Refuse: fail-closed host check with a remediation message.
    Refuse,
    /// RequireUnmanaged: require an explicit "unmanaged" file from the
    /// manager covering d2b interfaces before proceeding.
    RequireUnmanaged,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ifname_mapping_round_trips_via_serde() {
        let mapping = IfNameMapping {
            env: "work".to_owned(),
            vm: Some("corp-vm".to_owned()),
            bridge: IfName::new("br-work-lan").expect("ifname"),
            tap: Some(IfName::new("work-l10").expect("ifname")),
            role: Some(TapRoleW3::WorkloadLanIsolated),
        };
        let json = serde_json::to_string(&mapping).expect("serialize");
        let decoded: IfNameMapping = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, mapping);
    }

    #[test]
    fn coexistence_policy_uses_kebab_case() {
        let json = serde_json::to_string(&CoexistencePolicy::RequireUnmanaged).expect("serialize");
        assert_eq!(json, "\"require-unmanaged\"");
    }

    #[test]
    fn bridge_port_flags_denies_unknown_fields() {
        let err = serde_json::from_str::<BridgePortFlagsW3>(
            r#"{"role":"net-vm-lan","isolated":true,"neighSuppress":false,"learning":true,"unicastFlood":true,"rule":"r","extra":1}"#,
        )
        .expect_err("unknown field rejected");
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn route_intent_round_trips_minimal() {
        let intent = RouteIntent {
            destination: "default".to_owned(),
            via: Some("10.0.0.1".to_owned()),
            device: None,
            table: None,
            owned: true,
        };
        let json = serde_json::to_string(&intent).expect("serialize");
        let decoded: RouteIntent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, intent);
    }

    #[test]
    fn sysctl_intent_round_trips_with_link_scope() {
        let intent = SysctlIntent {
            key: "net.ipv6.conf.br-work-lan.disable_ipv6".to_owned(),
            value: "1".to_owned(),
            if_name: Some(IfName::new("br-work-lan").expect("ifname")),
        };
        let json = serde_json::to_string(&intent).expect("serialize");
        let decoded: SysctlIntent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, intent);
    }

    /// Suppress dead-code warnings while [`TapRole`] from W2 is only
    /// referenced via the migration note above. Scope s2 replaces this
    /// with a real usage of [`TapRole`] in the IfNameMapping migration
    /// guide tests.
    #[test]
    fn legacy_tap_role_w2_still_constructible() {
        let _ = TapRole::WorkloadLan;
    }
}
