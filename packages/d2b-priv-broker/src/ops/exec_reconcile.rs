//! Broker reconcile-op executors.
//!
//! Pure `ops/*.rs` modules already compute the reconcile decisions
//! (audit envelope + the nft script / route specs / sysctl entries
//! / hosts entries to write). This module adds the **side-effectful
//! executor surface** that actually shells out to `nft`, `ip`,
//! writes `/proc/sys/*`, and atomically updates `/etc/hosts`.
//!
//! Each executor is split into:
//!
//! - a [`ReconcileExecutor`] trait the broker dispatch consumes;
//! - a [`SystemReconcileExecutor`] production implementation that
//!   shells out via `std::process::Command` and writes via
//!   `std::fs`;
//! - a [`FakeReconcileExecutor`] (gated by `#[cfg(any(test,
//!   feature = "fake-backends"))]`) that records intent for
//!   unit-test assertions without touching the live host.
//!
//! The split lets the broker dispatch take a trait object so integration
//! tests (and the test-harness L1c canaries) can drive every executor
//! path without root.

use d2b_core::bundle_resolver::ResolvedStoreViewIntent;
use d2b_host::hardlink_farm::{self, GenerationMarker, HardlinkFarmError};
use serde::{Deserialize, Serialize};
use std::env;
use std::io::{self, Read};
use std::os::fd::AsFd;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const USBIP_DRIVER_HELPER_TIMEOUT: Duration = Duration::from_secs(10);
const USBIP_DRIVER_MAX_ATTEMPTS: usize = 4;
const USBIP_DRIVER_RETRY_BASE: Duration = Duration::from_millis(40);
const USBIP_DRIVER_STDERR_LIMIT: usize = 8 * 1024;
const USBIP_DRIVER_STDERR_DRAIN_GRACE: Duration = Duration::from_millis(250);
const USBIP_STREAM_FD_RELEASE_GRACE: Duration = Duration::from_millis(750);
const USBIP_STATUS_USED: &str = "2";

/// Errors any executor can return. Maps cleanly onto the broker
/// runtime's typed error catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum ReconcileExecError {
    /// `nft` / `ip` / other shellout missing or unexecutable.
    BinaryMissing { which: String, detail: String },
    /// Shellout returned a non-zero exit code.
    NonZeroExit {
        which: String,
        exit_code: i32,
        stderr: String,
    },
    /// A helper that may block in a kernel/sysfs path exceeded its bounded wait.
    TimedOut {
        which: String,
        timeout_ms: u64,
        remediation: String,
    },
    /// The store-view farm root and source closure live on different
    /// filesystems, so hardlinks are impossible.
    DifferentFilesystem {
        a: String,
        a_dev: u64,
        b: String,
        b_dev: u64,
    },
    /// The target generation dir exists but lacks the trusted marker.
    MarkerMissing { generation_dir: String },
    /// The target store-view generation dir already holds a different
    /// closure (u32 generation-number collision); refused fail-closed.
    StoreViewGenerationCollision {
        generation_dir: String,
        existing: String,
        incoming: String,
    },
    /// I/O error writing /proc/sys/* or /etc/hosts.
    Io { path: String, detail: String },
    /// Mount-namespace staging or activation-script setup failed.
    MountNamespace { detail: String },
    /// Input was structurally invalid (binary path non-absolute,
    /// nft script empty, etc.).
    InvalidInput { detail: String },
}

impl std::fmt::Display for ReconcileExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BinaryMissing { which, detail } => {
                write!(f, "{which} binary unavailable: {detail}")
            }
            Self::NonZeroExit {
                which,
                exit_code,
                stderr,
            } => write!(f, "{which} exited {exit_code}: {stderr}"),
            Self::TimedOut {
                which,
                timeout_ms,
                remediation,
            } => write!(f, "{which} timed out after {timeout_ms}ms; {remediation}"),
            Self::DifferentFilesystem { a, a_dev, b, b_dev } => write!(
                f,
                "paths on different filesystems: {a} (dev={a_dev}) vs {b} (dev={b_dev})"
            ),
            Self::MarkerMissing { generation_dir } => {
                write!(f, "generation {generation_dir} lacks marker.json")
            }
            Self::StoreViewGenerationCollision {
                generation_dir,
                existing,
                incoming,
            } => write!(
                f,
                "store-view generation collision at {generation_dir}: already holds closure \
                 `{existing}`, refusing to build `{incoming}` over it"
            ),
            Self::Io { path, detail } => write!(f, "I/O error on {path}: {detail}"),
            Self::MountNamespace { detail } => write!(f, "mount namespace: {detail}"),
            Self::InvalidInput { detail } => write!(f, "invalid input: {detail}"),
        }
    }
}

impl std::error::Error for ReconcileExecError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedSshKey {
    pub public_key_fingerprint: String,
}

/// Trait the broker dispatch consumes. Production daemon wires the
/// [`SystemReconcileExecutor`]; tests use [`FakeReconcileExecutor`].
pub trait ReconcileExecutor: Send + Sync {
    /// Run `nft -f -` with the supplied script. Atomic per nft(8):
    /// the kernel applies the whole batch or nothing.
    fn apply_nft_script(&self, nft_binary: &Path, script: &str) -> Result<(), ReconcileExecError>;

    /// Write `value` to `/proc/sys/<key>` (key without the
    /// `/proc/sys/` prefix). Validates the key to refuse absolute
    /// paths / `..` traversal.
    fn write_sysctl(&self, key: &str, value: &str) -> Result<(), ReconcileExecError>;

    /// Atomically write `contents` to `path` via tmp+rename+fsync.
    /// Used for /etc/hosts updates.
    fn write_atomic_file(
        &self,
        path: &Path,
        contents: &[u8],
        mode: u32,
    ) -> Result<(), ReconcileExecError>;

    /// Path-safe direct write used for sysfs/procfs-style attribute
    /// files that do not support temp-file + rename semantics.
    fn write_path_value(&self, path: &Path, value: &str) -> Result<(), ReconcileExecError>;

    /// Path-safe direct read used for live readback verification.
    fn read_path_value(&self, path: &Path) -> Result<String, ReconcileExecError>;

    /// Best-effort host-side USBIP stream abort before driver unbind.
    ///
    /// Implementations MUST only target the usbip-host per-device stream
    /// control surface (`<sysfs_root>/<bus_id>/usbip_sockfd`) and MUST NOT
    /// claim or implement a generic sysfs revoke primitive. The default no-op
    /// exists for narrow tests; production overrides it.
    fn shutdown_usbip_streams(
        &self,
        _sysfs_root: &Path,
        _bus_id: &str,
    ) -> Result<(), ReconcileExecError> {
        Ok(())
    }

    /// Wait for the usbip-host per-device stream fd to be released after
    /// [`Self::shutdown_usbip_streams`].
    ///
    /// Production polls the kernel-owned `usbip_status` liveness surface before
    /// the sysfs driver-unbind helper runs. Tests may use the default no-op when
    /// exercising unrelated executor paths.
    fn wait_usbip_stream_fd_release(
        &self,
        _sysfs_root: &Path,
        _bus_id: &str,
    ) -> Result<(), ReconcileExecError> {
        Ok(())
    }

    /// Run `ip route <verb> <route_spec>` (verb = "add" / "del" /
    /// "replace"). Refuses non-absolute `ip_binary`.
    fn ip_route(
        &self,
        ip_binary: &Path,
        verb: IpRouteVerb,
        route_spec: &str,
    ) -> Result<(), ReconcileExecError>;

    /// Run `usbip <subcommand> --busid <bus_id>`. Refuses non-absolute
    /// `usbip_binary`. Bus id is validated by the caller via
    /// `d2b_host::usbip_argv::validate_bus_id`.
    fn run_usbip(
        &self,
        usbip_binary: &Path,
        subcommand: UsbipSubcommand,
        bus_id: &str,
    ) -> Result<(), ReconcileExecError>;

    /// Build or reconcile the per-VM hardlink farm generation that the
    /// native activation flow runs from.
    fn prepare_store_view(
        &self,
        intent: &ResolvedStoreViewIntent,
    ) -> Result<(), ReconcileExecError>;

    /// Build or reconcile a store-view generation for in-guest activation
    /// without publishing generation metadata as current.
    fn prepare_activation_store_view(
        &self,
        intent: &ResolvedStoreViewIntent,
    ) -> Result<(), ReconcileExecError> {
        self.prepare_store_view(intent)
    }

    /// Prepare the per-role mount-namespace staging root under the VM's
    /// state dir and return the bind-mount target path.
    fn setup_mount_namespace(
        &self,
        vm: &str,
        role_id: &str,
        source_view_path: &Path,
        mount_root: &Path,
    ) -> Result<PathBuf, ReconcileExecError>;

    /// Run the activation script from the prepared store view.
    fn run_activation_script(
        &self,
        mode_arg: &str,
        source_view_path: &Path,
        mount_view_path: &Path,
    ) -> Result<String, ReconcileExecError>;

    /// Host GC fallback shellout.
    fn run_gc(&self, keep_generations: Option<u32>) -> Result<String, ReconcileExecError>;

