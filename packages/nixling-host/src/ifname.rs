//! W3 host-prepare module: `ifname` — owned by scope s2.
//!
//! Implements the hash-derived [`IfName`] scheme + emitter-time
//! collision detection per plan.md §"W3 IfName hash collision + mapping
//! exposure". The underlying IFNAMSIZ-validated newtype is owned by
//! [`nixling_core::host::IfName`]; this module wraps it with:
//!
//! - configurable nixling prefix (default `nl-`);
//! - deterministic short-hash derivation from `(env, vm?)`;
//! - `detect_collisions` over a slice of `IfNameMapping` consumed by
//!   the bundle emitter and re-validated by the broker.
//!
//! The hash is FNV-1a 64-bit, base32-encoded (Crockford alphabet
//! without I/L/O/U), truncated to 8 chars. Bridges use prefix `b`,
//! TAPs prefix `t`. The full name fits inside the IFNAMSIZ-1 (15 byte)
//! limit by construction.

use nixling_core::host::{IfName as CoreIfName, IfNameError as CoreIfNameError};
use nixling_core::host_w3::IfNameMapping;

pub use nixling_core::host::IfName;

/// Default nixling prefix. Operators can override per site.
pub const DEFAULT_PREFIX: &str = "nl-";

/// Bridge role single-character tag used in derived names.
pub const BRIDGE_TAG: char = 'b';
/// TAP role single-character tag used in derived names.
pub const TAP_TAG: char = 't';

/// Hash-suffix length used by the derivation. 8 chars × 5 bits =
/// 40 bits of namespace, ~1.1T entries, collision risk negligible at
/// the bundle scale this targets.
pub const HASH_SUFFIX_LEN: usize = 8;

/// W3 errors layered over [`CoreIfNameError`]. Maps every emitter and
/// broker fail-closed path required by the W3 plan to a stable
/// variant tag the audit log can reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IfNameError {
    /// Name exceeds IFNAMSIZ-1 (15 bytes).
    IfNameTooLong { len: usize, value: String },
    /// Name is empty.
    Empty,
    /// Name contains bytes outside `[A-Za-z0-9_-]`.
    InvalidCharacter { value: String },
    /// Prefix is invalid (empty, oversized, or non-conforming alphabet).
    InvalidPrefix { prefix: String },
    /// Two different `(env, vm?, role)` keys derived the same
    /// IfName. The emitter and broker fail closed and audit both
    /// sides.
    IfNameCollision(Box<IfNameCollisionDetail>),
    /// A single mapping declared two different IfNames for the same
    /// `(env, vm?)` key.
    MappingInconsistent {
        key: String,
        first: String,
        second: String,
    },
}

/// Detail payload for [`IfNameError::IfNameCollision`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct IfNameCollisionDetail {
    pub ifname: String,
    pub mapping_a: CollisionParty,
    pub mapping_b: CollisionParty,
}

/// Human-readable identifier for one side of a collision report.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CollisionParty {
    pub env: String,
    pub vm: Option<String>,
    pub role: &'static str,
}

impl std::fmt::Display for IfNameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IfNameTooLong { len, value } => write!(
                f,
                "interface name {value:?} exceeds IFNAMSIZ-1 (got {len}, max 15 bytes)"
            ),
            Self::Empty => write!(f, "interface name must not be empty"),
            Self::InvalidCharacter { value } => write!(
                f,
                "interface name {value:?} contains characters outside [A-Za-z0-9_-]"
            ),
            Self::InvalidPrefix { prefix } => {
                write!(f, "invalid nixling ifname prefix: {prefix:?}")
            }
            Self::IfNameCollision(detail) => write!(
                f,
                "ifname-collision: {ifname:?} derived from {a:?} and {b:?}",
                ifname = detail.ifname,
                a = detail.mapping_a,
                b = detail.mapping_b,
            ),
            Self::MappingInconsistent { key, first, second } => write!(
                f,
                "ifname mapping {key:?} declared inconsistent names {first:?} vs {second:?}"
            ),
        }
    }
}

impl std::error::Error for IfNameError {}

impl From<CoreIfNameError> for IfNameError {
    fn from(value: CoreIfNameError) -> Self {
        match value {
            CoreIfNameError::Empty => Self::Empty,
            CoreIfNameError::TooLong => Self::IfNameTooLong {
                len: 0,
                value: String::new(),
            },
            CoreIfNameError::InvalidCharacter => Self::InvalidCharacter {
                value: String::new(),
            },
        }
    }
}

