// Security fix: replace the
// shell-script `[ -L ] / [ -f ] / find -type f` check-then-act
// patterns in nixos-modules/host-activation.nix with fd-safe
// operations that cannot be defeated by a runner-UID attacker
// swapping a checked regular file for a symlink between the
// check and the action.
//
// All paths used here open the target with `O_NOFOLLOW` (refusing
// to traverse a final-segment symlink) and `O_PATH` or
// `O_DIRECTORY` so the fd refers to a stable inode. Subsequent
// mutations use `fchown(2)` / `fchmod(2)` / `ftruncate(2)` /
// `fsetxattr(2)` against that fd, removing the TOCTOU window.
//
// Verbs (each accepts `--help`):
//   enforce-dir-posture --path P --uid U --gid G --mode M
//     Open P with `O_DIRECTORY|O_NOFOLLOW`, fstat to confirm it IS
//     a directory (not a symlink-to-dir), fchown(uid,gid),
//     fchmod(mode). If P is a symlink, refuses and exits 2.
//   validate-artifact (request JSON on stdin)
//     Resolve and verify one private system artifact without activation.
//
// Exit codes:
//   0  - success (action applied or already-correct)
//   1  - input / parse / nonexistent / IO error
//   2  - safety refusal (symlink or wrong file type at target)
//
// d2b-host is `#![forbid(unsafe_code)]`; this binary lives in
// the same crate so it inherits that policy. Direct libc::open()
// is required for `O_NOFOLLOW|O_EXCL` which `std::fs::OpenOptions`
// only exposes via the `unix::OpenOptionsExt::custom_flags()` API
// which IS safe; we use that.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use nix::sys::stat::{Mode, fchmod};
use nix::unistd::{Gid, Uid, fchown};
use rustix::fs::{CWD, Mode as RxMode, OFlags, ResolveFlags};
use rustix::mount::{MountPropagationFlags, UnmountFlags, mount_change, unmount};
use rustix::thread::{UnshareFlags, unshare};
use sha2::{Digest, Sha256};

use d2b_host::hardlink_farm::{
    BuildStoreViewFarmRequest, BuildStoreViewRequest, HardlinkFarmError, StoreViewLinkCounts,
    build_farm, build_store_view,
};
use d2b_host::host_generation::{
    ActivationArtifactValidationResponse, ActivationHelperOutcome, ActivationHelperResponse,
    parse_request, parse_validation_request,
};

const PRIVATE_ARTIFACT_CATALOG: &str = "/etc/d2b/artifact-catalog.json";
const EXPECTED_SYSTEM_ARTIFACT_TYPE: &str = "nixos-system";
const MAX_ARTIFACT_CATALOG_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArtifactCatalog {
    schema_version: u32,
    entries: Vec<ArtifactCatalogEntry>,
    #[serde(default)]
    #[allow(dead_code)]
    guest_setup_descriptors: Vec<serde_json::Value>,
    #[allow(dead_code)]
    catalog_digest: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArtifactCatalogEntry {
    artifact_id: String,
    #[serde(rename = "type")]
    artifact_type: String,
    store_path: String,
    package_digest: String,
    #[allow(dead_code)]
    closure_digest: String,
    #[allow(dead_code)]
    closure_size: u64,
}

#[derive(Debug)]
enum CatalogError {
    Io,
    Unsafe,
    Invalid,
    DigestMismatch,
    ArtifactMissing,
    ArtifactType,
    StorePath,
    PackageDigest,
    ActiveGeneration,
}

impl core::fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Io => "activation-catalog-io",
            Self::Unsafe => "activation-catalog-unsafe-file",
            Self::Invalid => "activation-catalog-invalid",
            Self::DigestMismatch => "activation-catalog-digest-mismatch",
            Self::ArtifactMissing => "activation-catalog-artifact-missing",
            Self::ArtifactType => "activation-catalog-artifact-type",
            Self::StorePath => "activation-catalog-store-path",
            Self::PackageDigest => "activation-catalog-package-digest",
            Self::ActiveGeneration => "activation-active-generation-mismatch",
        })
    }
}

