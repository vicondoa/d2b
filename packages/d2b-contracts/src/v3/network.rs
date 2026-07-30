//! Network primitive ResourceType base spec.
//!
//! `Network` is an independently shared network fabric. The CIDR, layer-2 and
//! layer-3, isolation, routing, DHCP and DNS, external-attachment, mDNS,
//! net-VM, and per-execution-target attachment fields are all Layer 2 base
//! fields; only genuinely implementation-only desired settings belong to the
//! Layer 3 `spec.provider` envelope on the universal `ResourceSpec`.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    ResourceRef,
    execution_policy::{
        BoundedToken, PrimitiveSpecError, parsed_deserialize, redacted_debug,
        require_execution_ref, string_schema,
    },
};

/// The canonical ResourceType name for this module.
pub const NETWORK_RESOURCE_TYPE: &str = "Network";
/// Maximum bytes in one CIDR string.
pub const MAX_CIDR_BYTES: usize = 43;
/// Lowest admitted per-execution-target attachment index.
pub const MIN_ATTACHMENT_INDEX: u8 = 2;
/// Highest admitted per-execution-target attachment index.
pub const MAX_ATTACHMENT_INDEX: u8 = 250;
/// Maximum per-execution-target attachments on one Network.
pub const MAX_NETWORK_ATTACHMENT_ENTRIES: usize = 64;
/// Maximum blocklist or allowlist CIDR entries on one Network.
pub const MAX_CIDR_LIST_ENTRIES: usize = 64;
/// Maximum DNS forwarders on one Network.
pub const MAX_DNS_FORWARDERS: usize = 8;
/// Maximum port forwards on one external attachment.
pub const MAX_PORT_FORWARDS: usize = 64;

/// A validated IPv4 CIDR in `a.b.c.d/prefix` form.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Ipv4Cidr(String);

impl Ipv4Cidr {
    /// Parse exactly one dotted-quad address and prefix length.
    pub fn parse(value: impl Into<String>) -> Result<Self, PrimitiveSpecError> {
        let value = value.into();
        let (address, prefix) = value
            .split_once('/')
            .ok_or(PrimitiveSpecError::InvalidText)?;
        parse_ipv4(address)?;
        if prefix.len() > 2 || (prefix.len() > 1 && prefix.starts_with('0')) {
            return Err(PrimitiveSpecError::InvalidText);
        }
        let prefix: u8 = prefix
            .parse()
            .map_err(|_| PrimitiveSpecError::InvalidText)?;
        if prefix > 32 {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        Ok(Self(value))
    }

    /// Borrow the canonical CIDR spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(Ipv4Cidr);
parsed_deserialize!(Ipv4Cidr);
string_schema!(Ipv4Cidr, 9, MAX_CIDR_BYTES);

/// A validated IPv4 address.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Ipv4Address(String);

impl Ipv4Address {
    /// Parse exactly one dotted-quad address.
    pub fn parse(value: impl Into<String>) -> Result<Self, PrimitiveSpecError> {
        let value = value.into();
        parse_ipv4(&value)?;
        Ok(Self(value))
    }

    /// Borrow the canonical address spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(Ipv4Address);
parsed_deserialize!(Ipv4Address);
string_schema!(Ipv4Address, 7, 15);

/// A validated unicast MAC address in colon notation.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct MacAddress(String);

impl MacAddress {
    /// Parse exactly one lower-case unicast MAC address.
    pub fn parse(value: impl Into<String>) -> Result<Self, PrimitiveSpecError> {
        let value = value.into();
        let octets: Vec<&str> = value.split(':').collect();
        if octets.len() != 6
            || octets.iter().any(|octet| {
                octet.len() != 2
                    || !octet
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            })
        {
            return Err(PrimitiveSpecError::InvalidText);
        }
        let first =
            u8::from_str_radix(octets[0], 16).map_err(|_| PrimitiveSpecError::InvalidText)?;
        if first & 1 == 1 {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        Ok(Self(value))
    }

    /// Borrow the canonical MAC spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(MacAddress);
parsed_deserialize!(MacAddress);
string_schema!(MacAddress, 17, 17);

/// Per-Network isolation policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IsolationSpec {
    /// The explicit per-Network east-west opt-in. No Zone-level gate applies.
    #[serde(default)]
    pub allow_east_west: bool,
}

/// Egress routing policy.
#[derive(Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoutingSpec {
    host_blocklist: Vec<Ipv4Cidr>,
}

impl RoutingSpec {
    /// Construct a routing policy after checking the list bound.
    pub fn new(host_blocklist: Vec<Ipv4Cidr>) -> Result<Self, PrimitiveSpecError> {
        if host_blocklist.len() > MAX_CIDR_LIST_ENTRIES {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        Ok(Self { host_blocklist })
    }

    /// Borrow the caller-supplied additive blocklist.
    pub fn host_blocklist(&self) -> &[Ipv4Cidr] {
        &self.host_blocklist
    }
}

redacted_debug!(RoutingSpec);

impl<'de> Deserialize<'de> for RoutingSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            host_blocklist: Vec<Ipv4Cidr>,
        }
        Self::new(Wire::deserialize(deserializer)?.host_blocklist).map_err(serde::de::Error::custom)
    }
}

/// DHCP settings for the LAN.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DhcpSpec {
    domain: Option<BoundedToken>,
    ignore_client_names: bool,
}

