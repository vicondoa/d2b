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

use nixling_constellation_core::{RealmId, RealmPath, TargetName};
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
    /// The conventional local gateway VM name for a realm cannot be represented
    /// as a target address.
    InvalidGatewayTarget {
        realm: String,
        gateway: String,
        reason: String,
    },
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
            RouteError::InvalidGatewayTarget {
                realm,
                gateway,
                reason,
            } => write!(
                f,
                "realm `{realm}` maps to gateway VM `{gateway}`, but that gateway target is \
                 invalid: {reason}"
            ),
        }
    }
}

/// Why a `nixling realm <verb> <realm>` argument was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealmArgError {
    Empty,
    BadLabel { label: String, reason: String },
    BadPath { realm: String },
}

impl core::fmt::Display for RealmArgError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RealmArgError::Empty => write!(f, "realm path is empty"),
            RealmArgError::BadLabel { label, reason } => {
                write!(f, "realm label `{label}` is invalid: {reason}")
            }
            RealmArgError::BadPath { realm } => {
                write!(
                    f,
                    "realm path `{realm}` is empty, too long, or has too many labels"
                )
            }
        }
    }
}

/// A target's conventional local gateway entrypoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayHint {
    pub target: String,
    pub realm: RealmPath,
    pub gateway_vm: String,
    pub gateway_target: String,
}

/// Parse a CLI realm argument (`work`, `payments.work`) into a realm path.
pub fn parse_realm_arg(raw: &str) -> Result<RealmPath, RealmArgError> {
    if raw.is_empty() {
        return Err(RealmArgError::Empty);
    }
    let labels = raw
        .split('.')
        .map(|label| {
            RealmId::parse(label).map_err(|err| RealmArgError::BadLabel {
                label: label.to_owned(),
                reason: err.to_string(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    RealmPath::new(labels).ok_or_else(|| RealmArgError::BadPath {
        realm: raw.to_owned(),
    })
}

/// Conventional local gateway VM name for a realm (`work` ->
/// `sys-work-gateway`).
pub fn gateway_vm_name(realm: &RealmPath) -> String {
    format!("sys-{}-gateway", realm.target_form().replace('.', "-"))
}

/// Conventional local gateway target address for a realm gateway VM.
pub fn gateway_target_name(realm: &RealmPath) -> Result<TargetName, RouteError> {
    let gateway = gateway_vm_name(realm);
    let raw = format!("{gateway}.nixling");
    TargetName::parse(&raw).map_err(|err| RouteError::InvalidGatewayTarget {
        realm: realm.target_form(),
        gateway,
        reason: err.to_string(),
    })
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
#[cfg(test)]
pub fn gateway_candidate(raw: &str) -> Option<String> {
    let target = TargetName::parse(raw).ok()?;
    if target.node_is_this() && target.realm == RealmPath::local() {
        None
    } else {
        Some(target.to_string())
    }
}

/// Return the realm/gateway hint for a fully-qualified non-local target.
/// Bare/local names return `None` and stay on the local fast path.
pub fn gateway_hint(raw: &str) -> Result<Option<GatewayHint>, RouteError> {
    let target = match TargetName::parse(raw) {
        Ok(target) => target,
        Err(_) => return Ok(None),
    };
    if target.node_is_this() && target.realm == RealmPath::local() {
        return Ok(None);
    }
    let realm = target.realm.clone();
    let gateway_vm = gateway_vm_name(&realm);
    let gateway_target = gateway_target_name(&realm)?;
    Ok(Some(GatewayHint {
        target: target.to_string(),
        realm,
        gateway_vm,
        gateway_target: gateway_target.to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_constellation_core::{EntrypointMode, RealmId, TargetName};
    use nixling_constellation_router::RealmEntrypoint;

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

    #[test]
    fn realm_arg_and_gateway_name_follow_gateway_vm_convention() {
        let work = parse_realm_arg("work").unwrap();
        assert_eq!(work.target_form(), "work");
        assert_eq!(gateway_vm_name(&work), "sys-work-gateway");
        assert_eq!(
            gateway_target_name(&work).unwrap().to_string(),
            "nl://sys-work-gateway.this.local.nixling"
        );

        let nested = parse_realm_arg("payments.work").unwrap();
        assert_eq!(nested.target_form(), "payments.work");
        assert_eq!(gateway_vm_name(&nested), "sys-payments-work-gateway");
    }

    #[test]
    fn realm_arg_rejects_bad_labels() {
        assert!(matches!(parse_realm_arg(""), Err(RealmArgError::Empty)));
        assert!(matches!(
            parse_realm_arg("Work"),
            Err(RealmArgError::BadLabel { .. })
        ));
        assert!(matches!(
            parse_realm_arg("work."),
            Err(RealmArgError::BadLabel { .. })
        ));
    }

    #[test]
    fn gateway_hint_describes_gateway_backed_target_without_routing_it() {
        let hint = gateway_hint("demo.aca.work.nixling")
            .unwrap()
            .expect("realm target has a gateway hint");
        assert_eq!(hint.target, "nl://demo.aca.work.nixling");
        assert_eq!(hint.realm.target_form(), "work");
        assert_eq!(hint.gateway_vm, "sys-work-gateway");
        assert_eq!(
            hint.gateway_target,
            "nl://sys-work-gateway.this.local.nixling"
        );
        assert!(gateway_hint("vm-a").unwrap().is_none());
        assert!(gateway_hint("demo.aca.work").unwrap().is_none());
    }

    #[test]
    fn malformed_gateway_backed_entry_without_gateway_fails_closed() {
        let mut table = RealmEntrypointTable::new();
        table.insert(
            realm(&["work"]),
            RealmEntrypoint {
                mode: EntrypointMode::GatewayBacked,
                gateway: None,
            },
        );
        let err = route("demo.gw.work.nixling", &table).unwrap_err();
        match err {
            RouteError::MissingGateway { realm, .. } => assert_eq!(realm, "work"),
            other => panic!("expected MissingGateway, got {other:?}"),
        }
    }
}
