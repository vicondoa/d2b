//! `ApplyRoute` op.
//!
//! The broker resolves each route from a trusted Network intent, compares it
//! with observable kernel tuples, and records the exact ownership provenance
//! required for later replacement or deletion.

use crate::live_handlers::LiveHandlerError;
use crate::ops::exec_reconcile::{IpRouteVerb, ReconcileExecError, ReconcileExecutor};
use d2b_core::bundle_resolver::ResolvedRouteIntent;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::process::{Command, Stdio};

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
    /// A Network route was not marked as d2b-owned.
    ForeignRoute,
    ReconcileExec(ReconcileExecError),
}

impl std::fmt::Display for ApplyWithPreflightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RouteQuery(err) => write!(f, "apply-route query: {err}"),
            Self::ForeignRoute => write!(f, "apply-route: foreign route"),
            Self::ReconcileExec(err) => write!(f, "apply-route: {err}"),
        }
    }
}

impl std::error::Error for ApplyWithPreflightError {}

/// Production route entry-point with a durable ownership ledger.
///
/// Kernel routes have no portable arbitrary ownership marker. The broker
/// therefore records the exact UID-bound route tuple and marker before it
/// will replace or delete an existing route. A present route without a
/// matching record is foreign and remains untouched.
pub fn apply_with_preflight_owned(
    executor: &dyn ReconcileExecutor,
    ip_binary: &Path,
    state_dir: &Path,
    intent: &ResolvedRouteIntent,
    provenance: &d2b_contracts_resource::v3::NetworkProvenance,
    destroy: bool,
) -> Result<(), ApplyWithPreflightError> {
    let Some(route_name) = intent.route_name.as_deref() else {
        return Err(ApplyWithPreflightError::ForeignRoute);
    };
    let Some(marker) = intent.ownership_marker.as_deref() else {
        return Err(ApplyWithPreflightError::ForeignRoute);
    };
    if intent.provenance.as_ref() != Some(provenance) {
        return Err(ApplyWithPreflightError::ForeignRoute);
    }
    if route_name.is_empty()
        || route_name.len() > 64
        || !route_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ApplyWithPreflightError::ForeignRoute);
    }
    let expected_marker = format!(
        "d2b managed: {}",
        d2b_contracts_resource::v3::derive_network_ownership_marker(
            provenance,
            &format!("route:{route_name}"),
        )
    );
    if marker != expected_marker {
        return Err(ApplyWithPreflightError::ForeignRoute);
    }
    let intent_provenance = intent
        .provenance
        .as_ref()
        .ok_or(ApplyWithPreflightError::ForeignRoute)?;
    let root = state_dir.join("network-routes");
    let record_path = root.join(format!("{route_name}.json"));
    let expected = RouteOwnershipRecord {
        route_name: route_name.to_owned(),
        marker: marker.to_owned(),
        destination: intent.destination.clone(),
        via: intent.via.clone(),
        device: intent.device.clone(),
        table: intent.table.clone().unwrap_or_else(|| "main".to_owned()),
        provenance: intent_provenance.clone(),
    };
    let current = read_route_record(&record_path)?;
    let observed_routes = read_existing_routes(ip_binary, intent)?;
    let requested = requested_route_conflict_key(intent);
    let conflict = observed_routes
        .iter()
        .find(|route| route_conflicts(route, &requested));
    validate_route_state(current.as_ref(), &expected, conflict)?;

    // A destroy of an already-absent route needs no ledger creation or
    // mutation. In particular, an unmarked pre-Zone route must not leave a
    // broker-side artifact behind while refusing the request.
    if destroy && current.is_none() && conflict.is_none() {
        return Ok(());
    }

    ensure_route_ledger_root(&root)?;
    let _lock = acquire_route_ledger_lock(&root)?;
    let current = read_route_record(&record_path)?;
    let observed_routes = read_existing_routes(ip_binary, intent)?;
    let exact = observed_routes.iter().any(|route| {
        route.destination == requested.destination
            && route.via == requested.via
            && route.device == requested.device
            && route.table == requested.table
    });
    let conflict = observed_routes
        .iter()
        .find(|route| route_conflicts(route, &requested));
    validate_route_state(current.as_ref(), &expected, conflict)?;

    if destroy {
        if conflict.is_some() {
            crate::live_handlers::live_apply_route(
                executor,
                ip_binary,
                IpRouteVerb::Del,
                &intent.route_spec,
            )
            .map_err(map_live_route_error)?;
            remove_route_record(&record_path)?;
        } else if current.is_some() {
            remove_route_record(&record_path)?;
        }
        return Ok(());
    }

    if conflict.is_some() {
        crate::live_handlers::live_apply_route(
            executor,
            ip_binary,
            IpRouteVerb::Replace,
            &intent.route_spec,
        )
        .map_err(map_live_route_error)?;
        write_route_record(&record_path, &expected)?;
        return Ok(());
    }

    let verb = if exact {
        IpRouteVerb::Replace
    } else {
        IpRouteVerb::Add
    };
    crate::live_handlers::live_apply_route(executor, ip_binary, verb, &intent.route_spec)
        .map_err(map_live_route_error)?;
    // A route has no kernel ownership marker. If the durable marker cannot
    // be recorded, do not issue an unproven delete that could race with a
    // foreign route; the next reconcile will fail closed on the tuple.
    write_route_record(&record_path, &expected)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RouteOwnershipRecord {
    route_name: String,
    marker: String,
    destination: String,
    via: Option<String>,
    device: Option<String>,
    table: String,
    provenance: d2b_contracts_resource::v3::NetworkProvenance,
}