impl DhcpSpec {
    /// Construct DHCP settings.
    pub const fn new(domain: Option<BoundedToken>, ignore_client_names: bool) -> Self {
        Self {
            domain,
            ignore_client_names,
        }
    }

    /// Borrow the optional LAN domain name.
    pub const fn domain(&self) -> Option<&BoundedToken> {
        self.domain.as_ref()
    }

    /// Whether client-supplied host names are ignored.
    pub const fn ignore_client_names(&self) -> bool {
        self.ignore_client_names
    }
}

impl Default for DhcpSpec {
    fn default() -> Self {
        Self::new(None, true)
    }
}

redacted_debug!(DhcpSpec);

impl<'de> Deserialize<'de> for DhcpSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            domain: Option<BoundedToken>,
            #[serde(default = "yes")]
            ignore_client_names: bool,
        }
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self::new(wire.domain, wire.ignore_client_names))
    }
}

/// DNS settings for the LAN.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DnsSpec {
    forwarders: Vec<Ipv4Address>,
    cache_size: u32,
}

impl DnsSpec {
    /// Construct DNS settings after checking the forwarder bound.
    pub fn new(forwarders: Vec<Ipv4Address>, cache_size: u32) -> Result<Self, PrimitiveSpecError> {
        if forwarders.len() > MAX_DNS_FORWARDERS {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        Ok(Self {
            forwarders,
            cache_size,
        })
    }

    /// Borrow the upstream resolvers.
    pub fn forwarders(&self) -> &[Ipv4Address] {
        &self.forwarders
    }

    /// Return the resolver cache size.
    pub const fn cache_size(&self) -> u32 {
        self.cache_size
    }
}

impl Default for DnsSpec {
    fn default() -> Self {
        Self {
            forwarders: Vec::new(),
            cache_size: 1000,
        }
    }
}

redacted_debug!(DnsSpec);

impl<'de> Deserialize<'de> for DnsSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            forwarders: Vec<Ipv4Address>,
            #[serde(default = "thousand")]
            cache_size: u32,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.forwarders, wire.cache_size).map_err(serde::de::Error::custom)
    }
}

/// mDNS settings.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MdnsSpec {
    enable: bool,
    reflector: bool,
    dnsmasq_local: bool,
    dnsmasq_local_port: u16,
    publish_workstation: bool,
}

