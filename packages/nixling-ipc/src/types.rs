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
}