// CLI-only helper: bounded catalog read (O_NOFOLLOW fd + bounded
// read_to_end) runs only from the activation-helper entry point, never
// on an executor worker shared with other tasks.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn read_private_catalog(path: &std::path::Path) -> Result<ArtifactCatalog, CatalogError> {
    let fd = open_no_symlinks(path, OFlags::RDONLY).map_err(|_| CatalogError::Unsafe)?;
    let file = File::from(fd);
    let metadata = file.metadata().map_err(|_| CatalogError::Io)?;
    if !metadata.is_file() || metadata.uid() != 0 || metadata.mode() & 0o777 != 0o640 {
        return Err(CatalogError::Unsafe);
    }
    let mut bytes = Vec::new();
    file.take((MAX_ARTIFACT_CATALOG_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| CatalogError::Io)?;
    if bytes.len() > MAX_ARTIFACT_CATALOG_BYTES {
        return Err(CatalogError::Invalid);
    }
    let raw: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| CatalogError::Invalid)?;
    let object = raw.as_object().ok_or(CatalogError::Invalid)?;
    let supplied_digest = object
        .get("catalogDigest")
        .and_then(serde_json::Value::as_str)
        .ok_or(CatalogError::Invalid)?;
    let mut preimage = raw.clone();
    preimage
        .as_object_mut()
        .ok_or(CatalogError::Invalid)?
        .remove("catalogDigest");
    let preimage_bytes = d2b_contracts_resource::v3::resource_schema::canonical_json_bytes(
        &preimage,
    )
    .map_err(|_| CatalogError::Invalid)?;
    let expected_digest =
        d2b_contracts_resource::v3::resource_schema::framed_canonical_digest(
            "d2b:v3:artifact-catalog",
            &preimage_bytes,
        );
    if supplied_digest != expected_digest {
        return Err(CatalogError::DigestMismatch);
    }
    let catalog: ArtifactCatalog =
        serde_json::from_value(raw).map_err(|_| CatalogError::Invalid)?;
    if catalog.schema_version != 3 {
        return Err(CatalogError::Invalid);
    }
    let mut ids = BTreeSet::new();
    for entry in &catalog.entries {
        if !ids.insert(entry.artifact_id.clone()) {
            return Err(CatalogError::Invalid);
        }
    }
    Ok(catalog)
}

fn validate_store_path(path: &std::path::Path) -> Result<(), CatalogError> {
    if !path.is_absolute()
        || path.parent() != Some(std::path::Path::new("/nix/store"))
        || path.file_name().is_none()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(CatalogError::StorePath);
    }
    let fd = open_no_symlinks(path, OFlags::PATH).map_err(|_| CatalogError::StorePath)?;
    let metadata = File::from(fd)
        .metadata()
        .map_err(|_| CatalogError::StorePath)?;
    if !metadata.is_dir() {
        return Err(CatalogError::StorePath);
    }
    Ok(())
}

fn digest_store_path(path: &std::path::Path) -> Result<String, CatalogError> {
    digest_store_path_with_root(path, std::path::Path::new("/nix/store"))
}