impl MdnsSpec {
    /// Construct mDNS settings after checking the local port.
    pub fn new(
        enable: bool,
        reflector: bool,
        dnsmasq_local: bool,
        dnsmasq_local_port: u16,
        publish_workstation: bool,
    ) -> Result<Self, PrimitiveSpecError> {
        if dnsmasq_local_port == 0 {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        Ok(Self {
            enable,
            reflector,
            dnsmasq_local,
            dnsmasq_local_port,
            publish_workstation,
        })
    }

    /// Whether mDNS is enabled.
    pub const fn enable(&self) -> bool {
        self.enable
    }
}

impl Default for MdnsSpec {
    fn default() -> Self {
        Self {
            enable: false,
            reflector: true,
            dnsmasq_local: false,
            dnsmasq_local_port: 53530,
            publish_workstation: false,
        }
    }
}

redacted_debug!(MdnsSpec);

impl<'de> Deserialize<'de> for MdnsSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            enable: bool,
            #[serde(default = "yes")]
            reflector: bool,
            #[serde(default)]
            dnsmasq_local: bool,
            #[serde(default = "mdns_port")]
            dnsmasq_local_port: u16,
            #[serde(default)]
            publish_workstation: bool,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.enable,
            wire.reflector,
            wire.dnsmasq_local,
            wire.dnsmasq_local_port,
            wire.publish_workstation,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// External attachment mode.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ExternalAttachmentMode {
    Macvtap,
}

/// macvtap operating mode.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum MacvtapMode {
    Bridge,
    Private,
    Vepa,
    Passthru,
}

/// How an external physical NIC claim is arbitrated.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum SharingPolicy {
    Exclusive,
    Multiplexed,
}

/// IPv4 address acquisition method.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Ipv4Method {
    Dhcp,
    Static,
}

/// External IPv4 configuration.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalIpv4Spec {
    method: Ipv4Method,
    address: Option<Ipv4Cidr>,
    gateway: Option<Ipv4Address>,
    dns: Vec<Ipv4Address>,
}

impl ExternalIpv4Spec {
    /// Construct IPv4 settings, requiring an address for the static method.
    pub fn new(
        method: Ipv4Method,
        address: Option<Ipv4Cidr>,
        gateway: Option<Ipv4Address>,
        dns: Vec<Ipv4Address>,
    ) -> Result<Self, PrimitiveSpecError> {
        match method {
            Ipv4Method::Static if address.is_none() => {
                return Err(PrimitiveSpecError::MissingRequiredField);
            }
            Ipv4Method::Dhcp if address.is_some() || gateway.is_some() || !dns.is_empty() => {
                return Err(PrimitiveSpecError::ConflictingFields);
            }
            _ => {}
        }
        if dns.len() > MAX_DNS_FORWARDERS {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        Ok(Self {
            method,
            address,
            gateway,
            dns,
        })
    }

    /// Return the acquisition method.
    pub const fn method(&self) -> Ipv4Method {
        self.method
    }
}

impl Default for ExternalIpv4Spec {
    fn default() -> Self {
        Self {
            method: Ipv4Method::Dhcp,
            address: None,
            gateway: None,
            dns: Vec::new(),
        }
    }
}

redacted_debug!(ExternalIpv4Spec);

impl<'de> Deserialize<'de> for ExternalIpv4Spec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default = "dhcp")]
            method: Ipv4Method,
            #[serde(default)]
            address: Option<Ipv4Cidr>,
            #[serde(default)]
            gateway: Option<Ipv4Address>,
            #[serde(default)]
            dns: Vec<Ipv4Address>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.method, wire.address, wire.gateway, wire.dns)
            .map_err(serde::de::Error::custom)
    }
}

/// External egress policy.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EgressSpec {
    enable: bool,
    allowed_cidrs: Vec<Ipv4Cidr>,
    masquerade: bool,
}