    /// Generate a replacement ed25519 keypair and atomically publish it
    /// at `key_path` + `key_path.pub`.
    fn run_ssh_keygen(
        &self,
        key_path: &Path,
        comment: &str,
    ) -> Result<GeneratedSshKey, ReconcileExecError>;
}

/// USBIP subcommand selector mirrored from
/// `d2b_host::usbip_argv::UsbipSubcommand` so callers in the broker
/// don't need to depend on the host crate just to dispatch one of two
/// strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipSubcommand {
    Bind,
    Unbind,
}

impl UsbipSubcommand {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bind => "bind",
            Self::Unbind => "unbind",
        }
    }
}

/// Verb for [`ReconcileExecutor::ip_route`]. Closed enum so the
/// broker dispatch can't accidentally pass an attacker-controlled
/// string into the argv.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IpRouteVerb {
    Add,
    Del,
    Replace,
}

impl IpRouteVerb {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Del => "del",
            Self::Replace => "replace",
        }
    }
}

/// Production executor. Shells out via `std::process::Command`.
pub struct SystemReconcileExecutor;

fn materialize_store_view(intent: &ResolvedStoreViewIntent) -> Result<(), ReconcileExecError> {
    if intent.vm.is_empty() {
        return Err(ReconcileExecError::InvalidInput {
            detail: "store-view vm is empty".to_owned(),
        });
    }

    if !intent.hardlink_farm_path.is_absolute() || !intent.target_view_path.is_absolute() {
        return Err(ReconcileExecError::InvalidInput {
            detail: format!(
                "store-view paths must be absolute, got farm={} target={}",
                intent.hardlink_farm_path.display(),
                intent.target_view_path.display(),
            ),
        });
    }
    let generation_number =
        u32::try_from(intent.generation).map_err(|_| ReconcileExecError::InvalidInput {
            detail: format!("generation {} exceeds u32", intent.generation),
        })?;
    let marker = GenerationMarker {
        closure_hash: intent.closure_identity(),
        d2b_version: env!("CARGO_PKG_VERSION").to_owned(),
        activated_at: format!("unix-{}", current_unix_ms()),
        vm: intent.vm.clone(),
        generation_number,
    };
    let generation_dir = crate::ops::store_view_farm::build_farm_cross_mount_safe(
        &intent.hardlink_farm_path,
        intent.generation,
        &intent.closure_paths,
        &marker,
    )
    .map_err(map_hardlink_farm_error)?;
    let generation_id = hardlink_farm::generation_id(
        &intent.closure_paths,
        hardlink_farm::system_store_path(&intent.closure_paths),
    );
    crate::ops::store_view_farm::build_store_view_cross_mount_safe(
        &intent.hardlink_farm_path,
        &generation_id,
        &intent.closure_paths,
        &marker,
    )
    .map_err(map_hardlink_farm_error)?;
    let _ =
        hardlink_farm::read_generation_marker(&generation_dir).map_err(map_hardlink_farm_error)?;
    if !intent.target_view_path.exists() {
        return Err(ReconcileExecError::Io {
            path: intent.target_view_path.display().to_string(),
            detail: "target store-view path missing after hardlink-farm build".to_owned(),
        });
    }
    Ok(())
}

/// Live-op helper surface shared by the broker dispatch arms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemLiveExec {
    d2bd_uid: u32,
    d2bd_gid: u32,
}

impl SystemLiveExec {
    pub fn new(d2bd_uid: u32, d2bd_gid: u32) -> Self {
        Self { d2bd_uid, d2bd_gid }
    }

    pub fn d2bd_uid(&self) -> u32 {
        self.d2bd_uid
    }

    pub fn d2bd_gid(&self) -> u32 {
        self.d2bd_gid
    }

    pub fn run_modprobe(&self, module: &str) -> Result<(), ReconcileExecError> {
        run_modprobe(module)
    }
}

