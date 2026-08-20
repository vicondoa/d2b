use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use d2b_core::realm_controller_config::{RealmControllerMetadataSummary, RealmControllersJson};
use d2b_realm_core::{RealmIdentityConfigJson, RealmIdentityConfigSummary};
use serde::{Deserialize, Serialize};

use crate::typed_error::TypedError;

pub const DEFAULT_CONFIG_PATH: &str = "/etc/d2b/daemon-config.json";
pub const DEFAULT_GATEWAY_CONFIG_PATH: &str = "/etc/d2b/gateway.json";
pub const DEFAULT_REALM_CONTROLLERS_CONFIG_PATH: &str = "/etc/d2b/realm-controllers.json";
pub const DEFAULT_REALM_IDENTITY_CONFIG_PATH: &str = "/etc/d2b/realm-identity.json";
pub const DEFAULT_SERVER_VERSION: &str = "0.4.0";
pub const DEFAULT_ACCEPTED_VERSION_RANGE: &str = ">=0.4.0, <0.5.0";
pub const DEFAULT_DAEMON_STATE_DIR: &str = "/var/lib/d2b/daemon-state";
const DEFAULT_GRACEFUL_SHUTDOWN_TIMEOUT_SECONDS: u64 = 90;
const DEFAULT_LIVE_ACTIVATION_TIMEOUT_SECONDS: u64 = 600;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactPaths {
    pub public_manifest_path: PathBuf,
    pub bundle_path: PathBuf,
    pub host_path: PathBuf,
    pub processes_path: PathBuf,
    pub closures_dir: PathBuf,
}