impl EgressSpec {
    /// Construct an egress policy after checking the allowlist bound.
    pub fn new(
        enable: bool,
        allowed_cidrs: Vec<Ipv4Cidr>,
        masquerade: bool,
    ) -> Result<Self, PrimitiveSpecError> {
        if allowed_cidrs.len() > MAX_CIDR_LIST_ENTRIES {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        Ok(Self {
            enable,
            allowed_cidrs,
            masquerade,
        })
    }

    /// Whether external egress is enabled.
    pub const fn enable(&self) -> bool {
        self.enable
    }
}

impl Default for EgressSpec {
    fn default() -> Self {
        Self {
            enable: false,
            allowed_cidrs: Vec::new(),
            masquerade: true,
        }
    }
}

redacted_debug!(EgressSpec);

impl<'de> Deserialize<'de> for EgressSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            enable: bool,
            #[serde(default)]
            allowed_cidrs: Vec<Ipv4Cidr>,
            #[serde(default = "yes")]
            masquerade: bool,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.enable, wire.allowed_cidrs, wire.masquerade)
            .map_err(serde::de::Error::custom)
    }
}

/// Forwarded-port transport protocol.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ForwardProtocol {
    Tcp,
    Udp,
}

/// One inbound port forward on the external interface.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PortForwardSpec {
    protocol: ForwardProtocol,
    listen_port: u16,
    target_ref: Option<ResourceRef>,
    target_ip: Option<Ipv4Address>,
    target_port: u16,
    source_cidrs: Vec<Ipv4Cidr>,
}

impl PortForwardSpec {
    /// Construct a port forward, requiring exactly one target selector.
    pub fn new(
        protocol: ForwardProtocol,
        listen_port: u16,
        target_ref: Option<ResourceRef>,
        target_ip: Option<Ipv4Address>,
        target_port: u16,
        source_cidrs: Vec<Ipv4Cidr>,
    ) -> Result<Self, PrimitiveSpecError> {
        if listen_port == 0 || target_port == 0 {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        match (&target_ref, &target_ip) {
            (None, None) => return Err(PrimitiveSpecError::MissingRequiredField),
            (Some(_), Some(_)) => return Err(PrimitiveSpecError::ConflictingFields),
            (Some(reference), None) => require_execution_ref(reference)?,
            (None, Some(_)) => {}
        }
        if source_cidrs.len() > MAX_CIDR_LIST_ENTRIES {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        Ok(Self {
            protocol,
            listen_port,
            target_ref,
            target_ip,
            target_port,
            source_cidrs,
        })
    }

    /// Return the forwarded protocol.
    pub const fn protocol(&self) -> ForwardProtocol {
        self.protocol
    }

    /// Return the external listen port.
    pub const fn listen_port(&self) -> u16 {
        self.listen_port
    }
}

redacted_debug!(PortForwardSpec);

impl<'de> Deserialize<'de> for PortForwardSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            protocol: ForwardProtocol,
            listen_port: u16,
            #[serde(default)]
            target_ref: Option<ResourceRef>,
            #[serde(default)]
            target_ip: Option<Ipv4Address>,
            target_port: u16,
            #[serde(default)]
            source_cidrs: Vec<Ipv4Cidr>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.protocol,
            wire.listen_port,
            wire.target_ref,
            wire.target_ip,
            wire.target_port,
            wire.source_cidrs,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// The optional external physical-NIC attachment.
///
/// `parentInterface` is a requested host inventory selector, not the
/// authority identity: core resolves it against trusted Host inventory and
/// derives a non-reversible identity that never appears in a spec.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAttachmentSpec {
    mode: ExternalAttachmentMode,
    parent_interface: BoundedToken,
    macvtap_mode: MacvtapMode,
    sharing_policy: SharingPolicy,
    mac: Option<MacAddress>,
    ipv4: ExternalIpv4Spec,
    egress: EgressSpec,
    port_forwards: Vec<PortForwardSpec>,
}

impl ExternalAttachmentSpec {
    /// Construct an external attachment.
    ///
    /// Multiplexed arbitration is admitted only for `bridge`; every other
    /// macvtap mode is always exclusive.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mode: ExternalAttachmentMode,
        parent_interface: BoundedToken,
        macvtap_mode: MacvtapMode,
        sharing_policy: SharingPolicy,
        mac: Option<MacAddress>,
        ipv4: ExternalIpv4Spec,
        egress: EgressSpec,
        port_forwards: Vec<PortForwardSpec>,
    ) -> Result<Self, PrimitiveSpecError> {
        if sharing_policy == SharingPolicy::Multiplexed && macvtap_mode != MacvtapMode::Bridge {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        if port_forwards.len() > MAX_PORT_FORWARDS {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        Ok(Self {
            mode,
            parent_interface,
            macvtap_mode,
            sharing_policy,
            mac,
            ipv4,
            egress,
            port_forwards,
        })
    }

    /// Return the macvtap operating mode.
    pub const fn macvtap_mode(&self) -> MacvtapMode {
        self.macvtap_mode
    }

    /// Return the arbitration policy.
    pub const fn sharing_policy(&self) -> SharingPolicy {
        self.sharing_policy
    }
}

redacted_debug!(ExternalAttachmentSpec);

impl<'de> Deserialize<'de> for ExternalAttachmentSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default = "macvtap")]
            mode: ExternalAttachmentMode,
            parent_interface: BoundedToken,
            #[serde(default = "bridge")]
            macvtap_mode: MacvtapMode,
            #[serde(default = "exclusive")]
            sharing_policy: SharingPolicy,
            #[serde(default)]
            mac: Option<MacAddress>,
            #[serde(default)]
            ipv4: ExternalIpv4Spec,
            #[serde(default)]
            egress: EgressSpec,
            #[serde(default)]
            port_forwards: Vec<PortForwardSpec>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.mode,
            wire.parent_interface,
            wire.macvtap_mode,
            wire.sharing_policy,
            wire.mac,
            wire.ipv4,
            wire.egress,
            wire.port_forwards,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// One reserved LAN address and MAC for a Host or Guest.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkAttachmentEntry {
    execution_ref: ResourceRef,
    index: u8,
    mac: Option<MacAddress>,
}