impl ReconcileExecutor for SystemReconcileExecutor {
    fn apply_nft_script(&self, nft_binary: &Path, script: &str) -> Result<(), ReconcileExecError> {
        if !nft_binary
            .to_str()
            .map(|s| s.starts_with('/'))
            .unwrap_or(false)
        {
            return Err(ReconcileExecError::InvalidInput {
                detail: format!(
                    "nft binary path must be absolute, got {:?}",
                    nft_binary.display().to_string()
                ),
            });
        }
        if script.is_empty() {
            return Err(ReconcileExecError::InvalidInput {
                detail: "nft script is empty".to_owned(),
            });
        }
        use std::io::Write;
        let mut child = Command::new(nft_binary)
            .arg("-f")
            .arg("-")
            .env_remove("NOTIFY_SOCKET")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| ReconcileExecError::BinaryMissing {
                which: "nft".to_owned(),
                detail: e.to_string(),
            })?;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(script.as_bytes())
                .map_err(|e| ReconcileExecError::Io {
                    path: "<nft stdin>".to_owned(),
                    detail: e.to_string(),
                })?;
        }
        let output = child
            .wait_with_output()
            .map_err(|e| ReconcileExecError::Io {
                path: "<nft wait>".to_owned(),
                detail: e.to_string(),
            })?;
        if !output.status.success() {
            return Err(ReconcileExecError::NonZeroExit {
                which: "nft".to_owned(),
                exit_code: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(())
    }

    fn write_sysctl(&self, key: &str, value: &str) -> Result<(), ReconcileExecError> {
        if key.is_empty() {
            return Err(ReconcileExecError::InvalidInput {
                detail: "sysctl key is empty".to_owned(),
            });
        }
        if key.starts_with('/') {
            return Err(ReconcileExecError::InvalidInput {
                detail: format!("sysctl key must NOT be absolute (no leading /): {key:?}"),
            });
        }
        if key.contains("..") {
            return Err(ReconcileExecError::InvalidInput {
                detail: format!("sysctl key contains traversal: {key:?}"),
            });
        }
        for c in key.chars() {
            if !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/')) {
                return Err(ReconcileExecError::InvalidInput {
                    detail: format!("sysctl key contains unsafe char {c:?}: {key:?}"),
                });
            }
        }
        if value.contains('\n') {
            return Err(ReconcileExecError::InvalidInput {
                detail: format!("sysctl value contains newline: {value:?}"),
            });
        }
        let path = PathBuf::from("/proc/sys").join(key.replace('.', "/"));
        std::fs::write(&path, value).map_err(|e| ReconcileExecError::Io {
            path: path.display().to_string(),
            detail: e.to_string(),
        })?;
        Ok(())
    }

    fn write_atomic_file(
        &self,
        path: &Path,
        contents: &[u8],
        mode: u32,
    ) -> Result<(), ReconcileExecError> {
        if !path.to_str().map(|s| s.starts_with('/')).unwrap_or(false) {
            return Err(ReconcileExecError::InvalidInput {
                detail: format!("path must be absolute: {:?}", path.display().to_string()),
            });
        }
        let parent = path
            .parent()
            .ok_or_else(|| ReconcileExecError::InvalidInput {
                detail: format!("path has no parent: {:?}", path.display().to_string()),
            })?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ReconcileExecError::InvalidInput {
                detail: format!(
                    "path has no UTF-8 basename: {:?}",
                    path.display().to_string()
                ),
            })?;
        let dir_fd = crate::sys::path_safe::open_dir_path_safe(parent).map_err(|e| {
            ReconcileExecError::Io {
                path: parent.display().to_string(),
                detail: e.to_string(),
            }
        })?;
        crate::sys::path_safe::atomic_replace_fd(&dir_fd, name, contents, mode).map_err(|e| {
            ReconcileExecError::Io {
                path: path.display().to_string(),
                detail: e.to_string(),
            }
        })?;
        Ok(())
    }

    fn write_path_value(&self, path: &Path, value: &str) -> Result<(), ReconcileExecError> {
        if !path.to_str().map(|s| s.starts_with('/')).unwrap_or(false) {
            return Err(ReconcileExecError::InvalidInput {
                detail: format!("path must be absolute: {:?}", path.display().to_string()),
            });
        }
        if value.contains('\n') {
            return Err(ReconcileExecError::InvalidInput {
                detail: format!("path value contains newline: {value:?}"),
            });
        }
        crate::sys::path_safe::write_nofollow(path, value.as_bytes()).map_err(|e| {
            ReconcileExecError::Io {
                path: path.display().to_string(),
                detail: e.to_string(),
            }
        })
    }

    fn read_path_value(&self, path: &Path) -> Result<String, ReconcileExecError> {
        if !path.to_str().map(|s| s.starts_with('/')).unwrap_or(false) {
            return Err(ReconcileExecError::InvalidInput {
                detail: format!("path must be absolute: {:?}", path.display().to_string()),
            });
        }
        crate::sys::path_safe::read_to_string_nofollow(path).map_err(|e| ReconcileExecError::Io {
            path: path.display().to_string(),
            detail: e.to_string(),
        })
    }

    fn ip_route(
        &self,
        ip_binary: &Path,
        verb: IpRouteVerb,
        route_spec: &str,
    ) -> Result<(), ReconcileExecError> {
        if !ip_binary
            .to_str()
            .map(|s| s.starts_with('/'))
            .unwrap_or(false)
        {
            return Err(ReconcileExecError::InvalidInput {
                detail: format!(
                    "ip binary path must be absolute, got {:?}",
                    ip_binary.display().to_string()
                ),
            });
        }
        if route_spec.is_empty() {
            return Err(ReconcileExecError::InvalidInput {
                detail: "route spec is empty".to_owned(),
            });
        }
        // Split the route spec on whitespace to build the argv;
        // any embedded shell metacharacter is safe because we
        // never feed this to a shell.
        let mut args = vec!["route".to_owned(), verb.as_str().to_owned()];
        for part in route_spec.split_whitespace() {
            args.push(part.to_owned());
        }
        let output = Command::new(ip_binary)
            .args(&args)
            .env_remove("NOTIFY_SOCKET")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| ReconcileExecError::BinaryMissing {
                which: "ip".to_owned(),
                detail: e.to_string(),
            })?;
        if !output.status.success() {
            // `ip route del` on a route that doesn't exist returns
            // non-zero; the broker callers tolerate this via the
            // typed error.
            return Err(ReconcileExecError::NonZeroExit {
                which: "ip route".to_owned(),
                exit_code: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(())
    }

    fn run_usbip(
        &self,
        usbip_binary: &Path,
        subcommand: UsbipSubcommand,
        bus_id: &str,
    ) -> Result<(), ReconcileExecError> {
        if !usbip_binary
            .to_str()
            .map(|s| s.starts_with('/'))
            .unwrap_or(false)
        {
            return Err(ReconcileExecError::InvalidInput {
                detail: format!(
                    "usbip binary path must be absolute, got {:?}",
                    usbip_binary.display().to_string()
                ),
            });
        }
        if bus_id.is_empty() {
            return Err(ReconcileExecError::InvalidInput {
                detail: "usbip bus_id is empty".to_owned(),
            });
        }
        d2b_host::usbip_argv::validate_bus_id(bus_id).map_err(|err| {
            ReconcileExecError::InvalidInput {
                detail: format!("invalid usbip bus_id {bus_id:?}: {err:?}"),
            }
        })?;
        if matches!(subcommand, UsbipSubcommand::Bind | UsbipSubcommand::Unbind) {
            return run_usbip_driver_isolated(usbip_binary, subcommand, bus_id);
        }
        let output = Command::new(usbip_binary)
            .arg(subcommand.as_str())
            .arg("--busid")
            .arg(bus_id)
            .env_remove("NOTIFY_SOCKET")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| ReconcileExecError::BinaryMissing {
                which: "usbip".to_owned(),
                detail: e.to_string(),
            })?;
        if !output.status.success() {
            return Err(ReconcileExecError::NonZeroExit {
                which: format!("usbip {}", subcommand.as_str()),
                exit_code: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(())
    }

    fn shutdown_usbip_streams(
        &self,
        sysfs_root: &Path,
        bus_id: &str,
    ) -> Result<(), ReconcileExecError> {
        d2b_host::usbip_argv::validate_bus_id(bus_id).map_err(|err| {
            ReconcileExecError::InvalidInput {
                detail: format!("invalid usbip bus_id {bus_id:?}: {err:?}"),
            }
        })?;
        let device_dir = sysfs_root.join(bus_id);
        let status_path = device_dir.join("usbip_status");
        let sockfd_path = device_dir.join("usbip_sockfd");
        let status = read_usbip_status(&status_path)?;
        if status != USBIP_STATUS_USED {
            return Ok(());
        }

        match std::fs::write(&sockfd_path, b"-1\n") {
            Ok(()) => Ok(()),
            Err(error) if usbip_stream_shutdown_error_is_ignorable(&error) => Ok(()),
            Err(error) => {
                if read_usbip_status(&status_path).is_ok_and(|latest| latest != USBIP_STATUS_USED) {
                    return Ok(());
                }
                Err(ReconcileExecError::Io {
                    path: sockfd_path.display().to_string(),
                    detail: format!(
                        "usbip-host stream shutdown before driver unbind failed: {error}; operator must detach or recover the USBIP stream manually before retrying"
                    ),
                })
            }
        }
    }

    fn wait_usbip_stream_fd_release(
        &self,
        sysfs_root: &Path,
        bus_id: &str,
    ) -> Result<(), ReconcileExecError> {
        d2b_host::usbip_argv::validate_bus_id(bus_id).map_err(|err| {
            ReconcileExecError::InvalidInput {
                detail: format!("invalid usbip bus_id {bus_id:?}: {err:?}"),
            }
        })?;
        wait_usbip_stream_fd_release(sysfs_root, bus_id, USBIP_STREAM_FD_RELEASE_GRACE)
    }

    fn prepare_store_view(
        &self,
        intent: &ResolvedStoreViewIntent,
    ) -> Result<(), ReconcileExecError> {
        materialize_store_view(intent)?;
        let generation_number =
            u32::try_from(intent.generation).map_err(|_| ReconcileExecError::InvalidInput {
                detail: format!("generation {} exceeds u32", intent.generation),
            })?;
        // Publish the freshly-built generation as the legacy activation
        // pointer by atomically swapping
        // `store-view/current -> generations/<N>`. Modern guest serving
        // uses the split layout under `store-view/live`, `state/`, and
        // `meta/`; live activation commit publishes those split pointers
        // separately after the guest reports success. Keep this legacy
        // pointer only while rollback/current compatibility code still
        // reads it.
        hardlink_farm::swap_current_symlink(&intent.hardlink_farm_path, generation_number)
            .map_err(map_hardlink_farm_error)?;
        Ok(())
    }

    fn prepare_activation_store_view(
        &self,
        intent: &ResolvedStoreViewIntent,
    ) -> Result<(), ReconcileExecError> {
        materialize_store_view(intent)
    }

    fn setup_mount_namespace(
        &self,
        vm: &str,
        role_id: &str,
        source_view_path: &Path,
        mount_root: &Path,
    ) -> Result<PathBuf, ReconcileExecError> {
        if vm.is_empty() || role_id.is_empty() {
            return Err(ReconcileExecError::InvalidInput {
                detail: "mount-namespace vm/role_id must not be empty".to_owned(),
            });
        }
        if !source_view_path.is_absolute() || !mount_root.is_absolute() {
            return Err(ReconcileExecError::InvalidInput {
                detail: format!(
                    "mount-namespace paths must be absolute, got source={} root={}",
                    source_view_path.display(),
                    mount_root.display(),
                ),
            });
        }
        if !source_view_path.exists() {
            return Err(ReconcileExecError::MountNamespace {
                detail: format!(
                    "source store view does not exist: {}",
                    source_view_path.display()
                ),
            });
        }
        let mount_view_path = mount_root.join("store-view");
        std::fs::create_dir_all(&mount_view_path).map_err(|e| {
            ReconcileExecError::MountNamespace {
                detail: format!(
                    "failed to create mount root {} for vm={} role={}: {e}",
                    mount_view_path.display(),
                    vm,
                    role_id,
                ),
            }
        })?;
        Ok(mount_view_path)
    }

    fn run_activation_script(
        &self,
        mode_arg: &str,
        source_view_path: &Path,
        mount_view_path: &Path,
    ) -> Result<String, ReconcileExecError> {
        let _ = (mode_arg, source_view_path, mount_view_path);
        Err(ReconcileExecError::InvalidInput {
            detail: "broker-side VM activation script execution is disabled; run activation inside the guest and commit metadata with RunActivation phase=commit".to_owned(),
        })
    }

    fn run_gc(&self, keep_generations: Option<u32>) -> Result<String, ReconcileExecError> {
        if let Some(keep_generations) = keep_generations {
            return Err(ReconcileExecError::InvalidInput {
                detail: format!(
                    "keep_generations={keep_generations} is not supported by the v0.4 nix-collect-garbage fallback"
                ),
            });
        }
        let output = Command::new("/run/current-system/sw/bin/nix-collect-garbage")
            .arg("-d")
            .env_remove("NOTIFY_SOCKET")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| ReconcileExecError::BinaryMissing {
                which: "nix-collect-garbage".to_owned(),
                detail: e.to_string(),
            })?;
        if !output.status.success() {
            return Err(ReconcileExecError::NonZeroExit {
                which: "nix-collect-garbage".to_owned(),
                exit_code: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(command_summary(
            &output.stdout,
            "nix-collect-garbage -d succeeded",
        ))
    }

    fn run_ssh_keygen(
        &self,
        key_path: &Path,
        comment: &str,
    ) -> Result<GeneratedSshKey, ReconcileExecError> {
        if !key_path.is_absolute() {
            return Err(ReconcileExecError::InvalidInput {
                detail: format!(
                    "ssh-keygen path must be absolute, got {:?}",
                    key_path.display().to_string()
                ),
            });
        }
        if comment.contains('\n') {
            return Err(ReconcileExecError::InvalidInput {
                detail: "ssh-keygen comment must be single-line".to_owned(),
            });
        }
        let parent = key_path
            .parent()
            .ok_or_else(|| ReconcileExecError::InvalidInput {
                detail: format!(
                    "key path has no parent: {:?}",
                    key_path.display().to_string()
                ),
            })?;
        let basename = key_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ReconcileExecError::InvalidInput {
                detail: format!(
                    "key path has no UTF-8 basename: {:?}",
                    key_path.display().to_string()
                ),
            })?;
        let staging = parent.join(format!(".{basename}.rotate.{}", std::process::id()));
        let staging_pub = PathBuf::from(format!("{}.pub", staging.display()));
        let pub_path = PathBuf::from(format!("{}.pub", key_path.display()));
        let existing_owner = file_owner(key_path);
        let existing_pub_owner = file_owner(&pub_path).or(existing_owner);
        let result = (|| {
            let output = Command::new("/run/current-system/sw/bin/ssh-keygen")
                .arg("-q")
                .arg("-t")
                .arg("ed25519")
                .arg("-N")
                .arg("")
                .arg("-C")
                .arg(comment)
                .arg("-f")
                .arg(&staging)
                .env_remove("NOTIFY_SOCKET")
                .stdin(Stdio::null())
                .output()
                .map_err(|e| ReconcileExecError::BinaryMissing {
                    which: "ssh-keygen".to_owned(),
                    detail: e.to_string(),
                })?;
            if !output.status.success() {
                return Err(ReconcileExecError::NonZeroExit {
                    which: "ssh-keygen".to_owned(),
                    exit_code: output.status.code().unwrap_or(-1),
                    stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
                });
            }
            let fingerprint_output = Command::new("/run/current-system/sw/bin/ssh-keygen")
                .arg("-lf")
                .arg(&staging_pub)
                .env_remove("NOTIFY_SOCKET")
                .stdin(Stdio::null())
                .output()
                .map_err(|e| ReconcileExecError::BinaryMissing {
                    which: "ssh-keygen".to_owned(),
                    detail: e.to_string(),
                })?;
            if !fingerprint_output.status.success() {
                return Err(ReconcileExecError::NonZeroExit {
                    which: "ssh-keygen -lf".to_owned(),
                    exit_code: fingerprint_output.status.code().unwrap_or(-1),
                    stderr: String::from_utf8_lossy(&fingerprint_output.stderr)
                        .trim()
                        .to_owned(),
                });
            }
            let private_key = std::fs::read(&staging).map_err(|e| ReconcileExecError::Io {
                path: staging.display().to_string(),
                detail: e.to_string(),
            })?;
            let public_key = std::fs::read(&staging_pub).map_err(|e| ReconcileExecError::Io {
                path: staging_pub.display().to_string(),
                detail: e.to_string(),
            })?;
            let fingerprint = parse_fingerprint(&fingerprint_output.stdout)?;
            self.write_atomic_file(key_path, &private_key, 0o640)?;
            self.write_atomic_file(&pub_path, &public_key, 0o644)?;
            if let Some((uid, gid)) = existing_owner {
                chown_atomic_target(key_path, uid, gid)?;
            }
            if let Some((uid, gid)) = existing_pub_owner {
                chown_atomic_target(&pub_path, uid, gid)?;
            }
            Ok(GeneratedSshKey {
                public_key_fingerprint: fingerprint,
            })
        })();
        let _ = crate::sys::path_safe::remove_nofollow(&staging);
        let _ = crate::sys::path_safe::remove_nofollow(&staging_pub);
        result
    }
}

fn run_modprobe(module: &str) -> Result<(), ReconcileExecError> {
    let modprobe = env::var("D2B_MODPROBE_PATH")
        .unwrap_or_else(|_| "/run/current-system/sw/bin/modprobe".to_owned());
    let output = Command::new(&modprobe)
        .arg(module)
        .env_remove("NOTIFY_SOCKET")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => ReconcileExecError::BinaryMissing {
                which: modprobe.clone(),
                detail: err.to_string(),
            },
            _ => ReconcileExecError::Io {
                path: modprobe.clone(),
                detail: err.to_string(),
            },
        })?;
    if !output.status.success() {
        return Err(ReconcileExecError::NonZeroExit {
            which: modprobe,
            exit_code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(())
}

fn map_hardlink_farm_error(error: HardlinkFarmError) -> ReconcileExecError {
    match error {
        HardlinkFarmError::DifferentFilesystem { a, a_dev, b, b_dev } => {
            ReconcileExecError::DifferentFilesystem { a, a_dev, b, b_dev }
        }
        // CrossMountLink is normally consumed by
        // `store_view_farm::build_farm_cross_mount_safe` (which retries
        // in a mount namespace). If it reaches here it means even the
        // namespaced build still hit a same-fs cross-mount EXDEV — map
        // it to DifferentFilesystem so the broker surfaces the typed
        // store-view filesystem error rather than a generic I/O error.
        HardlinkFarmError::CrossMountLink {
            source,
            destination,
            dev,
        } => ReconcileExecError::DifferentFilesystem {
            a: source,
            a_dev: dev,
            b: destination,
            b_dev: dev,
        },
        HardlinkFarmError::MarkerMissing { generation_dir } => {
            ReconcileExecError::MarkerMissing { generation_dir }
        }
        HardlinkFarmError::GenerationCollision {
            generation_dir,
            existing,
            incoming,
        } => ReconcileExecError::StoreViewGenerationCollision {
            generation_dir,
            existing,
            incoming,
        },
        HardlinkFarmError::MarkerUnparseable { path, detail }
        | HardlinkFarmError::Io { path, detail } => ReconcileExecError::Io { path, detail },
    }
}

fn run_usbip_driver_isolated(
    usbip_binary: &Path,
    subcommand: UsbipSubcommand,
    bus_id: &str,
) -> Result<(), ReconcileExecError> {
    let deadline = Instant::now() + USBIP_DRIVER_HELPER_TIMEOUT;
    let mut last_error = None;

    for attempt in 0..USBIP_DRIVER_MAX_ATTEMPTS {
        let attempt_started = Instant::now();
        let result = run_usbip_driver_helper_once(usbip_binary, subcommand, bus_id, deadline);
        let elapsed_ms = attempt_started.elapsed().as_millis() as u64;
        let remaining_ms = deadline
            .checked_duration_since(Instant::now())
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or_default();
        match result {
            Ok(()) => {
                tracing::debug!(
                    usbip_subcommand = subcommand.as_str(),
                    attempt = attempt + 1,
                    elapsed_ms,
                    deadline_remaining_ms = remaining_ms,
                    "usbip driver helper completed"
                );
                return Ok(());
            }
            Err(error)
                if usbip_unbind_error_is_transient(&error)
                    && attempt + 1 < USBIP_DRIVER_MAX_ATTEMPTS =>
            {
                tracing::debug!(
                    usbip_subcommand = subcommand.as_str(),
                    attempt = attempt + 1,
                    elapsed_ms,
                    deadline_remaining_ms = remaining_ms,
                    "usbip driver helper retrying transient failure"
                );
                last_error = Some(error);
                let delay = usbip_unbind_retry_delay(bus_id, attempt);
                let now = Instant::now();
                if now + delay >= deadline {
                    break;
                }
                std::thread::sleep(delay);
            }
            Err(error) => {
                tracing::debug!(
                    usbip_subcommand = subcommand.as_str(),
                    attempt = attempt + 1,
                    elapsed_ms,
                    deadline_remaining_ms = remaining_ms,
                    "usbip driver helper failed"
                );
                return Err(error);
            }
        }
    }
    tracing::debug!(
        usbip_subcommand = subcommand.as_str(),
        attempts = USBIP_DRIVER_MAX_ATTEMPTS,
        "usbip driver helper retry budget exhausted"
    );
    Err(last_error.unwrap_or_else(|| ReconcileExecError::TimedOut {
        which: format!("usbip {}", subcommand.as_str()),
        timeout_ms: USBIP_DRIVER_HELPER_TIMEOUT.as_millis() as u64,
        remediation: usbip_driver_timeout_remediation(subcommand, false),
    }))
}

fn run_usbip_driver_helper_once(
    usbip_binary: &Path,
    subcommand: UsbipSubcommand,
    bus_id: &str,
    deadline: Instant,
) -> Result<(), ReconcileExecError> {
    let mut child = Command::new(usbip_binary)
        .arg(subcommand.as_str())
        .arg("--busid")
        .arg(bus_id)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| ReconcileExecError::BinaryMissing {
            which: "usbip".to_owned(),
            detail: e.to_string(),
        })?;
    let stderr_rx = child.stderr.take().map(spawn_bounded_stderr_reader);

    loop {
        match child.try_wait().map_err(|e| ReconcileExecError::Io {
            path: format!("<usbip {} wait>", subcommand.as_str()),
            detail: e.to_string(),
        })? {
            Some(status) => {
                let stderr = collect_bounded_child_stderr(stderr_rx);
                if !status.success() {
                    return Err(ReconcileExecError::NonZeroExit {
                        which: format!("usbip {}", subcommand.as_str()),
                        exit_code: status.code().unwrap_or(-1),
                        stderr,
                    });
                }
                return Ok(());
            }
            None if Instant::now() >= deadline => {
                tracing::warn!(
                    usbip_subcommand = subcommand.as_str(),
                    timeout_ms = USBIP_DRIVER_HELPER_TIMEOUT.as_millis() as u64,
                    "usbip driver helper deadline expired"
                );
                if let Ok(pid) = i32::try_from(child.id()) {
                    let _ = nix::sys::signal::killpg(
                        nix::unistd::Pid::from_raw(pid),
                        nix::sys::signal::Signal::SIGKILL,
                    );
                } else {
                    let _ = child.kill();
                }
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Err(ReconcileExecError::TimedOut {
                    which: format!("usbip {}", subcommand.as_str()),
                    timeout_ms: USBIP_DRIVER_HELPER_TIMEOUT.as_millis() as u64,
                    remediation: usbip_driver_timeout_remediation(subcommand, true),
                });
            }
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}

fn usbip_driver_timeout_remediation(subcommand: UsbipSubcommand, killed: bool) -> String {
    match subcommand {
        UsbipSubcommand::Bind if killed => {
            "driver bind may be stuck in sysfs; verify the device is still present and retry `d2b usb attach <vm> <busid> --apply`".to_owned()
        }
        UsbipSubcommand::Bind => {
            "driver bind did not complete before the retry budget expired; verify the device is present and retry `d2b usb attach <vm> <busid> --apply`".to_owned()
        }
        UsbipSubcommand::Unbind if killed => {
            "driver detach may be stuck in sysfs; the broker kept the USBIP session claim, so verify the device and clear it manually before retrying".to_owned()
        }
        UsbipSubcommand::Unbind => {
            "driver detach did not complete before the retry budget expired; the broker kept the USBIP session claim for manual recovery".to_owned()
        }
    }
}

fn spawn_bounded_stderr_reader(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut captured = Vec::with_capacity(USBIP_DRIVER_STDERR_LIMIT.min(4096));
        let mut buf = [0u8; 4096];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let remaining = USBIP_DRIVER_STDERR_LIMIT.saturating_sub(captured.len());
                    if remaining > 0 {
                        captured.extend_from_slice(&buf[..n.min(remaining)]);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        let _ = tx.send(String::from_utf8_lossy(&captured).trim().to_owned());
    });
    rx
}

fn collect_bounded_child_stderr(stderr_rx: Option<mpsc::Receiver<String>>) -> String {
    stderr_rx
        .and_then(|rx| rx.recv_timeout(USBIP_DRIVER_STDERR_DRAIN_GRACE).ok())
        .unwrap_or_default()
}

fn read_usbip_status(path: &Path) -> Result<String, ReconcileExecError> {
    match std::fs::read_to_string(path) {
        Ok(raw) => Ok(raw.trim().to_owned()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err(ReconcileExecError::InvalidInput {
                detail: format!(
                    "usbip-host stream status {} is missing; cannot prove stream shutdown before unbind, so manual recovery is required",
                    path.display()
                ),
            })
        }
        Err(error) => Err(ReconcileExecError::Io {
            path: path.display().to_string(),
            detail: error.to_string(),
        }),
    }
}

fn wait_usbip_stream_fd_release(
    sysfs_root: &Path,
    bus_id: &str,
    grace: Duration,
) -> Result<(), ReconcileExecError> {
    let status_path = sysfs_root.join(bus_id).join("usbip_status");
    let deadline = Instant::now() + grace;
    loop {
        let status = read_usbip_status(&status_path)?;
        if status != USBIP_STATUS_USED {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(ReconcileExecError::TimedOut {
                which: "usbip stream fd release".to_owned(),
                timeout_ms: grace.as_millis() as u64,
                remediation:
                    "usbip-host still reports an in-use stream after sockfd shutdown; the broker kept the USBIP session claim so the operator can drain or recover the device manually before retrying driver unbind"
                        .to_owned(),
            });
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn usbip_stream_shutdown_error_is_ignorable(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(code)
            if code == nix::libc::ENOTSOCK
                || code == nix::libc::ENOTCONN
                || code == nix::libc::EOPNOTSUPP
                || code == nix::libc::EBADF
    )
}

fn usbip_unbind_error_is_transient(error: &ReconcileExecError) -> bool {
    match error {
        ReconcileExecError::NonZeroExit { stderr, .. } => {
            let stderr = stderr.to_ascii_lowercase();
            stderr.contains("ebusy")
                || stderr.contains("busy")
                || stderr.contains("eagain")
                || stderr.contains("temporarily unavailable")
                || stderr.contains("interrupted")
                || stderr.contains("eintr")
        }
        ReconcileExecError::Io { detail, .. } => {
            let detail = detail.to_ascii_lowercase();
            detail.contains("ebusy")
                || detail.contains("busy")
                || detail.contains("eagain")
                || detail.contains("temporarily unavailable")
                || detail.contains("interrupted")
                || detail.contains("eintr")
        }
        _ => false,
    }
}

fn usbip_unbind_retry_delay(bus_id: &str, attempt: usize) -> Duration {
    let shift = attempt.min(5) as u32;
    let base = USBIP_DRIVER_RETRY_BASE
        .checked_mul(1u32 << shift)
        .unwrap_or(Duration::from_millis(500));
    let jitter_seed = bus_id
        .bytes()
        .fold(attempt as u64 + 0x9e37_79b9, |acc, byte| {
            acc.wrapping_mul(33).wrapping_add(u64::from(byte))
        });
    base + Duration::from_millis(jitter_seed % 37)
}

fn current_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn command_summary(stdout: &[u8], fallback: &str) -> String {
    let rendered = String::from_utf8_lossy(stdout);
    rendered
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_owned())
        .unwrap_or_else(|| fallback.to_owned())
}

fn parse_fingerprint(stdout: &[u8]) -> Result<String, ReconcileExecError> {
    let rendered = String::from_utf8_lossy(stdout);
    let line = rendered
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .ok_or_else(|| ReconcileExecError::InvalidInput {
            detail: "ssh-keygen -lf produced no fingerprint output".to_owned(),
        })?;
    line.split_whitespace()
        .nth(1)
        .map(str::to_owned)
        .ok_or_else(|| ReconcileExecError::InvalidInput {
            detail: format!("ssh-keygen -lf output missing fingerprint: {line}"),
        })
}

fn file_owner(path: &Path) -> Option<(u32, u32)> {
    std::fs::metadata(path)
        .ok()
        .map(|metadata| (metadata.uid(), metadata.gid()))
}

fn chown_atomic_target(path: &Path, uid: u32, gid: u32) -> Result<(), ReconcileExecError> {
    let parent = path.parent().ok_or_else(|| ReconcileExecError::Io {
        path: path.display().to_string(),
        detail: format!("missing parent for {}", path.display()),
    })?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ReconcileExecError::Io {
            path: path.display().to_string(),
            detail: format!("missing basename for {}", path.display()),
        })?;
    let parent_fd =
        crate::sys::path_safe::open_dir_path_safe(parent).map_err(|e| ReconcileExecError::Io {
            path: path.display().to_string(),
            detail: format!("failed to open parent of {}: {e}", path.display()),
        })?;
    let target_fd = crate::sys::path_safe::open_at(
        parent_fd.as_fd(),
        Path::new(name),
        rustix::fs::OFlags::RDONLY,
    )
    .map_err(|e| ReconcileExecError::Io {
        path: path.display().to_string(),
        detail: format!(
            "failed to open {} without following symlinks: {e}",
            path.display()
        ),
    })?;
    crate::sys::path_safe::fchown(target_fd.as_fd(), Some(uid), Some(gid)).map_err(|e| {
        ReconcileExecError::Io {
            path: path.display().to_string(),
            detail: e.to_string(),
        }
    })
}

#[cfg(any(test, feature = "fake-backends"))]
mod fake {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// Recording fake executor for unit/integration tests. Captures
    /// each operation in order; assert with [`Self::take_log`].
    #[derive(Debug, Default)]
    pub struct FakeReconcileExecutor {
        log: Mutex<Vec<ReconcileOp>>,
        file_values: Mutex<BTreeMap<PathBuf, Vec<u8>>>,
        wait_usbip_stream_fd_release_error: Mutex<Option<ReconcileExecError>>,
        run_usbip_error: Mutex<Option<ReconcileExecError>>,
        bind_creates_regular_driver: Mutex<Option<(PathBuf, String)>>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum ReconcileOp {
        ApplyNftScript {
            binary: PathBuf,
            script: String,
        },
        WriteSysctl {
            key: String,
            value: String,
        },
        WriteAtomicFile {
            path: PathBuf,
            contents: Vec<u8>,
            mode: u32,
        },
        WritePathValue {
            path: PathBuf,
            value: String,
        },
        ShutdownUsbipStreams {
            sysfs_root: PathBuf,
            bus_id: String,
        },
        WaitUsbipStreamFdRelease {
            sysfs_root: PathBuf,
            bus_id: String,
        },
        IpRoute {
            binary: PathBuf,
            verb: IpRouteVerb,
            route_spec: String,
        },
        RunUsbip {
            binary: PathBuf,
            subcommand: UsbipSubcommand,
            bus_id: String,
        },
        PrepareStoreView {
            vm: String,
            generation: u64,
            hardlink_farm_path: PathBuf,
            target_view_path: PathBuf,
        },
        SetupMountNamespace {
            vm: String,
            role_id: String,
            source_view_path: PathBuf,
            mount_root: PathBuf,
            mount_view_path: PathBuf,
        },
        RunActivationScript {
            mode_arg: String,
            source_view_path: PathBuf,
            mount_view_path: PathBuf,
            script_path: PathBuf,
        },
        RunGc {
            keep_generations: Option<u32>,
        },
        RunSshKeygen {
            key_path: PathBuf,
            comment: String,
        },
    }

    impl FakeReconcileExecutor {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn take_log(&self) -> Vec<ReconcileOp> {
            std::mem::take(&mut *self.log.lock().unwrap())
        }
        pub fn fail_wait_usbip_stream_fd_release(&self, error: ReconcileExecError) {
            *self.wait_usbip_stream_fd_release_error.lock().unwrap() = Some(error);
        }
        pub fn fail_run_usbip(&self, error: ReconcileExecError) {
            *self.run_usbip_error.lock().unwrap() = Some(error);
        }
        pub fn bind_creates_regular_driver(&self, sysfs_root: PathBuf, bus_id: String) {
            *self.bind_creates_regular_driver.lock().unwrap() = Some((sysfs_root, bus_id));
        }
    }

    impl ReconcileExecutor for FakeReconcileExecutor {
        fn apply_nft_script(
            &self,
            nft_binary: &Path,
            script: &str,
        ) -> Result<(), ReconcileExecError> {
            self.log.lock().unwrap().push(ReconcileOp::ApplyNftScript {
                binary: nft_binary.to_path_buf(),
                script: script.to_owned(),
            });
            Ok(())
        }
        fn write_sysctl(&self, key: &str, value: &str) -> Result<(), ReconcileExecError> {
            self.log.lock().unwrap().push(ReconcileOp::WriteSysctl {
                key: key.to_owned(),
                value: value.to_owned(),
            });
            Ok(())
        }
        fn write_atomic_file(
            &self,
            path: &Path,
            contents: &[u8],
            mode: u32,
        ) -> Result<(), ReconcileExecError> {
            self.log.lock().unwrap().push(ReconcileOp::WriteAtomicFile {
                path: path.to_path_buf(),
                contents: contents.to_vec(),
                mode,
            });
            self.file_values
                .lock()
                .unwrap()
                .insert(path.to_path_buf(), contents.to_vec());
            Ok(())
        }
        fn write_path_value(&self, path: &Path, value: &str) -> Result<(), ReconcileExecError> {
            self.log.lock().unwrap().push(ReconcileOp::WritePathValue {
                path: path.to_path_buf(),
                value: value.to_owned(),
            });
            self.file_values
                .lock()
                .unwrap()
                .insert(path.to_path_buf(), value.as_bytes().to_vec());
            Ok(())
        }
        fn read_path_value(&self, path: &Path) -> Result<String, ReconcileExecError> {
            let bytes = self
                .file_values
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or_else(|| ReconcileExecError::Io {
                    path: path.display().to_string(),
                    detail: "fake path not found".to_owned(),
                })?;
            String::from_utf8(bytes).map_err(|err| ReconcileExecError::Io {
                path: path.display().to_string(),
                detail: err.to_string(),
            })
        }
        fn shutdown_usbip_streams(
            &self,
            sysfs_root: &Path,
            bus_id: &str,
        ) -> Result<(), ReconcileExecError> {
            self.log
                .lock()
                .unwrap()
                .push(ReconcileOp::ShutdownUsbipStreams {
                    sysfs_root: sysfs_root.to_path_buf(),
                    bus_id: bus_id.to_owned(),
                });
            Ok(())
        }
        fn wait_usbip_stream_fd_release(
            &self,
            sysfs_root: &Path,
            bus_id: &str,
        ) -> Result<(), ReconcileExecError> {
            self.log
                .lock()
                .unwrap()
                .push(ReconcileOp::WaitUsbipStreamFdRelease {
                    sysfs_root: sysfs_root.to_path_buf(),
                    bus_id: bus_id.to_owned(),
                });
            if let Some(error) = self
                .wait_usbip_stream_fd_release_error
                .lock()
                .unwrap()
                .clone()
            {
                return Err(error);
            }
            Ok(())
        }
        fn ip_route(
            &self,
            ip_binary: &Path,
            verb: IpRouteVerb,
            route_spec: &str,
        ) -> Result<(), ReconcileExecError> {
            self.log.lock().unwrap().push(ReconcileOp::IpRoute {
                binary: ip_binary.to_path_buf(),
                verb,
                route_spec: route_spec.to_owned(),
            });
            Ok(())
        }
        fn run_usbip(
            &self,
            usbip_binary: &Path,
            subcommand: UsbipSubcommand,
            bus_id: &str,
        ) -> Result<(), ReconcileExecError> {
            if subcommand == UsbipSubcommand::Bind
                && let Some((sysfs_root, expected_bus_id)) =
                    self.bind_creates_regular_driver.lock().unwrap().clone()
                && expected_bus_id == bus_id
            {
                let _ = std::fs::write(sysfs_root.join(bus_id).join("driver"), b"not-a-symlink");
            }
            if subcommand == UsbipSubcommand::Unbind {
                let prior = self.log.lock().unwrap();
                if let Some(ReconcileOp::WaitUsbipStreamFdRelease { sysfs_root, .. }) =
                    prior.iter().rev().find(|op| {
                        matches!(
                            op,
                            ReconcileOp::WaitUsbipStreamFdRelease {
                                bus_id: recorded,
                                ..
                            } if recorded == bus_id
                        )
                    })
                {
                    let _ = std::fs::remove_file(sysfs_root.join(bus_id).join("driver"));
                }
                drop(prior);
            }
            self.log.lock().unwrap().push(ReconcileOp::RunUsbip {
                binary: usbip_binary.to_path_buf(),
                subcommand,
                bus_id: bus_id.to_owned(),
            });
            if let Some(error) = self.run_usbip_error.lock().unwrap().clone() {
                return Err(error);
            }
            Ok(())
        }

        fn prepare_store_view(
            &self,
            intent: &ResolvedStoreViewIntent,
        ) -> Result<(), ReconcileExecError> {
            self.log
                .lock()
                .unwrap()
                .push(ReconcileOp::PrepareStoreView {
                    vm: intent.vm.clone(),
                    generation: intent.generation,
                    hardlink_farm_path: intent.hardlink_farm_path.clone(),
                    target_view_path: intent.target_view_path.clone(),
                });
            let generation_number =
                u32::try_from(intent.generation).map_err(|_| ReconcileExecError::InvalidInput {
                    detail: format!("generation {} exceeds u32", intent.generation),
                })?;
            let marker = GenerationMarker {
                closure_hash: intent.closure_identity(),
                d2b_version: "fake".to_owned(),
                activated_at: "fake".to_owned(),
                vm: intent.vm.clone(),
                generation_number,
            };
            hardlink_farm::build_farm(
                &intent.hardlink_farm_path,
                intent.generation,
                &intent.closure_paths,
                &marker,
            )
            .map_err(map_hardlink_farm_error)?;
            let generation_id = hardlink_farm::generation_id(
                &intent.closure_paths,
                hardlink_farm::system_store_path(&intent.closure_paths),
            );
            hardlink_farm::build_store_view(
                &intent.hardlink_farm_path,
                &generation_id,
                &intent.closure_paths,
                &marker,
            )
            .map_err(map_hardlink_farm_error)?;
            Ok(())
        }

        fn setup_mount_namespace(
            &self,
            vm: &str,
            role_id: &str,
            source_view_path: &Path,
            mount_root: &Path,
        ) -> Result<PathBuf, ReconcileExecError> {
            let mount_view_path = mount_root.join("store-view");
            self.log
                .lock()
                .unwrap()
                .push(ReconcileOp::SetupMountNamespace {
                    vm: vm.to_owned(),
                    role_id: role_id.to_owned(),
                    source_view_path: source_view_path.to_path_buf(),
                    mount_root: mount_root.to_path_buf(),
                    mount_view_path: mount_view_path.clone(),
                });
            std::fs::create_dir_all(&mount_view_path).map_err(|e| {
                ReconcileExecError::MountNamespace {
                    detail: e.to_string(),
                }
            })?;
            Ok(mount_view_path)
        }

        fn run_activation_script(
            &self,
            mode_arg: &str,
            source_view_path: &Path,
            mount_view_path: &Path,
        ) -> Result<String, ReconcileExecError> {
            self.log
                .lock()
                .unwrap()
                .push(ReconcileOp::RunActivationScript {
                    mode_arg: mode_arg.to_owned(),
                    source_view_path: source_view_path.to_path_buf(),
                    mount_view_path: mount_view_path.to_path_buf(),
                    script_path: mount_view_path.join("bin/switch-to-configuration"),
                });
            Ok(format!(
                "{} {} succeeded",
                mount_view_path
                    .join("bin/switch-to-configuration")
                    .display(),
                mode_arg,
            ))
        }

        fn run_gc(&self, keep_generations: Option<u32>) -> Result<String, ReconcileExecError> {
            self.log
                .lock()
                .unwrap()
                .push(ReconcileOp::RunGc { keep_generations });
            Ok("nix-collect-garbage -d succeeded".to_owned())
        }

        fn run_ssh_keygen(
            &self,
            key_path: &Path,
            comment: &str,
        ) -> Result<GeneratedSshKey, ReconcileExecError> {
            self.log.lock().unwrap().push(ReconcileOp::RunSshKeygen {
                key_path: key_path.to_path_buf(),
                comment: comment.to_owned(),
            });
            Ok(GeneratedSshKey {
                public_key_fingerprint: format!("SHA256:fake:{}", key_path.display()),
            })
        }
    }
}

#[cfg(any(test, feature = "fake-backends"))]
pub use fake::{FakeReconcileExecutor, ReconcileOp};

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static HELPER_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Sysctl key validation: the broker callers pass dotted keys
    /// (`net.ipv4.ip_forward`); the executor translates dots to
    /// slashes for the /proc/sys path. Rejects absolute paths,
    /// traversal, and unsafe characters.
    #[test]
    fn system_write_sysctl_rejects_absolute_key() {
        let exec = SystemReconcileExecutor;
        let err = exec
            .write_sysctl("/proc/sys/net/ipv4/ip_forward", "1")
            .unwrap_err();
        assert!(matches!(err, ReconcileExecError::InvalidInput { .. }));
    }

    #[test]
    fn system_write_sysctl_rejects_traversal() {
        let exec = SystemReconcileExecutor;
        let err = exec
            .write_sysctl("net.ipv4/../../../etc/passwd", "x")
            .unwrap_err();
        assert!(matches!(err, ReconcileExecError::InvalidInput { .. }));
    }

    #[test]
    fn system_write_sysctl_rejects_unsafe_char() {
        let exec = SystemReconcileExecutor;
        let err = exec
            .write_sysctl("net.ipv4.ip_forward;rm -rf /", "1")
            .unwrap_err();
        assert!(matches!(err, ReconcileExecError::InvalidInput { .. }));
    }

    #[test]
    fn system_write_sysctl_rejects_empty_key() {
        let exec = SystemReconcileExecutor;
        let err = exec.write_sysctl("", "1").unwrap_err();
        assert!(matches!(err, ReconcileExecError::InvalidInput { .. }));
    }

    #[test]
    fn system_write_sysctl_rejects_newline_in_value() {
        let exec = SystemReconcileExecutor;
        let err = exec
            .write_sysctl("net.ipv4.ip_forward", "1\necho pwned")
            .unwrap_err();
        assert!(matches!(err, ReconcileExecError::InvalidInput { .. }));
    }

    #[test]
    fn system_apply_nft_rejects_non_absolute_binary() {
        let exec = SystemReconcileExecutor;
        let err = exec
            .apply_nft_script(Path::new("nft"), "table inet d2b {}")
            .unwrap_err();
        assert!(matches!(err, ReconcileExecError::InvalidInput { .. }));
    }

    #[test]
    fn system_apply_nft_rejects_empty_script() {
        let exec = SystemReconcileExecutor;
        let err = exec
            .apply_nft_script(Path::new("/usr/sbin/nft"), "")
            .unwrap_err();
        assert!(matches!(err, ReconcileExecError::InvalidInput { .. }));
    }

    #[test]
    fn system_ip_route_rejects_non_absolute_binary() {
        let exec = SystemReconcileExecutor;
        let err = exec
            .ip_route(Path::new("ip"), IpRouteVerb::Add, "1.2.3.4 dev eth0")
            .unwrap_err();
        assert!(matches!(err, ReconcileExecError::InvalidInput { .. }));
    }

    #[test]
    fn system_ip_route_rejects_empty_spec() {
        let exec = SystemReconcileExecutor;
        let err = exec
            .ip_route(Path::new("/usr/sbin/ip"), IpRouteVerb::Add, "")
            .unwrap_err();
        assert!(matches!(err, ReconcileExecError::InvalidInput { .. }));
    }

    #[test]
    fn system_write_atomic_file_rejects_non_absolute_path() {
        let exec = SystemReconcileExecutor;
        let err = exec
            .write_atomic_file(Path::new("etc/hosts"), b"x", 0o644)
            .unwrap_err();
        assert!(matches!(err, ReconcileExecError::InvalidInput { .. }));
    }

    #[test]
    fn system_write_atomic_file_round_trip_in_tempdir() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let target = dir.path().join("hosts");
        let exec = SystemReconcileExecutor;
        exec.write_atomic_file(&target, b"127.0.0.1 localhost\n", 0o644)
            .unwrap();
        let read = std::fs::read_to_string(&target).unwrap();
        assert_eq!(read, "127.0.0.1 localhost\n");
        let perms = std::fs::metadata(&target).unwrap().permissions();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(perms.mode() & 0o777, 0o644);
    }

    #[test]
    fn ip_route_verb_string_round_trip() {
        assert_eq!(IpRouteVerb::Add.as_str(), "add");
        assert_eq!(IpRouteVerb::Del.as_str(), "del");
        assert_eq!(IpRouteVerb::Replace.as_str(), "replace");
    }

    fn usbip_stream_test_root(name: &str) -> PathBuf {
        let root = std::env::current_dir()
            .expect("cwd")
            .join("target")
            .join(format!("usbip-stream-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create root");
        root
    }

    fn usbip_unbind_helper_script(name: &str, body: &str) -> PathBuf {
        let root = std::env::current_dir()
            .expect("cwd")
            .join("target")
            .join(format!(
                "usbip-unbind-helper-{name}-{}-{}-{}",
                std::process::id(),
                current_unix_ms(),
                HELPER_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create helper root");
        let helper = root.join("usbip");
        {
            let mut file = std::fs::File::create(&helper).expect("create helper");
            file.write_all(body.as_bytes()).expect("write helper");
            file.sync_all().expect("sync helper");
        }
        let mut permissions = std::fs::metadata(&helper).unwrap().permissions();
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o755);
        std::fs::set_permissions(&helper, permissions).expect("chmod helper");
        helper
    }

    #[test]
    fn system_shutdown_usbip_streams_writes_sockfd_down_when_used() {
        let root = usbip_stream_test_root("used");
        let device = root.join("1-2");
        std::fs::create_dir_all(&device).expect("device");
        std::fs::write(device.join("usbip_status"), b"2\n").expect("status");
        std::fs::write(device.join("usbip_sockfd"), b"7\n").expect("sockfd");

        SystemReconcileExecutor
            .shutdown_usbip_streams(&root, "1-2")
            .expect("shutdown succeeds");

        assert_eq!(
            std::fs::read_to_string(device.join("usbip_sockfd")).unwrap(),
            "-1\n"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn system_shutdown_usbip_streams_skips_available_device() {
        let root = usbip_stream_test_root("available");
        let device = root.join("1-2");
        std::fs::create_dir_all(&device).expect("device");
        std::fs::write(device.join("usbip_status"), b"1\n").expect("status");
        std::fs::write(device.join("usbip_sockfd"), b"7\n").expect("sockfd");

        SystemReconcileExecutor
            .shutdown_usbip_streams(&root, "1-2")
            .expect("available has no stream");

        assert_eq!(
            std::fs::read_to_string(device.join("usbip_sockfd")).unwrap(),
            "7\n"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn system_waits_for_usbip_stream_fd_release_before_unbind() {
        let root = usbip_stream_test_root("release");
        let device = root.join("1-2");
        std::fs::create_dir_all(&device).expect("device");
        std::fs::write(device.join("usbip_status"), b"1\n").expect("status");

        SystemReconcileExecutor
            .wait_usbip_stream_fd_release(&root, "1-2")
            .expect("available status proves fd release");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn usbip_stream_fd_release_timeout_is_fail_closed() {
        let root = usbip_stream_test_root("release-timeout");
        let device = root.join("1-2");
        std::fs::create_dir_all(&device).expect("device");
        std::fs::write(device.join("usbip_status"), b"2\n").expect("status");

        let err = super::wait_usbip_stream_fd_release(&root, "1-2", Duration::from_millis(1))
            .expect_err("still-used stream must time out");
        match err {
            ReconcileExecError::TimedOut {
                which, remediation, ..
            } => {
                assert_eq!(which, "usbip stream fd release");
                assert!(remediation.contains("kept the USBIP session claim"));
            }
            other => panic!("expected TimedOut, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn usbip_stream_shutdown_ignores_socket_race_errnos() {
        for errno in [
            nix::libc::ENOTSOCK,
            nix::libc::ENOTCONN,
            nix::libc::EOPNOTSUPP,
            nix::libc::EBADF,
        ] {
            assert!(usbip_stream_shutdown_error_is_ignorable(
                &io::Error::from_raw_os_error(errno)
            ));
        }
        assert!(!usbip_stream_shutdown_error_is_ignorable(
            &io::Error::from_raw_os_error(nix::libc::EACCES)
        ));
    }

    #[test]
    fn usbip_unbind_retry_classifier_and_delay_are_bounded_with_jitter() {
        assert!(usbip_unbind_error_is_transient(
            &ReconcileExecError::NonZeroExit {
                which: "usbip unbind".to_owned(),
                exit_code: 1,
                stderr: "write: Device or resource busy (EBUSY)".to_owned(),
            }
        ));
        assert!(!usbip_unbind_error_is_transient(
            &ReconcileExecError::NonZeroExit {
                which: "usbip unbind".to_owned(),
                exit_code: 1,
                stderr: "device is not bound to usbip-host driver".to_owned(),
            }
        ));
        let first = usbip_unbind_retry_delay("1-2", 0);
        let second = usbip_unbind_retry_delay("1-3", 0);
        assert_ne!(first, second, "busid-derived jitter should vary delay");
        assert!(first >= USBIP_DRIVER_RETRY_BASE);
        assert!(first < Duration::from_millis(100));
    }

    #[test]
    fn usbip_unbind_helper_drains_large_stderr_without_timeout() {
        let helper = usbip_unbind_helper_script(
            "large-stderr",
            r#"#!/bin/sh
i=0
while [ "$i" -lt 4096 ]; do
  printf 'busy usbip stderr line %s abcdefghijklmnopqrstuvwxyz0123456789\n' "$i" >&2
  i=$((i + 1))
done
exit 7
"#,
        );

        let err =
            match run_usbip_driver_helper_once(
                &helper,
                UsbipSubcommand::Unbind,
                "1-2",
                Instant::now() + Duration::from_secs(2),
            )
            .expect_err("large stderr should drain and preserve helper exit status")
            {
                ReconcileExecError::BinaryMissing { detail, .. }
                    if detail.contains("Text file busy") =>
                {
                    std::thread::sleep(Duration::from_millis(25));
                    run_usbip_driver_helper_once(
                        &helper,
                        UsbipSubcommand::Unbind,
                        "1-2",
                        Instant::now() + Duration::from_secs(2),
                    )
                    .expect_err("large stderr retry should preserve helper exit status")
                }
                err => err,
            };
        match err {
            ReconcileExecError::NonZeroExit {
                which,
                exit_code,
                stderr,
            } => {
                assert_eq!(which, "usbip unbind");
                assert_eq!(exit_code, 7);
                assert!(stderr.contains("busy usbip stderr line"));
                assert!(
                    stderr.len() <= USBIP_DRIVER_STDERR_LIMIT,
                    "stderr was not bounded: {} bytes",
                    stderr.len()
                );
            }
            other => panic!("expected NonZeroExit, got {other:?}"),
        }

        if let Some(root) = helper.parent() {
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    fn usbip_bind_uses_bounded_driver_helper() {
        let helper = usbip_unbind_helper_script(
            "bind-ok",
            r#"#!/bin/sh
test "$1" = "bind"
test "$2" = "--busid"
test "$3" = "1-2"
exit 0
"#,
        );

        run_usbip_driver_helper_once(
            &helper,
            UsbipSubcommand::Bind,
            "1-2",
            Instant::now() + Duration::from_secs(2),
        )
        .expect("bind helper should succeed");

        if let Some(root) = helper.parent() {
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    fn system_prepare_store_view_rejects_relative_paths() {
        let exec = SystemReconcileExecutor;
        let err = exec
            .prepare_store_view(&ResolvedStoreViewIntent {
                intent_id: "store-view:vm:vm-a".to_owned(),
                vm: "vm-a".to_owned(),
                generation: 1,
                hardlink_farm_path: PathBuf::from("relative"),
                target_view_path: PathBuf::from(
                    "/var/lib/d2b/vms/vm-a/store-view/generations/1/vm-a-system",
                ),
                closure_paths: Vec::new(),
                db_dump_path: PathBuf::from("/nix/store/vm-a-registration"),
            })
            .unwrap_err();
        assert!(matches!(err, ReconcileExecError::InvalidInput { .. }));
    }

    #[test]
    fn system_run_ssh_keygen_rejects_relative_path() {
        let exec = SystemReconcileExecutor;
        let err = exec
            .run_ssh_keygen(Path::new("vm_ed25519"), "d2b:vm-a")
            .unwrap_err();
        assert!(matches!(err, ReconcileExecError::InvalidInput { .. }));
    }

    #[test]
    fn system_run_gc_rejects_keep_generations_hint() {
        let exec = SystemReconcileExecutor;
        let err = exec.run_gc(Some(3)).unwrap_err();
        assert!(matches!(err, ReconcileExecError::InvalidInput { .. }));
    }

    // ---- Fake executor regression tests ----

    #[test]
    fn fake_records_nft_apply() {
        let f = FakeReconcileExecutor::new();
        f.apply_nft_script(Path::new("/usr/sbin/nft"), "table inet d2b {}")
            .unwrap();
        let log = f.take_log();
        assert_eq!(log.len(), 1);
        match &log[0] {
            ReconcileOp::ApplyNftScript { binary, script } => {
                assert!(binary.ends_with("nft"));
                assert!(script.contains("inet d2b"));
            }
            other => panic!("unexpected op: {other:?}"),
        }
    }

    #[test]
    fn fake_records_sysctl_write() {
        let f = FakeReconcileExecutor::new();
        f.write_sysctl("net.ipv4.ip_forward", "1").unwrap();
        let log = f.take_log();
        assert!(matches!(
            &log[0],
            ReconcileOp::WriteSysctl { key, value }
                if key == "net.ipv4.ip_forward" && value == "1"
        ));
    }

    #[test]
    fn fake_records_atomic_write() {
        let f = FakeReconcileExecutor::new();
        f.write_atomic_file(Path::new("/etc/hosts"), b"127.0.0.1 host\n", 0o644)
            .unwrap();
        let log = f.take_log();
        assert!(matches!(
            &log[0],
            ReconcileOp::WriteAtomicFile { mode: 0o644, .. }
        ));
    }

    #[test]
    fn fake_records_ip_route() {
        let f = FakeReconcileExecutor::new();
        f.ip_route(
            Path::new("/usr/sbin/ip"),
            IpRouteVerb::Add,
            "10.0.0.0/24 dev tap0",
        )
        .unwrap();
        let log = f.take_log();
        match &log[0] {
            ReconcileOp::IpRoute {
                verb, route_spec, ..
            } => {
                assert_eq!(*verb, IpRouteVerb::Add);
                assert!(route_spec.contains("10.0.0.0/24"));
            }
            other => panic!("unexpected op: {other:?}"),
        }
    }

    #[test]
    fn fake_records_native_activation_ops() {
        let root = tempfile::tempdir().unwrap();
        let source_view = root.path().join("source-view/vm-a-system");
        std::fs::create_dir_all(source_view.join("bin")).unwrap();
        std::fs::write(
            source_view.join("bin/switch-to-configuration"),
            b"#!/bin/sh\n",
        )
        .unwrap();
        let intent = ResolvedStoreViewIntent {
            intent_id: "store-view:vm:vm-a".to_owned(),
            vm: "vm-a".to_owned(),
            generation: 7,
            hardlink_farm_path: root.path().join("farm"),
            target_view_path: root.path().join("farm/live/vm-a-system"),
            closure_paths: vec![source_view.clone()],
            db_dump_path: root.path().join("db.dump"),
        };

        let f = FakeReconcileExecutor::new();
        f.prepare_store_view(&intent).unwrap();
        let mount_view_path = f
            .setup_mount_namespace(
                "vm-a",
                "activation",
                &intent.target_view_path,
                &root.path().join("mount/activation"),
            )
            .unwrap();
        f.run_activation_script("switch", &intent.target_view_path, &mount_view_path)
            .unwrap();

        let log = f.take_log();
        assert!(matches!(
            &log[0],
            ReconcileOp::PrepareStoreView { vm, generation, .. }
                if vm == "vm-a" && *generation == 7
        ));
        assert!(matches!(
            &log[1],
            ReconcileOp::SetupMountNamespace { vm, role_id, .. }
                if vm == "vm-a" && role_id == "activation"
        ));
        assert!(matches!(
            &log[2],
            ReconcileOp::RunActivationScript { mode_arg, script_path, .. }
                if mode_arg == "switch"
                    && script_path == &mount_view_path.join("bin/switch-to-configuration")
        ));
    }

    #[test]
    fn system_executor_run_activation_script_fails_closed() {
        let exec = SystemReconcileExecutor;
        let err = exec
            .run_activation_script(
                "switch",
                Path::new("/var/lib/d2b/vms/vm-a/store-view/live/vm-a-system"),
                Path::new("/var/lib/d2b/vms/vm-a/mount-ns/activation/store-view"),
            )
            .unwrap_err();
        assert!(matches!(
            err,
            ReconcileExecError::InvalidInput { ref detail }
                if detail.contains("broker-side VM activation script execution is disabled")
        ));
    }

    #[test]
    fn fake_records_gc() {
        let f = FakeReconcileExecutor::new();
        f.run_gc(Some(3)).unwrap();
        let log = f.take_log();
        assert!(matches!(
            &log[0],
            ReconcileOp::RunGc {
                keep_generations: Some(3)
            }
        ));
    }

    #[test]
    fn fake_records_ssh_keygen() {
        let f = FakeReconcileExecutor::new();
        let result = f
            .run_ssh_keygen(Path::new("/var/lib/d2b/keys/vm-a_ed25519"), "d2b:vm-a")
            .unwrap();
        assert!(result.public_key_fingerprint.starts_with("SHA256:fake:"));
        let log = f.take_log();
        assert!(matches!(
            &log[0],
            ReconcileOp::RunSshKeygen { key_path, comment }
                if key_path == Path::new("/var/lib/d2b/keys/vm-a_ed25519")
                    && comment == "d2b:vm-a"
        ));
    }
}