/// Constructs an [`IfName`] enforcing IFNAMSIZ-1 + nixling alphabet,
/// preserving the input string in the error so the audit record can
/// fingerprint the offending value.
pub fn new_validated(value: &str) -> Result<IfName, IfNameError> {
    if value.is_empty() {
        return Err(IfNameError::Empty);
    }
    if value.len() > 15 {
        return Err(IfNameError::IfNameTooLong {
            len: value.len(),
            value: value.to_owned(),
        });
    }
    CoreIfName::new(value).map_err(|err| match err {
        CoreIfNameError::Empty => IfNameError::Empty,
        CoreIfNameError::TooLong => IfNameError::IfNameTooLong {
            len: value.len(),
            value: value.to_owned(),
        },
        CoreIfNameError::InvalidCharacter => IfNameError::InvalidCharacter {
            value: value.to_owned(),
        },
    })
}

/// Validates a nixling ifname prefix (`<=8` bytes,
/// `[A-Za-z0-9_-]+`, must end with `-` so the hash suffix is visually
/// separated).
pub fn validate_prefix(prefix: &str) -> Result<(), IfNameError> {
    if prefix.is_empty() || prefix.len() > 8 || !prefix.ends_with('-') {
        return Err(IfNameError::InvalidPrefix {
            prefix: prefix.to_owned(),
        });
    }
    if !prefix
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(IfNameError::InvalidPrefix {
            prefix: prefix.to_owned(),
        });
    }
    Ok(())
}

/// Derived role indicator embedded in the IfName.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DerivedRole {
    Bridge,
    Tap,
}

impl DerivedRole {
    pub fn tag(self) -> char {
        match self {
            Self::Bridge => BRIDGE_TAG,
            Self::Tap => TAP_TAG,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bridge => "bridge",
            Self::Tap => "tap",
        }
    }
}

/// Derives an IfName from `(env, vm?)` for `role`, using `prefix`
/// (default [`DEFAULT_PREFIX`]). The encoding is
/// `<prefix><role-tag><HASH_SUFFIX_LEN-char base32 of fnv1a(env|0x1f|vm.unwrap_or(""))>`.
///
/// Guaranteed `<= 15` bytes for the default prefix (`nl-` + `b` +
/// 8 hash chars = 12 bytes).
pub fn derive_from_env_vm(
    env: &str,
    vm: Option<&str>,
    role: DerivedRole,
    prefix: Option<&str>,
) -> Result<IfName, IfNameError> {
    let prefix = prefix.unwrap_or(DEFAULT_PREFIX);
    validate_prefix(prefix)?;
    let mut hasher = Fnv1a::new();
    hasher.write(env.as_bytes());
    hasher.write(&[0x1f]);
    if let Some(vm) = vm {
        hasher.write(vm.as_bytes());
    }
    hasher.write(&[0x1e]);
    hasher.write(&[role.tag() as u8]);
    let digest = hasher.finish();
    let suffix = base32_crockford(digest, HASH_SUFFIX_LEN);
    let candidate = format!("{prefix}{tag}{suffix}", tag = role.tag());
    new_validated(&candidate)
}

/// Returns whether `name` matches the nixling-derived shape (prefix +
/// role tag + base32 suffix). Used by host-LAN-CIDR derivation to
/// skip nixling-owned links.
pub fn looks_nixling_owned(name: &str, prefix: &str) -> bool {
    if !name.starts_with(prefix) {
        return false;
    }
    let rest = &name[prefix.len()..];
    let mut chars = rest.chars();
    match chars.next() {
        Some(tag) if tag == BRIDGE_TAG || tag == TAP_TAG => {}
        _ => return false,
    }
    let tail: String = chars.collect();
    tail.len() == HASH_SUFFIX_LEN && tail.chars().all(|c| CROCKFORD_ALPHABET.contains(c))
}