impl Default for ArtifactPaths {
    fn default() -> Self {
        Self {
            public_manifest_path: PathBuf::from("/run/current-system/sw/share/d2b/vms.json"),
            bundle_path: PathBuf::from("/etc/d2b/bundle.json"),
            host_path: PathBuf::from("/etc/d2b/host.json"),
            processes_path: PathBuf::from("/etc/d2b/processes.json"),
            closures_dir: PathBuf::from("/etc/d2b/closures"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DaemonConfig {
    pub public_socket_path: PathBuf,
    pub broker_socket_path: PathBuf,
    pub state_lock_path: PathBuf,
    pub locks_dir: PathBuf,
    pub daemon_user: String,
    pub daemon_group: String,
    pub public_socket_group: String,
    #[serde(default)]
    pub unsafe_local_helper_socket_path: Option<PathBuf>,
    #[serde(default)]
    pub unsafe_local_helper_socket_group: Option<String>,
    #[serde(default)]
    pub unsafe_local_helper_users: Vec<String>,
    #[serde(default)]
    pub launcher_users: Vec<String>,
    #[serde(default)]
    pub admin_users: Vec<String>,
    #[serde(default = "default_server_version")]
    pub server_version: String,
    #[serde(default = "default_accepted_version_range")]
    pub accepted_client_version_range: String,
    /// Whether this daemon instance may publish the v3 Zone resource plane.
    ///
    /// The host-primary daemon owns the resource plane. Per-realm auxiliary
    /// daemons keep their legacy lifecycle socket surface but must not open
    /// every Zone store from the shared host bundle.
    #[serde(default = "default_enable_resource_plane")]
    pub enable_resource_plane: bool,
    #[serde(default)]
    pub artifacts: ArtifactPaths,
    #[serde(default = "default_gateway_config_path")]
    pub gateway_config_path: PathBuf,
    #[serde(default = "default_realm_controllers_config_path")]
    pub realm_controllers_config_path: PathBuf,
    #[serde(default = "default_realm_identity_config_path")]
    pub realm_identity_config_path: PathBuf,
    /// Concurrency cap for the autostart pass that runs on daemon
    /// startup. Default `3`.
    /// Mirrors `d2b.daemon.autostart.parallelism`.
    #[serde(default = "default_autostart_parallelism")]
    pub autostart_parallelism: usize,
    /// Default provider graceful-shutdown wait before forced cleanup.
    #[serde(default = "default_graceful_shutdown_timeout_seconds")]
    pub graceful_shutdown_timeout_seconds: u64,
    /// Default in-guest live activation wait before timeout.
    #[serde(default = "default_live_activation_timeout_seconds")]
    pub live_activation_timeout_seconds: u64,
}

fn default_autostart_parallelism() -> usize {
    crate::autostart::DEFAULT_PARALLELISM
}

fn default_graceful_shutdown_timeout_seconds() -> u64 {
    DEFAULT_GRACEFUL_SHUTDOWN_TIMEOUT_SECONDS
}

fn default_live_activation_timeout_seconds() -> u64 {
    DEFAULT_LIVE_ACTIVATION_TIMEOUT_SECONDS
}

fn default_enable_resource_plane() -> bool {
    true
}

fn default_gateway_config_path() -> PathBuf {
    PathBuf::from(DEFAULT_GATEWAY_CONFIG_PATH)
}

fn default_realm_controllers_config_path() -> PathBuf {
    PathBuf::from(DEFAULT_REALM_CONTROLLERS_CONFIG_PATH)
}

fn default_realm_identity_config_path() -> PathBuf {
    PathBuf::from(DEFAULT_REALM_IDENTITY_CONFIG_PATH)
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            public_socket_path: PathBuf::from("/run/d2b/public.sock"),
            broker_socket_path: PathBuf::from("/run/d2b/priv.sock"),
            state_lock_path: PathBuf::from("/run/d2b/daemon.lock"),
            locks_dir: PathBuf::from("/run/d2b/locks"),
            daemon_user: "d2bd".to_owned(),
            daemon_group: "d2bd".to_owned(),
            public_socket_group: "d2b".to_owned(),
            unsafe_local_helper_socket_path: None,
            unsafe_local_helper_socket_group: None,
            unsafe_local_helper_users: Vec::new(),
            launcher_users: Vec::new(),
            admin_users: Vec::new(),
            server_version: default_server_version(),
            accepted_client_version_range: default_accepted_version_range(),
            enable_resource_plane: default_enable_resource_plane(),
            artifacts: ArtifactPaths::default(),
            gateway_config_path: default_gateway_config_path(),
            realm_controllers_config_path: default_realm_controllers_config_path(),
            realm_identity_config_path: default_realm_identity_config_path(),
            autostart_parallelism: crate::autostart::DEFAULT_PARALLELISM,
            graceful_shutdown_timeout_seconds: DEFAULT_GRACEFUL_SHUTDOWN_TIMEOUT_SECONDS,
            live_activation_timeout_seconds: DEFAULT_LIVE_ACTIVATION_TIMEOUT_SECONDS,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServeOptions {
    pub config_path: PathBuf,
    pub public_socket_path: Option<PathBuf>,
    pub broker_socket_path: Option<PathBuf>,
    pub state_lock_path: Option<PathBuf>,
    pub locks_dir: Option<PathBuf>,
    pub once: bool,
    pub allow_unprivileged_runtime_dir: bool,
    pub drop_privileges: bool,
    pub daemon_state_dir: Option<PathBuf>,
    pub test_state_restore_report_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct LockOnlyOptions {
    pub config_path: PathBuf,
    pub state_lock_path: Option<PathBuf>,
    pub allow_unprivileged_runtime_dir: bool,
    pub hold_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct TestClientOptions {
    pub socket_path: PathBuf,
    pub frame_json: Vec<String>,
}

fn default_server_version() -> String {
    DEFAULT_SERVER_VERSION.to_owned()
}

fn default_accepted_version_range() -> String {
    DEFAULT_ACCEPTED_VERSION_RANGE.to_owned()
}

pub fn effective_daemon_state_dir(options: &ServeOptions) -> PathBuf {
    options
        .daemon_state_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DAEMON_STATE_DIR))
}

pub fn pidfd_table_state_path(daemon_state_dir: &Path) -> PathBuf {
    daemon_state_dir.join("pidfd-table.json")
}

/// Path of the persisted kernel-module-check report. `d2b host
/// doctor --read-only` reads this file to surface the kernel-module
/// matrix posture without re-running the bundle resolver in the CLI
/// process.
pub fn kernel_module_report_path(daemon_state_dir: &Path) -> PathBuf {
    daemon_state_dir.join("kernel-module-report.json")
}

/// Path of the persisted autostart-pass report (summary + per-VM
/// outcomes). `d2b host doctor --read-only` reads this file to report
/// degraded-VM count.
pub fn autostart_report_path(daemon_state_dir: &Path) -> PathBuf {
    daemon_state_dir.join("autostart-report.json")
}

/// Path of the persisted storage/restart/sync startup contract report.
pub fn storage_lifecycle_report_path(daemon_state_dir: &Path) -> PathBuf {
    daemon_state_dir.join("storage-lifecycle-report.json")
}

/// Path of the persisted graceful-shutdown degraded marker report.
pub fn shutdown_degraded_report_path(daemon_state_dir: &Path) -> PathBuf {
    daemon_state_dir.join("shutdown-degraded.json")
}

pub fn persist_json_report(path: &Path, json: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = File::create(&tmp)?;
        file.write_all(json)?;
        // host doctor is an unprivileged read-only CLI surface. Keep
        // diagnostic reports world-readable beneath the ACL-gated daemon-state
        // tree; they contain bounded posture data, not authority or secrets.
        file.set_permissions(fs::Permissions::from_mode(0o644))?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    if let Some(parent) = path.parent()
        && let Ok(parent_dir) = File::open(parent)
    {
        let _ = parent_dir.sync_all();
    }
    Ok(())
}

/// Persist the latest kernel-module-check report to
/// `kernel-module-report.json`. Best-effort: a write failure logs a
/// warning but does NOT abort daemon startup.
pub fn persist_kernel_module_report(
    daemon_state_dir: &Path,
    report: &crate::kernel_module_check::ModuleCheckReport,
) {
    let path = kernel_module_report_path(daemon_state_dir);
    let json = match serde_json::to_vec_pretty(report) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!(error = %err, "kernel-module-check: serialize report failed");
            return;
        }
    };
    if let Err(err) = persist_json_report(&path, &json) {
        tracing::warn!(
            error = %err,
            path = %path.display(),
            "kernel-module-check: persist report failed",
        );
    }
}

/// Persist the latest autostart-pass report to
/// `autostart-report.json`. Best-effort: a write failure logs a
/// warning but does NOT abort daemon startup.
pub fn persist_autostart_report(
    daemon_state_dir: &Path,
    report: &crate::autostart::AutostartReport,
) {
    let path = autostart_report_path(daemon_state_dir);
    let json = match serde_json::to_vec_pretty(report) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!(error = %err, "autostart: serialize report failed");
            return;
        }
    };
    if let Err(err) = persist_json_report(&path, &json) {
        tracing::warn!(
            error = %err,
            path = %path.display(),
            "autostart: persist report failed",
        );
    }
}

pub fn startup_autostart_pre_degraded_vms(
    module_degraded_vms: &BTreeSet<String>,
    net_failed_envs: &BTreeSet<String>,
) -> BTreeSet<String> {
    if !net_failed_envs.is_empty() {
        tracing::warn!(
            failed_envs = ?net_failed_envs,
            "net-route-preflight: preserving startup autostart despite pre-existing bridge failures",
        );
    }
    module_degraded_vms.clone()
}

#[cfg(test)]
mod startup_autostart_pre_degraded_tests {
    use super::startup_autostart_pre_degraded_vms;
    use std::collections::BTreeSet;