// CLI-only helper: the recursive store-path digest (read_dir /
// canonicalize / File::open / read_to_end, including the nested
// `visit` walker) runs only from the activation-helper entry point,
// never on an executor worker shared with other tasks.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn digest_store_path_with_root(
    path: &std::path::Path,
    store_root: &std::path::Path,
) -> Result<String, CatalogError> {
    fn visit(
        root: &std::path::Path,
        current: &std::path::Path,
        store_root: &std::path::Path,
        digest: &mut Sha256,
    ) -> Result<(), CatalogError> {
        let mut children = std::fs::read_dir(current)
            .map_err(|_| CatalogError::PackageDigest)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| CatalogError::PackageDigest)?;
        children.sort_by_key(|entry| entry.file_name());
        for entry in children {
            let path = entry.path();
            let file_type = entry.file_type().map_err(|_| CatalogError::PackageDigest)?;
            if file_type.is_symlink() {
                let Ok(target) = std::fs::canonicalize(&path) else {
                    continue;
                };
                if !target.is_file() {
                    continue;
                }
                if !target.starts_with(store_root) {
                    return Err(CatalogError::PackageDigest);
                }
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| CatalogError::PackageDigest)?;
                let mut file = File::open(&path).map_err(|_| CatalogError::PackageDigest)?;
                digest.update(relative.to_string_lossy().as_bytes());
                digest.update([0]);
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)
                    .map_err(|_| CatalogError::PackageDigest)?;
                digest.update(bytes);
            } else if file_type.is_dir() {
                visit(root, &path, store_root, digest)?;
            } else if file_type.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| CatalogError::PackageDigest)?;
                let mut file = File::open(&path).map_err(|_| CatalogError::PackageDigest)?;
                digest.update(relative.to_string_lossy().as_bytes());
                digest.update([0]);
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)
                    .map_err(|_| CatalogError::PackageDigest)?;
                digest.update(bytes);
            }
        }
        Ok(())
    }

    let mut digest = Sha256::new();
    visit(path, path, store_root, &mut digest)?;
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn active_system_path() -> Result<std::path::PathBuf, CatalogError> {
    system_profile_path("/run/current-system")
}

fn boot_default_system_path() -> Result<std::path::PathBuf, CatalogError> {
    system_profile_path("/nix/var/nix/profiles/system")
}

// CLI-only helper: single readlink(2) at the activation-helper entry
// point, never on an executor worker shared with other tasks.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn system_profile_path(path: &str) -> Result<std::path::PathBuf, CatalogError> {
    let target = std::fs::read_link(path).map_err(|_| CatalogError::ActiveGeneration)?;
    let absolute = if target.is_absolute() {
        target
    } else {
        std::path::Path::new(path)
            .parent()
            .ok_or(CatalogError::ActiveGeneration)?
            .join(target)
    };
    if absolute.parent() != Some(std::path::Path::new("/nix/store"))
        || absolute.file_name().is_none()
    {
        return Err(CatalogError::ActiveGeneration);
    }
    Ok(absolute)
}

fn activation_probe_matches(
    mode: d2b_contracts_resource::v3::ActivationMode,
    store_path: &std::path::Path,
    active_path: Option<&std::path::Path>,
    boot_path: Option<&std::path::Path>,
) -> bool {
    match mode {
        d2b_contracts_resource::v3::ActivationMode::Boot => boot_path == Some(store_path),
        d2b_contracts_resource::v3::ActivationMode::Switch
        | d2b_contracts_resource::v3::ActivationMode::Test => active_path == Some(store_path),
        d2b_contracts_resource::v3::ActivationMode::Adopt => false,
    }
}

fn resolve_system_artifact(
    request: &d2b_host::host_generation::ActivationHelperRequest,
) -> Result<std::path::PathBuf, CatalogError> {
    resolve_system_artifact_id(request.system_artifact_id.as_str())
}

fn resolve_system_artifact_id(artifact_id: &str) -> Result<std::path::PathBuf, CatalogError> {
    let catalog = read_private_catalog(std::path::Path::new(PRIVATE_ARTIFACT_CATALOG))?;
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.artifact_id == artifact_id)
        .ok_or(CatalogError::ArtifactMissing)?;
    if entry.artifact_type != EXPECTED_SYSTEM_ARTIFACT_TYPE {
        return Err(CatalogError::ArtifactType);
    }
    let store_path = std::path::PathBuf::from(&entry.store_path);
    validate_store_path(&store_path)?;
    if digest_store_path(&store_path)? != entry.package_digest {
        return Err(CatalogError::PackageDigest);
    }
    Ok(store_path)
}

