use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Metadata, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Deserializer, Serialize};

use crate::runtime::RuntimeMetadata;

/// Linux interface name constrained to IFNAMSIZ-1 bytes and nixling's safe alphabet.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct IfName(String);

impl IfName {
    /// Creates a validated interface name (<=15 ASCII bytes, `[A-Za-z0-9_-]+`).
    pub fn new(value: impl Into<String>) -> Result<Self, IfNameError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IfNameError::Empty);
        }
        if value.len() > 15 {
            return Err(IfNameError::TooLong);
        }
        if !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(IfNameError::InvalidCharacter);
        }
        Ok(Self(value))
    }

    /// Returns the validated interface name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Validation failures for IfName.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IfNameError {
    /// Empty names cannot address a link.
    Empty,
    /// Linux interface names have at most fifteen visible bytes.
    TooLong,
    /// Only ASCII alphanumeric, underscore, and hyphen are accepted.
    InvalidCharacter,
}

impl std::fmt::Display for IfNameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "interface name must not be empty"),
            Self::TooLong => write!(f, "interface name must be at most 15 bytes"),
            Self::InvalidCharacter => write!(
                f,
                "interface name contains characters outside [A-Za-z0-9_-]"
            ),
        }
    }
}

impl std::error::Error for IfNameError {}

impl<'de> Deserialize<'de> for IfName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for IfName {
    fn schema_name() -> String {
        "IfName".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        let mut obj = SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
            string: Some(Box::new(StringValidation {
                max_length: Some(15),
                min_length: Some(1),
                pattern: Some("^[A-Za-z0-9_-]+$".to_owned()),
            })),
            ..Default::default()
        };
        obj.metadata = Some(Box::new(Metadata {
            description: Some(
                "Linux interface name: <=15 ASCII bytes matching [A-Za-z0-9_-]+".to_owned(),
            ),
            ..Default::default()
        }));
        Schema::Object(obj)
    }
}

/// Private host topology and host-owned capability contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostJson {
    /// Schema version used by this artifact.
    pub schema_version: String,
    /// Host-wide site policy toggles threaded from v0.4.0.
    pub site: SitePolicy,
    /// Per-environment network policy and derived host firewall inputs.
    pub environments: Vec<NetEnv>,
    /// Exact nftables `inet nixling` table declaration.
    pub nftables: NftablesModel,
    /// NetworkManager unmanaged file materialization rules.
    pub network_manager: NetworkManagerUnmanaged,
    /// `/etc/hosts` marked-block ownership rule.
    pub hosts_file: HostsFileOwnership,
    /// Kernel module requirements and gates.
    pub kernel_modules: Vec<KernelModulesEntry>,
    /// Broker-opened file descriptor ownership table.
    pub fd_ownership: Vec<FdOwnershipEntry>,
    /// Runtime/provider catalog advertised by this host bundle.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_providers: Vec<RuntimeMetadata>,
    /// Per-VM runtime/provider rows plus provider-neutral host topology.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vm_runtimes: Vec<VmRuntimeRow>,
    /// QEMU media runtime contract. Physical USB rows carry only opaque media
    /// refs and root-owned registry/rule locations; direct image-file rows may
    /// carry operator-authored absolute image paths from Nix config. Raw USB
    /// identities are never part of the Nix-store-backed bundle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qemu_media: Option<HostQemuMedia>,
    /// VMM/device capability matrix anchored to Cloud Hypervisor.
    pub cloud_hypervisor_capabilities: Vec<CloudHypervisorCapability>,
    /// Hash-derived IfName mapping exposure. One row per managed
    /// bridge/TAP gives the broker the user-visible →
    /// derived `<=15`-byte name pair and a stable role tag for the
    /// per-role bridge-port-flags table. Empty for V1-shaped bundles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub if_name_mappings: Vec<IfNameMapping>,
    /// Cloud Hypervisor net handoff probe result. Optional for backward
    /// compatibility with V1 host.json fixtures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ch: Option<HostChConfig>,
    /// W4a-H3: per-host firewall coexistence policy emitted by the Nix
    /// `host-json.nix` module. The broker reads this at runtime to
    /// decide whether `ApplyNftables` runs (Coexist), refuses
    /// (Refuse), or demands an explicit unmanaged drop-in
    /// (RequireUnmanaged). Optional for backward compatibility with
    /// pre-W4a host.json fixtures; the broker treats `None` as the
    /// implicit "no managed firewall detected" Coexist default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firewall_coexistence_policy: Option<crate::host_w3::FirewallCoexistencePolicy>,
}

