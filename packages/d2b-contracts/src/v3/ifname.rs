//! Provider-neutral Linux interface-name contracts.
//!
//! Derived names are deterministic, fit Linux `IFNAMSIZ - 1`, and reserve a
//! recognizable prefix without exposing a caller-supplied name through
//! diagnostics. Collision detection is a mandatory admission step because a
//! truncated hash is not an identity proof.

use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Schema, SchemaObject, SingleOrVec},
};
use serde::{Deserialize, Deserializer, Serialize};

use super::ResourceName;

/// Maximum visible bytes in a Linux interface name.
pub const MAX_IFNAME_BYTES: usize = 15;
/// Default prefix reserved for d2b-derived links.
pub const DEFAULT_PREFIX: &str = "d2b-";
/// Bridge role tag embedded in derived names.
pub const BRIDGE_TAG: char = 'b';
/// TAP and macvtap role tag embedded in derived names.
pub const TAP_TAG: char = 't';
/// Number of Crockford base32 characters retained from the hash.
pub const HASH_SUFFIX_LEN: usize = 8;

const MAX_PREFIX_BYTES: usize = 8;
const CROCKFORD_ALPHABET: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// A Linux interface name constrained to the shared d2b-safe alphabet.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct IfName(String);

impl IfName {
    /// Validate a nonempty interface name of at most 15 ASCII bytes.
    pub fn parse(value: impl Into<String>) -> Result<Self, IfNameError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IfNameError::Empty);
        }
        if value.len() > MAX_IFNAME_BYTES {
            return Err(IfNameError::TooLong);
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(IfNameError::InvalidCharacter);
        }
        Ok(Self(value))
    }

    /// Borrow the validated name for an explicitly authorized kernel call.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for IfName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("IfName(<redacted>)")
    }
}

impl core::fmt::Display for IfName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("IfName(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for IfName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for IfName {
    fn schema_name() -> String {
        "IfName".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        let mut schema = SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
            ..Default::default()
        };
        schema.string().min_length = Some(1);
        schema.string().max_length = Some(MAX_IFNAME_BYTES as u32);
        schema.string().pattern = Some("^[A-Za-z0-9_-]+$".to_owned());
        Schema::Object(schema)
    }
}

/// Stable, value-free interface-name rejection classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IfNameError {
    /// Empty names cannot address a kernel link.
    Empty,
    /// Linux interface names have at most fifteen visible bytes.
    TooLong,
    /// The name contains a byte outside the shared safe alphabet.
    InvalidCharacter,
    /// A derivation prefix is empty, oversized, unsafe, or lacks its separator.
    InvalidPrefix,
    /// Distinct logical mappings resolve to one kernel interface name.
    Collision,
    /// One logical mapping declares more than one kernel interface name.
    MappingInconsistent,
}

impl core::fmt::Display for IfNameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let message = match self {
            Self::Empty => "interface name must not be empty",
            Self::TooLong => "interface name exceeds the Linux byte limit",
            Self::InvalidCharacter => "interface name contains an invalid character",
            Self::InvalidPrefix => "interface-name prefix is invalid",
            Self::Collision => "ifname-collision",
            Self::MappingInconsistent => "ifname-mapping-inconsistent",
        };
        f.write_str(message)
    }
}

impl std::error::Error for IfNameError {}

/// Kernel link role represented by a derived interface name.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum DerivedRole {
    /// A LAN or uplink bridge.
    Bridge,
    /// A TAP or macvtap endpoint.
    Tap,
}

impl DerivedRole {
    /// Return the single-character role tag used by the derivation.
    pub const fn tag(self) -> char {
        match self {
            Self::Bridge => BRIDGE_TAG,
            Self::Tap => TAP_TAG,
        }
    }
}

/// Logical Network link role included in the canonical Network derivation.
///
/// Several roles share the same visible bridge or TAP tag but remain distinct
/// hash inputs, so one Network's LAN and uplink bridges cannot self-collide by
/// construction.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkIfRole {
    LanBridge,
    UplinkBridge,
    NetVmLanTap,
    NetVmUplinkTap,
    WorkloadGuestTap,
    ExternalMacvtap,
}

impl NetworkIfRole {
    /// Return the visible bridge or TAP class tag.
    pub const fn tag(self) -> char {
        match self {
            Self::LanBridge | Self::UplinkBridge => BRIDGE_TAG,
            Self::NetVmLanTap
            | Self::NetVmUplinkTap
            | Self::WorkloadGuestTap
            | Self::ExternalMacvtap => TAP_TAG,
        }
    }

    const fn hash_tag(self) -> &'static [u8] {
        match self {
            Self::LanBridge => b"lan-bridge",
            Self::UplinkBridge => b"uplink-bridge",
            Self::NetVmLanTap => b"net-vm-lan-tap",
            Self::NetVmUplinkTap => b"net-vm-uplink-tap",
            Self::WorkloadGuestTap => b"workload-guest-tap",
            Self::ExternalMacvtap => b"external-macvtap",
        }
    }
}