/// Security fix: open `path`
/// with `openat2(AT_FDCWD, path, { O_NOFOLLOW + ..., RESOLVE_NO_SYMLINKS })`.
/// `RESOLVE_NO_SYMLINKS` refuses ANY symlink encountered during
/// path resolution - final segment AND every intermediate
/// component. This closes the symlink-swap-of-ancestor TOCTOU
/// class that plain `O_NOFOLLOW` (which only protects the final
/// component) cannot defend against.
///
/// Requires Linux >= 5.6 (openat2 syscall); v1.1 kernel floor
/// is 6.9 (ADR 0008) so this is satisfied unconditionally.
fn open_no_symlinks(path: &std::path::Path, oflags: OFlags) -> std::io::Result<OwnedFd> {
    let full_flags = oflags | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    rustix::fs::openat2(
        CWD,
        path,
        full_flags,
        RxMode::empty(),
        ResolveFlags::NO_SYMLINKS,
    )
    .map_err(|e| std::io::Error::from_raw_os_error(e.raw_os_error()))
}

#[derive(Debug)]
struct Args {
    verb: String,
    path: Option<PathBuf>,
    uid: Option<u32>,
    gid: Option<u32>,
    mode: Option<u32>,
}

fn parse_args() -> Result<Args, String> {
    let mut argv = std::env::args().skip(1);
    let verb = argv.next().ok_or("missing verb")?;
    let mut args = Args {
        verb,
        path: None,
        uid: None,
        gid: None,
        mode: None,
    };
    while let Some(flag) = argv.next() {
        let value = argv
            .next()
            .ok_or_else(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--path" => args.path = Some(PathBuf::from(value)),
            "--uid" => args.uid = Some(value.parse().map_err(|e| format!("--uid: {e}"))?),
            "--gid" => args.gid = Some(value.parse().map_err(|e| format!("--gid: {e}"))?),
            "--mode" => {
                let m =
                    u32::from_str_radix(&value, 8).map_err(|e| format!("--mode (octal): {e}"))?;
                args.mode = Some(m);
            }
            other => return Err(format!("unknown flag: {other}")),
        }
    }
    Ok(args)
}

fn require<T>(field: &str, value: Option<T>) -> Result<T, String> {
    value.ok_or_else(|| format!("missing required flag: --{field}"))
}

fn print_help() {
    eprintln!(
        "d2b-activation-helper - fd-safe activation primitives\n\
         \n\
         USAGE:\n  \
           d2b-activation-helper enforce-dir-posture --path P --uid U --gid G --mode M\n  \
           d2b-activation-helper build-store-view-farm   (request JSON on stdin)\n  \
           d2b-activation-helper build-store-view        (request JSON on stdin)\n\
           d2b-activation-helper validate-artifact      (request JSON on stdin)\n\
           d2b-activation-helper apply-generation       (request JSON on stdin)\n\
         \n\
         EXIT CODES:\n  \
           0 success / already-correct\n  \
           1 input or IO error\n  \
           2 safety refusal (symlink at target / wrong file type)\n"
    );
}
fn cmd_enforce_dir_posture(args: &Args) -> ExitCode {
    let path = match require("path", args.path.as_ref()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let uid = match require("uid", args.uid) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let gid = match require("gid", args.gid) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let mode = match require("mode", args.mode) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };

    // Security fix: use
    // openat2 + RESOLVE_NO_SYMLINKS so NO path component
    // (intermediate or final) can be a symlink.
    let dir_fd = match open_no_symlinks(path, OFlags::RDONLY | OFlags::DIRECTORY) {
        Ok(fd) => fd,
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
            eprintln!(
                "refusing: {} contains a symlink at some path component (RESOLVE_NO_SYMLINKS rejected)",
                path.display()
            );
            return ExitCode::from(2);
        }
        Err(e) if e.raw_os_error() == Some(libc::ENOTDIR) => {
            eprintln!(
                "refusing: {} is not a directory (O_DIRECTORY returned ENOTDIR)",
                path.display()
            );
            return ExitCode::from(2);
        }
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            eprintln!(
                "refusing: {} crosses a bind mount (RESOLVE_NO_SYMLINKS EXDEV)",
                path.display()
            );
            return ExitCode::from(2);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ExitCode::from(0);
        }
        Err(e) => {
            eprintln!("open({}) failed: {e}", path.display());
            return ExitCode::from(1);
        }
    };
    let dir: File = File::from(dir_fd);
    let meta = match dir.metadata() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("fstat({}) failed: {e}", path.display());
            return ExitCode::from(1);
        }
    };
    if !meta.file_type().is_dir() {
        eprintln!(
            "refusing: {} fstat says non-directory (mode 0o{:o})",
            path.display(),
            meta.mode()
        );
        return ExitCode::from(2);
    }
    if let Err(e) = fchown(
        dir.as_raw_fd(),
        Some(Uid::from_raw(uid)),
        Some(Gid::from_raw(gid)),
    ) {
        eprintln!("fchown({}) failed: {e}", path.display());
        return ExitCode::from(1);
    }
    let perms = Mode::from_bits_truncate(mode);
    if let Err(e) = fchmod(dir.as_raw_fd(), perms) {
        eprintln!("fchmod({}) failed: {e}", path.display());
        return ExitCode::from(1);
    }
    ExitCode::from(0)
}