/// Per-VM runtime row emitted in host.json for daemon lifecycle/status joins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmRuntimeRow {
    pub vm: String,
    pub runtime: RuntimeMetadata,
    pub env: Option<String>,
    pub state_dir: String,
    pub tap: String,
    pub bridge: Option<String>,
    pub static_ip: Option<String>,
    pub net_vm: Option<String>,
}

/// Host-side QEMU media registry/rule contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostQemuMedia {
    /// Root-only registry directory outside the Nix store.
    pub registry_dir: String,
    /// Runtime udev rule path. The broker writes this as a root-only artifact.
    pub runtime_rules_path: String,
    /// Closed-text reload description; no shell fragments or raw device ids.
    pub reload_behavior: String,
    /// Declared opaque media references.
    pub sources: Vec<QemuMediaSourceIntent>,
}

/// Opaque source intent for one QEMU media slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QemuMediaSourceIntent {
    pub vm: String,
    /// Opaque physical USB ref, or a generated stable source id for direct
    /// image-file rows that omit `ref` in Nix config.
    pub media_ref: String,
    pub slot: String,
    pub source_kind: QemuMediaSourceKind,
    pub format: QemuMediaFormat,
    pub read_only: bool,
    pub registry_scope: QemuMediaRegistryScope,
    /// Direct image-file path from operator-authored Nix config. Present only
    /// when `source_kind = image-file`; physical USB identity stays in the
    /// root-only runtime registry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum QemuMediaSourceKind {
    PhysicalUsb,
    ImageFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum QemuMediaFormat {
    Raw,
    Qcow2,
    Iso,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum QemuMediaRegistryScope {
    RootOnlyRuntimeState,
    DirectConfigPath,
}

/// Hash-derived IfName mapping row exposed in `host.json`. The broker's
/// `nixling_host::ifname::detect_collisions` re-validates uniqueness over
/// `derived_ifname` at runtime; the Nix
/// emitter rejects duplicates at bundle build with a deterministic
/// emitter error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IfNameMapping {
    /// Environment name this mapping belongs to.
    pub env: String,
    /// Optional VM name; absent for env-wide bridges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vm: Option<String>,
    /// TAP role for the bridge-port flag matrix.
    pub role: TapRole,
    /// User-visible interface name as it appears in `nixling.envs.*`
    /// and operator docs (e.g. `br-work-lan`).
    pub user_visible_name: String,
    /// Deterministic hash-derived IFNAMSIZ-safe interface name with
    /// `nl-` prefix (e.g. `nl-br-a1b2c3d4`). Bundle build refuses any
    /// collision.
    pub derived_ifname: IfName,
}

/// Cloud Hypervisor host-side configuration probed at bundle build.
/// Selects the runner network-handoff mode based on the packaged CH
/// binary's capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostChConfig {
    /// Net handoff mode selected for every CH runner on this host.
    pub net_handoff_mode: ChNetHandoffMode,
}

/// Cloud Hypervisor net handoff modes. `TapFd` keeps the long-lived
/// runner without `CAP_NET_ADMIN`; `PersistentTap` falls back to
/// pre-created TAPs handed off via `TUNSETOWNER`/`TUNSETGROUP`. The
/// emitter records the declared mode; the broker's host-check probe
/// validates it against the packaged CH binary at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ChNetHandoffMode {
    /// Preferred: broker opens TAP + `/dev/vhost-net` and passes fds.
    TapFd,
    /// Fallback: broker creates persistent TAP owned by the runner.
    PersistentTap,
}

/// Host-wide policy bits used when deriving per-env settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SitePolicy {
    pub allow_unsafe_east_west: bool,
}