/// One logical-to-kernel interface-name mapping.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IfNameMapping {
    network_name: ResourceName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    guest_name: Option<ResourceName>,
    role: NetworkIfRole,
    derived_ifname: IfName,
}

impl IfNameMapping {
    /// Construct one mapping for collision admission.
    pub fn new(
        network_name: ResourceName,
        guest_name: Option<ResourceName>,
        role: NetworkIfRole,
        derived_ifname: IfName,
    ) -> Self {
        Self {
            network_name,
            guest_name,
            role,
            derived_ifname,
        }
    }

    /// Borrow the derived name for an explicitly authorized resolver.
    pub const fn derived_ifname(&self) -> &IfName {
        &self.derived_ifname
    }

    fn same_key(&self, other: &Self) -> bool {
        self.network_name == other.network_name
            && self.guest_name == other.guest_name
            && self.role == other.role
    }
}

impl core::fmt::Debug for IfNameMapping {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("IfNameMapping(<redacted>)")
    }
}

/// Validate a configurable d2b prefix.
///
/// The adapted contract retains the existing 1-to-8 byte bound, safe alphabet,
/// and required trailing hyphen. The complete derived name is independently
/// checked against the Linux 15-byte limit.
pub fn validate_prefix(prefix: &str) -> Result<(), IfNameError> {
    if prefix.is_empty() || prefix.len() > MAX_PREFIX_BYTES || !prefix.ends_with('-') {
        return Err(IfNameError::InvalidPrefix);
    }
    if !prefix
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(IfNameError::InvalidPrefix);
    }
    Ok(())
}

/// Derive an interface name from a network, optional guest, and kernel role.
pub fn derive_ifname(
    network_name: &str,
    role: NetworkIfRole,
    guest_name: Option<&str>,
    prefix: Option<&str>,
) -> Result<IfName, IfNameError> {
    let prefix = prefix.unwrap_or(DEFAULT_PREFIX);
    validate_prefix(prefix)?;

    let mut hash = FNV_OFFSET;
    hash_bytes(&mut hash, network_name.as_bytes());
    hash_bytes(&mut hash, &[0x1f]);
    if let Some(guest_name) = guest_name {
        hash_bytes(&mut hash, guest_name.as_bytes());
    }
    hash_bytes(&mut hash, &[0x1e]);
    hash_bytes(&mut hash, role.hash_tag());

    let suffix = base32_crockford(hash, HASH_SUFFIX_LEN);
    IfName::parse(format!("{prefix}{}{suffix}", role.tag()))
}

/// Adapt the established host derivation without changing its byte contract.
///
/// The hash input is `network || 0x1f || guest-or-empty || 0x1e || role-tag`.
/// FNV-1a 64-bit is encoded with the Crockford alphabet and truncated to eight
/// characters. The hash is a deterministic namespace aid, not an identity or
/// authorization proof.
pub fn derive_from_env_vm(
    env: &str,
    vm: Option<&str>,
    role: DerivedRole,
    prefix: Option<&str>,
) -> Result<IfName, IfNameError> {
    let prefix = prefix.unwrap_or(DEFAULT_PREFIX);
    validate_prefix(prefix)?;

    let mut hash = FNV_OFFSET;
    hash_bytes(&mut hash, env.as_bytes());
    hash_bytes(&mut hash, &[0x1f]);
    if let Some(vm) = vm {
        hash_bytes(&mut hash, vm.as_bytes());
    }
    hash_bytes(&mut hash, &[0x1e, role.tag() as u8]);

    let suffix = base32_crockford(hash, HASH_SUFFIX_LEN);
    IfName::parse(format!("{prefix}{}{suffix}", role.tag()))
}

/// Return whether a name has the reserved d2b-derived shape.
pub fn looks_d2b_owned(name: &str, prefix: &str) -> bool {
    let Some(rest) = name.strip_prefix(prefix) else {
        return false;
    };
    let mut chars = rest.chars();
    if !matches!(chars.next(), Some(BRIDGE_TAG | TAP_TAG)) {
        return false;
    }
    let suffix: String = chars.collect();
    suffix.len() == HASH_SUFFIX_LEN
        && suffix
            .chars()
            .all(|character| CROCKFORD_ALPHABET.contains(character))
}

