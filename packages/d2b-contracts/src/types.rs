//! Opaque identifier newtypes for the broker wire contract.
//!
//! These identifiers prevent the daemon from passing authority-bearing
//! payloads to the broker. Every mutating broker request carries opaque
//! IDs that the broker resolves against its **own trusted copy of the
//! bundle**. The daemon never names raw paths, raw uids/gids, raw argv,
//! raw nft rule text, raw routes or raw sysctl values - those derive
//! exclusively from the broker-side `Bundle::find_*_intent` lookups
//! anchored by these IDs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

macro_rules! opaque_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Debug,
            Clone,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Serialize,
            Deserialize,
            JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

opaque_id! {
    /// Opaque identifier for a launching/admin caller subject. The
    /// broker resolves this against the bundle's `subjects` table -
    /// it is never a raw uid/gid on the wire.
    SubjectId
}

opaque_id! {
    /// Opaque identifier for an authorization scope (env, VM, role,
    /// global). The broker resolves this against the bundle's
    /// `scopes` table.
    ScopeId
}

opaque_id! {
    /// Opaque identifier for a per-VM authorization scope. Resolved
    /// against `bundle.vms[<vm_id>]`. The VM name string is
    /// derivation-internal; the daemon should not synthesize one.
    VmId
}

opaque_id! {
    /// Opaque identifier for a per-role authorization scope (a
    /// minijailed runner role inside a VM).
    RoleId
}

opaque_id! {
    /// Opaque identifier for a single trusted-bundle intent row
    /// (an `NftIntent`, `RouteIntent`, `SysctlIntent`,
    /// `NmUnmanagedEntry`, `HostsEntry`, etc.). The broker uses this
    /// to look up the typed intent in its own bundle copy; the
    /// daemon never passes inline rule text, route specs, sysctl
    /// values, ifname sets, or hosts entries. Bundle-derived intents
    /// are the only authority for ApplyNftables / ApplyRoute /
    /// ApplySysctl / ApplyNmUnmanaged / UpdateHostsFile.
    BundleOpId
}

opaque_id! {
    /// Opaque identifier for a single tracing span chained through
    /// the broker. Used purely for audit correlation.
    TracingSpanId
}

opaque_id! {
    /// Opaque identifier for a per-VM store-view closure intent row
    /// (resolved against `BundleResolver::find_store_view_intent`
    /// keyed by Zone and VM).
    /// The daemon never names raw `/nix/store` closure paths on
    /// the wire - only this reference. Canonical form is the
    /// `intent_id_store_view(zone, vm)` string
    /// (`"store-view:zone:<zone>:vm:<vm>"`).
    BundleClosureRef
}

opaque_id! {
    /// Opaque qemu-media reference declared in public config. The broker
    /// resolves it against the trusted bundle and root-only runtime registry;
    /// callers never pass by-id paths, serials, block paths, or image paths.
    MediaRef
}

/// Failure classes for [`MediaRef`] shape validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaRefError {
    Empty,
    TooLong { max: usize },
    BadStart,
    BadShape,
}

impl core::fmt::Display for MediaRefError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MediaRefError::Empty => f.write_str("media ref must not be empty"),
            MediaRefError::TooLong { max } => write!(f, "media ref must be at most {max} bytes"),
            MediaRefError::BadStart => {
                f.write_str("media ref must start with a lowercase ASCII letter")
            }
            MediaRefError::BadShape => f.write_str(
                "media ref may contain only lowercase ASCII letters, digits, and '-'",
            ),
        }
    }
}

impl std::error::Error for MediaRefError {}

impl MediaRef {
    /// Validate a media-ref spelling. Returns [`MediaRefError`] on malformed
    /// input (fail-closed).
    pub fn validate_value(value: &str) -> Result<(), MediaRefError> {
        if value.is_empty() {
            return Err(MediaRefError::Empty);
        }
        if value.len() > 63 {
            return Err(MediaRefError::TooLong { max: 63 });
        }
        let mut chars = value.chars();
        let first = chars.next().ok_or(MediaRefError::Empty)?;
        if !first.is_ascii_lowercase() {
            return Err(MediaRefError::BadStart);
        }
        if !std::iter::once(first)
            .chain(chars)
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
        {
            return Err(MediaRefError::BadShape);
        }
        Ok(())
    }
}

/// Parse-gate construction: a media ref can only be built from a spelling
/// that passes [`MediaRef::validate_value`].
impl TryFrom<&str> for MediaRef {
    type Error = MediaRefError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::validate_value(value)?;
        Ok(Self(value.to_owned()))
    }
}

/// Failure classes for USB bus-id shape validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbBusIdError {
    Empty,
    TooLong { max: usize },
    InvalidEdgePunctuation,
    InvalidCharacter,
    MissingSeparator,
}

impl core::fmt::Display for UsbBusIdError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            UsbBusIdError::Empty => f.write_str("USB busid must not be empty"),
            UsbBusIdError::TooLong { max } => write!(f, "USB busid must be at most {max} bytes"),
            UsbBusIdError::InvalidEdgePunctuation => {
                f.write_str("USB busid has invalid edge punctuation")
            }
            UsbBusIdError::InvalidCharacter => {
                f.write_str("USB busid may contain only digits, '-' and '.'")
            }
            UsbBusIdError::MissingSeparator => {
                f.write_str("USB busid must include a bus-port separator '-'")
            }
        }
    }
}

impl std::error::Error for UsbBusIdError {}

pub fn validate_usb_bus_id(value: &str) -> Result<(), UsbBusIdError> {
    if value.is_empty() {
        return Err(UsbBusIdError::Empty);
    }
    if value.len() > 64 {
        return Err(UsbBusIdError::TooLong { max: 64 });
    }
    if value.starts_with('-') || value.ends_with('-') || value.ends_with('.') {
        return Err(UsbBusIdError::InvalidEdgePunctuation);
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_digit() || ch == '-' || ch == '.')
    {
        return Err(UsbBusIdError::InvalidCharacter);
    }
    if !value.contains('-') {
        return Err(UsbBusIdError::MissingSeparator);
    }
    Ok(())
}

/// Path classifier for the `PrepareStateDir` / `PrepareRuntimeDir` broker
/// requests declared in `d2b-contracts-broker`'s `broker_wire` module.
/// The broker derives the concrete path from the bundle anchored by
/// `vm_id` + `path_class`; the daemon never passes a raw path.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum PathClass {
    /// Per-VM state directory under `/var/lib/d2b/vms/<vm>/`.
    Vm,
    /// Per-VM runtime directory under `/run/d2b/<vm>/`.
    Runtime,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_ids_are_transparent_strings() {
        let bid = BundleOpId::new("nft-intent-corp-egress");
        let json = serde_json::to_string(&bid).expect("serialize");
        assert_eq!(json, "\"nft-intent-corp-egress\"");
        let parsed: BundleOpId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, bid);
    }

    #[test]
    fn path_class_serializes_kebab_case() {
        assert_eq!(
            serde_json::to_string(&PathClass::Runtime).expect("serialize"),
            "\"runtime\""
        );
    }

    #[test]
    fn validates_media_refs_and_usb_busids() {
        assert!(MediaRef::validate_value("installer-usb").is_ok());
        assert!(MediaRef::validate_value("/dev/disk/by-id/secret").is_err());
        assert!(validate_usb_bus_id("1-2.3").is_ok());
        assert!(validate_usb_bus_id("/dev/sda").is_err());
    }
}
