//! W4-fu broker reconcile-op executors.
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
//! The split lets the broker dispatch take a trait object so the
//! W4-fu integration tests (and the test-harness L1c canaries) can
//! drive every executor path without root.

use nixling_core::bundle_resolver::ResolvedStoreViewIntent;
use nixling_host::hardlink_farm::{self, GenerationMarker, HardlinkFarmError};
use serde::{Deserialize, Serialize};
use std::env;
use std::os::fd::AsFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

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
            Self::DifferentFilesystem { a, a_dev, b, b_dev } => write!(
                f,
                "paths on different filesystems: {a} (dev={a_dev}) vs {b} (dev={b_dev})"
            ),
            Self::MarkerMissing { generation_dir } => {
                write!(f, "generation {generation_dir} lacks marker.json")
            }
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

    /// Run `ip route <verb> <route_spec>` (verb = "add" / "del" /
    /// "replace"). Refuses non-absolute `ip_binary`.
    fn ip_route(
        &self,
        ip_binary: &Path,
        verb: IpRouteVerb,
        route_spec: &str,
    ) -> Result<(), ReconcileExecError>;

    /// W13 (W6-fu): run `usbip <subcommand> --busid <bus_id>`.
    /// Refuses non-absolute `usbip_binary`. Bus id is validated by
    /// the caller via `nixling_host::usbip_argv::validate_bus_id`.
    fn run_usbip(
        &self,
        usbip_binary: &Path,
        subcommand: UsbipSubcommand,
        bus_id: &str,
    ) -> Result<(), ReconcileExecError>;

    /// W7/W14: build or reconcile the per-VM hardlink farm generation
    /// that the native activation flow runs from.
    fn prepare_store_view(
        &self,
        intent: &ResolvedStoreViewIntent,
    ) -> Result<(), ReconcileExecError>;

    /// W7/W14: prepare the per-role mount-namespace staging root under
    /// the VM's state dir and return the bind-mount target path.
    fn setup_mount_namespace(
        &self,
        vm: &str,
        role_id: &str,
        source_view_path: &Path,
        mount_root: &Path,
    ) -> Result<PathBuf, ReconcileExecError>;

    /// W14: run the activation script from the prepared store view.
    fn run_activation_script(
        &self,
        mode_arg: &str,
        source_view_path: &Path,
        mount_view_path: &Path,
    ) -> Result<String, ReconcileExecError>;

    /// W14 LiveNative: host GC fallback shellout.
    fn run_gc(&self, keep_generations: Option<u32>) -> Result<String, ReconcileExecError>;

    /// W14 LiveNative: generate a replacement ed25519 keypair and atomically
    /// publish it at `key_path` + `key_path.pub`.
    fn run_ssh_keygen(
        &self,
        key_path: &Path,
        comment: &str,
    ) -> Result<GeneratedSshKey, ReconcileExecError>;
}

/// W13: USBIP subcommand selector mirrored from
/// `nixling_host::usbip_argv::UsbipSubcommand` so callers in the
/// broker don't need to depend on the host crate just to dispatch
/// one of two strings.
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

/// W3 live-op helper surface shared by the broker dispatch arms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemLiveExec {
    nixlingd_uid: u32,
    nixlingd_gid: u32,
}

impl SystemLiveExec {
    pub fn new(nixlingd_uid: u32, nixlingd_gid: u32) -> Self {
        Self {
            nixlingd_uid,
            nixlingd_gid,
        }
    }

    pub fn nixlingd_uid(&self) -> u32 {
        self.nixlingd_uid
    }