    fn set(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn startup_bridge_failures_do_not_skip_autostart_vms() {
        let module_degraded = set(&["work-aad"]);
        let net_failed_envs = set(&["obs", "work"]);

        let pre_degraded = startup_autostart_pre_degraded_vms(&module_degraded, &net_failed_envs);

        assert_eq!(pre_degraded, module_degraded);
        assert!(!pre_degraded.contains("sys-obs-net"));
        assert!(!pre_degraded.contains("sys-work-net"));
    }
}

pub fn persist_storage_lifecycle_report<T: serde::Serialize>(daemon_state_dir: &Path, report: &T) {
    let path = storage_lifecycle_report_path(daemon_state_dir);
    let json = match serde_json::to_vec_pretty(report) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!(error = %err, "storage-lifecycle: serialize report failed");
            return;
        }
    };
    if let Err(err) = persist_json_report(&path, &json) {
        tracing::warn!(
            error = %err,
            path = %path.display(),
            "storage-lifecycle: persist report failed",
        );
    }
}

pub fn banner() -> String {
    "d2bd 0.0.0-bootstrap (bootstrap stub)".to_owned()
}

pub fn banner_note() -> String {
    "daemon skeleton: start with `d2bd serve` or use hidden test modes for Layer-1 gates."
        .to_owned()
}