/// Per-environment network contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetEnv {
    /// Environment name from the public manifest.
    pub env: String,
    /// Host bridge name for the environment LAN.
    pub bridge: IfName,
    /// Per-env MTU value applied to bridges and TAPs.
    pub mtu: u16,
    /// Optional MSS clamp applied to forwarded traffic.
    pub mss_clamp: Option<u16>,
    /// Per-env LAN policy including east-west intent.
    pub lan: LanPolicy,
    /// Forwarding blocklist derived from env config and host LAN CIDRs.
    pub net_vm_forward_blocklist: Vec<String>,
    /// TAP bridge-port flags by role.
    pub bridge_port_flags: Vec<BridgePortFlags>,
    /// Per-link IPv6-off sysctl contract.
    pub ipv6_sysctls: Vec<Ipv6SysctlEntry>,
    /// USBIP-capable VMs in this env; daemon owns the runtime per-busid locks.
    pub usbip_busid_locks: Vec<UsbipBusidLock>,
}

/// LAN policy inputs and effective result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LanPolicy {
    pub allow_east_west: bool,
    pub effective_east_west: bool,
}

/// Bridge-port flags applied to a TAP role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BridgePortFlags {
    pub role: TapRole,
    pub isolated: bool,
    pub neigh_suppress: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learning: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unicast_flood: Option<bool>,
    pub rule: String,
}

impl BridgePortFlags {
    pub fn resolved_learning(&self) -> bool {
        self.learning.unwrap_or(match self.role {
            TapRole::NetVmLan | TapRole::WorkloadLan => true,
            TapRole::Uplink => false,
        })
    }

    pub fn resolved_unicast_flood(&self) -> bool {
        self.unicast_flood.unwrap_or(match self.role {
            TapRole::NetVmLan => true,
            TapRole::WorkloadLan => !self.isolated,
            TapRole::Uplink => false,
        })
    }
}

/// TAP roles with distinct isolation behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TapRole {
    NetVmLan,
    WorkloadLan,
    Uplink,
}

/// IPv6-off sysctls applied per link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Ipv6SysctlEntry {
    pub if_name: IfName,
    pub disable_ipv6: u8,
    pub accept_ra: u8,
    pub autoconf: u8,
    pub addr_gen_mode: u8,
    #[serde(default = "default_arp_ignore")]
    pub arp_ignore: u8,
}

const fn default_arp_ignore() -> u8 {
    1
}

/// USBIP daemon-owned lock policy for one env.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipBusidLock {
    /// VM name allowed to request USBIP inside this env.
    pub vm: String,
    /// Ownership of the runtime exclusivity lock.
    pub lock_owner: UsbipLockOwner,
    /// Lock scope enforced by the daemon at runtime.
    pub scope: UsbipLockScope,
    /// Concrete USBIP busids allowed for this VM in this env.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bus_ids: Vec<String>,
    /// Optional vendor:product allowlist enforced before bind/attach.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vendor_product_allowlist: Vec<VendorProductPair>,
}

/// USBIP vendor:product allowlist entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VendorProductPair {
    pub vendor: u16,
    pub product: u16,
}

/// USBIP busid lock owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipLockOwner {
    Daemon,
}

/// USBIP exclusivity scope enforced by the daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipLockScope {
    PerBusid,
}

/// Exact nftables table, hook priorities, and chain layout owned by nixling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NftablesModel {
    pub family: String,
    pub table: String,
    pub chains: Vec<NftChain>,
    /// Drift-detection digest of the applied `inet nixling` table.
    /// `None` in the emitted bundle; the broker
    /// fills it in after `ApplyNftables` so pre-VM-start drift checks
    /// can compare against the previously applied digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table_hash_after_apply: Option<String>,
    /// Stable per-table comment marker installed on every rule
    /// (`comment "nixling managed: <ownership-id>"`). Empty
    /// for V1-shaped bundles.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ownership_id: String,
}

/// Single nftables chain declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NftChain {
    pub name: String,
    pub hook: Option<String>,
    pub priority: Option<i16>,
    pub policy: Option<String>,
    pub purpose: String,
}