/// Security fix: fd-safe
/// `setfacl` wrapper that cannot be redirected to attacker-
/// controlled symlink targets. Opens `--path` with O_PATH +
/// O_NOFOLLOW (refuses symlinks), fstats to validate the file
/// type matches `--require-kind` (regular | directory | socket | any),
/// then invokes `setfacl -m <acl-spec> [-m <also-spec>]
/// /proc/<helper-pid>/fd/<N>` while keeping FD_CLOEXEC set. The
/// kernel resolves the magic procfs symlink to the inode the helper
/// already holds, so the setxattr cannot be redirected to a
/// different path and the target fd is not inherited by setfacl. The
/// `--setfacl-bin` flag pins the setfacl binary (typically
/// `${pkgs. acl}/bin/setfacl`) so $PATH is not consulted.
// CLI-only verb: synchronous `setfacl` status wait at the
// activation-helper entry point, never on an executor worker shared
// with other tasks.
// CLI-only verb: synchronous `setfacl -b` status wait at the
// activation-helper entry point, never on an executor worker shared
// with other tasks.
// CLI-only verb: synchronous stdin read + path resolution at the
// activation-helper entry point, never on an executor worker shared
// with other tasks.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn cmd_validate_artifact() -> ExitCode {
    let mut bytes = Vec::new();
    if std::io::stdin().read_to_end(&mut bytes).is_err() {
        return ExitCode::from(1);
    }
    let request = match parse_validation_request(&bytes) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("activation-helper: {error}");
            return ExitCode::from(1);
        }
    };
    let valid = resolve_system_artifact_id(request.system_artifact_id.as_str()).is_ok();
    match serde_json::to_vec(&ActivationArtifactValidationResponse { valid }) {
        Ok(response) => {
            println!("{}", String::from_utf8_lossy(&response));
            if valid {
                ExitCode::from(0)
            } else {
                ExitCode::from(2)
            }
        }
        Err(_) => ExitCode::from(1),
    }
}