pub fn load_config(path: &Path) -> Result<DaemonConfig, TypedError> {
    if !path.exists() {
        return Ok(DaemonConfig::default());
    }
    let bytes = fs::read(path).map_err(|err| TypedError::InternalIo {
        context: format!("read config {}", path.display()),
        detail: err.to_string(),
    })?;
    serde_json::from_slice(&bytes).map_err(|err| TypedError::InternalConfig {
        detail: format!("{}: {err}", path.display()),
    })
}

#[derive(Debug, Clone)]
pub struct LoadedRealmControllersConfig {
    pub config: RealmControllersJson,
    pub summary: RealmControllerMetadataSummary,
}

pub fn load_realm_controllers_config(
    path: &Path,
) -> Result<Option<LoadedRealmControllersConfig>, TypedError> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|err| TypedError::InternalIo {
        context: "read realm controllers config".to_owned(),
        detail: err.to_string(),
    })?;
    let config: RealmControllersJson =
        serde_json::from_slice(&bytes).map_err(|err| TypedError::InternalConfig {
            detail: format!("invalid realm controllers config: {err}"),
        })?;
    let summary = config
        .validate_metadata_only()
        .map_err(|err| TypedError::InternalConfig {
            detail: format!("invalid realm controllers config: {err}"),
        })?;
    Ok(Some(LoadedRealmControllersConfig { config, summary }))
}

#[derive(Debug, Clone)]
pub struct LoadedRealmIdentityConfig {
    pub config: RealmIdentityConfigJson,
    pub summary: RealmIdentityConfigSummary,
}

pub fn load_realm_identity_config(
    path: &Path,
) -> Result<Option<LoadedRealmIdentityConfig>, TypedError> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|err| TypedError::InternalIo {
        context: "read realm identity config".to_owned(),
        detail: err.to_string(),
    })?;
    let config: RealmIdentityConfigJson =
        serde_json::from_slice(&bytes).map_err(|err| TypedError::InternalConfig {
            detail: format!("invalid realm identity config: {err}"),
        })?;
    let summary = config
        .validate_metadata_only()
        .map_err(|err| TypedError::InternalConfig {
            detail: format!("invalid realm identity config: {err}"),
        })?;
    Ok(Some(LoadedRealmIdentityConfig { config, summary }))
}

#[cfg(test)]
mod config_loading_tests {
    use super::*;