/// NetworkManager unmanaged config ownership and reload behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkManagerUnmanaged {
    pub file_path: String,
    pub match_criteria: Vec<String>,
    pub reload_behavior: String,
    pub ownership: OwnershipRule,
}

/// File ownership policy for host materialized files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnershipRule {
    pub owner: String,
    pub group: String,
    pub mode: String,
    pub drift_policy: String,
}

/// `/etc/hosts` marked-block ownership contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostsFileOwnership {
    pub start_marker: String,
    pub end_marker: String,
    pub rule: String,
}

/// Kernel module requirement row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KernelModulesEntry {
    pub module: String,
    pub feature: String,
    pub requirement: ModuleRequirement,
    pub gate: String,
    pub sysctls: Vec<String>,
    pub jail_visible_device: bool,
}

/// Kernel module requirement class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ModuleRequirement {
    Required,
    Alternatives,
    Optional,
    Deferred,
}

/// Broker-opened fd ownership row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FdOwnershipEntry {
    pub resource: String,
    pub broker_operation: String,
    pub recipient: String,
    pub transfer: String,
    pub jail_visible_device: bool,
    pub notes: String,
}

/// Cloud Hypervisor capability matrix row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudHypervisorCapability {
    pub capability: String,
    pub status: CapabilityStatus,
    pub devices_or_modules: Vec<String>,
    pub sidecars: Vec<String>,
    pub readiness: Vec<String>,
    pub notes: String,
}

/// Capability support state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityStatus {
    Required,
    Optional,
    Deferred,
}

#[cfg(test)]
mod tests {
    use super::{
        BridgePortFlags, HostJson, IfName, IfNameError, Ipv6SysctlEntry, TapRole, UsbipBusidLock,
        UsbipLockOwner, UsbipLockScope, VendorProductPair,
    };

    #[test]
    fn if_name_accepts_safe_linux_names() {
        let name = IfName::new("nl-br_1").expect("valid name");
        assert_eq!(name.as_str(), "nl-br_1");
    }

    #[test]
    fn if_name_rejects_invalid_names() {
        assert_eq!(IfName::new(""), Err(IfNameError::Empty));
        assert_eq!(IfName::new("abcdefghijklmnop"), Err(IfNameError::TooLong));
        assert_eq!(IfName::new("bad.name"), Err(IfNameError::InvalidCharacter));
    }

    #[test]
    fn host_json_denies_unknown_fields() {
        let err = serde_json::from_str::<HostJson>(r#"{"schemaVersion":"v1","extra":true}"#)
            .expect_err("unknown fields fail closed");
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn bridge_port_flags_resolve_legacy_defaults() {
        let row = BridgePortFlags {
            role: TapRole::WorkloadLan,
            isolated: true,
            neigh_suppress: true,
            learning: None,
            unicast_flood: None,
            rule: "legacy".to_owned(),
        };
        assert!(row.resolved_learning());
        assert!(!row.resolved_unicast_flood());
    }

    #[test]
    fn ipv6_sysctl_entry_defaults_arp_ignore_to_one() {
        let entry = serde_json::from_str::<Ipv6SysctlEntry>(
            r#"{"ifName":"nl-bAAAA000","disableIpv6":1,"acceptRa":0,"autoconf":0,"addrGenMode":1}"#,
        )
        .expect("deserialize");
        assert_eq!(entry.arp_ignore, 1);
    }

    #[test]
    fn usbip_lock_round_trips_vendor_product_allowlist() {
        let lock = UsbipBusidLock {
            vm: "work-entra".to_owned(),
            lock_owner: UsbipLockOwner::Daemon,
            scope: UsbipLockScope::PerBusid,
            bus_ids: vec!["1-3".to_owned()],
            vendor_product_allowlist: vec![VendorProductPair {
                vendor: 0x1050,
                product: 0x0407,
            }],
        };

        let rendered = serde_json::to_string(&lock).expect("serialize usbip lock");
        let parsed = serde_json::from_str::<UsbipBusidLock>(&rendered).expect("parse usbip lock");

        assert_eq!(parsed, lock);
    }
}