/// Validates that no two mappings collide on the same derived bridge
/// or TAP name. Returns the first collision found (deterministic
/// emitter behaviour).
pub fn detect_collisions(mappings: &[IfNameMapping]) -> Result<(), IfNameError> {
    let mut seen: Vec<(String, CollisionParty)> = Vec::with_capacity(mappings.len() * 2);
    for mapping in mappings {
        let bridge_str = mapping.bridge.as_str().to_owned();
        let party_bridge = CollisionParty {
            env: mapping.env.clone(),
            vm: mapping.vm.clone(),
            role: "bridge",
        };
        if let Some((_, first)) = seen.iter().find(|(name, _)| *name == bridge_str) {
            return Err(IfNameError::IfNameCollision(Box::new(
                IfNameCollisionDetail {
                    ifname: bridge_str,
                    mapping_a: first.clone(),
                    mapping_b: party_bridge,
                },
            )));
        }
        seen.push((bridge_str, party_bridge));

        if let Some(tap) = &mapping.tap {
            let tap_str = tap.as_str().to_owned();
            let party_tap = CollisionParty {
                env: mapping.env.clone(),
                vm: mapping.vm.clone(),
                role: "tap",
            };
            if let Some((_, first)) = seen.iter().find(|(name, _)| *name == tap_str) {
                return Err(IfNameError::IfNameCollision(Box::new(
                    IfNameCollisionDetail {
                        ifname: tap_str,
                        mapping_a: first.clone(),
                        mapping_b: party_tap,
                    },
                )));
            }
            seen.push((tap_str, party_tap));
        }
    }
    Ok(())
}

// -------------------------------------------------------------------
// FNV-1a 64-bit (deterministic, no external dep)
// -------------------------------------------------------------------

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

struct Fnv1a(u64);

impl Fnv1a {
    fn new() -> Self {
        Self(FNV_OFFSET)
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(FNV_PRIME);
        }
    }
    fn finish(self) -> u64 {
        self.0
    }
}