impl NetworkAttachmentEntry {
    /// Construct an attachment entry after checking the reserved index range.
    ///
    /// Index 1 is reserved for the net VM's LAN interface.
    pub fn new(
        execution_ref: ResourceRef,
        index: u8,
        mac: Option<MacAddress>,
    ) -> Result<Self, PrimitiveSpecError> {
        require_execution_ref(&execution_ref)?;
        if !(MIN_ATTACHMENT_INDEX..=MAX_ATTACHMENT_INDEX).contains(&index) {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        Ok(Self {
            execution_ref,
            index,
            mac,
        })
    }

    /// Borrow the attached Host or Guest.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Return the stable LAN index.
    pub const fn index(&self) -> u8 {
        self.index
    }
}

redacted_debug!(NetworkAttachmentEntry);

impl<'de> Deserialize<'de> for NetworkAttachmentEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            execution_ref: ResourceRef,
            index: u8,
            #[serde(default)]
            mac: Option<MacAddress>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.execution_ref, wire.index, wire.mac).map_err(serde::de::Error::custom)
    }
}

/// The Network ResourceType base spec.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkSpec {
    lan_cidr: Ipv4Cidr,
    uplink_cidr: Ipv4Cidr,
    mtu: Option<u32>,
    mss_clamp: bool,
    isolation: IsolationSpec,
    routing: RoutingSpec,
    dhcp: DhcpSpec,
    dns: DnsSpec,
    external_attachment: Option<ExternalAttachmentSpec>,
    mdns: MdnsSpec,
    net_vm_name_override: Option<BoundedToken>,
    net_vm_system_artifact_id: BoundedToken,
    attachments: Vec<NetworkAttachmentEntry>,
}

