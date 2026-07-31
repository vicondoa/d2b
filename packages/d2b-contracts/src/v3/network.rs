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
    ConditionState, ManagedBy, ResourceGeneration, ResourcePhase, ResourceRef, ResourceUid,
    UpdateState,
    execution_policy::{
        BoundedToken, PrimitiveSpecError, parsed_deserialize, redacted_debug,
        require_execution_ref, string_schema,
    },
    ifname::IfName,
    user::{OsUsername, UserSpec},
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
/// Mandatory host-network ranges that an authored blocklist can only extend.
pub const DEFAULT_HOST_BLOCKLIST: [&str; 4] = [
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "169.254.0.0/16",
];
/// Canonical reserved User resource name for the Network controller account.
pub const NET_LOCAL_CONTROLLER_USER_NAME: &str = "net-local-controller";
/// Canonical OS account resolved for the Network controller User resource.
pub const NET_LOCAL_CONTROLLER_OS_USERNAME: &str = "net-local-controller";
/// Provider that owns the reserved Network controller User resource.
pub const NET_LOCAL_CONTROLLER_OWNER_REF: &str = "Provider/network-local";
/// Provider that verifies the reserved account through NSS.
pub const NET_LOCAL_CONTROLLER_VERIFIER_REF: &str = "Provider/system-core";

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

    fn address_and_prefix(&self) -> ([u8; 4], u8) {
        let (address, prefix) = self
            .0
            .split_once('/')
            .expect("validated CIDR contains a separator");
        let address = parse_ipv4(address).expect("validated CIDR contains an IPv4 address");
        let prefix = prefix
            .parse()
            .expect("validated CIDR contains a numeric prefix");
        (address, prefix)
    }

    /// Return the validated prefix length.
    pub fn prefix_len(&self) -> u8 {
        self.address_and_prefix().1
    }

    /// Return whether the address is the network base for its prefix.
    pub fn is_network_base(&self) -> bool {
        let (address, prefix) = self.address_and_prefix();
        let address = u32::from_be_bytes(address);
        address & prefix_mask(prefix) == address
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
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoutingSpec {
    host_blocklist: Vec<Ipv4Cidr>,
}

impl RoutingSpec {
    /// Construct a routing policy that contains every mandatory default.
    pub fn new(mut host_blocklist: Vec<Ipv4Cidr>) -> Result<Self, PrimitiveSpecError> {
        if host_blocklist.len() > MAX_CIDR_LIST_ENTRIES {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        for required in DEFAULT_HOST_BLOCKLIST {
            let required = Ipv4Cidr::parse(required).expect("default CIDRs are valid");
            if !host_blocklist.contains(&required) {
                return Err(PrimitiveSpecError::ConflictingFields);
            }
        }
        host_blocklist.sort();
        host_blocklist.dedup();
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

impl Default for RoutingSpec {
    fn default() -> Self {
        Self::new(
            DEFAULT_HOST_BLOCKLIST
                .into_iter()
                .map(|cidr| Ipv4Cidr::parse(cidr).expect("default CIDRs are valid"))
                .collect(),
        )
        .expect("mandatory routing defaults fit the list bound")
    }
}

impl<'de> Deserialize<'de> for RoutingSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            host_blocklist: Option<Vec<Ipv4Cidr>>,
        }
        match Wire::deserialize(deserializer)?.host_blocklist {
            Some(host_blocklist) => Self::new(host_blocklist),
            None => Ok(Self::default()),
        }
        .map_err(serde::de::Error::custom)
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
    /// Construct IPv4 settings, requiring an address and gateway for static.
    pub fn new(
        method: Ipv4Method,
        address: Option<Ipv4Cidr>,
        gateway: Option<Ipv4Address>,
        dns: Vec<Ipv4Address>,
    ) -> Result<Self, PrimitiveSpecError> {
        match method {
            Ipv4Method::Static if address.is_none() || gateway.is_none() => {
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
    parent_interface: IfName,
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
        parent_interface: IfName,
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
            parent_interface: IfName,
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

/// Canonical authored Network attachment spec.
pub type AttachmentSpec = NetworkAttachmentEntry;

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
        if lan_cidr.prefix_len() != 24 || !lan_cidr.is_network_base() {
            return Err(PrimitiveSpecError::InvalidText);
        }
        if uplink_cidr.prefix_len() != 30 || !uplink_cidr.is_network_base() {
            return Err(PrimitiveSpecError::InvalidText);
        }
        if cidr_overlaps(&lan_cidr, &uplink_cidr) {
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
        if net_vm_name_override
            .as_ref()
            .is_some_and(|name| name.as_str() == "launcher" || name.as_str().starts_with("sys-"))
        {
            return Err(PrimitiveSpecError::ConflictingFields);
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

/// Closed Network condition types written by the Network controller.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum NetworkConditionType {
    FabricReady,
    NetVmReady,
    DhcpReady,
    FirewallReady,
    CidrConflict,
    ExternalNicAuthorityReady,
    ExternalAttachmentReady,
    ReconcileError,
    ConfigVolumeReady,
    NetworkDraining,
}

/// Provider-neutral phase of a Network child or attachment projection.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum NetworkComponentPhase {
    Pending,
    Ready,
    Degraded,
    Unknown,
}

/// Public readiness of one host bridge, without its kernel interface name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkFabricStatus {
    phase: NetworkComponentPhase,
}

impl NetworkFabricStatus {
    /// Construct one bridge readiness projection.
    pub const fn new(phase: NetworkComponentPhase) -> Self {
        Self { phase }
    }

    /// Return the projected bridge phase.
    pub const fn phase(self) -> NetworkComponentPhase {
        self.phase
    }
}

/// Public readiness of one Host or Guest attachment.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentStatus {
    execution_ref: ResourceRef,
    phase: NetworkComponentPhase,
}

impl AttachmentStatus {
    /// Construct one attachment status after checking its execution reference.
    pub fn new(
        execution_ref: ResourceRef,
        phase: NetworkComponentPhase,
    ) -> Result<Self, PrimitiveSpecError> {
        require_execution_ref(&execution_ref)?;
        Ok(Self {
            execution_ref,
            phase,
        })
    }

    /// Borrow the attached Host or Guest reference.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Return the public attachment phase.
    pub const fn phase(&self) -> NetworkComponentPhase {
        self.phase
    }
}

redacted_debug!(AttachmentStatus);

impl<'de> Deserialize<'de> for AttachmentStatus {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            execution_ref: ResourceRef,
            phase: NetworkComponentPhase,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.execution_ref, wire.phase).map_err(serde::de::Error::custom)
    }
}

/// Bounded public observation of one external physical-NIC authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalNicAuthorityStatus {
    available: bool,
    holder_count: u32,
    queue_depth: u32,
    arbitration: SharingPolicy,
    update_currency: UpdateState,
}

impl ExternalNicAuthorityStatus {
    /// Construct a path-free, identity-free authority observation.
    pub const fn new(
        available: bool,
        holder_count: u32,
        queue_depth: u32,
        arbitration: SharingPolicy,
        update_currency: UpdateState,
    ) -> Self {
        Self {
            available,
            holder_count,
            queue_depth,
            arbitration,
            update_currency,
        }
    }

    /// Whether another compatible holder may currently be admitted.
    pub const fn available(self) -> bool {
        self.available
    }

    /// Return the bounded number of current holders.
    pub const fn holder_count(self) -> u32 {
        self.holder_count
    }

    /// Return the bounded number of queued claimants.
    pub const fn queue_depth(self) -> u32 {
        self.queue_depth
    }

    /// Return the active arbitration policy.
    pub const fn arbitration(self) -> SharingPolicy {
        self.arbitration
    }

    /// Return the authority realization's update currency.
    pub const fn update_currency(self) -> UpdateState {
        self.update_currency
    }
}

/// Provider-neutral external attachment observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalAttachmentStatus {
    phase: NetworkComponentPhase,
    authority: ExternalNicAuthorityStatus,
}

