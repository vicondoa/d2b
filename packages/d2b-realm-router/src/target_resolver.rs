//! Realm entrypoint table + target resolution (`RealmTargetResolver`).
//!
//! Resolution is **policy, not address decoding**: the
//! [`d2b_realm_core::RealmTarget`] grammar never encodes whether a realm is
//! host-resident or gateway-backed — the entrypoint table does. Given a
//! parsed target, [`RealmEntrypointTable::resolve`] selects the realm
//! entrypoint by **longest-suffix match** over the target's realm path
//! (most-specific realm first) and returns a [`DispatchTarget`].
//!
//! Longest-suffix match means a parent realm's entrypoint owns its child
//! realms unless a more specific entry overrides it: a query for realm
//! `payments.work` matches a `work` entry (the parent) but a `payments.work`
//! entry, if present, wins. Resolution is **fail-closed**: a realm with no
//! matching entry returns [`ResolveError::NoEntrypoint`] rather than
//! defaulting to local dispatch.
//!
//! This module owns no provider or transport code: it reasons
//! only over the codec-neutral core target/realm types.

use std::collections::HashMap;

use d2b_realm_core::{EntrypointMode, RealmPath, RealmTarget};

/// One realm's entrypoint binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmEntrypoint {
    /// Where the realm's entrypoint runs.
    pub mode: EntrypointMode,
    /// The gateway-guest target for a [`EntrypointMode::GatewayBacked`]
    /// realm; `None` for a host-resident realm.
    pub gateway: Option<RealmTarget>,
}

impl RealmEntrypoint {
    /// A host-resident entrypoint (dispatched on the local `d2bd`).
    pub fn host_resident() -> Self {
        Self {
            mode: EntrypointMode::HostResident,
            gateway: None,
        }
    }

    /// A gateway-backed entrypoint fronted by `gateway`.
    pub fn gateway_backed(gateway: impl Into<RealmTarget>) -> Self {
        Self {
            mode: EntrypointMode::GatewayBacked,
            gateway: Some(gateway.into()),
        }
    }
}

/// The resolved dispatch decision for a target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchTarget {
    /// The realm entrypoint is this host: local `d2bd` applies the realm
    /// policy and then the local fast path.
    HostResident {
        /// The target being dispatched.
        target: RealmTarget,
    },
    /// The realm is fronted by a gateway guest; the host does not resolve
    /// nodes or workloads inside the realm.
    GatewayBacked {
        /// The realm gateway guest to route through.
        gateway: RealmTarget,
        /// The target being dispatched.
        target: RealmTarget,
    },
}

/// Why a target could not be resolved to a dispatch decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// No entrypoint-table entry matched the target's realm path. Fail-closed
    /// (resolution never defaults an unknown realm to local dispatch).
    NoEntrypoint(RealmPath),
    /// A gateway-backed entry was missing its gateway target (malformed
    /// table). Fail-closed.
    MissingGateway(RealmPath),
}

impl core::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ResolveError::NoEntrypoint(r) => {
                write!(f, "no realm entrypoint for `{}`", r.target_form())
            }
            ResolveError::MissingGateway(r) => write!(
                f,
                "gateway-backed realm `{}` has no gateway entrypoint",
                r.target_form()
            ),
        }
    }
}

impl std::error::Error for ResolveError {}

/// A local realm entrypoint table. Maps a realm path to its entrypoint
/// binding; [`resolve`](RealmEntrypointTable::resolve) does longest-suffix
/// matching so a parent realm's entry covers its children.
#[derive(Debug, Clone, Default)]
pub struct RealmEntrypointTable {
    entries: HashMap<RealmPath, RealmEntrypoint>,
}

impl RealmEntrypointTable {
    /// An empty table. A realm with no entry resolves fail-closed.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty table seeded with the reserved `local` realm as
    /// host-resident — the common case for host-local substrates.
    pub fn with_local_default() -> Self {
        let mut t = Self::new();
        t.insert(RealmPath::local(), RealmEntrypoint::host_resident());
        t
    }

    /// Insert or replace a realm's entrypoint binding.
    pub fn insert(&mut self, realm: RealmPath, entry: RealmEntrypoint) {
        self.entries.insert(realm, entry);
    }

    /// Mark a realm host-resident.
    pub fn host_resident(&mut self, realm: RealmPath) {
        self.insert(realm, RealmEntrypoint::host_resident());
    }

    /// Mark a realm gateway-backed, fronted by `gateway`.
    pub fn gateway_backed(&mut self, realm: RealmPath, gateway: impl Into<RealmTarget>) {
        self.insert(realm, RealmEntrypoint::gateway_backed(gateway));
    }

