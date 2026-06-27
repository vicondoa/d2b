//! `ApplyRoute` op.
//!
//! Deterministic owner table over [`RouteIntent`]; the actual netlink
//! mutations are delegated to a backend trait so the L1c canary
//! matrix can drive `route-preflight-no-default-route` and
//! `route-preflight-foreign-default-route` without `CAP_NET_ADMIN`.

use crate::live_handlers::LiveHandlerError;
use crate::ops::exec_reconcile::{IpRouteVerb, ReconcileExecError, ReconcileExecutor};
use d2b_core::bundle_resolver::ResolvedRouteIntent;
use d2b_core::host_w3::RouteIntent;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
pub struct ApplyRouteRequest {
    pub intents: Vec<RouteIntent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyRouteDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: Vec<String>,
}

/// Owner-table snapshot: every d2b-owned route key currently
/// installed. Routes outside this set are foreign and never touched.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnerTable {
    pub owned: BTreeMap<String, RouteIntent>,
}

impl OwnerTable {
    pub fn from_intents(intents: &[RouteIntent]) -> Self {
        let mut owned = BTreeMap::new();
        for intent in intents.iter().filter(|i| i.owned) {
            owned.insert(route_key(intent), intent.clone());
        }
        Self { owned }
    }

    pub fn diff_against(&self, next: &OwnerTable) -> ApplyRouteDiff {
        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut unchanged = Vec::new();
        for (key, intent) in &next.owned {
            match self.owned.get(key) {
                Some(prev) if prev == intent => unchanged.push(key.clone()),
                Some(_) => {
                    removed.push(key.clone());
                    added.push(key.clone());
                }
                None => added.push(key.clone()),
            }
        }
        for key in self.owned.keys() {
            if !next.owned.contains_key(key) {
                removed.push(key.clone());
            }
        }
        added.sort();
        added.dedup();
        removed.sort();
        removed.dedup();
        unchanged.sort();
        ApplyRouteDiff {
            added,
            removed,
            unchanged,
        }
    }
}

pub fn route_key(intent: &RouteIntent) -> String {
    format!(
        "{dest}|via={via}|dev={dev}|table={table}",
        dest = intent.destination,
        via = intent.via.as_deref().unwrap_or("-"),
        dev = intent.device.as_ref().map(|d| d.as_str()).unwrap_or("-"),
        table = intent.table.as_deref().unwrap_or("main"),
    )
}

/// `ApplyRoute` entry: re-runs the shared `d2b_host::routes`
/// preflight before producing the route diff. The runtime calls this
/// from the broker dispatcher for the `ApplyRoute` op and the
/// pre-VM-start hook. The preflight call is shared with
/// [`d2b_host::routes::run_route_preflight_for_vm`] so neither path
/// can skip the firewall coexistence predicate.
pub fn apply(
    request: &ApplyRouteRequest,
    preflight: &d2b_host::routes::RoutePreflightInput<'_>,
) -> Result<ApplyRouteDiff, d2b_host::routes::RoutePreflightError> {
    d2b_host::routes::run_route_preflight(preflight)?;
    let next = OwnerTable::from_intents(&request.intents);
    let prev = OwnerTable::default();
    Ok(prev.diff_against(&next))
}

/// Pre-VM-start hook: re-runs the shared preflight via
/// [`d2b_host::routes::run_route_preflight_for_vm`]. The broker
/// dispatcher calls this from the `Up` request path before any VM
/// startup so a foreign actor that mutated routes/nft after
/// `host prepare --apply` cannot let the VM come up.
pub fn pre_vm_start(
    vm_id: &str,
    preflight: &d2b_host::routes::RoutePreflightInput<'_>,
) -> Result<(), d2b_host::routes::RoutePreflightError> {
    d2b_host::routes::run_route_preflight_for_vm(vm_id, preflight)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteConflictKey {
    pub destination: String,
    pub via: Option<String>,
    pub device: Option<String>,
    pub metric: Option<String>,
    pub protocol: Option<String>,
    pub table: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyWithPreflightError {
    RouteQuery(ReconcileExecError),
    ConflictingRoute {
        existing: RouteConflictKey,
        requested: RouteConflictKey,
    },
    ReconcileExec(ReconcileExecError),
}

impl std::fmt::Display for ApplyWithPreflightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RouteQuery(err) => write!(f, "apply-route query: {err}"),
            Self::ConflictingRoute {
                existing,
                requested,
            } => write!(
                f,
                "apply-route preflight conflict for {}: existing {}, requested {}",
                requested.destination,
                format_route_conflict_key(existing),
                format_route_conflict_key(requested)
            ),
            Self::ReconcileExec(err) => write!(f, "apply-route: {err}"),
        }
    }
}