impl ExternalAttachmentStatus {
    /// Construct a status that cannot carry a NIC selector or authority key.
    pub const fn new(phase: NetworkComponentPhase, authority: ExternalNicAuthorityStatus) -> Self {
        Self { phase, authority }
    }
}

/// ResourceType-common Network status layer.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkStatus {
    net_vm_ref: ResourceRef,
    lan_bridge: NetworkFabricStatus,
    uplink_bridge: NetworkFabricStatus,
    external_attachment: Option<ExternalAttachmentStatus>,
    attachments: Vec<AttachmentStatus>,
}

impl NetworkStatus {
    /// Construct the common status layer without runtime interface identities.
    pub fn new(
        net_vm_ref: ResourceRef,
        lan_bridge: NetworkFabricStatus,
        uplink_bridge: NetworkFabricStatus,
        external_attachment: Option<ExternalAttachmentStatus>,
        mut attachments: Vec<AttachmentStatus>,
    ) -> Result<Self, PrimitiveSpecError> {
        if net_vm_ref.resource_type().as_str() != "Guest" {
            return Err(PrimitiveSpecError::WrongResourceType);
        }
        if attachments.len() > MAX_NETWORK_ATTACHMENT_ENTRIES {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        attachments.sort_by(|left, right| left.execution_ref.cmp(&right.execution_ref));
        if attachments
            .windows(2)
            .any(|pair| pair[0].execution_ref == pair[1].execution_ref)
        {
            return Err(PrimitiveSpecError::DuplicateEntry);
        }
        Ok(Self {
            net_vm_ref,
            lan_bridge,
            uplink_bridge,
            external_attachment,
            attachments,
        })
    }

    /// Borrow the owned net-VM Guest reference.
    pub const fn net_vm_ref(&self) -> &ResourceRef {
        &self.net_vm_ref
    }

    /// Borrow the bounded attachment projections.
    pub fn attachments(&self) -> &[AttachmentStatus] {
        &self.attachments
    }
}

redacted_debug!(NetworkStatus);

impl<'de> Deserialize<'de> for NetworkStatus {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            net_vm_ref: ResourceRef,
            lan_bridge: NetworkFabricStatus,
            uplink_bridge: NetworkFabricStatus,
            external_attachment: Option<ExternalAttachmentStatus>,
            attachments: Vec<AttachmentStatus>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.net_vm_ref,
            wire.lan_bridge,
            wire.uplink_bridge,
            wire.external_attachment,
            wire.attachments,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Expected generations bound to an opaque attachment realization.
#[derive(Clone, PartialEq, Eq)]
pub struct AttachmentGenerationFence {
    network_uid: ResourceUid,
    network_generation: ResourceGeneration,
    attachment_uid: ResourceUid,
    attachment_generation: ResourceGeneration,
}

impl AttachmentGenerationFence {
    /// Bind a realization to exact Network and attachment identities.
    pub const fn new(
        network_uid: ResourceUid,
        network_generation: ResourceGeneration,
        attachment_uid: ResourceUid,
        attachment_generation: ResourceGeneration,
    ) -> Self {
        Self {
            network_uid,
            network_generation,
            attachment_uid,
            attachment_generation,
        }
    }

