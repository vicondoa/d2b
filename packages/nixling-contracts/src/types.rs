//! Opaque identifier newtypes for the broker wire contract.
//!
//! These identifiers prevent the daemon from passing authority-bearing
//! payloads to the broker. Every mutating broker request carries opaque
//! IDs that the broker resolves against its **own trusted copy of the
//! bundle**. The daemon never names raw paths, raw uids/gids, raw argv,
//! raw nft rule text, raw routes or raw sysctl values — those derive
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
    /// broker resolves this against the bundle's `subjects` table —
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
    /// keyed by VM).
    /// The daemon never names raw `/nix/store` closure paths on
    /// the wire — only this reference. Canonical form is the
    /// `intent_id_store_view(vm)` string (`"store-view:vm:<vm>"`).
    BundleClosureRef
}

opaque_id! {
    /// Opaque qemu-media reference declared in public config. The broker
    /// resolves it against the trusted bundle and root-only runtime registry;
    /// callers never pass by-id paths, serials, block paths, or image paths.
    MediaRef
}

impl MediaRef {
    pub fn validate_value(value: &str) -> Result<(), String> {
        if value.is_empty() {
            return Err("media ref must not be empty".to_owned());
        }
        if value.len() > 63 {
            return Err("media ref must be at most 63 bytes".to_owned());
        }
        let mut chars = value.chars();
        let first = chars
            .next()
            .ok_or_else(|| "media ref must not be empty".to_owned())?;
        if !first.is_ascii_lowercase() {
            return Err("media ref must start with a lowercase ASCII letter".to_owned());
        }
        if !std::iter::once(first)
            .chain(chars)
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
        {
            return Err(
                "media ref may contain only lowercase ASCII letters, digits, and '-'".to_owned(),
            );
        }
        Ok(())
    }
}

pub fn validate_usb_bus_id(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("USB busid must not be empty".to_owned());
    }
    if value.len() > 64 {
        return Err("USB busid must be at most 64 bytes".to_owned());
    }
    if value.starts_with('-') || value.ends_with('-') || value.ends_with('.') {
        return Err("USB busid has invalid edge punctuation".to_owned());
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_digit() || ch == '-' || ch == '.')
    {
        return Err("USB busid may contain only digits, '-' and '.'".to_owned());
    }
    if !value.contains('-') {
        return Err("USB busid must include a bus-port separator '-'".to_owned());
    }
    Ok(())
}

/// Path classifier for [`PrepareStateDir`] / [`PrepareRuntimeDir`].
/// The broker derives the concrete path from the bundle anchored by
/// `vm_id` + `path_class`; the daemon never passes a raw path.
///
/// [`PrepareStateDir`]: crate::broker_wire::BrokerRequest::PrepareStateDir
/// [`PrepareRuntimeDir`]: crate::broker_wire::BrokerRequest::PrepareRuntimeDir
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum PathClass {
    /// Per-VM state directory under `/var/lib/nixling/vms/<vm>/`.
    Vm,
    /// Per-VM runtime directory under `/run/nixling/<vm>/`.
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