impl std::error::Error for ApplyWithPreflightError {}

/// Runtime entry-point for `ApplyRoute`.
///
/// Keep the live dispatcher anchored on `ops::route` so future route
/// ownership/coexistence work can harden this surface without another
/// runtime refactor. Today we preflight the current route set, refuse a
/// conflicting override, then call the live handler with `replace` for
/// owned intents and `add` for foreign ones. Host destroy sets
/// `destroy=true`, which skips the add/replace preflight and translates
/// directly to `ip route del`.
pub fn apply_with_preflight(
    executor: &dyn ReconcileExecutor,
    ip_binary: &Path,
    intent: &ResolvedRouteIntent,
    destroy: bool,
) -> Result<(), ApplyWithPreflightError> {
    if destroy {
        return crate::live_handlers::live_apply_route(
            executor,
            ip_binary,
            IpRouteVerb::Del,
            &intent.route_spec,
        )
        .map_err(map_live_route_error);
    }
    let observed_routes = read_existing_routes(ip_binary, intent)?;
    apply_with_preflight_from_routes(executor, ip_binary, intent, &observed_routes)
}

fn apply_with_preflight_from_routes(
    executor: &dyn ReconcileExecutor,
    ip_binary: &Path,
    intent: &ResolvedRouteIntent,
    observed_routes: &[RouteConflictKey],
) -> Result<(), ApplyWithPreflightError> {
    let requested = requested_route_conflict_key(intent);
    if let Some(existing) = observed_routes
        .iter()
        .find(|route| route.destination == requested.destination && *route != &requested)
    {
        return Err(ApplyWithPreflightError::ConflictingRoute {
            existing: existing.clone(),
            requested,
        });
    }

    let verb = if intent.owned {
        IpRouteVerb::Replace
    } else {
        IpRouteVerb::Add
    };
    crate::live_handlers::live_apply_route(executor, ip_binary, verb, &intent.route_spec)
        .map_err(map_live_route_error)
}

fn map_live_route_error(err: LiveHandlerError) -> ApplyWithPreflightError {
    match err {
        LiveHandlerError::ReconcileExec(inner) => ApplyWithPreflightError::ReconcileExec(inner),
        other => ApplyWithPreflightError::ReconcileExec(ReconcileExecError::InvalidInput {
            detail: other.to_string(),
        }),
    }
}

fn read_existing_routes(
    ip_binary: &Path,
    intent: &ResolvedRouteIntent,
) -> Result<Vec<RouteConflictKey>, ApplyWithPreflightError> {
    if !ip_binary.is_absolute() {
        return Err(ApplyWithPreflightError::RouteQuery(
            ReconcileExecError::InvalidInput {
                detail: format!("ip route binary must be absolute: {}", ip_binary.display()),
            },
        ));
    }

    let family_flag = if route_uses_ipv6(intent) { "-6" } else { "-4" };
    let output = Command::new(ip_binary)
        .args([family_flag, "-j", "route", "show", "table", "all"])
        .env_remove("NOTIFY_SOCKET")
        .stdin(Stdio::null())
        .output()
        .map_err(|err| {
            ApplyWithPreflightError::RouteQuery(ReconcileExecError::BinaryMissing {
                which: "ip route show".to_owned(),
                detail: err.to_string(),
            })
        })?;
    if !output.status.success() {
        return Err(ApplyWithPreflightError::RouteQuery(
            ReconcileExecError::NonZeroExit {
                which: "ip route show".to_owned(),
                exit_code: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            },
        ));
    }
    parse_observed_routes(&String::from_utf8_lossy(&output.stdout))
}

fn parse_observed_routes(
    route_show_output: &str,
) -> Result<Vec<RouteConflictKey>, ApplyWithPreflightError> {
    let routes = serde_json::from_str::<Vec<Value>>(route_show_output).map_err(|err| {
        ApplyWithPreflightError::RouteQuery(ReconcileExecError::InvalidInput {
            detail: format!("invalid ip -j route output: {err}"),
        })
    })?;
    Ok(routes.iter().filter_map(parse_observed_route).collect())
}