fn ensure_route_ledger_root(root: &Path) -> Result<(), ApplyWithPreflightError> {
    match fs::symlink_metadata(root) {
        Ok(metadata) => {
            if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.mode() & 0o022 != 0
            {
                return Err(ApplyWithPreflightError::ForeignRoute);
            }

            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(root).map_err(|error| {
                ApplyWithPreflightError::RouteQuery(ReconcileExecError::Io {
                    path: root.display().to_string(),
                    detail: error.to_string(),
                })
            })?;
            fs::set_permissions(
                root,
                std::os::unix::fs::PermissionsExt::from_mode(0o750),
            )
            .map_err(|error| {
                ApplyWithPreflightError::RouteQuery(ReconcileExecError::Io {
                    path: root.display().to_string(),
                    detail: error.to_string(),
                })
            })?;
            Ok(())
        }
        Err(error) => Err(ApplyWithPreflightError::RouteQuery(
            ReconcileExecError::Io {
                path: root.display().to_string(),
                detail: error.to_string(),
            },
        )),
    }
}

fn acquire_route_ledger_lock(root: &Path) -> Result<fs::File, ApplyWithPreflightError> {
    let path = root.join(".lock");
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW)
        .mode(0o640)
        .open(&path)
        .map_err(|error| {
            ApplyWithPreflightError::RouteQuery(ReconcileExecError::Io {
                path: path.display().to_string(),
                detail: error.to_string(),
            })
        })?;
    let lock = nix::libc::flock {
        l_type: nix::libc::F_WRLCK as _,
        l_whence: nix::libc::SEEK_SET as _,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    nix::fcntl::fcntl(
        file.as_raw_fd(),
        nix::fcntl::FcntlArg::F_OFD_SETLKW(&lock),
    )
    .map_err(|_| ApplyWithPreflightError::ForeignRoute)?;
    Ok(file)
}

fn read_route_record(
    path: &Path,
) -> Result<Option<RouteOwnershipRecord>, ApplyWithPreflightError> {
    match fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => {
            let metadata = file.metadata().map_err(|_| ApplyWithPreflightError::ForeignRoute)?;
            if !metadata.is_file() || metadata.mode() & 0o022 != 0 {
                return Err(ApplyWithPreflightError::ForeignRoute);
            }
            serde_json::from_reader(file)
                .map(Some)
                .map_err(|_| ApplyWithPreflightError::ForeignRoute)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ApplyWithPreflightError::RouteQuery(
            ReconcileExecError::Io {
                path: path.display().to_string(),
                detail: error.to_string(),
            },
        )),
    }
}