    /// Resolve `target` to a [`DispatchTarget`] by longest-suffix match over
    /// its realm path. Fail-closed on a realm with no entrypoint.
    pub fn resolve(&self, target: &RealmTarget) -> Result<DispatchTarget, ResolveError> {
        let labels = target.realm.labels();
        // labels are most-specific first; progressively drop the most-specific
        // labels to test successively shorter suffixes (parent realms). The
        // first hit is the longest match.
        for start in 0..labels.len() {
            let suffix = RealmPath::new(labels[start..].to_vec())
                .expect("a non-empty sub-slice of a valid realm path is a valid realm path");
            if let Some(entry) = self.entries.get(&suffix) {
                return match entry.mode {
                    EntrypointMode::HostResident => Ok(DispatchTarget::HostResident {
                        target: target.clone(),
                    }),
                    EntrypointMode::GatewayBacked => {
                        let gateway = entry
                            .gateway
                            .clone()
                            .ok_or_else(|| ResolveError::MissingGateway(suffix.clone()))?;
                        Ok(DispatchTarget::GatewayBacked {
                            gateway,
                            target: target.clone(),
                        })
                    }
                };
            }
        }
        Err(ResolveError::NoEntrypoint(target.realm.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_realm_core::RealmId;

    fn realm(labels: &[&str]) -> RealmPath {
        RealmPath::new(labels.iter().map(|l| RealmId::parse(*l).unwrap()).collect()).unwrap()
    }

    fn target(raw: &str) -> RealmTarget {
        RealmTarget::parse(raw).unwrap()
    }

    /// The realm-native example table.
    fn example_table() -> RealmEntrypointTable {
        let mut t = RealmEntrypointTable::with_local_default();
        t.host_resident(realm(&["personal"]));
        t.gateway_backed(realm(&["work"]), target("work-gateway.work.d2b"));
        t.gateway_backed(realm(&["ops"]), target("ops-gateway.ops.d2b"));
        t
    }

    #[test]
    fn local_realm_is_host_resident() {
        let t = example_table();
        let d = t.resolve(&target("demo.local.d2b")).unwrap();
        assert_eq!(
            d,
            DispatchTarget::HostResident {
                target: target("demo.local.d2b")
            }
        );
    }

    #[test]
    fn host_resident_named_realm() {
        let t = example_table();
        // dev-vm.personal.d2b -> personal -> host-resident.
        let d = t.resolve(&target("dev-vm.personal.d2b")).unwrap();
        assert!(matches!(d, DispatchTarget::HostResident { .. }));
    }

    #[test]
    fn gateway_backed_realm_returns_its_gateway() {
        let t = example_table();
        let d = t.resolve(&target("demo.work.d2b")).unwrap();
        match d {
            DispatchTarget::GatewayBacked {
                gateway,
                target: tgt,
            } => {
                assert_eq!(gateway, target("work-gateway.work.d2b"));
                assert_eq!(tgt, target("demo.work.d2b"));
            }
            other => panic!("expected gateway-backed, got {other:?}"),
        }
    }

    #[test]
    fn nested_child_realm_matches_parent_by_longest_suffix() {
        // No `payments.work` entry; the parent `work` (gateway-backed) owns it.
        let t = example_table();
        let d = t.resolve(&target("api.payments.work.d2b")).unwrap();
        assert!(matches!(d, DispatchTarget::GatewayBacked { .. }));
    }

    #[test]
    fn more_specific_child_entry_overrides_parent() {
        let mut t = example_table();
        // payments.work is host-resident even though work is gateway-backed.
        t.host_resident(realm(&["payments", "work"]));
        let d = t.resolve(&target("api.payments.work.d2b")).unwrap();
        assert!(matches!(d, DispatchTarget::HostResident { .. }));
        // a sibling child still falls through to the parent gateway.
        let sibling = t.resolve(&target("api.billing.work.d2b")).unwrap();
        assert!(matches!(sibling, DispatchTarget::GatewayBacked { .. }));
    }

    #[test]
    fn unknown_realm_fails_closed() {
        let t = example_table();
        let err = t.resolve(&target("x.unknown.d2b")).unwrap_err();
        assert_eq!(err, ResolveError::NoEntrypoint(realm(&["unknown"])));
    }

    #[test]
    fn gateway_backed_without_gateway_fails_closed() {
        let mut t = RealmEntrypointTable::new();
        t.insert(
            realm(&["work"]),
            RealmEntrypoint {
                mode: EntrypointMode::GatewayBacked,
                gateway: None,
            },
        );
        let err = t.resolve(&target("demo.work.d2b")).unwrap_err();
        assert_eq!(err, ResolveError::MissingGateway(realm(&["work"])));
    }
}