fn parse_observed_route(route: &Value) -> Option<RouteConflictKey> {
    let route = route.as_object()?;
    let destination = route
        .get("dst")
        .and_then(json_value_to_string)
        .unwrap_or_else(|| "default".to_owned());
    let via = route
        .get("gateway")
        .and_then(json_value_to_string)
        .or_else(|| route.get("via").and_then(route_via_to_string));
    let device = route.get("dev").and_then(json_value_to_string);
    let metric = route.get("metric").and_then(json_value_to_string);
    let protocol = route
        .get("protocol")
        .and_then(json_value_to_string)
        .or_else(|| route.get("proto").and_then(json_value_to_string));
    let table = normalize_table_name(
        route
            .get("table")
            .and_then(json_value_to_string)
            .as_deref()
            .unwrap_or("main"),
    );

    Some(RouteConflictKey {
        destination,
        via,
        device,
        metric,
        protocol,
        table,
    })
}

fn requested_route_conflict_key(intent: &ResolvedRouteIntent) -> RouteConflictKey {
    let tokens: Vec<_> = intent.route_spec.split_whitespace().collect();
    RouteConflictKey {
        destination: intent.destination.clone(),
        via: route_spec_value(&tokens, "via").or_else(|| intent.via.clone()),
        device: route_spec_value(&tokens, "dev").or_else(|| intent.device.clone()),
        metric: route_spec_value(&tokens, "metric"),
        protocol: route_spec_value(&tokens, "proto").or_else(|| Some("static".to_owned())),
        table: normalize_table_name(
            route_spec_value(&tokens, "table")
                .as_deref()
                .or(intent.table.as_deref())
                .unwrap_or("main"),
        ),
    }
}

fn route_uses_ipv6(intent: &ResolvedRouteIntent) -> bool {
    [Some(intent.destination.as_str()), intent.via.as_deref()]
        .into_iter()
        .flatten()
        .any(|value| value.contains(':'))
}

fn format_route_conflict_key(route: &RouteConflictKey) -> String {
    format!(
        "via={:?} dev={:?} metric={:?} proto={:?} table={}",
        route.via, route.device, route.metric, route.protocol, route.table
    )
}

fn route_spec_value(tokens: &[&str], key: &str) -> Option<String> {
    tokens.windows(2).find_map(|pair| {
        if pair[0] == key {
            Some(pair[1].to_owned())
        } else {
            None
        }
    })
}

fn route_via_to_string(value: &Value) -> Option<String> {
    json_value_to_string(value)
        .or_else(|| value.get("host").and_then(json_value_to_string))
        .or_else(|| value.get("addr").and_then(json_value_to_string))
}