    fn temp_root() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp config root")
    }

    fn realm_controllers_json() -> &'static str {
        r#"{
          "schemaVersion": "v2",
          "runtimeState": "metadata-only",
          "controllers": [
            {
              "realmName": "Work",
              "realmId": "work",
              "realmPath": "corp.work",
              "placement": "host-local",
              "daemon": {
                "user": "d2br-0123456789abcdef",
                "group": "d2br-0123456789abcdef",
                "publicSocketGroup": "d2br-0123456789abcdef",
                "serviceName": "d2b-realm-work-daemon.service",
                "configPath": "/etc/d2b/realms/work/daemon-config.json",
                "stateLockPath": "/run/d2b/realms/work/daemon.lock",
                "locksDir": "/run/d2b/realms/work/locks",
                "socketActivated": false,
                "materializedService": false
              },
              "broker": {
                "enabled": true,
                "hostMutation": false,
                "user": "root",
                "group": "d2br-0123456789abcdef",
                "socketPath": "/run/d2b/realms/work/priv.sock",
                "socketUnitName": "d2b-realm-work-priv-broker.socket",
                "serviceUnitName": "d2b-realm-work-priv-broker.service",
                "auditDir": "/var/lib/d2b/realms/work/audit",
                "materializedSocket": false,
                "materializedService": false
              },
              "paths": {
                "runDir": "/run/d2b/realms/work",
                "stateDir": "/var/lib/d2b/realms/work",
                "auditDir": "/var/lib/d2b/realms/work/audit"
              },
              "sockets": {
                "publicSocketPath": "/run/d2b/realms/work/public.sock",
                "brokerSocketPath": "/run/d2b/realms/work/priv.sock"
              },
              "allocator": {
                "kind": "local-root-metadata",
                "configPath": "/etc/d2b/allocator.json",
                "rootSocket": "/run/d2b/allocator.sock"
              },
              "access": {
                "allowedUsers": ["alice"],
                "allowedGroups": ["d2b"],
                "inheritedAdminUsers": ["admin"]
              }
            }
          ],
          "invariants": {
            "metadataOnly": true,
            "noSystemdUnitsMaterialized": true,
            "preservesGlobalDaemonBehavior": true,
            "preservesDirectUnixSocketSemantics": true
          }
        }"#
    }

    fn realm_identity_json() -> &'static str {
        r#"{
          "schemaVersion": "v2",
          "runtimeState": "metadata-only",
          "realms": [
            {
              "realm": ["work"],
              "realmIdentityRef": "idref-work",
              "realmIdentityFingerprint": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
              "controllerCredentialRef": "cgref-work",
              "controllerCredentialFingerprint": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
              "trustBundleRef": "trust-work",
              "enrollmentRef": "enroll-work",
              "rotationPolicyRef": "rotate-work"
            }
          ],
          "invariants": {
            "metadataOnly": true,
            "noSecretMaterial": true,
            "preservesRuntimeBehavior": true
          }
        }"#
    }

    #[test]
    fn daemon_config_missing_uses_realm_metadata_default_paths() {
        let root = temp_root();
        let config = load_config(&root.path().join("missing-daemon-config.json"))
            .expect("missing daemon config uses defaults");
        assert_eq!(
            config.realm_controllers_config_path,
            PathBuf::from(DEFAULT_REALM_CONTROLLERS_CONFIG_PATH)
        );
        assert_eq!(
            config.realm_identity_config_path,
            PathBuf::from(DEFAULT_REALM_IDENTITY_CONFIG_PATH)
        );
        assert!(config.enable_resource_plane);
    }

    #[test]
    fn daemon_config_strictly_parses_realm_controller_path() {
        let root = temp_root();
        let config_path = root.path().join("daemon-config.json");
        fs::write(
            &config_path,
            r#"{
              "publicSocketPath": "/run/custom/public.sock",
              "brokerSocketPath": "/run/custom/priv.sock",
              "stateLockPath": "/run/custom/daemon.lock",
              "locksDir": "/run/custom/locks",
              "daemonUser": "d2bd",
              "daemonGroup": "d2bd",
              "publicSocketGroup": "d2b",
              "enableResourcePlane": false,
              "realmControllersConfigPath": "/etc/d2b/custom-realm-controllers.json",
              "realmIdentityConfigPath": "/etc/d2b/custom-realm-identity.json"
            }"#,
        )
        .expect("write daemon config");
        let config = load_config(&config_path).expect("daemon config parses");
        assert_eq!(
            config.realm_controllers_config_path,
            PathBuf::from("/etc/d2b/custom-realm-controllers.json")
        );
        assert_eq!(
            config.realm_identity_config_path,
            PathBuf::from("/etc/d2b/custom-realm-identity.json")
        );
        assert!(!config.enable_resource_plane);

        fs::write(
            &config_path,
            r#"{
              "publicSocketPath": "/run/custom/public.sock",
              "brokerSocketPath": "/run/custom/priv.sock",
              "stateLockPath": "/run/custom/daemon.lock",
              "locksDir": "/run/custom/locks",
              "daemonUser": "d2bd",
              "daemonGroup": "d2bd",
              "publicSocketGroup": "d2b",
              "unknown": true
            }"#,
        )
        .expect("write strict daemon config");
        assert!(load_config(&config_path).is_err());

        fs::write(
            &config_path,
            r#"{
              "publicSocketPath": "/run/custom/public.sock",
              "brokerSocketPath": "/run/custom/priv.sock",
              "stateLockPath": "/run/custom/daemon.lock",
              "locksDir": "/run/custom/locks",
              "daemonUser": "d2bd",
              "daemonGroup": "d2bd",
              "publicSocketGroup": "d2b",
              "realm": {
                "id": "home",
                "controllerConfigPath": "/etc/d2b/realms/home/daemon-config.json"
              }
            }"#,
        )
        .expect("write legacy realm daemon config");
        assert!(load_config(&config_path).is_err());

        fs::write(
            &config_path,
            r#"{
              "publicSocketPath": "/run/custom/public.sock",
              "brokerSocketPath": "/run/custom/priv.sock",
              "stateLockPath": "/run/custom/daemon.lock",
              "locksDir": "/run/custom/locks",
              "daemonUser": "d2bd",
              "daemonGroup": "d2bd",
              "publicSocketGroup": "d2b",
              "artifacts": {
                "publicManifestPath": "/run/current-system/sw/share/d2b/vms.json",
                "bundlePath": "/etc/d2b/bundle.json",
                "hostPath": "/etc/d2b/host.json",
                "processesPath": "/etc/d2b/processes.json",
                "closuresDir": "/etc/d2b/closures",
                "realmControllersPath": "/etc/d2b/realm-controllers.json"
              }
            }"#,
        )
        .expect("write legacy artifact daemon config");
        assert!(load_config(&config_path).is_err());
    }

    #[test]
    fn daemon_realm_controller_loader_handles_missing_and_validates_metadata() {
        let root = temp_root();
        let missing_path = root.path().join("missing-realm-controllers.json");
        assert!(
            load_realm_controllers_config(&missing_path)
                .expect("missing realm controllers is optional")
                .is_none()
        );

        let config_path = root.path().join("realm-controllers.json");
        fs::write(&config_path, realm_controllers_json()).expect("write realm controllers");
        let loaded = load_realm_controllers_config(&config_path)
            .expect("realm controllers parse")
            .expect("realm controllers present");
        assert_eq!(loaded.summary.controller_count, 1);
        let controller = &loaded.config.controllers[0];
        assert_eq!(controller.daemon.user.as_str(), "d2br-0123456789abcdef");
        assert_eq!(controller.broker.group.as_str(), "d2br-0123456789abcdef");
        assert_eq!(
            controller.broker.socket_path.as_str(),
            "/run/d2b/realms/work/priv.sock"
        );

        let materialized_path = root.path().join("materialized-realm-controllers.json");
        let materialized = realm_controllers_json()
            .replace(
                r#""materializedService": false"#,
                r#""materializedService": true"#,
            )
            .replace(
                r#""materializedSocket": false"#,
                r#""materializedSocket": true"#,
            )
            .replace(
                r#""noSystemdUnitsMaterialized": true"#,
                r#""noSystemdUnitsMaterialized": false"#,
            );
        fs::write(&materialized_path, materialized).expect("write materialized realm controllers");
        let materialized_loaded = load_realm_controllers_config(&materialized_path)
            .expect("materialized host-local unit metadata remains loadable")
            .expect("realm controllers present");
        assert_eq!(materialized_loaded.summary.host_local_controller_count, 1);
    }

    #[test]
    fn daemon_realm_identity_loader_handles_missing_and_validates_metadata_only() {
        let root = temp_root();
        let missing_path = root.path().join("missing-realm-identity.json");
        assert!(
            load_realm_identity_config(&missing_path)
                .expect("missing realm identity is optional")
                .is_none()
        );

        let config_path = root.path().join("realm-identity.json");
        fs::write(&config_path, realm_identity_json()).expect("write realm identity");
        let loaded = load_realm_identity_config(&config_path)
            .expect("realm identity parses")
            .expect("realm identity present");
        assert_eq!(loaded.summary.realm_count, 1);
        assert_eq!(loaded.summary.identity_ref_count, 1);
        assert_eq!(loaded.summary.controller_credential_ref_count, 1);

        let secret_path = root.path().join("secret-realm-identity.json");
        fs::write(
            &secret_path,
            realm_identity_json().replace(
                r#""rotationPolicyRef": "rotate-work""#,
                r#""rotationPolicyRef": "rotate-work", "privateKey": "nope""#,
            ),
        )
        .expect("write invalid realm identity");
        let err = load_realm_identity_config(&secret_path)
            .expect_err("secret material field is rejected");
        let err_text = format!("{err:?}");
        assert!(
            !err_text.contains(secret_path.to_string_lossy().as_ref()),
            "identity parse errors must not log config paths: {err_text}"
        );

        let secret_ref_path = root.path().join("secret-ref-realm-identity.json");
        fs::write(
            &secret_ref_path,
            realm_identity_json().replace("idref-work", "secret-identity"),
        )
        .expect("write invalid realm identity ref");
        let err = load_realm_identity_config(&secret_ref_path)
            .expect_err("secret-shaped identity refs are rejected");
        let err_text = format!("{err:?}");
        assert!(
            !err_text.contains(secret_ref_path.to_string_lossy().as_ref()),
            "identity ref parse errors must not log config paths: {err_text}"
        );

        let invariant_path = root.path().join("bad-realm-identity.json");
        fs::write(
            &invariant_path,
            realm_identity_json().replace(
                r#""noSecretMaterial": true"#,
                r#""noSecretMaterial": false"#,
            ),
        )
        .expect("write invalid realm identity invariant");
        assert!(load_realm_identity_config(&invariant_path).is_err());
    }
}