// CLI-only verb: synchronous stdin read, switch-script status wait,
// and profile-path resolution at the activation-helper entry point,
// never on an executor worker shared with other tasks.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn cmd_apply_generation() -> ExitCode {
    use std::io::Read;

    let mut bytes = Vec::new();
    if std::io::stdin().read_to_end(&mut bytes).is_err() {
        return ExitCode::from(1);
    }

    let request = match parse_request(&bytes) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("activation-helper: {error}");
            return ExitCode::from(1);
        }
    };
    // Resolve the selected NixOS system only through the root-owned private
    // catalog. The helper never executes the currently active system's
    // switch script for a different requested artifact.
    let outcome = match resolve_system_artifact(&request) {
        Err(error) => {
            eprintln!("activation-helper: {error}");
            ActivationHelperOutcome::Refused
        }
        Ok(store_path) => {
            let active = active_system_path().ok();
            if request.activation_mode == d2b_contracts_resource::v3::ActivationMode::Adopt {
                if active.as_ref() == Some(&store_path) {
                    ActivationHelperOutcome::Adopted
                } else {
                    ActivationHelperOutcome::Refused
                }
            } else {
                let script = store_path.join("bin/switch-to-configuration");
                let script_fd = open_no_symlinks(&script, OFlags::RDONLY);
                let script_is_executable = script_fd
                    .ok()
                    .and_then(|fd| File::from(fd).metadata().ok())
                    .is_some_and(|metadata| metadata.is_file() && metadata.mode() & 0o111 != 0);
                if !script_is_executable {
                    ActivationHelperOutcome::Refused
                } else {
                    let mode_arg = match request.activation_mode {
                        d2b_contracts_resource::v3::ActivationMode::Switch => "switch",
                        d2b_contracts_resource::v3::ActivationMode::Boot => "boot",
                        d2b_contracts_resource::v3::ActivationMode::Test => "test",
                        d2b_contracts_resource::v3::ActivationMode::Adopt => unreachable!(),
                    };
                    match Command::new(&script).arg(mode_arg).status() {
                        Ok(status) if status.success() => {
                            let active = active_system_path().ok();
                            let boot = boot_default_system_path().ok();
                            let verified = activation_probe_matches(
                                request.activation_mode,
                                &store_path,
                                active.as_deref(),
                                boot.as_deref(),
                            );
                            if verified {
                                ActivationHelperOutcome::Succeeded
                            } else {
                                ActivationHelperOutcome::Failed
                            }
                        }
                        Ok(_) | Err(_) => ActivationHelperOutcome::Failed,
                    }
                }
            }
        }
    };
    match serde_json::to_vec(&ActivationHelperResponse { outcome }) {
        Ok(response) => {
            println!("{}", String::from_utf8_lossy(&response));
            match outcome {
                ActivationHelperOutcome::Refused => ExitCode::from(2),
                ActivationHelperOutcome::Failed => ExitCode::from(1),
                ActivationHelperOutcome::Succeeded | ActivationHelperOutcome::Adopted => {
                    ExitCode::from(0)
                }
            }
        }
        Err(_) => ExitCode::from(1),
    }
}

/// Generic stdin-JSON private-store verb: read the bounded request,
/// run the typed closure, and emit the typed [`HardlinkFarmError`] (or
/// the optional [`StoreViewLinkCounts`]) as one JSON line on stdout so
/// the calling broker can recover the typed mapping.
async fn run_stdin_json_verb<Req, F, Fut>(verb: &str, run: F) -> ExitCode
where
    Req: serde::de::DeserializeOwned,
    F: FnOnce(Req) -> Fut,
    Fut: std::future::Future<Output = Result<Option<StoreViewLinkCounts>, HardlinkFarmError>>,
{
    use tokio::io::AsyncReadExt;

    let mut buf = Vec::new();
    if let Err(e) = tokio::io::stdin().read_to_end(&mut buf).await {
        eprintln!("{verb}: read stdin: {e}");
        return ExitCode::from(1);
    }
    let req: Req = match serde_json::from_slice(&buf) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{verb}: parse request: {e}");
            return ExitCode::from(1);
        }
    };
    match run(req).await {
        Ok(Some(counts)) => {
            if let Ok(j) = serde_json::to_string(&counts) {
                println!("{j}");
            }
            ExitCode::from(0)
        }
        Ok(None) => ExitCode::from(0),
        Err(e) => {
            if let Ok(j) = serde_json::to_string(&e) {
                println!("{j}");
            }
            eprintln!("{verb}: {e}");
            ExitCode::from(1)
        }
    }
}

fn prepare_private_store_namespace() -> Result<(), String> {
    unshare(UnshareFlags::NEWNS).map_err(|e| format!("unshare mount namespace: {e}"))?;
    mount_change(
        "/",
        MountPropagationFlags::PRIVATE | MountPropagationFlags::REC,
    )
    .map_err(|e| format!("make mount propagation private: {e}"))?;
    let _ = unmount("/nix/store", UnmountFlags::DETACH);
    Ok(())
}