fn json_value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn normalize_table_name(table: &str) -> String {
    match table {
        "254" => "main".to_owned(),
        "253" => "default".to_owned(),
        "255" => "local".to_owned(),
        other => other.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::exec_reconcile::{FakeReconcileExecutor, ReconcileOp};
    use d2b_core::host::IfName;

    fn ri(dest: &str, dev: Option<&str>) -> RouteIntent {
        RouteIntent {
            destination: dest.into(),
            via: None,
            device: dev.map(|d| IfName::new(d).unwrap()),
            table: None,
            owned: true,
        }
    }

    fn resolved_route_intent(owned: bool, via: Option<&str>) -> ResolvedRouteIntent {
        ResolvedRouteIntent {
            intent_id: if owned {
                "route:owned"
            } else {
                "route:foreign"
            }
            .to_owned(),
            destination: "10.0.0.0/24".to_owned(),
            via: via.map(ToOwned::to_owned),
            device: Some("tap0".to_owned()),
            table: Some("main".to_owned()),
            route_spec: match via {
                Some(gateway) => format!("10.0.0.0/24 via {gateway} dev tap0"),
                None => "10.0.0.0/24 dev tap0".to_owned(),
            },
            owned,
        }
    }

    fn observed_route(
        via: Option<&str>,
        dev: &str,
        metric: Option<&str>,
        proto: &str,
    ) -> RouteConflictKey {
        RouteConflictKey {
            destination: "10.0.0.0/24".to_owned(),
            via: via.map(ToOwned::to_owned),
            device: Some(dev.to_owned()),
            metric: metric.map(ToOwned::to_owned),
            protocol: Some(proto.to_owned()),
            table: "main".to_owned(),
        }
    }

    #[test]
    fn diff_detects_add_remove_unchanged() {
        let prev = OwnerTable::from_intents(&[ri("10.0.0.0/24", Some("d2b-bX"))]);
        let next = OwnerTable::from_intents(&[
            ri("10.0.0.0/24", Some("d2b-bX")),
            ri("10.0.1.0/24", Some("d2b-bY")),
        ]);
        let d = prev.diff_against(&next);
        assert_eq!(d.added.len(), 1);
        assert!(d.removed.is_empty());
        assert_eq!(d.unchanged.len(), 1);
    }

    #[test]
    fn route_key_is_deterministic() {
        let a = ri("10.0.0.0/24", Some("d2b-bX"));
        let b = ri("10.0.0.0/24", Some("d2b-bX"));
        assert_eq!(route_key(&a), route_key(&b));
    }

    #[test]
    fn only_owned_intents_are_tracked() {
        let mut intent = ri("0.0.0.0/0", Some("wlp0"));
        intent.owned = false;
        let table = OwnerTable::from_intents(&[intent]);
        assert!(table.owned.is_empty());
    }

    #[test]
    fn apply_with_preflight_uses_replace_for_owned_intents() {
        let exec = FakeReconcileExecutor::new();
        let intent = resolved_route_intent(true, Some("10.0.0.1"));
        apply_with_preflight_from_routes(&exec, Path::new("/usr/sbin/ip"), &intent, &[]).unwrap();
        let log = exec.take_log();
        assert_eq!(log.len(), 1);
        match &log[0] {
            ReconcileOp::IpRoute {
                binary,
                verb,
                route_spec,
            } => {
                assert!(binary.ends_with("ip"));
                assert_eq!(*verb, IpRouteVerb::Replace);
                assert_eq!(route_spec, &intent.route_spec);
            }
            other => panic!("unexpected op: {other:?}"),
        }
    }

    #[test]
    fn apply_with_preflight_uses_add_for_non_owned_intents() {
        let exec = FakeReconcileExecutor::new();
        let intent = resolved_route_intent(false, Some("10.0.0.1"));
        apply_with_preflight_from_routes(&exec, Path::new("/usr/sbin/ip"), &intent, &[]).unwrap();
        let log = exec.take_log();
        assert_eq!(log.len(), 1);
        match &log[0] {
            ReconcileOp::IpRoute { verb, .. } => assert_eq!(*verb, IpRouteVerb::Add),
            other => panic!("unexpected op: {other:?}"),
        }
    }

    #[test]
    fn apply_with_preflight_refuses_conflicting_route_attributes() {
        let exec = FakeReconcileExecutor::new();
        let mut intent = resolved_route_intent(true, Some("10.0.0.1"));
        intent.route_spec =
            "10.0.0.0/24 via 10.0.0.1 dev tap0 metric 100 proto static table main".to_owned();
        let err = apply_with_preflight_from_routes(
            &exec,
            Path::new("/usr/sbin/ip"),
            &intent,
            &[observed_route(
                Some("10.0.0.1"),
                "eth0",
                Some("50"),
                "static",
            )],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ApplyWithPreflightError::ConflictingRoute { existing, requested }
                if existing.device.as_deref() == Some("eth0")
                    && existing.metric.as_deref() == Some("50")
                    && requested.device.as_deref() == Some("tap0")
                    && requested.metric.as_deref() == Some("100")
                    && requested.protocol.as_deref() == Some("static")
                    && requested.table == "main"
        ));
        assert!(exec.take_log().is_empty());
    }

    #[test]
    fn parse_observed_routes_reads_json_route_fields() {
        let routes = parse_observed_routes(
            r#"[
                {
                    "dst": "10.0.0.0/24",
                    "via": "10.0.0.1",
                    "dev": "tap0",
                    "metric": 100,
                    "protocol": "static",
                    "table": 254
                },
                {
                    "dev": "eth0",
                    "via": "2001:db8::1",
                    "protocol": "ra",
                    "table": "main"
                }
            ]"#,
        )
        .expect("parse route json");
        assert_eq!(
            routes[0],
            RouteConflictKey {
                destination: "10.0.0.0/24".to_owned(),
                via: Some("10.0.0.1".to_owned()),
                device: Some("tap0".to_owned()),
                metric: Some("100".to_owned()),
                protocol: Some("static".to_owned()),
                table: "main".to_owned(),
            }
        );
        assert_eq!(routes[1].destination, "default");
        assert_eq!(routes[1].table, "main");
        assert_eq!(routes[1].protocol.as_deref(), Some("ra"));
    }
}