    /// Return the expected Network generation.
    pub const fn network_generation(&self) -> ResourceGeneration {
        self.network_generation
    }

    /// Borrow the expected Network identity.
    pub const fn network_uid(&self) -> &ResourceUid {
        &self.network_uid
    }

    /// Return the expected attachment generation.
    pub const fn attachment_generation(&self) -> ResourceGeneration {
        self.attachment_generation
    }

    /// Borrow the expected attachment identity.
    pub const fn attachment_uid(&self) -> &ResourceUid {
        &self.attachment_uid
    }
}

redacted_debug!(AttachmentGenerationFence);

/// Core-private attachment realization handle and its identity fence.
///
/// This type intentionally has no `Serialize`, `Display`, or schema surface, so
/// it cannot be inserted into public resource status, audit, or telemetry by a
/// generic encoder.
#[derive(Clone, PartialEq, Eq)]
pub struct AttachmentHandle {
    opaque_id: ResourceUid,
    generation_fence: AttachmentGenerationFence,
}

impl AttachmentHandle {
    /// Construct a private handle bound to its expected generations.
    pub const fn new(opaque_id: ResourceUid, generation_fence: AttachmentGenerationFence) -> Self {
        Self {
            opaque_id,
            generation_fence,
        }
    }

    /// Borrow the opaque ID for a private effect adapter only.
    pub const fn opaque_id(&self) -> &ResourceUid {
        &self.opaque_id
    }