// Crockford's base32 alphabet (no I, L, O, U; case-insensitive).
const CROCKFORD_ALPHABET: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn base32_crockford(mut value: u64, chars: usize) -> String {
    let alphabet: Vec<char> = CROCKFORD_ALPHABET.chars().collect();
    let mut out = String::with_capacity(chars);
    for _ in 0..chars {
        let idx = (value & 0x1f) as usize;
        out.push(alphabet[idx]);
        value >>= 5;
    }
    // Encoded LSB-first; reverse so the most significant nibble comes
    // first for human stability.
    out.chars().rev().collect()
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_within_ifnamsiz_for_default_prefix() {
        let n = derive_from_env_vm("work", Some("corp-vm"), DerivedRole::Bridge, None).unwrap();
        assert!(
            n.as_str().len() <= 15,
            "len {}: {}",
            n.as_str().len(),
            n.as_str()
        );
        assert!(n.as_str().starts_with("nl-b"), "{}", n.as_str());
    }

    #[test]
    fn derivation_is_deterministic() {
        let a = derive_from_env_vm("e", Some("v"), DerivedRole::Tap, None).unwrap();
        let b = derive_from_env_vm("e", Some("v"), DerivedRole::Tap, None).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn role_distinguishes_bridge_vs_tap() {
        let br = derive_from_env_vm("e", Some("v"), DerivedRole::Bridge, None).unwrap();
        let tp = derive_from_env_vm("e", Some("v"), DerivedRole::Tap, None).unwrap();
        assert_ne!(br, tp);
    }

    #[test]
    fn vm_changes_derivation() {
        let a = derive_from_env_vm("e", Some("v1"), DerivedRole::Tap, None).unwrap();
        let b = derive_from_env_vm("e", Some("v2"), DerivedRole::Tap, None).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn ifname_too_long_rejected() {
        let too_long = "abcdefghijklmnop"; // 16 bytes
        let err = new_validated(too_long).unwrap_err();
        assert!(matches!(err, IfNameError::IfNameTooLong { len: 16, .. }));
    }

    #[test]
    fn ifname_invalid_character_rejected() {
        let err = new_validated("br work").unwrap_err();
        assert!(matches!(err, IfNameError::InvalidCharacter { .. }));
    }

    #[test]
    fn looks_nixling_owned_recognises_default_prefix() {
        let n = derive_from_env_vm("e", Some("v"), DerivedRole::Bridge, None).unwrap();
        assert!(looks_nixling_owned(n.as_str(), DEFAULT_PREFIX));
        assert!(!looks_nixling_owned("br-foreign-0", DEFAULT_PREFIX));
        assert!(!looks_nixling_owned("lo", DEFAULT_PREFIX));
    }

    #[test]
    fn invalid_prefix_rejected() {
        assert!(validate_prefix("").is_err());
        assert!(validate_prefix("no-trailing-dash").is_err());
        assert!(validate_prefix("with space-").is_err());
        assert!(validate_prefix("nl-").is_ok());
    }

    #[test]
    fn detect_collisions_passes_unique_set() {
        let mappings = vec![
            IfNameMapping {
                env: "e1".into(),
                vm: None,
                bridge: derive_from_env_vm("e1", None, DerivedRole::Bridge, None).unwrap(),
                tap: None,
                role: None,
            },
            IfNameMapping {
                env: "e2".into(),
                vm: None,
                bridge: derive_from_env_vm("e2", None, DerivedRole::Bridge, None).unwrap(),
                tap: None,
                role: None,
            },
        ];
        detect_collisions(&mappings).expect("unique");
    }

    #[test]
    fn detect_collisions_flags_duplicate_bridge() {
        let bridge = new_validated("nl-bAAAAAAAA").unwrap();
        let mappings = vec![
            IfNameMapping {
                env: "e1".into(),
                vm: None,
                bridge: bridge.clone(),
                tap: None,
                role: None,
            },
            IfNameMapping {
                env: "e2".into(),
                vm: None,
                bridge,
                tap: None,
                role: None,
            },
        ];
        let err = detect_collisions(&mappings).unwrap_err();
        match err {
            IfNameError::IfNameCollision(detail) => {
                assert_eq!(detail.ifname, "nl-bAAAAAAAA");
                assert_eq!(detail.mapping_a.env, "e1");
                assert_eq!(detail.mapping_b.env, "e2");
            }
            other => panic!("expected collision, got {other:?}"),
        }
    }

    #[test]
    fn detect_collisions_flags_bridge_vs_tap() {
        let n = new_validated("nl-bXXXXX").unwrap();
        let mappings = vec![
            IfNameMapping {
                env: "e".into(),
                vm: Some("v1".into()),
                bridge: n.clone(),
                tap: None,
                role: None,
            },
            IfNameMapping {
                env: "e".into(),
                vm: Some("v2".into()),
                bridge: new_validated("nl-bYYYYY").unwrap(),
                tap: Some(n),
                role: None,
            },
        ];
        let err = detect_collisions(&mappings).unwrap_err();
        assert!(matches!(err, IfNameError::IfNameCollision(_)));
    }

    /// W3fu3 H8 (test-1): the bash gate
    /// `tests/ifname-nix-rust-parity.sh` originally used a hardcoded
    /// shell regex (`^nl-[bt][0-9A-F]{8}$`) as the oracle, which would
    /// silently keep passing if a future Rust change tightened
    /// `looks_nixling_owned`. This test re-validates the same
    /// invariant against the real Rust predicate when the bash gate
    /// supplies a host.json path via the
    /// `NIXLING_IFNAME_PARITY_HOST_JSON` env var.
    ///
    /// W3fu4 H3 (test-1 from R4): when the env var IS set (i.e., this
    /// test is being driven by the bash parity gate), missing or empty
    /// `ifNameMappings` now panics instead of trivially passing. A
    /// regression that drops all Nix-emitted mappings used to escape
    /// both the bash and Rust sides of the gate; the bash side now
    /// fails closed first (see the parity script's empty-count guard),
    /// and this side fails closed second as the per-process oracle.
    /// When the env var is UNSET the test still returns early so plain
    /// `cargo test` runs remain unaffected.
    #[test]
    fn nix_emitted_ifnames_pass_looks_nixling_owned() {
        let path = match std::env::var("NIXLING_IFNAME_PARITY_HOST_JSON") {
            Ok(p) if !p.is_empty() => p,
            _ => return,
        };
        let json = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read host.json at {path}: {err}"));
        let value: serde_json::Value = serde_json::from_str(&json)
            .unwrap_or_else(|err| panic!("parse host.json at {path}: {err}"));
        let mappings = value
            .get("ifNameMappings")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| {
                panic!(
                    "host.json {path} missing `ifNameMappings` array; W3 emitter regression suspected"
                )
            });
        if mappings.is_empty() {
            panic!("host.json {path} has empty `ifNameMappings`; W3 emitter regression suspected");
        }
        let mut violations: Vec<String> = Vec::new();
        for row in mappings {
            let name = row
                .get("derivedIfname")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| panic!("ifNameMappings row missing derivedIfname: {row}"));
            if !looks_nixling_owned(name, DEFAULT_PREFIX) {
                violations.push(format!(
                    "{name} is not accepted by looks_nixling_owned(prefix={DEFAULT_PREFIX:?})"
                ));
            }
        }
        if !violations.is_empty() {
            panic!(
                "Nix-emitted ifnames failed Rust looks_nixling_owned ({} of {}):\n  {}",
                violations.len(),
                mappings.len(),
                violations.join("\n  ")
            );
        }
        eprintln!(
            "nix_emitted_ifnames_pass_looks_nixling_owned: {} derivedIfname values accepted",
            mappings.len()
        );
    }
}