impl NetworkSpec {
    /// Construct a Network base spec after checking every frozen bound.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        lan_cidr: Ipv4Cidr,
        uplink_cidr: Ipv4Cidr,
        mtu: Option<u32>,
        mss_clamp: bool,
        isolation: IsolationSpec,
        routing: RoutingSpec,
        dhcp: DhcpSpec,
        dns: DnsSpec,
        external_attachment: Option<ExternalAttachmentSpec>,
        mdns: MdnsSpec,
        net_vm_name_override: Option<BoundedToken>,
        net_vm_system_artifact_id: BoundedToken,
        attachments: Vec<NetworkAttachmentEntry>,
    ) -> Result<Self, PrimitiveSpecError> {
        if lan_cidr == uplink_cidr {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        if let Some(mtu) = mtu
            && !(576..=9216).contains(&mtu)
        {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        if attachments.len() > MAX_NETWORK_ATTACHMENT_ENTRIES {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        let mut indexes: Vec<u8> = attachments.iter().map(|entry| entry.index).collect();
        let declared = indexes.len();
        indexes.sort_unstable();
        indexes.dedup();
        if indexes.len() != declared {
            return Err(PrimitiveSpecError::DuplicateEntry);
        }
        Ok(Self {
            lan_cidr,
            uplink_cidr,
            mtu,
            mss_clamp,
            isolation,
            routing,
            dhcp,
            dns,
            external_attachment,
            mdns,
            net_vm_name_override,
            net_vm_system_artifact_id,
            attachments,
        })
    }

    /// Construct the canonical minimal Network base spec.
    pub fn minimal(
        lan_cidr: Ipv4Cidr,
        uplink_cidr: Ipv4Cidr,
        net_vm_system_artifact_id: BoundedToken,
    ) -> Result<Self, PrimitiveSpecError> {
        Self::new(
            lan_cidr,
            uplink_cidr,
            None,
            false,
            IsolationSpec::default(),
            RoutingSpec::default(),
            DhcpSpec::default(),
            DnsSpec::default(),
            None,
            MdnsSpec::default(),
            None,
            net_vm_system_artifact_id,
            Vec::new(),
        )
    }

    /// Borrow the LAN CIDR.
    pub const fn lan_cidr(&self) -> &Ipv4Cidr {
        &self.lan_cidr
    }

    /// Borrow the uplink CIDR.
    pub const fn uplink_cidr(&self) -> &Ipv4Cidr {
        &self.uplink_cidr
    }

    /// Return the isolation policy.
    pub const fn isolation(&self) -> IsolationSpec {
        self.isolation
    }

    /// Borrow the external attachment.
    pub const fn external_attachment(&self) -> Option<&ExternalAttachmentSpec> {
        self.external_attachment.as_ref()
    }

    /// Borrow the required net-VM system artifact ID.
    pub const fn net_vm_system_artifact_id(&self) -> &BoundedToken {
        &self.net_vm_system_artifact_id
    }

    /// Borrow the reserved per-execution-target attachments.
    pub fn attachments(&self) -> &[NetworkAttachmentEntry] {
        &self.attachments
    }
}

redacted_debug!(NetworkSpec);

impl<'de> Deserialize<'de> for NetworkSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            lan_cidr: Ipv4Cidr,
            uplink_cidr: Ipv4Cidr,
            #[serde(default)]
            mtu: Option<u32>,
            #[serde(default)]
            mss_clamp: bool,
            #[serde(default)]
            isolation: IsolationSpec,
            #[serde(default)]
            routing: RoutingSpec,
            #[serde(default)]
            dhcp: DhcpSpec,
            #[serde(default)]
            dns: DnsSpec,
            #[serde(default)]
            external_attachment: Option<ExternalAttachmentSpec>,
            #[serde(default)]
            mdns: MdnsSpec,
            #[serde(default)]
            net_vm_name_override: Option<BoundedToken>,
            net_vm_system_artifact_id: BoundedToken,
            #[serde(default)]
            attachments: Vec<NetworkAttachmentEntry>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.lan_cidr,
            wire.uplink_cidr,
            wire.mtu,
            wire.mss_clamp,
            wire.isolation,
            wire.routing,
            wire.dhcp,
            wire.dns,
            wire.external_attachment,
            wire.mdns,
            wire.net_vm_name_override,
            wire.net_vm_system_artifact_id,
            wire.attachments,
        )
        .map_err(serde::de::Error::custom)
    }
}

