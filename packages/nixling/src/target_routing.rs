//! CLI target routing (ADR 0032, P0).
//!
//! `nixling vm <verb> <target>` accepts either a **local** VM name (the v1
//! fast path, preserved exactly) or a **realm** target in the
//! `<workload>.<node>.<realm-path>.nixling` / `nl://…` form. This module
//! classifies the argument and, for realm targets, resolves it against the
//! realm entrypoint table to a dispatch decision.
//!
//! Invariants (per ADR 0032 + the host-no-realm-credentials boundary):
//! - A bare workload name (no node/realm) always routes **local** — the
//!   existing host-daemon fast path is never disturbed.
//! - An unknown realm fails **closed** (`NoRealmEntrypoint`); resolution never
//!   silently defaults an unconfigured realm to local dispatch.
//! - On a host-mode daemon the entrypoint table holds only the reserved
//!   `local` realm (the host carries no realm config), so any realm target
//!   surfaces a typed, actionable diagnostic rather than a host dispatch.

use nixling_constellation_core::{RealmPath, TargetName};
use nixling_constellation_router::{DispatchTarget, RealmEntrypointTable, ResolveError};

/// The routing decision for a `vm start/exec <target>` argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Dispatch through the existing host-daemon local fast path. Carries the
    /// workload name the local path expects.
    Local { vm: String },
    /// A realm target fronted by a gateway guest. The host cannot dispatch into
    /// the realm; the realm's gateway-mode `nixlingd` owns it.
    Gateway { gateway: String, target: String },
}

/// Why a target could not be routed (fail-closed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteError {
    /// `target` is a realm target but no entrypoint is configured for its
    /// realm on this daemon. Never defaults to local.
    NoRealmEntrypoint { target: String, realm: String },
    /// A gateway-backed realm whose table entry is missing its gateway target
    /// (malformed table).
    MissingGateway { target: String, realm: String },
}

impl core::fmt::Display for RouteError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RouteError::NoRealmEntrypoint { target, realm } => write!(
                f,
                "target `{target}` is in realm `{realm}`, which has no entrypoint on this daemon; \
                 a gateway-backed realm is dispatched by the realm gateway's nixlingd, not the host \
                 daemon (the host holds no realm configuration)"
            ),
            RouteError::MissingGateway { target, realm } => write!(
                f,
                "target `{target}` is in gateway-backed realm `{realm}`, but its entrypoint is \
                 missing a gateway address (malformed realm entrypoint table)"
            ),
        }
    }
}

/// Classify and resolve a `vm`/target argument against `table`.
///
/// A string that does not parse as a target address is treated as a local VM
/// name and passed through verbatim, so the local path keeps performing its own
/// name validation and reporting (`no such VM …`).
pub fn route(raw: &str, table: &RealmEntrypointTable) -> Result<Route, RouteError> {
    let target = match TargetName::parse(raw) {
        Ok(t) => t,
        // Not a parseable target address (e.g. a v1 bare name the local path
        // validates itself). Preserve the local fast path verbatim.
        Err(_) => {
            return Ok(Route::Local { vm: raw.to_owned() });
        }
    };

    // A bare workload (default `this` node + reserved `local` realm) is the v1
    // local fast path.
    if target.node_is_this() && target.realm == RealmPath::local() {
        return Ok(Route::Local {
            vm: target.workload.as_str().to_owned(),
        });
    }

    match table.resolve(&target) {
        Ok(DispatchTarget::HostResident { target }) => Ok(Route::Local {
            vm: target.workload.as_str().to_owned(),
        }),
        Ok(DispatchTarget::GatewayBacked { gateway, target }) => Ok(Route::Gateway {
            gateway: gateway.to_string(),
            target: target.to_string(),
        }),
        Err(ResolveError::NoEntrypoint(realm)) => Err(RouteError::NoRealmEntrypoint {
            target: target.to_string(),
            realm: realm.target_form(),
        }),
        Err(ResolveError::MissingGateway(realm)) => Err(RouteError::MissingGateway {
            target: target.to_string(),
            realm: realm.target_form(),
        }),
    }
}

/// Return the canonical gateway target for a fully-qualified non-local target.
/// Bare/local names return `None` and stay on the local fast path.
pub fn gateway_candidate(raw: &str) -> Option<String> {
    let target = TargetName::parse(raw).ok()?;
    if target.node_is_this() && target.realm == RealmPath::local() {
        None
    } else {
        Some(target.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_constellation_core::{RealmId, TargetName};

    fn realm(labels: &[&str]) -> RealmPath {
        RealmPath::new(labels.iter().map(|l| RealmId::parse(*l).unwrap()).collect()).unwrap()
    }

    #[test]
    fn bare_name_is_local_fast_path() {
        let table = RealmEntrypointTable::with_local_default();
        assert_eq!(
            route("work-aca", &table).unwrap(),
            Route::Local {
                vm: "work-aca".to_owned()
            }
        );
    }

    #[test]
    fn unparseable_passes_through_to_local() {
        // A 3-label form without the `.nixling` suffix is not a valid target
        // address; the local path handles (and rejects) it as a VM name.
        let table = RealmEntrypointTable::with_local_default();
        assert_eq!(
            route("demo.aca.work", &table).unwrap(),
            Route::Local {
                vm: "demo.aca.work".to_owned()
            }
        );
    }

    #[test]
    fn host_default_table_fails_closed_on_a_realm_target() {
        // The host daemon's table only knows the reserved `local` realm, so a
        // `work` realm target is rejected fail-closed (never silently local).
        let table = RealmEntrypointTable::with_local_default();
        let err = route("demo.gw.work.nixling", &table).unwrap_err();
        match err {
            RouteError::NoRealmEntrypoint { realm, .. } => assert_eq!(realm, "work"),
            other => panic!("expected NoRealmEntrypoint, got {other:?}"),
        }
    }

    #[test]
    fn gateway_backed_realm_routes_to_its_gateway() {
        let mut table = RealmEntrypointTable::with_local_default();
        let gateway = TargetName::parse("gw.host.work.nixling").unwrap();
        table.gateway_backed(realm(&["work"]), gateway);
        let r = route("demo.gw.work.nixling", &table).unwrap();
        match r {
            Route::Gateway { gateway, target } => {
                assert!(gateway.contains("gw"));
                assert!(target.contains("demo"));
            }
            other => panic!("expected Gateway, got {other:?}"),
        }
    }

    #[test]
    fn host_resident_realm_routes_local() {
        let mut table = RealmEntrypointTable::new();
        table.host_resident(realm(&["work"]));
        let r = route("demo.gw.work.nixling", &table).unwrap();
        assert_eq!(
            r,
            Route::Local {
                vm: "demo".to_owned()
            }
        );
    }

    #[test]
    fn gateway_candidate_detects_fully_qualified_non_local() {
        assert_eq!(gateway_candidate("vm-a"), None);
        assert_eq!(gateway_candidate("demo.aca.work"), None);
        assert_eq!(
            gateway_candidate("demo.gw.work.nixling").as_deref(),
            Some("nl://demo.gw.work.nixling")
        );
    }
}