    /// Borrow the explicit deletion generation fence.
    pub const fn generation_fence(&self) -> &AttachmentGenerationFence {
        &self.generation_fence
    }
}

redacted_debug!(AttachmentHandle);

/// One claim already grouped by Core under one opaque physical-NIC identity.
#[derive(Clone, PartialEq, Eq)]
pub struct ExternalNicClaim {
    zone_uid: ResourceUid,
    macvtap_mode: MacvtapMode,
    sharing_policy: SharingPolicy,
}

impl ExternalNicClaim {
    /// Construct a claim without accepting a raw interface or authority key.
    pub const fn new(
        zone_uid: ResourceUid,
        macvtap_mode: MacvtapMode,
        sharing_policy: SharingPolicy,
    ) -> Self {
        Self {
            zone_uid,
            macvtap_mode,
            sharing_policy,
        }
    }

    /// Borrow the Zone that defines this claim's isolation domain.
    pub const fn zone_uid(&self) -> &ResourceUid {
        &self.zone_uid
    }

    /// Return the requested macvtap mode.
    pub const fn macvtap_mode(&self) -> MacvtapMode {
        self.macvtap_mode
    }

    /// Return the explicitly authored sharing policy.
    pub const fn sharing_policy(&self) -> SharingPolicy {
        self.sharing_policy
    }
}

redacted_debug!(ExternalNicClaim);

/// Fail-closed external physical-NIC admission reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalNicAdmissionError {
    ExternalPhysicalNicConflict,
    ExternalPhysicalNicCrossZoneL2,
}

impl ExternalNicAdmissionError {
    /// Return the stable condition reason.
    pub const fn code(self) -> &'static str {
        match self {
            Self::ExternalPhysicalNicConflict => "external-physical-nic-conflict",
            Self::ExternalPhysicalNicCrossZoneL2 => "external-physical-nic-cross-zone-l2",
        }
    }
}

impl core::fmt::Display for ExternalNicAdmissionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for ExternalNicAdmissionError {}

/// Admit claims for one Core-derived Host-global physical-NIC identity.
///
/// This function must run before any macvtap or VMM effect. Non-bridge modes
/// remain globally exclusive. Bridge multiplexing requires an explicitly
/// multiplexed claim set in one Zone and a sufficient signed holder limit. A
/// bridge claim set spanning Zones is rejected with the dedicated L2 reason
/// before sharing policy is considered.
pub fn admit_external_nic_claims(
    claims: &[ExternalNicClaim],
    signed_max_holders: usize,
) -> Result<(), ExternalNicAdmissionError> {
    if claims.len() <= 1 {
        return if claims.len() <= signed_max_holders {
            Ok(())
        } else {
            Err(ExternalNicAdmissionError::ExternalPhysicalNicConflict)
        };
    }
    if claims
        .iter()
        .any(|claim| claim.macvtap_mode != MacvtapMode::Bridge)
    {
        return Err(ExternalNicAdmissionError::ExternalPhysicalNicConflict);
    }
    let zone = &claims[0].zone_uid;
    if claims.iter().any(|claim| &claim.zone_uid != zone) {
        return Err(ExternalNicAdmissionError::ExternalPhysicalNicCrossZoneL2);
    }
    if claims.len() > signed_max_holders
        || claims
            .iter()
            .any(|claim| claim.sharing_policy != SharingPolicy::Multiplexed)
    {
        return Err(ExternalNicAdmissionError::ExternalPhysicalNicConflict);
    }
    Ok(())
}

/// Fixed lifecycle declaration for the reserved Network controller User.
#[derive(Clone, PartialEq, Eq)]
pub struct NetLocalControllerUserResource {
    resource_ref: ResourceRef,
    owner_ref: ResourceRef,
    verifier_ref: ResourceRef,
    managed_by: ManagedBy,
    spec: UserSpec,
}