fn write_route_record(
    path: &Path,
    record: &RouteOwnershipRecord,
) -> Result<(), ApplyWithPreflightError> {
    let bytes = serde_json::to_vec(record).map_err(|_| ApplyWithPreflightError::ForeignRoute)?;
    let temp = path.with_extension("json.tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW)
        .mode(0o640)
        .open(&temp)
        .map_err(|error| {
            ApplyWithPreflightError::RouteQuery(ReconcileExecError::Io {
                path: temp.display().to_string(),
                detail: error.to_string(),
            })
        })?;
    use std::io::Write as _;
    if file.write_all(&bytes).is_err() || file.sync_data().is_err() {
        let _ = fs::remove_file(&temp);
        return Err(ApplyWithPreflightError::ForeignRoute);
    }
    drop(file);
    fs::rename(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        ApplyWithPreflightError::RouteQuery(ReconcileExecError::Io {
            path: path.display().to_string(),
            detail: error.to_string(),
        })
    })?;
    if let Some(parent) = path.parent() {
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| {
                ApplyWithPreflightError::RouteQuery(ReconcileExecError::Io {
                    path: parent.display().to_string(),
                    detail: error.to_string(),
                })
            })?;
    }
    Ok(())
}

fn remove_route_record(path: &Path) -> Result<(), ApplyWithPreflightError> {
    match fs::remove_file(path) {
        Ok(()) => {
            if let Some(parent) = path.parent() {
                fs::File::open(parent)
                    .and_then(|directory| directory.sync_all())
                    .map_err(|error| {
                        ApplyWithPreflightError::RouteQuery(ReconcileExecError::Io {
                            path: parent.display().to_string(),
                            detail: error.to_string(),
                        })
                    })?;
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ApplyWithPreflightError::RouteQuery(
            ReconcileExecError::Io {
                path: path.display().to_string(),
                detail: error.to_string(),
            },
        )),
    }
}

fn route_matches_record(route: &RouteConflictKey, record: &RouteOwnershipRecord) -> bool {
    route.destination == record.destination
        && route.via == record.via
        && route.device == record.device
        && route.table == normalize_table_name(&record.table)
}

fn validate_route_state(
    current: Option<&RouteOwnershipRecord>,
    expected: &RouteOwnershipRecord,
    conflict: Option<&RouteConflictKey>,
) -> Result<(), ApplyWithPreflightError> {
    if current.is_some_and(|record| record != expected) {
        return Err(ApplyWithPreflightError::ForeignRoute);
    }
    if conflict.is_some_and(|existing| {
        current != Some(expected) || !route_matches_record(existing, expected)
    }) {
        return Err(ApplyWithPreflightError::ForeignRoute);
    }
    Ok(())
}

fn route_conflicts(existing: &RouteConflictKey, requested: &RouteConflictKey) -> bool {
    if existing.table != requested.table {
        return false;
    }
    if existing.destination == requested.destination {
        return true;
    }
    let Some(existing_cidr) =
        d2b_contracts_resource::v3::network::Ipv4Cidr::parse(existing.destination.clone()).ok()
    else {
        return false;
    };
    let Some(requested_cidr) =
        d2b_contracts_resource::v3::network::Ipv4Cidr::parse(requested.destination.clone()).ok()
    else {
        return false;
    };
    d2b_contracts_resource::v3::network::cidr_overlaps(&existing_cidr, &requested_cidr)
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
    use std::os::unix::fs::PermissionsExt;

    fn route_provenance() -> d2b_contracts_resource::v3::NetworkProvenance {
        d2b_contracts_resource::v3::NetworkProvenance::new(
            d2b_contracts_resource::v3::ResourceUid::parse(
                "123e4567-e89b-42d3-a456-426614174000",
            )
            .unwrap(),
            d2b_contracts_resource::v3::ResourceUid::parse(
                "223e4567-e89b-42d3-a456-426614174001",
            )
            .unwrap(),
            d2b_contracts_resource::v3::ResourceGeneration::new(4).unwrap(),
            d2b_contracts_resource::v3::ResourceGeneration::new(7).unwrap(),
            d2b_contracts_resource::v3::ResourceBundleGenerationId::parse(
                "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            )
            .unwrap(),
        )
    }

    fn owned_network_route() -> (
        ResolvedRouteIntent,
        d2b_contracts_resource::v3::NetworkProvenance,
    ) {
        let provenance = route_provenance();
        let route_name = d2b_contracts_resource::v3::derive_network_route_name(
            provenance.zone_uid(),
            provenance.network_uid(),
            0,
        );
        let marker = format!(
            "d2b managed: {}",
            d2b_contracts_resource::v3::derive_network_ownership_marker(
                &provenance,
                &format!("route:{route_name}"),
            )
        );
        (
            ResolvedRouteIntent {
                intent_id: format!(
                    "network-route:{}:{}:aaaaaaaaaaaaaaaa:0",
                    provenance.zone_uid().as_str(),
                    provenance.network_uid().as_str()
                ),
                destination: "10.20.0.0/24".to_owned(),
                via: Some("192.0.2.2".to_owned()),
                device: Some("d2b-b12345678".to_owned()),
                table: Some("main".to_owned()),
                route_spec: "10.20.0.0/24 via 192.0.2.2 dev d2b-b12345678 table main"
                    .to_owned(),
                owned: false,
                route_name: Some(route_name),
                provenance: Some(provenance.clone()),
                ownership_marker: Some(marker),
            },
            provenance,
        )
    }

    fn fake_ip(root: &std::path::Path, routes: &str) -> std::path::PathBuf {
        std::fs::create_dir_all(root).unwrap();
        let path = root.join("ip");
        std::fs::write(
            &path,
            format!("#!/bin/sh\nprintf '%s' '{routes}'\n"),
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }

    fn seed_route_record(
        state_dir: &std::path::Path,
        intent: &ResolvedRouteIntent,
        provenance: &d2b_contracts_resource::v3::NetworkProvenance,
    ) {
        let root = state_dir.join("network-routes");
        ensure_route_ledger_root(&root).unwrap();
        let route_name = intent.route_name.as_deref().unwrap();
        let marker = intent.ownership_marker.as_deref().unwrap();
        write_route_record(
            &root.join(format!("{route_name}.json")),
            &RouteOwnershipRecord {
                route_name: route_name.to_owned(),
                marker: marker.to_owned(),
                destination: intent.destination.clone(),
                via: intent.via.clone(),
                device: intent.device.clone(),
                table: intent.table.clone().unwrap_or_else(|| "main".to_owned()),
                provenance: provenance.clone(),
            },
        )
        .unwrap();
    }

    fn cleanup_route_test(root: &std::path::Path) {
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unmarked_route_is_unchanged_on_replace_and_delete() {
        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("route-unmarked-{}", std::process::id()));
        cleanup_route_test(&root);
        let ip_root = root.join("fake-ip");
        let ip = fake_ip(
            &ip_root,
            r#"[{"dst":"10.20.0.0/24","gateway":"192.0.2.2","dev":"d2b-b12345678","table":254}]"#,
        );
        let (intent, provenance) = owned_network_route();
        for destroy in [false, true] {
            let exec = FakeReconcileExecutor::new();
            assert_eq!(
                apply_with_preflight_owned(
                    &exec,
                    &ip,
                    &root,
                    &intent,
                    &provenance,
                    destroy,
                ),
                Err(ApplyWithPreflightError::ForeignRoute)
            );
            assert!(exec.take_log().is_empty());
        }
        assert!(
            !root.join("network-routes").exists(),
            "a foreign route must not create an ownership ledger"
        );
        cleanup_route_test(&root);
    }

    #[test]
    fn matching_synthetic_route_id_without_observable_record_is_foreign() {
        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("route-synthetic-id-{}", std::process::id()));
        cleanup_route_test(&root);
        let ip = fake_ip(
            &root,
            r#"[{"dst":"10.20.0.0/24","gateway":"192.0.2.2","dev":"d2b-b12345678","table":254}]"#,
        );
        let (intent, provenance) = owned_network_route();
        let exec = FakeReconcileExecutor::new();
        assert_eq!(
            apply_with_preflight_owned(
                &exec,
                &ip,
                &root,
                &intent,
                &provenance,
                false,
            ),
            Err(ApplyWithPreflightError::ForeignRoute)
        );
        assert!(exec.take_log().is_empty());
        cleanup_route_test(&root);
    }

    #[test]
    fn forged_route_marker_is_rejected_before_creation() {
        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("route-forged-marker-{}", std::process::id()));
        cleanup_route_test(&root);
        let ip = fake_ip(&root, "[]");
        let (mut intent, provenance) = owned_network_route();
        intent.ownership_marker = Some("d2b managed: forged".to_owned());
        let exec = FakeReconcileExecutor::new();
        assert_eq!(
            apply_with_preflight_owned(
                &exec,
                &ip,
                &root,
                &intent,
                &provenance,
                false,
            ),
            Err(ApplyWithPreflightError::ForeignRoute)
        );
        assert!(exec.take_log().is_empty());
        cleanup_route_test(&root);
    }

    #[test]
    fn matching_route_marker_preserves_add_replace_delete_semantics() {
        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("route-owned-{}", std::process::id()));
        cleanup_route_test(&root);
        let ip = fake_ip(&root, "[]");
        let (intent, provenance) = owned_network_route();
        let exec = FakeReconcileExecutor::new();
        apply_with_preflight_owned(
            &exec,
            &ip,
            &root,
            &intent,
            &provenance,
            false,
        )
        .unwrap();
        assert!(matches!(
            exec.take_log().as_slice(),
            [ReconcileOp::IpRoute {
                verb: IpRouteVerb::Add,
                ..
            }]
        ));

        fake_ip(
            &root,
            r#"[{"dst":"10.20.0.0/24","gateway":"192.0.2.2","dev":"d2b-b12345678","table":254}]"#,
        );
        apply_with_preflight_owned(
            &exec,
            &ip,
            &root,
            &intent,
            &provenance,
            false,
        )
        .unwrap();
        assert!(matches!(
            exec.take_log().as_slice(),
            [ReconcileOp::IpRoute {
                verb: IpRouteVerb::Replace,
                ..
            }]
        ));

        apply_with_preflight_owned(
            &exec,
            &ip,
            &root,
            &intent,
            &provenance,
            true,
        )
        .unwrap();
        assert!(matches!(
            exec.take_log().as_slice(),
            [ReconcileOp::IpRoute {
                verb: IpRouteVerb::Del,
                ..
            }]
        ));
        assert!(
            !root
                .join("network-routes")
                .join(format!("{}.json", intent.route_name.as_deref().unwrap()))
                .exists()
        );
        cleanup_route_test(&root);
    }

    #[test]
    fn mismatched_route_record_is_unchanged_on_replace_and_delete() {
        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("route-mismatched-record-{}", std::process::id()));
        cleanup_route_test(&root);
        let ip = fake_ip(
            &root,
            r#"[{"dst":"10.20.0.0/24","gateway":"192.0.2.2","dev":"d2b-b12345678","table":254}]"#,
        );
        let (intent, provenance) = owned_network_route();
        seed_route_record(&root, &intent, &provenance);
        let record_path = root
            .join("network-routes")
            .join(format!("{}.json", intent.route_name.as_deref().unwrap()));
        let before = std::fs::read(&record_path).unwrap();
        let mut foreign = intent.clone();
        foreign.ownership_marker = Some("d2b managed: foreign".to_owned());
        for destroy in [false, true] {
            let exec = FakeReconcileExecutor::new();
            assert_eq!(
                apply_with_preflight_owned(
                    &exec,
                    &ip,
                    &root,
                    &foreign,
                    &provenance,
                    destroy,
                ),
                Err(ApplyWithPreflightError::ForeignRoute)
            );
            assert!(exec.take_log().is_empty());
            assert_eq!(std::fs::read(&record_path).unwrap(), before);
        }
        cleanup_route_test(&root);
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