/// Reject inconsistent mappings and every derived-name collision.
///
/// Callers must run this across the complete Host collision domain before any
/// link effect. The error deliberately carries neither side of the collision.
pub fn detect_collisions(mappings: &[IfNameMapping]) -> Result<(), IfNameError> {
    for (index, mapping) in mappings.iter().enumerate() {
        for other in &mappings[..index] {
            if mapping.same_key(other) && mapping.derived_ifname != other.derived_ifname {
                return Err(IfNameError::MappingInconsistent);
            }
            if mapping.derived_ifname == other.derived_ifname {
                return Err(IfNameError::Collision);
            }
        }
    }
    Ok(())
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

fn base32_crockford(mut value: u64, characters: usize) -> String {
    let alphabet = CROCKFORD_ALPHABET.as_bytes();
    let mut output = Vec::with_capacity(characters);
    for _ in 0..characters {
        output.push(alphabet[(value & 0x1f) as usize]);
        value >>= 5;
    }
    output.reverse();
    String::from_utf8(output).expect("Crockford alphabet is ASCII")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derivation_matches_the_adapted_host_contract() {
        let bridge = derive_from_env_vm("work", Some("corp-vm"), DerivedRole::Bridge, None)
            .expect("derive bridge");
        assert_eq!(bridge.as_str(), "d2b-bETSY9AFS");
        assert!(bridge.as_str().len() <= MAX_IFNAME_BYTES);
        assert!(looks_d2b_owned(bridge.as_str(), DEFAULT_PREFIX));

        let repeated = derive_from_env_vm("work", Some("corp-vm"), DerivedRole::Bridge, None)
            .expect("derive repeated bridge");
        assert_eq!(bridge, repeated);
    }

    #[test]
    fn role_and_guest_are_part_of_the_hash_input() {
        let roles = [
            NetworkIfRole::LanBridge,
            NetworkIfRole::UplinkBridge,
            NetworkIfRole::NetVmLanTap,
            NetworkIfRole::NetVmUplinkTap,
            NetworkIfRole::WorkloadGuestTap,
            NetworkIfRole::ExternalMacvtap,
        ];
        let mut names: Vec<IfName> = roles
            .into_iter()
            .map(|role| derive_ifname("work", role, Some("corp-vm"), None).unwrap())
            .collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), roles.len());

        let tap = derive_ifname(
            "work",
            NetworkIfRole::WorkloadGuestTap,
            Some("corp-vm"),
            None,
        )
        .unwrap();
        let other_guest = derive_ifname(
            "work",
            NetworkIfRole::WorkloadGuestTap,
            Some("other-vm"),
            None,
        )
        .unwrap();
        assert_ne!(tap, other_guest);
    }

    #[test]
    fn linux_limit_alphabet_and_prefix_reservation_fail_closed() {
        assert_eq!(IfName::parse(""), Err(IfNameError::Empty));
        assert_eq!(IfName::parse("abcdefghijklmnop"), Err(IfNameError::TooLong));
        assert_eq!(
            IfName::parse("bad.name"),
            Err(IfNameError::InvalidCharacter)
        );
        assert_eq!(validate_prefix("d2b-"), Ok(()));
        assert_eq!(validate_prefix("d2b"), Err(IfNameError::InvalidPrefix));
        assert_eq!(
            validate_prefix("with space-"),
            Err(IfNameError::InvalidPrefix)
        );
        assert!(!looks_d2b_owned("d2b-bABCDEFGI", DEFAULT_PREFIX));
        assert!(!looks_d2b_owned("foreign0", DEFAULT_PREFIX));
    }

    #[test]
    fn collision_and_inconsistent_mapping_are_distinct_closed_failures() {
        let name = IfName::parse("d2b-bAAAAAAAA").unwrap();
        let collision = vec![
            IfNameMapping::new(
                ResourceName::parse("work").unwrap(),
                None,
                NetworkIfRole::LanBridge,
                name.clone(),
            ),
            IfNameMapping::new(
                ResourceName::parse("personal").unwrap(),
                None,
                NetworkIfRole::LanBridge,
                name,
            ),
        ];
        assert_eq!(detect_collisions(&collision), Err(IfNameError::Collision));

        let inconsistent = vec![
            IfNameMapping::new(
                ResourceName::parse("work").unwrap(),
                Some(ResourceName::parse("vm").unwrap()),
                NetworkIfRole::WorkloadGuestTap,
                IfName::parse("d2b-tAAAAAAAA").unwrap(),
            ),
            IfNameMapping::new(
                ResourceName::parse("work").unwrap(),
                Some(ResourceName::parse("vm").unwrap()),
                NetworkIfRole::WorkloadGuestTap,
                IfName::parse("d2b-tBBBBBBBB").unwrap(),
            ),
        ];
        assert_eq!(
            detect_collisions(&inconsistent),
            Err(IfNameError::MappingInconsistent)
        );
    }

    #[test]
    fn diagnostics_do_not_echo_names_or_mapping_identity() {
        let marker = format!("ifname-secret-{}", std::process::id());
        let error = IfName::parse(format!("{marker}.invalid")).unwrap_err();
        assert!(!format!("{error:?}").contains(&marker));
        assert!(!error.to_string().contains(&marker));

        let mapping = IfNameMapping::new(
            ResourceName::parse(&marker).unwrap(),
            Some(ResourceName::parse(&marker).unwrap()),
            NetworkIfRole::WorkloadGuestTap,
            IfName::parse("d2b-tAAAAAAAA").unwrap(),
        );
        assert!(!format!("{mapping:?}").contains(&marker));
        assert!(!format!("{:?}", mapping.derived_ifname()).contains("AAAAAAAA"));
    }
}