impl NetLocalControllerUserResource {
    /// Build the exact User declaration controllers and compilers share.
    pub fn declared() -> Self {
        Self {
            resource_ref: ResourceRef::parse(&format!("User/{NET_LOCAL_CONTROLLER_USER_NAME}"))
                .expect("fixed User reference is valid"),
            owner_ref: ResourceRef::parse(NET_LOCAL_CONTROLLER_OWNER_REF)
                .expect("fixed owner reference is valid"),
            verifier_ref: ResourceRef::parse(NET_LOCAL_CONTROLLER_VERIFIER_REF)
                .expect("fixed verifier reference is valid"),
            managed_by: ManagedBy::Controller,
            spec: UserSpec::minimal(
                OsUsername::parse(NET_LOCAL_CONTROLLER_OS_USERNAME)
                    .expect("fixed OS username is valid"),
            ),
        }
    }

    /// Borrow the canonical User resource reference.
    pub const fn resource_ref(&self) -> &ResourceRef {
        &self.resource_ref
    }

    /// Borrow the owning Network Provider reference.
    pub const fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }

    /// Borrow the system Provider that verifies the account through NSS.
    pub const fn verifier_ref(&self) -> &ResourceRef {
        &self.verifier_ref
    }

    /// Return the controller-managed lifecycle class.
    pub const fn managed_by(&self) -> ManagedBy {
        self.managed_by
    }

    /// Borrow the User base spec, which contains no numeric identity.
    pub const fn spec(&self) -> &UserSpec {
        &self.spec
    }

    /// Decide the config-Volume precondition from the universal User phase only.
    pub const fn config_volume_gate(&self, phase: ResourcePhase) -> NetLocalControllerUserGate {
        if matches!(phase, ResourcePhase::Ready) {
            NetLocalControllerUserGate::Ready
        } else {
            NetLocalControllerUserGate::AbortUserNotReady
        }
    }
}

redacted_debug!(NetLocalControllerUserResource);

/// Config-Volume action after checking the reserved User resource phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetLocalControllerUserGate {
    Ready,
    AbortUserNotReady,
}

impl NetLocalControllerUserGate {
    /// Return the affected Network condition when the gate blocks.
    pub const fn condition(self) -> Option<NetworkConditionType> {
        match self {
            Self::Ready => None,
            Self::AbortUserNotReady => Some(NetworkConditionType::ConfigVolumeReady),
        }
    }

    /// Return the condition state when the gate blocks.
    pub const fn condition_state(self) -> Option<ConditionState> {
        match self {
            Self::Ready => None,
            Self::AbortUserNotReady => Some(ConditionState::False),
        }
    }

    /// Return the stable failure reason when the gate blocks.
    pub const fn reason(self) -> Option<&'static str> {
        match self {
            Self::Ready => None,
            Self::AbortUserNotReady => Some("user-not-ready"),
        }
    }
}

/// Return whether two validated IPv4 CIDRs overlap, including containment.
pub fn cidr_overlaps(left: &Ipv4Cidr, right: &Ipv4Cidr) -> bool {
    let (left_address, left_prefix) = left.address_and_prefix();
    let (right_address, right_prefix) = right.address_and_prefix();
    let common_prefix = left_prefix.min(right_prefix);
    let mask = prefix_mask(common_prefix);
    u32::from_be_bytes(left_address) & mask == u32::from_be_bytes(right_address) & mask
}