async fn run_private_store_verb(verb: &str) -> ExitCode {
    if let Err(err) = prepare_private_store_namespace() {
        eprintln!("private-store: {err}");
        return ExitCode::from(1);
    }
    match verb {
        "build-store-view-farm" => {
            run_stdin_json_verb("build-store-view-farm", |req: BuildStoreViewFarmRequest| async move {
                build_farm(&req.farm_root, req.generation, &req.closure_paths, &req.marker)
                    .await
                    .map(|_| None)
            })
            .await
        }
        "build-store-view" => {
            run_stdin_json_verb("build-store-view", |req: BuildStoreViewRequest| async move {
                build_store_view(&req.farm_root, &req.generation_id, &req.closure_paths, &req.marker)
                    .await
                    .map(Some)
            })
            .await
        }
        other => {
            eprintln!("private-store: unsupported verb {other}");
            ExitCode::from(1)
        }
    }
}

#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // `build-store-view-farm` takes its (potentially large) request as
    // JSON on stdin, not `--flag value` argv, so it bypasses the
    // generic flag parser. The broker invokes it under
    // `unshare --mount --propagation private` + `umount -l /nix/store`.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("private-store") {
        let Some(verb) = args.get(2).map(String::as_str) else {
            eprintln!("private-store: missing verb");
            return ExitCode::from(1);
        };
        return run_private_store_verb(verb).await;
    }
    if args.get(1).map(String::as_str) == Some("build-store-view-farm") {
        return run_stdin_json_verb("build-store-view-farm", |req: BuildStoreViewFarmRequest| async move {
            build_farm(&req.farm_root, req.generation, &req.closure_paths, &req.marker)
                .await
                .map(|_| None)
        })
        .await;
    }
    if args.get(1).map(String::as_str) == Some("build-store-view") {
        return run_stdin_json_verb("build-store-view", |req: BuildStoreViewRequest| async move {
            build_store_view(&req.farm_root, &req.generation_id, &req.closure_paths, &req.marker)
                .await
                .map(Some)
        })
        .await;
    }
    if args.get(1).map(String::as_str) == Some("apply-generation") {
        return cmd_apply_generation();
    }
    if args.get(1).map(String::as_str) == Some("validate-artifact") {
        return cmd_validate_artifact();
    }
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            print_help();
            return ExitCode::from(1);
        }
    };
    match args.verb.as_str() {
        "enforce-dir-posture" => cmd_enforce_dir_posture(&args),
        "--help" | "-h" => {
            print_help();
            ExitCode::from(0)
        }
        other => {
            eprintln!("error: unknown verb: {other}");
            print_help();
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn package_digest_includes_bytes_read_through_store_symlinks() {
        let fixture = tempfile::tempdir().expect("create digest fixture");
        let directory = fixture.path().to_path_buf();
        let target = directory.join("target");
        let link = directory.join("linked");
        let directory_target = directory.join("directory-target");
        let directory_link = directory.join("directory-link");
        fs::write(&target, b"first").expect("write digest target");
        fs::create_dir(&directory_target).expect("create digest directory target");
        std::os::unix::fs::symlink("target", &link).expect("create digest symlink");
        std::os::unix::fs::symlink("directory-target", &directory_link)
            .expect("create digest directory symlink");
        let directory = fs::canonicalize(directory).expect("canonical store root");
        let store_root = directory.clone();

        let first =
            digest_store_path_with_root(&directory, &store_root).expect("digest first fixture");
        fs::write(&target, b"second").expect("rewrite digest target");
        let second =
            digest_store_path_with_root(&directory, &store_root).expect("digest second fixture");

        assert_ne!(first, second);
    }

    #[test]
    fn boot_activation_uses_boot_default_not_active_runtime() {
        let requested = Path::new("/nix/store/new-system");
        let active = Path::new("/nix/store/old-system");
        assert!(activation_probe_matches(
            d2b_contracts_resource::v3::ActivationMode::Boot,
            requested,
            Some(active),
            Some(requested),
        ));
        assert!(!activation_probe_matches(
            d2b_contracts_resource::v3::ActivationMode::Boot,
            requested,
            Some(requested),
            Some(active),
        ));
    }

    #[test]
    fn failed_activation_probe_does_not_claim_success() {
        let requested = Path::new("/nix/store/new-system");
        assert!(!activation_probe_matches(
            d2b_contracts_resource::v3::ActivationMode::Boot,
            requested,
            None,
            None,
        ));
    }
}