    pub fn nixlingd_gid(&self) -> u32 {
        self.nixlingd_gid
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
        let output = Command::new(usbip_binary)
            .arg(subcommand.as_str())
            .arg("--busid")
            .arg(bus_id)
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

    fn prepare_store_view(
        &self,
        intent: &ResolvedStoreViewIntent,
    ) -> Result<(), ReconcileExecError> {
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
        let generation_dir = hardlink_farm::build_farm(
            &intent.hardlink_farm_path,
            intent.generation,
            &intent.closure_paths,
            &GenerationMarker {
                closure_hash: format!("store-view:{}:{}", intent.vm, intent.generation),
                nixling_version: env!("CARGO_PKG_VERSION").to_owned(),
                activated_at: format!("unix-{}", current_unix_ms()),
                vm: intent.vm.clone(),
                generation_number,
            },
        )
        .map_err(map_hardlink_farm_error)?;
        let _ = hardlink_farm::read_generation_marker(&generation_dir)
            .map_err(map_hardlink_farm_error)?;
        if !intent.target_view_path.exists() {
            return Err(ReconcileExecError::Io {
                path: intent.target_view_path.display().to_string(),
                detail: "target store-view path missing after hardlink-farm build".to_owned(),
            });
        }
        Ok(())
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
        if mode_arg.is_empty() {
            return Err(ReconcileExecError::InvalidInput {
                detail: "activation mode arg is empty".to_owned(),
            });
        }
        if !source_view_path.is_absolute() || !mount_view_path.is_absolute() {
            return Err(ReconcileExecError::InvalidInput {
                detail: format!(
                    "activation script paths must be absolute, got source={} mount={}",
                    source_view_path.display(),
                    mount_view_path.display(),
                ),
            });
        }
        let source_script_path = source_view_path.join("bin/switch-to-configuration");
        if !source_script_path.exists() {
            return Err(ReconcileExecError::Io {
                path: source_script_path.display().to_string(),
                detail: "switch-to-configuration missing from prepared store view".to_owned(),
            });
        }
        let output = Command::new("/run/current-system/sw/bin/unshare")
            .arg("--mount")
            .arg("--propagation")
            .arg("private")
            .arg("/bin/sh")
            .arg("-ceu")
            .arg(
                "mount_bin=\"$1\"; src=\"$2\"; dst=\"$3\"; mode=\"$4\"; \"$mount_bin\" --bind \"$src\" \"$dst\"; exec \"$dst/bin/switch-to-configuration\" \"$mode\"",
            )
            .arg("--")
            .arg("/run/current-system/sw/bin/mount")
            .arg(source_view_path)
            .arg(mount_view_path)
            .arg(mode_arg)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| ReconcileExecError::BinaryMissing {
                which: "unshare".to_owned(),
                detail: e.to_string(),
            })?;
        if !output.status.success() {
            return Err(ReconcileExecError::NonZeroExit {
                which: format!("switch-to-configuration {mode_arg}"),
                exit_code: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(command_summary(
            &output.stdout,
            &format!(
                "{} {} succeeded",
                mount_view_path
                    .join("bin/switch-to-configuration")
                    .display(),
                mode_arg,
            ),
        ))
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
    let modprobe = env::var("NIXLING_MODPROBE_PATH")
        .unwrap_or_else(|_| "/run/current-system/sw/bin/modprobe".to_owned());
    let output = Command::new(&modprobe)
        .arg(module)
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
        HardlinkFarmError::MarkerMissing { generation_dir } => {
            ReconcileExecError::MarkerMissing { generation_dir }
        }
        HardlinkFarmError::MarkerUnparseable { path, detail }
        | HardlinkFarmError::Io { path, detail } => ReconcileExecError::Io { path, detail },
    }
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
            self.log.lock().unwrap().push(ReconcileOp::RunUsbip {
                binary: usbip_binary.to_path_buf(),
                subcommand,
                bus_id: bus_id.to_owned(),
            });
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
            hardlink_farm::build_farm(
                &intent.hardlink_farm_path,
                intent.generation,
                &intent.closure_paths,
                &GenerationMarker {
                    closure_hash: format!("fake-store-view:{}:{}", intent.vm, intent.generation),
                    nixling_version: "fake".to_owned(),
                    activated_at: "fake".to_owned(),
                    vm: intent.vm.clone(),
                    generation_number,
                },
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
            .apply_nft_script(Path::new("nft"), "table inet nixling {}")
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
                    "/var/lib/nixling/vms/vm-a/store-view/generations/1/vm-a-system",
                ),
                closure_paths: Vec::new(),
            })
            .unwrap_err();
        assert!(matches!(err, ReconcileExecError::InvalidInput { .. }));
    }

    #[test]
    fn system_run_ssh_keygen_rejects_relative_path() {
        let exec = SystemReconcileExecutor;
        let err = exec
            .run_ssh_keygen(Path::new("vm_ed25519"), "nixling:vm-a")
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
        f.apply_nft_script(Path::new("/usr/sbin/nft"), "table inet nixling {}")
            .unwrap();
        let log = f.take_log();
        assert_eq!(log.len(), 1);
        match &log[0] {
            ReconcileOp::ApplyNftScript { binary, script } => {
                assert!(binary.ends_with("nft"));
                assert!(script.contains("inet nixling"));
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
            target_view_path: root.path().join("farm/generations/7/vm-a-system"),
            closure_paths: vec![source_view.clone()],
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
            .run_ssh_keygen(
                Path::new("/var/lib/nixling/keys/vm-a_ed25519"),
                "nixling:vm-a",
            )
            .unwrap();
        assert!(result.public_key_fingerprint.starts_with("SHA256:fake:"));
        let log = f.take_log();
        assert!(matches!(
            &log[0],
            ReconcileOp::RunSshKeygen { key_path, comment }
                if key_path == Path::new("/var/lib/nixling/keys/vm-a_ed25519")
                    && comment == "nixling:vm-a"
        ));
    }
}