fn prefix_mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
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

    const MINIMAL_NETWORK_SPEC: &[u8] = br#"{"attachments":[],"dhcp":{"domain":null,"ignoreClientNames":true},"dns":{"cacheSize":1000,"forwarders":[]},"externalAttachment":null,"isolation":{"allowEastWest":false},"lanCidr":"10.20.0.0/24","mdns":{"dnsmasqLocal":false,"dnsmasqLocalPort":53530,"enable":false,"publishWorkstation":false,"reflector":true},"mssClamp":false,"mtu":null,"netVmNameOverride":null,"netVmSystemArtifactId":"net-vm-system","routing":{"hostBlocklist":["10.0.0.0/8","169.254.0.0/16","172.16.0.0/12","192.168.0.0/16"]},"uplinkCidr":"192.0.2.0/30"}"#;
    const READY_NETWORK_STATUS: &[u8] = br#"{"attachments":[{"executionRef":"Guest/corp-vm","phase":"Ready"}],"externalAttachment":{"authority":{"arbitration":"exclusive","available":true,"holderCount":1,"queueDepth":0,"updateCurrency":"Current"},"phase":"Ready"},"lanBridge":{"phase":"Ready"},"netVmRef":"Guest/net-work","uplinkBridge":{"phase":"Ready"}}"#;

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
    fn network_cidr_shape_and_overlap_are_enforced_before_effects() {
        assert!(
            NetworkSpec::minimal(
                Ipv4Cidr::parse("10.20.0.0/25").unwrap(),
                Ipv4Cidr::parse("192.0.2.0/30").unwrap(),
                BoundedToken::parse("a").unwrap(),
            )
            .is_err()
        );
        assert!(
            NetworkSpec::minimal(
                Ipv4Cidr::parse("10.20.1.0/24").unwrap(),
                Ipv4Cidr::parse("192.0.2.4/30").unwrap(),
                BoundedToken::parse("a").unwrap(),
            )
            .is_ok()
        );
        assert!(
            NetworkSpec::minimal(
                Ipv4Cidr::parse("10.20.1.1/24").unwrap(),
                Ipv4Cidr::parse("192.0.2.4/30").unwrap(),
                BoundedToken::parse("a").unwrap(),
            )
            .is_err()
        );
        assert!(
            NetworkSpec::minimal(
                Ipv4Cidr::parse("192.0.2.0/24").unwrap(),
                Ipv4Cidr::parse("192.0.2.0/30").unwrap(),
                BoundedToken::parse("a").unwrap(),
            )
            .is_err()
        );
    }

    #[test]
    fn cidr_overlap_arithmetic_covers_disjoint_equal_and_contained_ranges() {
        let cidr = |value| Ipv4Cidr::parse(value).unwrap();
        assert!(cidr_overlaps(&cidr("10.20.0.0/24"), &cidr("10.20.0.0/24")));
        assert!(cidr_overlaps(&cidr("10.20.0.0/24"), &cidr("10.20.0.64/26")));
        assert!(cidr_overlaps(&cidr("10.20.0.64/26"), &cidr("10.20.0.0/24")));
        assert!(!cidr_overlaps(&cidr("10.20.0.0/24"), &cidr("10.20.1.0/24")));

        for third_octet in [0u8, 1, 127, 254] {
            let left = Ipv4Cidr::parse(format!("10.20.{third_octet}.0/24")).unwrap();
            let right = Ipv4Cidr::parse(format!("10.21.{third_octet}.0/24")).unwrap();
            assert!(!cidr_overlaps(&left, &right));
        }
    }

    #[test]
    fn host_blocklist_defaults_are_mandatory_and_additive() {
        let defaults = RoutingSpec::default();
        for required in DEFAULT_HOST_BLOCKLIST {
            assert!(
                defaults
                    .host_blocklist()
                    .contains(&Ipv4Cidr::parse(required).unwrap())
            );
        }
        let mut with_added = defaults.host_blocklist().to_vec();
        with_added.push(Ipv4Cidr::parse("203.0.113.0/24").unwrap());
        let added = RoutingSpec::new(with_added).unwrap();
        assert_eq!(
            added.host_blocklist().len(),
            DEFAULT_HOST_BLOCKLIST.len() + 1
        );

        let parsed: RoutingSpec = serde_json::from_slice(br#"{}"#).unwrap();
        assert_eq!(parsed, defaults);
        assert!(serde_json::from_slice::<RoutingSpec>(br#"{"hostBlocklist":[]}"#).is_err());
        assert_eq!(
            RoutingSpec::new(vec![Ipv4Cidr::parse("10.0.0.0/8").unwrap()]),
            Err(PrimitiveSpecError::ConflictingFields)
        );
    }

    #[test]
    fn reserved_net_vm_names_and_invalid_parent_ifnames_are_rejected() {
        let with_name = |name: &str| {
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
                Some(BoundedToken::parse(name).unwrap()),
                BoundedToken::parse("a").unwrap(),
                Vec::new(),
            )
        };
        assert_eq!(
            with_name("launcher"),
            Err(PrimitiveSpecError::ConflictingFields)
        );
        assert_eq!(
            with_name("sys-net"),
            Err(PrimitiveSpecError::ConflictingFields)
        );
        assert!(with_name("net-work").is_ok());

        let invalid = br#"{"mode":"macvtap","parentInterface":"bad.interface","macvtapMode":"bridge","sharingPolicy":"exclusive","mac":null,"ipv4":{},"egress":{},"portForwards":[]}"#;
        assert!(serde_json::from_slice::<ExternalAttachmentSpec>(invalid).is_err());
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
                    IfName::parse("eno1").unwrap(),
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
                IfName::parse("eno1").unwrap(),
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
    fn static_external_ipv4_requires_a_gateway() {
        let address = Ipv4Cidr::parse("192.0.2.8/29").unwrap();
        assert_eq!(
            ExternalIpv4Spec::new(Ipv4Method::Static, Some(address.clone()), None, Vec::new()),
            Err(PrimitiveSpecError::MissingRequiredField)
        );
        assert!(
            ExternalIpv4Spec::new(
                Ipv4Method::Static,
                Some(address),
                Some(Ipv4Address::parse("192.0.2.9").unwrap()),
                Vec::new(),
            )
            .is_ok()
        );
        assert!(
            serde_json::from_slice::<ExternalIpv4Spec>(
                br#"{"method":"static","address":"192.0.2.8/29"}"#,
            )
            .is_err()
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

    #[test]
    fn network_status_is_bounded_and_excludes_runtime_network_identity() {
        let attachment = AttachmentStatus::new(
            ResourceRef::parse("Guest/corp-vm").unwrap(),
            NetworkComponentPhase::Ready,
        )
        .unwrap();
        let status = NetworkStatus::new(
            ResourceRef::parse("Guest/net-work").unwrap(),
            NetworkFabricStatus::new(NetworkComponentPhase::Ready),
            NetworkFabricStatus::new(NetworkComponentPhase::Ready),
            Some(ExternalAttachmentStatus::new(
                NetworkComponentPhase::Ready,
                ExternalNicAuthorityStatus::new(
                    true,
                    1,
                    0,
                    SharingPolicy::Exclusive,
                    UpdateState::Current,
                ),
            )),
            vec![attachment],
        )
        .unwrap();
        let encoded = canonical_json_bytes(&status).unwrap();
        assert_eq!(encoded, READY_NETWORK_STATUS);
        let parsed: NetworkStatus = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(parsed, status);
        let text = String::from_utf8(encoded).unwrap();
        for forbidden in [
            "ifName",
            "bridgeName",
            "tapIfName",
            "parentInterface",
            "macAddress",
            "attachmentHandle",
            "authorityKey",
            "ownerProof",
        ] {
            assert!(!text.contains(forbidden));
        }
        assert_eq!(format!("{status:?}"), "NetworkStatus(<redacted>)");
    }

    #[test]
    fn opaque_attachment_handle_binds_both_generation_fences() {
        let fence = AttachmentGenerationFence::new(
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            ResourceGeneration::new(4).unwrap(),
            ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap(),
            ResourceGeneration::new(7).unwrap(),
        );
        let handle = AttachmentHandle::new(
            ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").unwrap(),
            fence,
        );
        assert_eq!(handle.generation_fence().network_generation().get(), 4);
        assert_eq!(handle.generation_fence().attachment_generation().get(), 7);
        assert_eq!(
            handle.generation_fence().network_uid(),
            &ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap()
        );
        assert_eq!(
            handle.generation_fence().attachment_uid(),
            &ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap()
        );
        assert_eq!(format!("{handle:?}"), "AttachmentHandle(<redacted>)");
    }

    #[test]
    fn external_physical_nic_multiplex_never_crosses_a_zone_boundary() {
        let work = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let personal = ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap();
        let claim = |zone, mode, policy| ExternalNicClaim::new(zone, mode, policy);

        assert_eq!(
            admit_external_nic_claims(
                &[
                    claim(
                        work.clone(),
                        MacvtapMode::Bridge,
                        SharingPolicy::Multiplexed
                    ),
                    claim(personal, MacvtapMode::Bridge, SharingPolicy::Multiplexed),
                ],
                8,
            ),
            Err(ExternalNicAdmissionError::ExternalPhysicalNicCrossZoneL2)
        );
        assert_eq!(
            admit_external_nic_claims(
                &[
                    claim(work.clone(), MacvtapMode::Bridge, SharingPolicy::Exclusive,),
                    claim(
                        ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap(),
                        MacvtapMode::Bridge,
                        SharingPolicy::Multiplexed,
                    ),
                ],
                1,
            ),
            Err(ExternalNicAdmissionError::ExternalPhysicalNicCrossZoneL2)
        );
        assert!(
            admit_external_nic_claims(
                &[
                    claim(
                        work.clone(),
                        MacvtapMode::Bridge,
                        SharingPolicy::Multiplexed
                    ),
                    claim(
                        work.clone(),
                        MacvtapMode::Bridge,
                        SharingPolicy::Multiplexed
                    ),
                ],
                2,
            )
            .is_ok()
        );
        assert_eq!(
            admit_external_nic_claims(
                &[
                    claim(work.clone(), MacvtapMode::Bridge, SharingPolicy::Exclusive),
                    claim(
                        work.clone(),
                        MacvtapMode::Bridge,
                        SharingPolicy::Multiplexed
                    ),
                ],
                8,
            ),
            Err(ExternalNicAdmissionError::ExternalPhysicalNicConflict)
        );
        assert_eq!(
            admit_external_nic_claims(
                &[
                    claim(
                        work.clone(),
                        MacvtapMode::Passthru,
                        SharingPolicy::Exclusive
                    ),
                    claim(work, MacvtapMode::Passthru, SharingPolicy::Exclusive),
                ],
                8,
            ),
            Err(ExternalNicAdmissionError::ExternalPhysicalNicConflict)
        );
        assert_eq!(
            admit_external_nic_claims(
                &[claim(
                    ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").unwrap(),
                    MacvtapMode::Bridge,
                    SharingPolicy::Exclusive,
                )],
                0,
            ),
            Err(ExternalNicAdmissionError::ExternalPhysicalNicConflict)
        );
    }

    #[test]
    fn reserved_controller_user_lifecycle_uses_resource_phase_not_numeric_identity() {
        let user = NetLocalControllerUserResource::declared();
        assert_eq!(
            user.resource_ref(),
            &ResourceRef::parse("User/net-local-controller").unwrap()
        );
        assert_eq!(
            user.owner_ref(),
            &ResourceRef::parse("Provider/network-local").unwrap()
        );
        assert_eq!(
            user.verifier_ref(),
            &ResourceRef::parse("Provider/system-core").unwrap()
        );
        assert_eq!(user.managed_by(), ManagedBy::Controller);
        assert_eq!(
            user.spec().os_username().as_str(),
            NET_LOCAL_CONTROLLER_OS_USERNAME
        );

        assert_eq!(
            user.config_volume_gate(ResourcePhase::Ready),
            NetLocalControllerUserGate::Ready
        );
        let blocked = user.config_volume_gate(ResourcePhase::Pending);
        assert_eq!(blocked, NetLocalControllerUserGate::AbortUserNotReady);
        assert_eq!(
            blocked.condition(),
            Some(NetworkConditionType::ConfigVolumeReady)
        );
        assert_eq!(blocked.condition_state(), Some(ConditionState::False));
        assert_eq!(blocked.reason(), Some("user-not-ready"));

        let spec = canonical_json_bytes(user.spec()).unwrap();
        let spec_text = String::from_utf8(spec).unwrap();
        for forbidden in ["uid", "gid", "managedBy", "ownerRef"] {
            assert!(!spec_text.contains(forbidden));
        }
        assert_eq!(
            format!("{user:?}"),
            "NetLocalControllerUserResource(<redacted>)"
        );
    }
}