fn parse_ipv4(value: &str) -> Result<[u8; 4], PrimitiveSpecError> {
    let mut octets = [0u8; 4];
    let mut seen = 0usize;
    for (slot, text) in octets.iter_mut().zip(value.split('.')) {
        if text.is_empty()
            || text.len() > 3
            || !text.bytes().all(|byte| byte.is_ascii_digit())
            || (text.len() > 1 && text.starts_with('0'))
        {
            return Err(PrimitiveSpecError::InvalidText);
        }
        *slot = text.parse().map_err(|_| PrimitiveSpecError::InvalidText)?;
        seen += 1;
    }
    if seen != 4 || value.split('.').count() != 4 {
        return Err(PrimitiveSpecError::InvalidText);
    }
    Ok(octets)
}

const fn yes() -> bool {
    true
}

const fn thousand() -> u32 {
    1000
}

const fn mdns_port() -> u16 {
    53530
}

const fn dhcp() -> Ipv4Method {
    Ipv4Method::Dhcp
}

const fn macvtap() -> ExternalAttachmentMode {
    ExternalAttachmentMode::Macvtap
}

const fn bridge() -> MacvtapMode {
    MacvtapMode::Bridge
}

const fn exclusive() -> SharingPolicy {
    SharingPolicy::Exclusive
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::{execution_policy::to_base_object, resource_schema::canonical_json_bytes};

    fn minimal_network() -> NetworkSpec {
        NetworkSpec::minimal(
            Ipv4Cidr::parse("10.20.0.0/24").unwrap(),
            Ipv4Cidr::parse("192.0.2.0/30").unwrap(),
            BoundedToken::parse("net-vm-system").unwrap(),
        )
        .unwrap()
    }

    const MINIMAL_NETWORK_SPEC: &[u8] = br#"{"attachments":[],"dhcp":{"domain":null,"ignoreClientNames":true},"dns":{"cacheSize":1000,"forwarders":[]},"externalAttachment":null,"isolation":{"allowEastWest":false},"lanCidr":"10.20.0.0/24","mdns":{"dnsmasqLocal":false,"dnsmasqLocalPort":53530,"enable":false,"publishWorkstation":false,"reflector":true},"mssClamp":false,"mtu":null,"netVmNameOverride":null,"netVmSystemArtifactId":"net-vm-system","routing":{"hostBlocklist":[]},"uplinkCidr":"192.0.2.0/30"}"#;

    #[test]
    fn schema_vector_pins_the_minimal_network_base_spec() {
        let spec = minimal_network();
        assert_eq!(canonical_json_bytes(&spec).unwrap(), MINIMAL_NETWORK_SPEC);
        let parsed: NetworkSpec = serde_json::from_slice(MINIMAL_NETWORK_SPEC).unwrap();
        assert_eq!(parsed, spec);
        let base = to_base_object(&spec).unwrap();
        for reserved in ["providerRef", "updatePolicy", "provider"] {
            assert!(base.get(reserved).is_none());
        }
    }

    #[test]
    fn the_net_vm_system_artifact_is_required_with_no_implicit_default() {
        assert!(
            serde_json::from_slice::<NetworkSpec>(
                br#"{"lanCidr":"10.20.0.0/24","uplinkCidr":"192.0.2.0/30"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn east_west_is_an_explicit_per_network_opt_in() {
        let spec = minimal_network();
        assert!(!spec.isolation().allow_east_west);
        let opted: NetworkSpec = serde_json::from_slice(
            br#"{"lanCidr":"10.20.0.0/24","uplinkCidr":"192.0.2.0/30","netVmSystemArtifactId":"a","isolation":{"allowEastWest":true}}"#,
        )
        .unwrap();
        assert!(opted.isolation().allow_east_west);
    }

    #[test]
    fn attachment_indexes_are_bounded_and_unique() {
        assert_eq!(
            NetworkAttachmentEntry::new(ResourceRef::parse("Guest/corp-vm").unwrap(), 1, None),
            Err(PrimitiveSpecError::OutOfRange)
        );
        assert_eq!(
            NetworkAttachmentEntry::new(ResourceRef::parse("Guest/corp-vm").unwrap(), 251, None),
            Err(PrimitiveSpecError::OutOfRange)
        );
        assert_eq!(
            NetworkAttachmentEntry::new(ResourceRef::parse("Volume/corp-vm").unwrap(), 10, None),
            Err(PrimitiveSpecError::WrongResourceType)
        );
        let entry = |index| {
            NetworkAttachmentEntry::new(ResourceRef::parse("Guest/corp-vm").unwrap(), index, None)
                .unwrap()
        };
        assert_eq!(
            NetworkSpec::new(
                Ipv4Cidr::parse("10.20.0.0/24").unwrap(),
                Ipv4Cidr::parse("192.0.2.0/30").unwrap(),
                None,
                false,
                IsolationSpec::default(),
                RoutingSpec::default(),
                DhcpSpec::default(),
                DnsSpec::default(),
                None,
                MdnsSpec::default(),
                None,
                BoundedToken::parse("a").unwrap(),
                vec![entry(10), entry(10)],
            ),
            Err(PrimitiveSpecError::DuplicateEntry)
        );
    }

    #[test]
    fn multiplexed_arbitration_is_admitted_only_for_bridge_mode() {
        for mode in [
            MacvtapMode::Private,
            MacvtapMode::Vepa,
            MacvtapMode::Passthru,
        ] {
            assert_eq!(
                ExternalAttachmentSpec::new(
                    ExternalAttachmentMode::Macvtap,
                    BoundedToken::parse("eno1").unwrap(),
                    mode,
                    SharingPolicy::Multiplexed,
                    None,
                    ExternalIpv4Spec::default(),
                    EgressSpec::default(),
                    Vec::new(),
                ),
                Err(PrimitiveSpecError::ConflictingFields)
            );
        }
        assert!(
            ExternalAttachmentSpec::new(
                ExternalAttachmentMode::Macvtap,
                BoundedToken::parse("eno1").unwrap(),
                MacvtapMode::Bridge,
                SharingPolicy::Multiplexed,
                None,
                ExternalIpv4Spec::default(),
                EgressSpec::default(),
                Vec::new(),
            )
            .is_ok()
        );
    }

    #[test]
    fn a_port_forward_names_exactly_one_target() {
        assert_eq!(
            PortForwardSpec::new(ForwardProtocol::Tcp, 2222, None, None, 22, Vec::new()),
            Err(PrimitiveSpecError::MissingRequiredField)
        );
        assert_eq!(
            PortForwardSpec::new(
                ForwardProtocol::Tcp,
                2222,
                Some(ResourceRef::parse("Guest/corp-vm").unwrap()),
                Some(Ipv4Address::parse("10.20.0.10").unwrap()),
                22,
                Vec::new(),
            ),
            Err(PrimitiveSpecError::ConflictingFields)
        );
        assert!(
            PortForwardSpec::new(
                ForwardProtocol::Tcp,
                2222,
                Some(ResourceRef::parse("Guest/corp-vm").unwrap()),
                None,
                22,
                Vec::new(),
            )
            .is_ok()
        );
    }

    #[test]
    fn address_scalars_reject_malformed_and_multicast_values() {
        assert!(Ipv4Cidr::parse("10.20.0.0").is_err());
        assert!(Ipv4Cidr::parse("10.20.0.0/33").is_err());
        assert!(Ipv4Cidr::parse("10.20.0.256/24").is_err());
        assert!(Ipv4Address::parse("10.20.0.01").is_err());
        assert!(MacAddress::parse("52:54:00:12:34:56").is_ok());
        assert_eq!(
            MacAddress::parse("53:54:00:12:34:56"),
            Err(PrimitiveSpecError::ConflictingFields)
        );
        assert!(MacAddress::parse("52-54-00-12-34-56").is_err());
    }

    #[test]
    fn diagnostics_stay_redacted() {
        let spec = minimal_network();
        assert_eq!(format!("{spec:?}"), "NetworkSpec(<redacted>)");
        assert!(!format!("{:?}", spec.lan_cidr()).contains("10.20"));
    }
}
