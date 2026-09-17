//! Hardlink-farm primitive for the per-VM store activation lifecycle.
//!
//! Each daemon-managed VM owns a per-VM store-view under
//! `/var/lib/d2b/vms/<vm>/store-view/`. The guest is served from
//! `live/`, a flat hardlink pool containing the retained VM closure
//! basenames.
//!
//! ## Layouts
//!
//! Two on-disk metadata layouts coexist during the daemon-native
//! migration:
//!
//! - **Legacy** ([`build_farm`]): `generations/<N>/` (keyed by the u32
//!   generation token) stores metadata only (`marker.json`,
//!   `store-paths`, the `system` symlink, and guest `meta.json`). Used by
//!   the activation/rollback reconcile path. Untouched by the W2 split.
//! - **Split** (ADR 0027, [`build_store_view`]): the StoreSync canonical
//!   target. `meta/` is the guest-served metadata root and `state/`,
//!   `gcroots/`, `sync.lock` are host-only. The on-disk key is a
//!   collision-free [`generation_id`] derived from the full closure
//!   identity, NOT the truncated u32 token (which remains
//!   display/wire metadata as `generation_token`):
//!
//!   ```text
//!   <store_root>/
//!     live/<basename>/ …               # flat hardlink pool, /nix/.ro-store
//!     live/.d2b-marker-<vm>         # zero-length readiness marker
//!     meta/                            # guest read-only share root
//!       current -> generations/<generation-id>
//!       generations/<generation-id>/{store-paths,db.dump,meta.json}
//!     state/                           # host-only broker state
//!       current -> generations/<generation-id>
//!       generations/<generation-id>/{system,marker.json,meta.json}
//!     gcroots/generation-<generation-id> -> /nix/store/…   # host-only
//!     sync.lock                        # host-only
//!   ```
//!
//! In both layouts a new generation only materialises top-level store
//! paths that are not already present in `live/`.
//!
//! The primitives in this module are:
//!
//! - [`assert_same_filesystem`]: refuses to hardlink across
//!   filesystems. The store lifecycle requires "same-filesystem fatal
//!   checks" because cross-fs hardlinks silently degrade to copy on
//!   POSIX `link(2)`
//!   (returns `EXDEV`).
//! - `activate_generation_marker`: writes a per-generation
//!   `marker.json` recording closure hash + d2b version. The
//!   activate path refuses to mutate any generation dir that lacks
//!   the marker - protects against an operator hand-rolling a
//!   directory and then having `d2b switch` activate it.
//! - [`swap_current_symlink`]: atomic tmp+rename of the
//!   `current -> generations/<N>` symlink. Crash-safe: the
//!   intermediate `current.tmp` symlink is removed by activation
//!   reconciliation if a previous swap crashed mid-way.
//!
//! All path-based primitives are async (`tokio::fs`) so the broker's
//! store-sync path can drive them from an async runtime without
//! blocking an executor worker; they touch the filesystem but do
//! not require root, and they accept the per-VM root path as a
//! parameter so tests can drive them in a `tempdir`. The
//! [`SyncLockOwnerRecord`] fd helpers stay synchronous: they are
//! microsecond `pread`/`pwrite`/`ftruncate` accessors over a
//! caller-held flocked fd (the flock itself lives in the broker), so
//! they add no blocking class of their own.
//!
//! Crate invariant `#![forbid(unsafe_code)]` is honoured.

use rustix::fs::{CWD, RenameFlags, renameat_with};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

use nix::unistd::{Gid, Uid, chown};

/// `EXDEV` ("Invalid cross-device link") errno. `link(2)` returns this
/// when source and destination are on different mounts. Defined locally
/// so this `#![forbid(unsafe_code)]` crate needs no `libc` dependency
/// for a single integer constant.
const EXDEV: i32 = 18;

/// `EMLINK` ("Too many links") errno. `link(2)` returns this when the
/// source inode is already at the filesystem's maximum hardlink count
/// (ext4 `EXT4_LINK_MAX` = 65000). Defined locally for the same reason
/// as `EXDEV`.
const EMLINK: i32 = 31;

/// Errors the hardlink-farm primitives can return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum HardlinkFarmError {
    /// Two paths live on genuinely different filesystems (distinct
    /// `st_dev`). Hardlinks across filesystems are impossible; the
    /// store-view farm cannot be built and this is FATAL - no mount
    /// namespace can help. Detected up-front by [`assert_same_filesystem`]
    /// before any `link(2)`.
    DifferentFilesystem {
        a: String,
        a_dev: u64,
        b: String,
        b_dev: u64,
    },
    /// `link(2)` returned `EXDEV` even though source and destination
    /// share the same `st_dev` - i.e. they are on the same underlying
    /// filesystem but in different *vfsmounts* (the canonical case is
    /// NixOS bind-mounting `/nix/store` read-only on top of itself).
    /// Unlike [`HardlinkFarmError::DifferentFilesystem`] this is RECOVERABLE: building the
    /// farm inside a private mount namespace where `/nix/store` is
    /// lazily detached makes the two paths share one mount and the
    /// hardlink succeeds.
    CrossMountLink {
        source: String,
        destination: String,
        dev: u64,
    },
    /// Generation directory exists but lacks the `marker.json` the
    /// activate path expects. Refuses to activate an
    /// operator-rolled directory.
    MarkerMissing { generation_dir: String },
    /// Marker file present but unparseable as JSON.
    MarkerUnparseable { path: String, detail: String },
    /// The target generation dir already holds a *different* closure
    /// than the one being built. This is the fail-closed guard for an
    /// (astronomically rare) u32 store-view generation-number collision
    /// between two distinct closures of the same VM: rather than union
    /// both closures into one generation dir (which would corrupt
    /// rollback + the activated store view), refuse the build.
    GenerationCollision {
        generation_dir: String,
        existing: String,
        incoming: String,
    },
    /// I/O error during a primitive operation.
    Io { path: String, detail: String },
}

impl std::fmt::Display for HardlinkFarmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DifferentFilesystem { a, a_dev, b, b_dev } => write!(
                f,
                "paths on different filesystems: {a} (dev={a_dev}) vs {b} (dev={b_dev})"
            ),
            Self::CrossMountLink {
                source,
                destination,
                dev,
            } => write!(
                f,
                "cross-mount hardlink refused (EXDEV) on same filesystem (dev={dev}): \
                 {source} -> {destination}"
            ),
            Self::MarkerMissing { generation_dir } => write!(
                f,
                "generation {generation_dir} lacks marker.json; refusing to activate"
            ),
            Self::MarkerUnparseable { path, detail } => {
                write!(f, "marker {path}: {detail}")
            }
            Self::GenerationCollision {
                generation_dir,
                existing,
                incoming,
            } => write!(
                f,
                "store-view generation collision at {generation_dir}: already holds closure \
                 `{existing}`, refusing to build `{incoming}` over it"
            ),
            Self::Io { path, detail } => write!(f, "I/O error on {path}: {detail}"),
        }
    }
}

impl std::error::Error for HardlinkFarmError {}

/// Marker file shape, one per `generations/<N>/marker.json`.
/// Validates that a generation was created by d2b itself (not
/// hand-rolled by an operator) and pins the closure hash + d2b
/// version at activate time for audit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenerationMarker {
    /// Closure hash the generation was built from.
    pub closure_hash: String,
    /// D2b version that wrote the marker.
    pub d2b_version: String,
    /// RFC 3339 wall-clock at activate time.
    pub activated_at: String,
    /// Per-VM scope id; cross-check against the activate-time
    /// scope to refuse activating a different VM's generation that
    /// was somehow placed under this VM's `generations/` dir.
    pub vm: String,
    /// Generation number; redundant with the directory name but
    /// pinned in the marker so a rename can be detected.
    pub generation_number: u32,
}

/// Schema version for the guest-served generation metadata document
/// (`meta.json`). Bump when the guest-safe allow-list changes.
pub const GUEST_META_SCHEMA_VERSION: u32 = 1;

/// Guest-served, host-authored generation metadata.
///
/// ADR 0027: produced by an **independent** serializer with an exact
/// positive allow-list. The guest serializer never receives the full
/// host audit struct, so it cannot leak `live/`, `state/`, `gcroots/`,
/// marker payloads, caller/authz fields, retained generations, swept
/// counts, timings, cleanup fields, error details, host-only paths, or
/// host-absolute symlinks. The key set is exactly:
/// `schema_version`, `generation_id`, `generation_token`,
/// `sync_status`, `closure_count`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestGenerationMeta {
    pub schema_version: u32,
    /// Collision-free generation identity (full closure identity); the
    /// canonical key, distinct from the truncated u32 display token.
    pub generation_id: String,
    /// Display/wire u32 token. Never the on-disk generation key.
    pub generation_token: u32,
    /// Only `ok` ever reaches the guest: `meta.json` is written after
    /// the generation has materialised successfully.
    pub sync_status: String,
    pub closure_count: u32,
}

/// Wire request for an out-of-process store-view farm build.
///
/// The privileged broker serialises this to a subprocess that runs
/// the hardlink farm build inside a private mount namespace where
/// `/nix/store` is lazily detached, so a cross-vfsmount `link(2)` EXDEV
/// (the NixOS `/nix/store` self-bind-mount) does not block the
/// hardlinks. The subprocess deserialises it and calls [`build_farm`].
/// Kept here, next to [`build_farm`] + [`GenerationMarker`], so the
/// broker (serialiser) and the `d2b-activation-helper` binary
/// (deserialiser) share one definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BuildStoreViewFarmRequest {
    /// Per-VM farm root (`.../store-view`), not the `live/` dir.
    pub farm_root: PathBuf,
    /// Content-derived u32 generation number (carried as u64 to match
    /// [`build_farm`]'s signature; validated to fit u32 upstream).
    pub generation: u64,
    /// Absolute `/nix/store/<...>` closure paths to hardlink in.
    pub closure_paths: Vec<PathBuf>,
    /// Marker pinned into `generations/<N>/marker.json`.
    pub marker: GenerationMarker,
}

/// Schema version for the host-only generation metadata document
/// (`state/generations/<id>/meta.json`). Host-only; never exposed to the
/// guest. Bump independently of the guest schema.
pub const HOST_META_SCHEMA_VERSION: u32 = 1;

/// Host-only generation metadata, written under `state/`.
///
/// ADR 0027: this is the host-confidential record root. It is NOT served
/// through `d2b-meta` (which is wired to `meta/`) and never reaches the
/// guest. W2 records the subset of audit fields that StoreSync can
/// already account for; the full audit schema (caller principal, authz,
/// timings, cleanup, integrity) is a follow-up wave.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostGenerationMeta {
    pub schema_version: u32,
    pub generation_id: String,
    pub generation_token: u32,
    pub vm: String,
    pub sync_status: String,
    pub closure_count: u32,
    pub linked_count: u32,
    pub skipped_count: u32,
}

/// Top-level link accounting for one store-view materialisation.
///
/// `linked` is the number of top-level closure basenames newly hardlinked
/// into `live/`; `skipped` is the number already present. For a complete
/// sync `linked + skipped == closure_count`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoreViewLinkCounts {
    pub linked: u32,
    pub skipped: u32,
}

/// Wire request for an out-of-process **split-layout** store-view build
/// ([`build_store_view`]).
///
/// Parallel to [`BuildStoreViewFarmRequest`] but for the ADR 0027 split
/// layout keyed by the collision-free [`generation_id`] rather than the
/// u32 token. The broker serialises this to the `build-store-view` helper
/// verb, which runs inside the same private mount namespace (so
/// cross-vfsmount `link(2)` EXDEV does not block the hardlinks) and calls
/// [`build_store_view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BuildStoreViewRequest {
    /// Per-VM farm root (`.../store-view`), not the `live/` dir.
    pub farm_root: PathBuf,
    /// Collision-free on-disk generation key (see [`generation_id`]).
    pub generation_id: String,
    /// Absolute `/nix/store/<...>` closure paths to hardlink in.
    pub closure_paths: Vec<PathBuf>,
    /// Marker pinned into `state/generations/<id>/marker.json`.
    pub marker: GenerationMarker,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplaceLivePathsRequest {
    /// Per-VM farm root (`.../store-view`), not the `live/` dir.
    pub farm_root: PathBuf,
    /// Stage tag for crash-identifiable repair directories.
    pub stage_tag: String,
    /// Absolute `/nix/store/<...>` closure paths to rebuild and exchange
    /// into `live/`.
    pub closure_paths: Vec<PathBuf>,
}

/// Return the flat live hardlink pool served by virtiofsd.
pub fn live_dir(store_root: &Path) -> PathBuf {
    store_root.join("live")
}

/// Return the legacy generation metadata directory (u32-token keyed).
pub fn generation_dir(store_root: &Path, generation_number: u64) -> PathBuf {
    store_root
        .join("generations")
        .join(generation_number.to_string())
}

/// Guest read-only metadata root (`meta/`), served as
/// `/run/d2b-store-meta`. Host-authored, guest-readable.
pub fn meta_dir(store_root: &Path) -> PathBuf {
    store_root.join("meta")
}

/// Host-only broker state root (`state/`). Never served to the guest.
pub fn state_dir(store_root: &Path) -> PathBuf {
    store_root.join("state")
}

/// Host-only GC-roots root (`gcroots/`). Never served to the guest.
pub fn gcroots_dir(store_root: &Path) -> PathBuf {
    store_root.join("gcroots")
}

/// Host-only broker sync lock path (`sync.lock`).
pub fn sync_lock_path(store_root: &Path) -> PathBuf {
    store_root.join("sync.lock")
}

/// Schema token of the `sync.lock` owner record.
pub const SYNC_LOCK_OWNER_SCHEMA: &str = "d2b.store-sync.lock-owner/v1";

/// Hard cap on the owner-record bytes. A larger `sync.lock` is refused
/// unread, so a foreign writer can never make the reader allocate.
pub const SYNC_LOCK_OWNER_MAX_BYTES: usize = 512;

/// Why a `sync.lock` owner record is not trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncLockOwnerError {
    /// The record bytes could not be read, or exceeded the byte cap.
    Io,
    /// The bytes are not the closed canonical record.
    Malformed,
    /// The record was written under a different boot.
    ForeignBoot,
    /// The recorded process is not alive with the recorded start time.
    ProcessGone,
    /// The live process's kernel-reported name differs from the record.
    OwnerMismatch,
}

impl core::fmt::Display for SyncLockOwnerError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Io => "sync-lock-owner-record-unreadable",
            Self::Malformed => "sync-lock-owner-record-malformed",
            Self::ForeignBoot => "sync-lock-owner-record-foreign-boot",
            Self::ProcessGone => "sync-lock-owner-record-process-gone",
            Self::OwnerMismatch => "sync-lock-owner-record-owner-mismatch",
        })
    }
}

impl std::error::Error for SyncLockOwnerError {}

/// The live-owner record the broker writes into `sync.lock` while it
/// holds the exclusive `flock(2)` on it.
///
/// The record is the `file-record` lease evidence the daemon verifies
/// before adopting a broker-owned lock. Ownership is proven against the
/// kernel, never against a caller-supplied name: [`Self::verify_live`]
/// re-reads `/proc/<pid>/stat` and requires the recorded start time and
/// the kernel-reported process name (`comm`) to match, and the record's
/// boot id to be the current boot. A lock whose record is missing,
/// malformed, stale, or names a dead process stays ambiguous and
/// quarantines.
///
/// Deliberately synchronous: every method is a microsecond accessor over
/// a caller-held flocked fd (`pread`/`pwrite`/`ftruncate`/`fstat`) or a
/// `/proc` read. The multi-process exclusion itself (the `flock(2)`) is
/// owned by the broker caller and survives unchanged; converting these
/// fd helpers to async would force an owned-fd handoff across await for
/// no blocking-class reduction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncLockOwnerRecord {
    /// Fixed schema token ([`SYNC_LOCK_OWNER_SCHEMA`]).
    pub schema: String,
    /// Kernel-reported process name (`comm`) of the owner.
    pub owner: String,
    /// Owner pid at record time.
    pub pid: i32,
    /// Owner `/proc/<pid>/stat` field 22 (start-time ticks).
    pub process_start_time_ticks: u64,
    /// `/proc/sys/kernel/random/boot_id` at record time.
    pub boot_id: String,
}

impl SyncLockOwnerRecord {
    /// Record the calling process as the live owner.
    ///
    /// The owner name is the kernel-reported `comm` of this process, so
    /// the record can only ever claim the identity the kernel reports.
    pub fn for_current_process() -> Result<Self, SyncLockOwnerError> {
        let pid = std::process::id() as i32;
        let (owner, start_ticks) = process_identity(pid)?;
        Ok(Self {
            schema: SYNC_LOCK_OWNER_SCHEMA.to_owned(),
            owner,
            pid,
            process_start_time_ticks: start_ticks,
            boot_id: current_boot_id()?,
        })
    }

    /// Parse and shape-check the closed record.
    pub fn parse(bytes: &[u8]) -> Result<Self, SyncLockOwnerError> {
        let record: Self = serde_json::from_slice(bytes).map_err(|_| SyncLockOwnerError::Malformed)?;
        record.validate_shape()?;
        Ok(record)
    }

    /// Serialize to the canonical record bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        // The shape is fixed by `validate_shape`, so serialization cannot
        // fail for a record this type constructed or parsed.
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Check the closed shape: schema token, bounded printable owner,
    /// positive pid, non-zero start time, boot-id spelling.
    pub fn validate_shape(&self) -> Result<(), SyncLockOwnerError> {
        if self.schema != SYNC_LOCK_OWNER_SCHEMA
            || self.owner.is_empty()
            || self.owner.len() > 64
            || !self
                .owner
                .bytes()
                .all(|byte| byte.is_ascii_graphic() || byte == b' ')
            || self.pid <= 1
            || self.process_start_time_ticks == 0
            || self.boot_id.len() != 36
            || !self
                .boot_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
        {
            return Err(SyncLockOwnerError::Malformed);
        }
        Ok(())
    }

    /// Verify the record against the live kernel: same boot, recorded
    /// process alive at exactly the recorded start time, and the
    /// kernel-reported process name equal to the recorded owner.
    pub fn verify_live(&self) -> Result<(), SyncLockOwnerError> {
        self.validate_shape()?;
        if self.boot_id != current_boot_id()? {
            return Err(SyncLockOwnerError::ForeignBoot);
        }
        let (owner, start_ticks) = process_identity(self.pid)?;
        if start_ticks != self.process_start_time_ticks {
            return Err(SyncLockOwnerError::ProcessGone);
        }
        if owner != self.owner {
            return Err(SyncLockOwnerError::OwnerMismatch);
        }
        Ok(())
    }

    /// Write the record at offset 0 of the locked file and truncate any
    /// trailing bytes a previous, longer record left behind.
    pub fn write_locked(&self, fd: impl std::os::fd::AsFd) -> Result<(), SyncLockOwnerError> {
        self.validate_shape()?;
        let bytes = self.to_bytes();
        if bytes.is_empty() || bytes.len() > SYNC_LOCK_OWNER_MAX_BYTES {
            return Err(SyncLockOwnerError::Malformed);
        }
        write_all_at(&fd, &bytes)?;
        rustix::fs::ftruncate(fd, bytes.len() as u64).map_err(|_| SyncLockOwnerError::Io)?;
        Ok(())
    }

    /// Read the record the owner wrote into the locked file.
    pub fn read_locked(fd: impl std::os::fd::AsFd) -> Result<Self, SyncLockOwnerError> {
        let stat = rustix::fs::fstat(&fd).map_err(|_| SyncLockOwnerError::Io)?;
        if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::RegularFile
            || stat.st_size <= 0
            || stat.st_size as usize > SYNC_LOCK_OWNER_MAX_BYTES
        {
            return Err(SyncLockOwnerError::Malformed);
        }
        let mut bytes = vec![0u8; stat.st_size as usize];
        let mut offset = 0usize;
        while offset < bytes.len() {
            let read = rustix::io::pread(&fd, &mut bytes[offset..], offset as u64)
                .map_err(|_| SyncLockOwnerError::Io)?;
            if read == 0 {
                return Err(SyncLockOwnerError::Io);
            }
            offset += read;
        }
        Self::parse(&bytes)
    }

    /// Record the calling process into an already-locked `sync.lock`.
    pub fn record_current_process(fd: impl std::os::fd::AsFd) -> Result<Self, SyncLockOwnerError> {
        let record = Self::for_current_process()?;
        record.write_locked(fd)?;
        Ok(record)
    }
}

/// Write the full buffer at offset 0 (short `pwrite` loops).
fn write_all_at(fd: impl std::os::fd::AsFd, bytes: &[u8]) -> Result<(), SyncLockOwnerError> {
    let mut offset = 0usize;
    while offset < bytes.len() {
        let written = rustix::io::pwrite(&fd, &bytes[offset..], offset as u64)
            .map_err(|_| SyncLockOwnerError::Io)?;
        if written == 0 {
            return Err(SyncLockOwnerError::Io);
        }
        offset += written;
    }
    Ok(())
}

/// The current boot id, as the kernel reports it.
///
/// Deliberately synchronous: one short `/proc` read at the sync lock-
/// owner boundary (see the [`SyncLockOwnerRecord`] module docs); the
/// multi-process exclusion is the broker's flock(2) and survives
/// unchanged.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
pub fn current_boot_id() -> Result<String, SyncLockOwnerError> {
    let raw = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_err(|_| SyncLockOwnerError::Io)?;
    Ok(raw.trim().to_owned())
}

/// Read one process's kernel-reported `(comm, start-time ticks)` from
/// `/proc/<pid>/stat`. Field 2 is the parenthesized `comm` (which may
/// itself contain spaces and parentheses, so the parse splits at the
/// last `)`) and field 22 is the start time in clock ticks.
// Sync companion of `current_boot_id`: one short /proc/<pid>/stat
// read at the same sync lock-owner boundary.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn process_identity(pid: i32) -> Result<(String, u64), SyncLockOwnerError> {
    if pid <= 1 {
        return Err(SyncLockOwnerError::ProcessGone);
    }
    let raw = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|_| SyncLockOwnerError::ProcessGone)?;
    let open = raw.find('(').ok_or(SyncLockOwnerError::Malformed)?;
    let close = raw.rfind(')').ok_or(SyncLockOwnerError::Malformed)?;
    if close <= open {
        return Err(SyncLockOwnerError::Malformed);
    }
    let owner = raw[open + 1..close].to_owned();
    let start_ticks = raw[close + 1..]
        .split_whitespace()
        .nth(19)
        .and_then(|field| field.parse::<u64>().ok())
        .ok_or(SyncLockOwnerError::Malformed)?;
    Ok((owner, start_ticks))
}

/// Guest-served per-generation metadata dir
/// (`meta/generations/<generation-id>`).
pub fn meta_generation_dir(store_root: &Path, generation_id: &str) -> PathBuf {
    meta_dir(store_root).join("generations").join(generation_id)
}

/// Host-only per-generation metadata dir
/// (`state/generations/<generation-id>`).
pub fn state_generation_dir(store_root: &Path, generation_id: &str) -> PathBuf {
    state_dir(store_root)
        .join("generations")
        .join(generation_id)
}

/// Derive the collision-free on-disk `generation_id` from the full
/// ordered closure identity plus the system/toplevel store path.
///
/// ADR 0027 requires the on-disk key to be derived from the full closure
/// identity (not the truncated u32 token, which only survives as
/// `generation_token` display/wire metadata). The id is a SHA-256 over
/// the deterministically-sorted, length-prefixed set of top-level closure
/// basenames plus the system path, so it is independent of the input
/// ordering and distinct closures cannot collide onto one directory key.
/// The `g-` prefix keeps the name a single, separator-free, filesystem-
/// safe path component.
pub fn generation_id(closure_paths: &[PathBuf], system_path: Option<&Path>) -> String {
    let mut basenames: Vec<String> = closure_paths
        .iter()
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_owned))
        .collect();
    basenames.sort_unstable();
    basenames.dedup();

    let mut hasher = Sha256::new();
    hasher.update((basenames.len() as u64).to_le_bytes());
    for name in &basenames {
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
    }
    hasher.update(b"system:");
    if let Some(system) = system_path {
        let rendered = system.to_string_lossy();
        hasher.update((rendered.len() as u64).to_le_bytes());
        hasher.update(rendered.as_bytes());
    } else {
        hasher.update((0u64).to_le_bytes());
    }
    let digest = hasher.finalize();
    let hex = digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    format!("g-{hex}")
}

/// Pick the system/toplevel store path from a closure for the `system`
/// symlink and GC root: prefer the `nixos-system-*` basename, else the
/// first path. Mirrors `write_system_symlink`'s selection so the
/// `generation_id`, the `state/generations/<id>/system` symlink, and the
/// `gcroots/generation-<id>` root all reference the same store path.
pub fn system_store_path(closure_paths: &[PathBuf]) -> Option<&Path> {
    closure_paths
        .iter()
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.contains("nixos-system-"))
                .unwrap_or(false)
        })
        .or_else(|| closure_paths.first())
        .map(|p| p.as_path())
}

/// Returns `Ok(())` iff `a` and `b` live on the same filesystem
/// (same `st_dev`). Surfaces [`HardlinkFarmError::DifferentFilesystem`]
/// otherwise - the broker uses this BEFORE issuing any `link(2)`
/// call so it can fail-fast with a typed error instead of EXDEV.
pub async fn assert_same_filesystem(a: &Path, b: &Path) -> Result<(), HardlinkFarmError> {
    let a_dev = tokio::fs::metadata(a)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: a.display().to_string(),
            detail: e.to_string(),
        })?
        .dev();
    let b_dev = tokio::fs::metadata(b)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: b.display().to_string(),
            detail: e.to_string(),
        })?
        .dev();
    if a_dev != b_dev {
        return Err(HardlinkFarmError::DifferentFilesystem {
            a: a.display().to_string(),
            a_dev,
            b: b.display().to_string(),
            b_dev,
        });
    }
    Ok(())
}

/// Write the per-generation marker file. Tmp+rename+fsync so a
/// crash mid-write leaves either the old marker or no marker. The
/// parent dir is fsynced AFTER rename so the directory entry is
/// durable on ext4 / xfs / btrfs (matters under power loss; an
/// in-process crash without power loss is already safe via the
/// rename atomicity).
pub async fn write_generation_marker(
    generation_dir: &Path,
    marker: &GenerationMarker,
) -> Result<(), HardlinkFarmError> {
    let marker_path = generation_dir.join("marker.json");
    let bytes = serde_json::to_vec_pretty(marker).map_err(|e| HardlinkFarmError::Io {
        path: marker_path.display().to_string(),
        detail: format!("serialize: {e}"),
    })?;
    tokio::fs::create_dir_all(generation_dir)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: generation_dir.display().to_string(),
            detail: e.to_string(),
        })?;
    let tmp = marker_path.with_extension("json.tmp");
    {
        let mut f = tokio::fs::File::create(&tmp)
            .await
            .map_err(|e| HardlinkFarmError::Io {
                path: tmp.display().to_string(),
                detail: e.to_string(),
            })?;
        f.write_all(&bytes).await.map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
        f.sync_all().await.map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
    }
    tokio::fs::rename(&tmp, &marker_path)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: marker_path.display().to_string(),
            detail: e.to_string(),
        })?;
    // Review note: fsync the parent dir after
    // rename so the directory entry is durable. tmpfs is a no-op
    // here (it has no on-disk backing) but ext4 / xfs / btrfs need
    // this for full crash safety. Best-effort: errors are
    // non-fatal - the marker file itself is already on disk via
    // the f.sync_all() above.
    if let Ok(dir) = tokio::fs::File::open(generation_dir).await {
        let _ = dir.sync_all().await;
    }
    Ok(())
}

/// Build or reconcile one generation of the per-VM hardlink farm.
///
/// `store_root` is the per-VM farm root (`.../store-view`). Every
/// source path lands under `live/<basename>` if it is not already
/// present. `generations/<N>/` contains metadata only.
pub async fn build_farm(
    store_root: &Path,
    generation_number: u64,
    closure_paths: &[PathBuf],
    marker: &GenerationMarker,
) -> Result<PathBuf, HardlinkFarmError> {
    let generation_dir = generation_dir(store_root, generation_number);
    let live_dir = live_dir(store_root);
    // Fail-closed collision guard: the store-view generation number is
    // a content-derived u32 (see closures-json.nix). If this generation
    // dir already exists with a marker for a DIFFERENT closure, two
    // distinct closures of this VM collided onto the same u32. Refuse
    // rather than hardlink the new closure on top of the old one (which
    // would produce a mixed store view and corrupt rollback). Reusing a
    // dir for the SAME closure stays idempotent.
    let existing_marker_path = generation_dir.join("marker.json");
    if tokio::fs::try_exists(&generation_dir).await.unwrap_or(false) {
        if tokio::fs::try_exists(&existing_marker_path)
            .await
            .unwrap_or(false)
        {
            let existing = read_generation_marker(&generation_dir).await?;
            if existing.closure_hash != marker.closure_hash {
                return Err(HardlinkFarmError::GenerationCollision {
                    generation_dir: generation_dir.display().to_string(),
                    existing: existing.closure_hash,
                    incoming: marker.closure_hash.clone(),
                });
            }
            let live_marker_present = tokio::fs::try_exists(
                live_dir.join(format!(".d2b-marker-{}", marker.vm)),
            )
            .await
            .unwrap_or(false);
            let mut all_closures_present = true;
            for p in closure_paths {
                let present = match p.file_name() {
                    Some(name) => {
                        tokio::fs::try_exists(live_dir.join(name)).await.unwrap_or(false)
                    }
                    None => false,
                };
                if !present {
                    all_closures_present = false;
                    break;
                }
            }
            if existing.vm == marker.vm
                && existing.generation_number == marker.generation_number
                && live_marker_present
                && all_closures_present
            {
                return Ok(generation_dir);
            }
        } else {
            // Populated generation dir with no trusted marker: a build
            // that crashed before write_generation_marker. It is never
            // activatable (swap_current_symlink + read_generation_marker
            // both require the marker) and its contents can't be trusted
            // to belong to this closure - so a colliding closure must not
            // be hardlinked on top of it. Rebuild the generation from
            // scratch instead of unioning the partial leftovers.
            tokio::fs::remove_dir_all(&generation_dir)
                .await
                .map_err(|e| HardlinkFarmError::Io {
                    path: generation_dir.display().to_string(),
                    detail: e.to_string(),
                })?;
        }
    }
    tokio::fs::create_dir_all(store_root)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: store_root.display().to_string(),
            detail: e.to_string(),
        })?;
    tokio::fs::create_dir_all(&live_dir)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: live_dir.display().to_string(),
            detail: e.to_string(),
        })?;
    assert_same_filesystem(store_root, &live_dir).await?;

    let _ = link_closures_into_live(store_root, &generation_number.to_string(), closure_paths)
        .await?;

    tokio::fs::create_dir_all(&generation_dir)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: generation_dir.display().to_string(),
            detail: e.to_string(),
        })?;
    assert_same_filesystem(store_root, &generation_dir).await?;
    write_store_paths(&generation_dir, closure_paths).await?;
    write_system_symlink(&generation_dir, closure_paths).await?;
    write_guest_meta(
        &generation_dir,
        &marker.closure_hash,
        marker,
        closure_paths.len(),
    )
    .await?;
    write_generation_marker(&generation_dir, marker).await?;
    write_live_marker(&live_dir, &marker.vm).await?;
    Ok(generation_dir)
}

/// Stage and hardlink every top-level closure basename into `live/`.
///
/// Shared by the legacy [`build_farm`] and the split-layout
/// [`build_store_view`]. Top-level paths already present in `live/` are
/// skipped (the flat pool is shared across retained generations); the
/// rest are hardlinked into a private `live.stage.<tag>.<pid>` sibling
/// and atomically renamed into `live/`. `store_root` and `live/` must
/// already exist and share one filesystem (the caller asserts this).
/// Returns the top-level link/skip accounting.
async fn link_closures_into_live(
    store_root: &Path,
    stage_tag: &str,
    closure_paths: &[PathBuf],
) -> Result<StoreViewLinkCounts, HardlinkFarmError> {
    let live_dir = live_dir(store_root);
    let stage_dir = store_root.join(format!("live.stage.{}.{}", stage_tag, std::process::id()));
    if tokio::fs::try_exists(&stage_dir).await.unwrap_or(false) {
        tokio::fs::remove_dir_all(&stage_dir)
            .await
            .map_err(|e| HardlinkFarmError::Io {
                path: stage_dir.display().to_string(),
                detail: e.to_string(),
            })?;
    }

    tokio::fs::create_dir_all(&stage_dir)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: stage_dir.display().to_string(),
            detail: e.to_string(),
        })?;

    let mut counts = StoreViewLinkCounts::default();
    let build_result: Result<(), HardlinkFarmError> = async {
        for source in closure_paths {
            assert_same_filesystem(source, store_root).await?;
            let file_name = source.file_name().ok_or_else(|| HardlinkFarmError::Io {
                path: source.display().to_string(),
                detail: "source path has no basename".to_owned(),
            })?;
            let live_path = live_dir.join(file_name);
            if tokio::fs::try_exists(&live_path).await.unwrap_or(false) {
                counts.skipped = counts.skipped.saturating_add(1);
                continue;
            }
            let staged_path = stage_dir.join(file_name);
            hardlink_tree(source, &staged_path).await?;
            fsync_tree_bottom_up(&staged_path).await?;
            fsync_dir(&stage_dir).await?;
            match tokio::fs::rename(&staged_path, &live_path).await {
                Ok(()) => {
                    counts.linked = counts.linked.saturating_add(1);
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    let _ = tokio::fs::remove_dir_all(&staged_path).await;
                    counts.skipped = counts.skipped.saturating_add(1);
                }
                Err(err) => {
                    return Err(HardlinkFarmError::Io {
                        path: live_path.display().to_string(),
                        detail: err.to_string(),
                    });
                }
            }
        }
        if counts.linked > 0 {
            fsync_dir(&live_dir).await?;
        }
        Ok(())
    }
    .await;

    if let Err(err) = build_result {
        let _ = tokio::fs::remove_dir_all(&stage_dir).await;
        return Err(err);
    }
    let _ = tokio::fs::remove_dir_all(&stage_dir).await;
    Ok(counts)
}

/// Atomic `renameat2(RENAME_EXCHANGE)` of two paths.
///
/// `tokio::fs` has no async form for `renameat2` with `RENAME_EXCHANGE`
/// (the only atomic swap of two existing paths), and `spawn_blocking`
/// is banned by the async-purity policy (plan KD2), so the single
/// exchange syscall runs inline. It is the same blocking class as a
/// plain `rename(2)` (path resolution + one metadata exchange,
/// microseconds) - not a long op - so it adds negligible pressure to
/// the runtime, exactly like the short-stat class the async-purity plan
/// explicitly accepts.
async fn rename_exchange(staged: &Path, live: &Path) -> Result<(), HardlinkFarmError> {
    renameat_with(CWD, staged, CWD, live, RenameFlags::EXCHANGE).map_err(|e| {
        HardlinkFarmError::Io {
            path: live.display().to_string(),
            detail: format!("renameat2(RENAME_EXCHANGE): {e}"),
        }
    })
}

pub async fn replace_live_top_level_paths(
    store_root: &Path,
    stage_tag: &str,
    closure_paths: &[PathBuf],
) -> Result<StoreViewLinkCounts, HardlinkFarmError> {
    let live_dir = live_dir(store_root);
    tokio::fs::create_dir_all(&live_dir)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: live_dir.display().to_string(),
            detail: e.to_string(),
        })?;
    assert_same_filesystem(store_root, &live_dir).await?;
    let stage_dir = store_root.join(format!(
        "live.repair.stage.{}.{}",
        stage_tag,
        std::process::id()
    ));
    if tokio::fs::try_exists(&stage_dir).await.unwrap_or(false) {
        tokio::fs::remove_dir_all(&stage_dir)
            .await
            .map_err(|e| HardlinkFarmError::Io {
                path: stage_dir.display().to_string(),
                detail: e.to_string(),
            })?;
    }
    tokio::fs::create_dir_all(&stage_dir)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: stage_dir.display().to_string(),
            detail: e.to_string(),
        })?;

    let mut counts = StoreViewLinkCounts::default();
    let result: Result<(), HardlinkFarmError> = async {
        for source in closure_paths {
            assert_same_filesystem(source, store_root).await?;
            let file_name = source.file_name().ok_or_else(|| HardlinkFarmError::Io {
                path: source.display().to_string(),
                detail: "source path has no basename".to_owned(),
            })?;
            let staged_path = stage_dir.join(file_name);
            let live_path = live_dir.join(file_name);
            hardlink_tree(source, &staged_path).await?;
            fsync_tree_bottom_up(&staged_path).await?;
            fsync_dir(&stage_dir).await?;
            if tokio::fs::symlink_metadata(&live_path).await.is_ok() {
                rename_exchange(&staged_path, &live_path).await?;
                fsync_dir(&live_dir).await?;
                remove_path_any(&staged_path).await?;
                fsync_dir(&stage_dir).await?;
            } else {
                tokio::fs::rename(&staged_path, &live_path)
                    .await
                    .map_err(|e| HardlinkFarmError::Io {
                        path: live_path.display().to_string(),
                        detail: e.to_string(),
                    })?;
                fsync_dir(&live_dir).await?;
            }
            counts.linked = counts.linked.saturating_add(1);
        }
        Ok(())
    }
    .await;
    if let Err(err) = result {
        let _ = tokio::fs::remove_dir_all(&stage_dir).await;
        return Err(err);
    }
    let _ = tokio::fs::remove_dir_all(&stage_dir).await;
    Ok(counts)
}

async fn remove_path_any(path: &Path) -> Result<(), HardlinkFarmError> {
    let meta = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: path.display().to_string(),
            detail: format!("stat before remove: {e}"),
        })?;
    if meta.is_dir() {
        tokio::fs::remove_dir_all(path)
            .await
            .map_err(|e| HardlinkFarmError::Io {
                path: path.display().to_string(),
                detail: format!("remove dir: {e}"),
            })
    } else {
        tokio::fs::remove_file(path)
            .await
            .map_err(|e| HardlinkFarmError::Io {
                path: path.display().to_string(),
                detail: format!("remove file: {e}"),
            })
    }
}

async fn fsync_dir(path: &Path) -> Result<(), HardlinkFarmError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: path.display().to_string(),
            detail: format!("open for fsync: {e}"),
        })?;
    file.sync_all().await.map_err(|e| HardlinkFarmError::Io {
        path: path.display().to_string(),
        detail: format!("fsync: {e}"),
    })
}

async fn fsync_tree_bottom_up(path: &Path) -> Result<(), HardlinkFarmError> {
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: path.display().to_string(),
            detail: format!("stat before fsync: {e}"),
        })?;
    if !metadata.is_dir() {
        return Ok(());
    }
    let mut entries = tokio::fs::read_dir(path)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: path.display().to_string(),
            detail: format!("read dir for fsync: {e}"),
        })?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: path.display().to_string(),
            detail: format!("read dir entry for fsync: {e}"),
        })?
    {
        let child = entry.path();
        if tokio::fs::symlink_metadata(&child)
            .await
            .map(|m| m.is_dir())
            .unwrap_or(false)
        {
            Box::pin(fsync_tree_bottom_up(&child)).await?;
        }
    }
    fsync_dir(path).await
}

/// Build (materialise) one generation of the ADR 0027 **split** store
/// view: link top-level closure basenames into the shared `live/` pool,
/// write guest-safe metadata under `meta/generations/<id>/`, host-only
/// metadata under `state/generations/<id>/`, and plant the host-only
/// `gcroots/generation-<id>` root.
///
/// `generation_id` is the collision-free on-disk key (see
/// [`generation_id`]). This function does NOT swap the `state/current` or
/// `meta/current` pointers, copy `db.dump`, or plant the live readiness
/// marker: those are the in-process "publish" steps the caller performs
/// after a successful (possibly cross-mount-retried) materialisation, in
/// the ADR-mandated order (state/current, then meta/current, then the
/// zero-length live marker LAST). Returns the top-level link accounting.
pub async fn build_store_view(
    store_root: &Path,
    generation_id: &str,
    closure_paths: &[PathBuf],
    marker: &GenerationMarker,
) -> Result<StoreViewLinkCounts, HardlinkFarmError> {
    let state_gen = state_generation_dir(store_root, generation_id);
    // Fail-closed collision/idempotency guard, scoped to the host-only
    // state generation marker. `generation_id` is a full-closure hash, so
    // a different `closure_hash` under the same id implies a SHA-256
    // collision: refuse rather than union two closures. The same closure
    // re-links idempotently; a markerless partial dir (crash before
    // marker write) is rebuilt from scratch.
    if tokio::fs::try_exists(&state_gen).await.unwrap_or(false) {
        if tokio::fs::try_exists(state_gen.join("marker.json"))
            .await
            .unwrap_or(false)
        {
            let existing = read_generation_marker(&state_gen).await?;
            if existing.closure_hash != marker.closure_hash {
                return Err(HardlinkFarmError::GenerationCollision {
                    generation_dir: state_gen.display().to_string(),
                    existing: existing.closure_hash,
                    incoming: marker.closure_hash.clone(),
                });
            }
        } else {
            tokio::fs::remove_dir_all(&state_gen)
                .await
                .map_err(|e| HardlinkFarmError::Io {
                    path: state_gen.display().to_string(),
                    detail: e.to_string(),
                })?;
        }
    }

    let live_dir = live_dir(store_root);
    tokio::fs::create_dir_all(store_root)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: store_root.display().to_string(),
            detail: e.to_string(),
        })?;
    tokio::fs::create_dir_all(&live_dir)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: live_dir.display().to_string(),
            detail: e.to_string(),
        })?;
    assert_same_filesystem(store_root, &live_dir).await?;

    let counts = link_closures_into_live(store_root, generation_id, closure_paths).await?;

    // Guest-served metadata (`meta/generations/<id>/`): store-paths +
    // guest-safe meta.json only. db.dump is copied in by the caller
    // before the meta/current swap.
    let meta_gen = meta_generation_dir(store_root, generation_id);
    tokio::fs::create_dir_all(&meta_gen)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: meta_gen.display().to_string(),
            detail: e.to_string(),
        })?;
    assert_same_filesystem(store_root, &meta_gen).await?;
    write_store_paths(&meta_gen, closure_paths).await?;
    write_guest_meta(&meta_gen, generation_id, marker, closure_paths.len()).await?;

    // Host-only metadata (`state/generations/<id>/`): system symlink,
    // broker marker, and the host record. Never served to the guest.
    tokio::fs::create_dir_all(&state_gen)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: state_gen.display().to_string(),
            detail: e.to_string(),
        })?;
    assert_same_filesystem(store_root, &state_gen).await?;
    write_system_symlink(&state_gen, closure_paths).await?;
    write_host_meta(
        &state_gen,
        generation_id,
        marker,
        &counts,
        closure_paths.len(),
    )
    .await?;
    write_generation_marker(&state_gen, marker).await?;

    plant_generation_gcroot(store_root, generation_id, closure_paths).await?;

    Ok(counts)
}

/// Plant the host-only `gcroots/generation-<id>` symlink pointing at the
/// generation's system store path, so concurrent host GC cannot collect a
/// source path already linked into `live/`. tmp+rename for crash safety.
async fn plant_generation_gcroot(
    store_root: &Path,
    generation_id: &str,
    closure_paths: &[PathBuf],
) -> Result<(), HardlinkFarmError> {
    let Some(system) = system_store_path(closure_paths) else {
        return Ok(());
    };
    let gcroots = gcroots_dir(store_root);
    tokio::fs::create_dir_all(&gcroots)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: gcroots.display().to_string(),
            detail: e.to_string(),
        })?;
    let link = gcroots.join(format!("generation-{generation_id}"));
    let tmp = gcroots.join(format!("generation-{generation_id}.tmp"));
    let _ = tokio::fs::remove_file(&tmp).await;
    tokio::fs::symlink(system, &tmp)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
    tokio::fs::rename(&tmp, &link)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: link.display().to_string(),
            detail: e.to_string(),
        })?;
    if let Ok(dir) = tokio::fs::File::open(&gcroots).await {
        let _ = dir.sync_all().await;
    }
    Ok(())
}

/// Copy the closure-scoped Nix DB dump into `meta/generations/<id>/db.dump`.
/// In-process (a byte copy, cross-mount-safe). tmp+rename for crash
/// safety. Must complete before the `meta/current` swap so the guest
/// never observes a current generation without its `db.dump`.
pub async fn write_meta_db_dump(
    store_root: &Path,
    generation_id: &str,
    db_dump_path: &Path,
) -> Result<(), HardlinkFarmError> {
    let meta_gen = meta_generation_dir(store_root, generation_id);
    tokio::fs::create_dir_all(&meta_gen)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: meta_gen.display().to_string(),
            detail: e.to_string(),
        })?;
    let target = meta_gen.join("db.dump");
    let tmp = meta_gen.join("db.dump.tmp");
    tokio::fs::copy(db_dump_path, &tmp)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: db_dump_path.display().to_string(),
            detail: format!("copy db.dump: {e}"),
        })?;
    tokio::fs::rename(&tmp, &target)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: target.display().to_string(),
            detail: format!("install db.dump: {e}"),
        })?;
    if let Ok(dir) = tokio::fs::File::open(&meta_gen).await {
        let _ = dir.sync_all().await;
    }
    Ok(())
}

async fn write_store_paths(
    generation_dir: &Path,
    closure_paths: &[PathBuf],
) -> Result<(), HardlinkFarmError> {
    let path = generation_dir.join("store-paths");
    let tmp = generation_dir.join("store-paths.tmp");
    // Build the whole payload first (one write, same bytes as the
    // per-line `writeln!` loop it replaces).
    let mut payload = String::new();
    for p in closure_paths {
        payload.push_str(&p.display().to_string());
        payload.push('\n');
    }
    {
        let mut file = tokio::fs::File::create(&tmp)
            .await
            .map_err(|e| HardlinkFarmError::Io {
                path: tmp.display().to_string(),
                detail: e.to_string(),
            })?;
        file.write_all(payload.as_bytes())
            .await
            .map_err(|e| HardlinkFarmError::Io {
                path: tmp.display().to_string(),
                detail: e.to_string(),
            })?;
        file.sync_all().await.map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
    }
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: path.display().to_string(),
            detail: e.to_string(),
        })
}

/// Write the guest-served generation metadata (`meta.json`).
///
/// ADR 0027: an independent allow-list serializer. This function is the
/// only path that authors the guest-visible `meta.json`; it builds the
/// document from primitives ([`GuestGenerationMeta`]) and never from the
/// full host audit record, so host-only fields cannot leak to the guest
/// even if a future field is added to the audit struct. tmp+rename+fsync
/// for crash safety.
async fn write_guest_meta(
    generation_dir: &Path,
    generation_id: &str,
    marker: &GenerationMarker,
    closure_count: usize,
) -> Result<(), HardlinkFarmError> {
    let meta = GuestGenerationMeta {
        schema_version: GUEST_META_SCHEMA_VERSION,
        generation_id: generation_id.to_owned(),
        generation_token: marker.generation_number,
        sync_status: "ok".to_owned(),
        closure_count: u32::try_from(closure_count).unwrap_or(u32::MAX),
    };
    let path = generation_dir.join("meta.json");
    let tmp = generation_dir.join("meta.json.tmp");
    let bytes = serde_json::to_vec_pretty(&meta).map_err(|e| HardlinkFarmError::Io {
        path: path.display().to_string(),
        detail: format!("serialize: {e}"),
    })?;
    {
        let mut file = tokio::fs::File::create(&tmp)
            .await
            .map_err(|e| HardlinkFarmError::Io {
                path: tmp.display().to_string(),
                detail: e.to_string(),
            })?;
        file.write_all(&bytes).await.map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
        file.sync_all().await.map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
    }
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: path.display().to_string(),
            detail: e.to_string(),
        })?;
    if let Ok(dir) = tokio::fs::File::open(generation_dir).await {
        let _ = dir.sync_all().await;
    }
    Ok(())
}

/// Write the host-only generation metadata
/// (`state/generations/<id>/meta.json`).
///
/// ADR 0027: host-confidential. Written under `state/` (never `meta/`),
/// so it is not exposed by the guest `d2b-meta` share. tmp+rename+fsync
/// for crash safety.
async fn write_host_meta(
    state_generation_dir: &Path,
    generation_id: &str,
    marker: &GenerationMarker,
    counts: &StoreViewLinkCounts,
    closure_count: usize,
) -> Result<(), HardlinkFarmError> {
    let meta = HostGenerationMeta {
        schema_version: HOST_META_SCHEMA_VERSION,
        generation_id: generation_id.to_owned(),
        generation_token: marker.generation_number,
        vm: marker.vm.clone(),
        sync_status: "ok".to_owned(),
        closure_count: u32::try_from(closure_count).unwrap_or(u32::MAX),
        linked_count: counts.linked,
        skipped_count: counts.skipped,
    };
    let path = state_generation_dir.join("meta.json");
    let tmp = state_generation_dir.join("meta.json.tmp");
    let bytes = serde_json::to_vec_pretty(&meta).map_err(|e| HardlinkFarmError::Io {
        path: path.display().to_string(),
        detail: format!("serialize: {e}"),
    })?;
    {
        let mut file = tokio::fs::File::create(&tmp)
            .await
            .map_err(|e| HardlinkFarmError::Io {
                path: tmp.display().to_string(),
                detail: e.to_string(),
            })?;
        file.write_all(&bytes).await.map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
        file.sync_all().await.map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
    }
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: path.display().to_string(),
            detail: e.to_string(),
        })?;
    if let Ok(dir) = tokio::fs::File::open(state_generation_dir).await {
        let _ = dir.sync_all().await;
    }
    Ok(())
}

async fn write_system_symlink(
    generation_dir: &Path,
    closure_paths: &[PathBuf],
) -> Result<(), HardlinkFarmError> {
    let Some(system) = closure_paths
        .iter()
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.contains("nixos-system-"))
                .unwrap_or(false)
        })
        .or_else(|| closure_paths.first())
    else {
        return Ok(());
    };
    let link = generation_dir.join("system");
    let tmp = generation_dir.join("system.tmp");
    let _ = tokio::fs::remove_file(&tmp).await;
    tokio::fs::symlink(system, &tmp)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
    tokio::fs::rename(&tmp, &link)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: link.display().to_string(),
            detail: e.to_string(),
        })
}

/// Plant the per-VM live readiness marker.
///
/// ADR 0027: the marker is a **zero-length** file. It is the
/// cold-start readiness signal and lives under the guest-served
/// `live/` pool, so it must carry no host paths, generation metadata,
/// counts, caller principal, or any other payload - its existence
/// alone is the signal and the readiness probe is a `test -e`.
///
/// Written via tmp+rename+fsync so a crash mid-plant leaves either the
/// old marker or no marker, never a torn file. The (empty) inode is
/// fsynced before the rename publishes it, and the `live/` directory is
/// fsynced after rename so the dirent is durable on ext4/xfs/btrfs.
async fn write_live_marker(live_dir: &Path, vm: &str) -> Result<(), HardlinkFarmError> {
    let marker = live_dir.join(format!(".d2b-marker-{vm}"));
    let tmp = live_dir.join(format!(".d2b-marker-{vm}.tmp"));
    {
        let file = tokio::fs::File::create(&tmp)
            .await
            .map_err(|e| HardlinkFarmError::Io {
                path: tmp.display().to_string(),
                detail: e.to_string(),
            })?;
        // Zero-length: write nothing. fsync the empty file so the
        // inode is durable before the rename makes it visible.
        file.sync_all().await.map_err(|e| HardlinkFarmError::Io {
            path: tmp.display().to_string(),
            detail: e.to_string(),
        })?;
    }
    tokio::fs::rename(&tmp, &marker)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: marker.display().to_string(),
            detail: e.to_string(),
        })?;
    if let Ok(dir) = tokio::fs::File::open(live_dir).await {
        let _ = dir.sync_all().await;
    }
    Ok(())
}

async fn hardlink_tree(source: &Path, destination: &Path) -> Result<(), HardlinkFarmError> {
    let metadata = tokio::fs::symlink_metadata(source)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: source.display().to_string(),
            detail: e.to_string(),
        })?;
    if metadata.file_type().is_symlink() {
        let target = tokio::fs::read_link(source)
            .await
            .map_err(|e| HardlinkFarmError::Io {
                path: source.display().to_string(),
                detail: e.to_string(),
            })?;
        if let Ok(existing_target) = tokio::fs::read_link(destination).await {
            if existing_target == target {
                return Ok(());
            }
            tokio::fs::remove_file(destination)
                .await
                .map_err(|e| HardlinkFarmError::Io {
                    path: destination.display().to_string(),
                    detail: e.to_string(),
                })?;
        } else if tokio::fs::symlink_metadata(destination).await.is_ok() {
            return Err(HardlinkFarmError::Io {
                path: destination.display().to_string(),
                detail: "existing destination is not a symlink".to_owned(),
            });
        }
        tokio::fs::symlink(&target, destination)
            .await
            .map_err(|e| HardlinkFarmError::Io {
                path: destination.display().to_string(),
                detail: e.to_string(),
            })?;
        return Ok(());
    }
    if metadata.is_dir() {
        tokio::fs::create_dir_all(destination)
            .await
            .map_err(|e| HardlinkFarmError::Io {
                path: destination.display().to_string(),
                detail: e.to_string(),
            })?;
        let mut entries = tokio::fs::read_dir(source)
            .await
            .map_err(|e| HardlinkFarmError::Io {
                path: source.display().to_string(),
                detail: e.to_string(),
            })?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| HardlinkFarmError::Io {
                path: source.display().to_string(),
                detail: e.to_string(),
            })?
        {
            Box::pin(hardlink_tree(&entry.path(), &destination.join(entry.file_name()))).await?;
        }
        mirror_metadata(source, destination, &metadata).await?;
        return Ok(());
    }
    if metadata.is_file() {
        if let Ok(existing) = tokio::fs::symlink_metadata(destination).await {
            if existing.is_file() {
                return Ok(());
            }
            return Err(HardlinkFarmError::Io {
                path: destination.display().to_string(),
                detail: "existing destination is not a file".to_owned(),
            });
        }
        if let Err(e) = tokio::fs::hard_link(source, destination).await {
            let src_dev = tokio::fs::metadata(source)
                .await
                .map(|m| m.dev())
                .unwrap_or(0);
            let dst_dev = match destination.parent() {
                Some(p) => tokio::fs::metadata(p).await.map(|m| m.dev()).unwrap_or(0),
                None => 0,
            };
            match classify_link_failure(e.raw_os_error(), src_dev, dst_dev) {
                // EXDEV on the SAME `st_dev`: source + destination are on
                // one underlying filesystem but different vfsmounts (the
                // NixOS `/nix/store` self-bind-mount). RECOVERABLE - the
                // broker retries inside a mount namespace where
                // `/nix/store` is lazily detached.
                LinkFailure::CrossMount => {
                    return Err(HardlinkFarmError::CrossMountLink {
                        source: source.display().to_string(),
                        destination: destination.display().to_string(),
                        dev: src_dev,
                    });
                }
                // EXDEV on DIFFERENT `st_dev`: genuinely different
                // filesystems (should already have been caught by
                // `assert_same_filesystem`). FATAL - no namespace helps.
                LinkFailure::DifferentFilesystem => {
                    return Err(HardlinkFarmError::DifferentFilesystem {
                        a: source.display().to_string(),
                        a_dev: src_dev,
                        b: destination.display().to_string(),
                        b_dev: dst_dev,
                    });
                }
                // EMLINK: the SOURCE inode is at the filesystem hardlink
                // ceiling (ext4 `EXT4_LINK_MAX` = 65000). `nix-store
                // --optimise` dedups every empty/tiny file onto a single
                // inode, so a long-lived host saturates those inodes -
                // after which no NEW hardlink to them can be created, in
                // ANY mount namespace (the limit is per-inode). Fall back
                // to a byte copy: the store file is read-only so the farm
                // view is identical, only already-saturated
                // (overwhelmingly empty) inodes pay the copy, and the copy
                // does not share the source inode (strictly safer for the
                // "never mutate a shared store inode" invariant).
                LinkFailure::CopyFallback => {
                    tokio::fs::copy(source, destination)
                        .await
                        .map_err(|ce| HardlinkFarmError::Io {
                            path: destination.display().to_string(),
                            detail: format!("copy fallback after EMLINK: {ce}"),
                        })?;
                    mirror_metadata(source, destination, &metadata).await?;
                    let copied = tokio::fs::File::open(destination)
                        .await
                        .map_err(|ce| HardlinkFarmError::Io {
                            path: destination.display().to_string(),
                            detail: format!("open copy fallback for fsync: {ce}"),
                        })?;
                    copied
                        .sync_all()
                        .await
                        .map_err(|ce| HardlinkFarmError::Io {
                            path: destination.display().to_string(),
                            detail: format!("fsync copy fallback after EMLINK: {ce}"),
                        })?;
                    return Ok(());
                }
                LinkFailure::Other => {
                    return Err(HardlinkFarmError::Io {
                        path: destination.display().to_string(),
                        detail: e.to_string(),
                    });
                }
            }
        }
        return Ok(());
    }
    Err(HardlinkFarmError::Io {
        path: source.display().to_string(),
        detail: "unsupported store path file type".to_owned(),
    })
}

async fn mirror_metadata(
    source: &Path,
    destination: &Path,
    metadata: &std::fs::Metadata,
) -> Result<(), HardlinkFarmError> {
    let dest_metadata = tokio::fs::symlink_metadata(destination)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: destination.display().to_string(),
            detail: format!("stat before metadata mirror from {}: {e}", source.display()),
        })?;
    if dest_metadata.uid() != metadata.uid() || dest_metadata.gid() != metadata.gid() {
        // Single chown syscall (microseconds); `nix` has no async form
        // and the async-purity policy bans `spawn_blocking`, so the
        // syscall runs inline - the short-syscall class the policy
        // explicitly accepts.
        chown(
            destination,
            Some(Uid::from_raw(metadata.uid())),
            Some(Gid::from_raw(metadata.gid())),
        )
        .map_err(|e| HardlinkFarmError::Io {
            path: destination.display().to_string(),
            detail: format!("mirror ownership from {}: {e}", source.display()),
        })?;
    }
    tokio::fs::set_permissions(destination, std::fs::Permissions::from_mode(metadata.mode() & 0o7777))
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: destination.display().to_string(),
            detail: format!("mirror mode from {}: {e}", source.display()),
        })?;
    Ok(())
}

/// Classification of a `link(2)` failure, used by `hardlink_tree` to
/// route the error. Kept as a pure function of `(errno, src_dev,
/// dst_dev)` so the EXDEV-vs-EMLINK branching can be unit-tested without
/// fabricating a cross-mount or a 65000-link inode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkFailure {
    /// EXDEV with matching `st_dev`: cross-vfsmount, retryable in a
    /// private mount namespace.
    CrossMount,
    /// EXDEV with differing `st_dev`: genuinely different filesystems,
    /// fatal.
    DifferentFilesystem,
    /// EMLINK: source inode at the hardlink ceiling, fall back to copy.
    CopyFallback,
    /// Any other errno: propagate as a generic I/O error.
    Other,
}

fn classify_link_failure(raw_os_error: Option<i32>, src_dev: u64, dst_dev: u64) -> LinkFailure {
    match raw_os_error {
        Some(EXDEV) if src_dev == dst_dev => LinkFailure::CrossMount,
        Some(EXDEV) => LinkFailure::DifferentFilesystem,
        Some(EMLINK) => LinkFailure::CopyFallback,
        _ => LinkFailure::Other,
    }
}

/// Read + parse the per-generation marker. Refuses to activate any
/// generation dir whose marker is missing or unparseable.
pub async fn read_generation_marker(
    generation_dir: &Path,
) -> Result<GenerationMarker, HardlinkFarmError> {
    let marker_path = generation_dir.join("marker.json");
    if !tokio::fs::try_exists(&marker_path).await.unwrap_or(false) {
        return Err(HardlinkFarmError::MarkerMissing {
            generation_dir: generation_dir.display().to_string(),
        });
    }
    let bytes = tokio::fs::read(&marker_path)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: marker_path.display().to_string(),
            detail: e.to_string(),
        })?;
    serde_json::from_slice(&bytes).map_err(|e| HardlinkFarmError::MarkerUnparseable {
        path: marker_path.display().to_string(),
        detail: e.to_string(),
    })
}

/// Atomic swap of the `<store_root>/current` symlink to point at
/// `<store_root>/generations/<N>`. Implementation:
///
/// 1. Verify the target generation has a valid marker (refuses
///    activation of marker-less dirs).
/// 2. Verify same-filesystem between the symlink parent and target
///    (hardlink farms only work on the same fs; symlinks are
///    cross-fs-tolerant, but the activation contract pins same-fs so
///    the surface stays consistent with the farm itself).
/// 3. Create `<store_root>/current.tmp -> generations/<N>` via
///    `symlinkat`.
/// 4. `renameat2` (via `nix::fcntl::renameat`) the tmp symlink over
///    `current`. POSIX `rename(2)` is atomic for symlinks; the
///    swap is either pre-state or post-state, never partial.
///
/// On a crash between step 3 and 4, `current.tmp` is left behind;
/// [`reconcile_stale_swap_tmp`] removes it on next activate-time.
pub async fn swap_current_symlink(
    store_root: &Path,
    generation_number: u32,
) -> Result<(), HardlinkFarmError> {
    let generation_dir = store_root
        .join("generations")
        .join(format!("{generation_number}"));
    // Step 1: marker validation.
    let marker = read_generation_marker(&generation_dir).await?;
    if marker.generation_number != generation_number {
        return Err(HardlinkFarmError::MarkerUnparseable {
            path: generation_dir.join("marker.json").display().to_string(),
            detail: format!(
                "marker.generationNumber={} does not match directory name {generation_number}",
                marker.generation_number
            ),
        });
    }

    let current_path = store_root.join("current");
    let tmp_path = store_root.join("current.tmp");

    // Step 2: same-filesystem check between store_root and the
    // generation dir. (Both are typically under the same prefix
    // anyway; the check catches a rare case where an operator
    // bind-mounted `generations/` from another fs.)
    assert_same_filesystem(store_root, &generation_dir).await?;

    // Step 3: clean up any stale tmp from a previous crashed swap.
    reconcile_stale_swap_tmp(store_root).await?;

    // Step 3: write the new tmp symlink.
    let relative_target = PathBuf::from("generations").join(format!("{generation_number}"));
    tokio::fs::symlink(&relative_target, &tmp_path)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: tmp_path.display().to_string(),
            detail: e.to_string(),
        })?;

    // Step 4: atomic rename over the existing current symlink.
    tokio::fs::rename(&tmp_path, &current_path)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: current_path.display().to_string(),
            detail: e.to_string(),
        })?;

    // Review note: fsync the store root AFTER
    // the rename so the directory entry update is durable under
    // power loss (ext4 with `data=writeback`, XFS, etc.). Best
    // effort: errors are non-fatal because the rename itself is
    // POSIX-atomic for symlinks - fsync only matters when the
    // filesystem batches metadata updates.
    if let Ok(dir) = tokio::fs::File::open(store_root).await {
        let _ = dir.sync_all().await;
    }

    Ok(())
}

/// Read the current generation number from `<store_root>/current`.
pub async fn current_generation(store_root: &Path) -> Result<Option<u64>, HardlinkFarmError> {
    let current = store_root.join("current");
    let target = match tokio::fs::read_link(&current).await {
        Ok(target) => target,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(HardlinkFarmError::Io {
                path: current.display().to_string(),
                detail: err.to_string(),
            });
        }
    };
    let Some(name) = target.file_name().and_then(|n| n.to_str()) else {
        return Ok(None);
    };
    match name.parse::<u64>() {
        Ok(n) => Ok(Some(n)),
        Err(_) => Ok(None),
    }
}

/// Sweep `live/` to the union of top-level store path basenames required by
/// `retained_generations`.
pub async fn sweep_live_pool(
    store_root: &Path,
    retained_generations: &[u64],
) -> Result<usize, HardlinkFarmError> {
    let live = live_dir(store_root);
    let mut desired = BTreeSet::new();
    for generation in retained_generations {
        let store_paths = generation_dir(store_root, *generation).join("store-paths");
        let content = match tokio::fs::read_to_string(&store_paths).await {
            Ok(content) => content,
            Err(_) => return Ok(0),
        };
        for line in content.lines().filter(|line| !line.trim().is_empty()) {
            let path = Path::new(line);
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                desired.insert(name.to_owned());
            }
        }
    }

    let mut removed = 0;
    let mut entries = match tokio::fs::read_dir(&live).await {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => {
            return Err(HardlinkFarmError::Io {
                path: live.display().to_string(),
                detail: err.to_string(),
            });
        }
    };

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: live.display().to_string(),
            detail: e.to_string(),
        })?
    {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if name_str.starts_with(".d2b-marker-") || name_str.starts_with("live.stage.") {
            continue;
        }
        if desired.contains(name_str) {
            continue;
        }
        let path = entry.path();
        let meta = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|e| HardlinkFarmError::Io {
                path: path.display().to_string(),
                detail: e.to_string(),
            })?;
        if meta.is_dir() {
            tokio::fs::remove_dir_all(&path)
                .await
                .map_err(|e| HardlinkFarmError::Io {
                    path: path.display().to_string(),
                    detail: e.to_string(),
                })?;
        } else {
            tokio::fs::remove_file(&path)
                .await
                .map_err(|e| HardlinkFarmError::Io {
                    path: path.display().to_string(),
                    detail: e.to_string(),
                })?;
        }
        removed += 1;
    }
    Ok(removed)
}

/// Remove a stale `current.tmp` left behind by a previous
/// activation that crashed between symlink-write and rename.
/// Idempotent: no error if the tmp doesn't exist.
pub async fn reconcile_stale_swap_tmp(store_root: &Path) -> Result<(), HardlinkFarmError> {
    let tmp_path = store_root.join("current.tmp");
    match tokio::fs::remove_file(&tmp_path).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(HardlinkFarmError::Io {
            path: tmp_path.display().to_string(),
            detail: err.to_string(),
        }),
    }
}

/// Atomically point `<base_dir>/current` at `generations/<generation_id>`.
///
/// `base_dir` is either `state/` or `meta/`. The target must already
/// exist (the caller materialised it). tmp symlink + POSIX-atomic
/// rename + directory fsync, mirroring [`swap_current_symlink`] but
/// keyed on the collision-free `generation_id` rather than a u32. The
/// relative target keeps `current` valid if the farm root is moved.
async fn swap_current_pointer(base_dir: &Path, generation_id: &str) -> Result<(), HardlinkFarmError> {
    let target_dir = base_dir.join("generations").join(generation_id);
    if !tokio::fs::try_exists(&target_dir).await.unwrap_or(false) {
        return Err(HardlinkFarmError::Io {
            path: target_dir.display().to_string(),
            detail: "cannot publish current: generation directory is missing".to_owned(),
        });
    }
    tokio::fs::create_dir_all(base_dir)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: base_dir.display().to_string(),
            detail: e.to_string(),
        })?;
    let current_path = base_dir.join("current");
    let tmp_path = base_dir.join("current.tmp");
    let _ = tokio::fs::remove_file(&tmp_path).await;
    let relative_target = PathBuf::from("generations").join(generation_id);
    tokio::fs::symlink(&relative_target, &tmp_path)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: tmp_path.display().to_string(),
            detail: e.to_string(),
        })?;
    tokio::fs::rename(&tmp_path, &current_path)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: current_path.display().to_string(),
            detail: e.to_string(),
        })?;
    if let Ok(dir) = tokio::fs::File::open(base_dir).await {
        let _ = dir.sync_all().await;
    }
    Ok(())
}

/// Publish `state/current -> generations/<generation_id>` (host-only).
/// ADR 0027 ordering: swap state BEFORE meta so the host broker's view
/// of the active generation is never behind the guest's.
pub async fn swap_state_current(
    store_root: &Path,
    generation_id: &str,
) -> Result<(), HardlinkFarmError> {
    swap_current_pointer(&state_dir(store_root), generation_id).await
}

/// Publish `meta/current -> generations/<generation_id>` (guest-served).
/// ADR 0027 ordering: swap meta AFTER state and BEFORE planting the live
/// marker.
pub async fn swap_meta_current(
    store_root: &Path,
    generation_id: &str,
) -> Result<(), HardlinkFarmError> {
    swap_current_pointer(&meta_dir(store_root), generation_id).await
}

/// Read the generation id a `<base_dir>/current` symlink points at, if any.
async fn read_current_pointer_id(base_dir: &Path) -> Option<String> {
    let current = base_dir.join("current");
    let target = tokio::fs::read_link(&current).await.ok()?;
    target
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_owned())
}

/// Read the active generation id under `state/current` (host-only view).
pub async fn read_state_current_id(store_root: &Path) -> Option<String> {
    read_current_pointer_id(&state_dir(store_root)).await
}

/// Read the active generation id under `meta/current` (guest-served view).
pub async fn read_meta_current_id(store_root: &Path) -> Option<String> {
    read_current_pointer_id(&meta_dir(store_root)).await
}

/// Remove stale `current.tmp` files left under `state/` and `meta/` by a
/// previous publish that crashed between symlink-write and rename.
/// Idempotent.
pub async fn reconcile_split_current_tmp(store_root: &Path) -> Result<(), HardlinkFarmError> {
    for base in [state_dir(store_root), meta_dir(store_root)] {
        let tmp_path = base.join("current.tmp");
        match tokio::fs::remove_file(&tmp_path).await {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(HardlinkFarmError::Io {
                    path: tmp_path.display().to_string(),
                    detail: err.to_string(),
                });
            }
        }
    }
    Ok(())
}

/// Plant the zero-length per-VM live readiness marker under `live/`.
///
/// ADR 0027: this is the LAST step of a publish - after `state/current`
/// and `meta/current` are swapped - so the marker's existence implies a
/// fully-published generation. Public wrapper over the private
/// `write_live_marker` so the split-layout StoreSync caller can plant
/// it explicitly (the legacy [`build_farm`] still plants it inline).
pub async fn plant_live_marker(store_root: &Path, vm: &str) -> Result<(), HardlinkFarmError> {
    let live = live_dir(store_root);
    tokio::fs::create_dir_all(&live)
        .await
        .map_err(|e| HardlinkFarmError::Io {
            path: live.display().to_string(),
            detail: e.to_string(),
        })?;
    write_live_marker(&live, vm).await
}

/// Read the u32 `generation_token` recorded in the host marker for a
/// materialised split generation, if present. Used to surface the wire
/// token for a fast-path (skip-relink) StoreSync without re-deriving it.
pub async fn read_generation_token(store_root: &Path, generation_id: &str) -> Option<u32> {
    let state_gen = state_generation_dir(store_root, generation_id);
    let marker = read_generation_marker(&state_gen).await.ok()?;
    Some(marker.generation_number)
}

/// Fast-path readiness probe for the split layout.
///
/// Returns true only when a COMPLETE, consistent same-generation layout
/// already exists, so StoreSync can skip relinking and republishing:
///   * `state/current` and `meta/current` both resolve to `generation_id`;
///   * the host state marker exists, parses, and names the same closure;
///   * the live readiness marker exists; and
///   * every top-level closure basename is already present in `live/`.
///
/// Any missing/mismatched component yields false (rebuild + republish),
/// never a success-shaped shortcut.
pub async fn split_fast_path_ready(
    store_root: &Path,
    generation_id: &str,
    vm: &str,
    closure_paths: &[PathBuf],
) -> bool {
    if read_state_current_id(store_root).await.as_deref() != Some(generation_id) {
        return false;
    }
    if read_meta_current_id(store_root).await.as_deref() != Some(generation_id) {
        return false;
    }
    let state_gen = state_generation_dir(store_root, generation_id);
    let marker = match read_generation_marker(&state_gen).await {
        Ok(marker) => marker,
        Err(_) => return false,
    };
    if marker.vm != vm {
        return false;
    }
    let live = live_dir(store_root);
    match tokio::fs::symlink_metadata(live.join(format!(".d2b-marker-{vm}"))).await {
        Ok(meta) if meta.is_file() && meta.len() == 0 => {}
        _ => return false,
    }
    let meta_gen = meta_generation_dir(store_root, generation_id);
    if !tokio::fs::metadata(meta_gen.join("store-paths"))
        .await
        .map(|m| m.is_file())
        .unwrap_or(false)
        || !tokio::fs::metadata(meta_gen.join("db.dump"))
            .await
            .map(|m| m.is_file())
            .unwrap_or(false)
    {
        return false;
    }
    for p in closure_paths {
        let present = match p.file_name() {
            Some(name) => tokio::fs::try_exists(live.join(name)).await.unwrap_or(false),
            None => false,
        };
        if !present {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    #[test]
    fn classify_link_failure_routes_each_errno() {
        // EXDEV with matching st_dev -> retryable cross-mount.
        assert_eq!(
            classify_link_failure(Some(EXDEV), 42, 42),
            LinkFailure::CrossMount
        );
        // EXDEV with differing st_dev -> fatal different-filesystem.
        assert_eq!(
            classify_link_failure(Some(EXDEV), 42, 99),
            LinkFailure::DifferentFilesystem
        );
        // EMLINK -> copy fallback regardless of devs.
        assert_eq!(
            classify_link_failure(Some(EMLINK), 42, 42),
            LinkFailure::CopyFallback
        );
        assert_eq!(
            classify_link_failure(Some(EMLINK), 42, 99),
            LinkFailure::CopyFallback
        );
        // Anything else -> generic Other (propagated as Io).
        assert_eq!(
            classify_link_failure(Some(libc_eacces()), 42, 42),
            LinkFailure::Other
        );
        assert_eq!(classify_link_failure(None, 42, 42), LinkFailure::Other);
    }

    // EACCES = 13; a representative "some other errno" without pulling in
    // libc just for the constant.
    fn libc_eacces() -> i32 {
        13
    }

    fn make_marker(r#gen: u32) -> GenerationMarker {
        GenerationMarker {
            closure_hash: format!("sha256:gen{gen}"),
            d2b_version: "0.4.0".to_owned(),
            activated_at: "2026-05-29T09:00:00Z".to_owned(),
            vm: "corp-vm".to_owned(),
            generation_number: r#gen,
        }
    }

    async fn build_generation(store: &Path, r#gen: u32) {
        let dir = store.join("generations").join(format!("{gen}"));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        write_generation_marker(&dir, &make_marker(r#gen)).await.unwrap();
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn assert_same_filesystem_matches_self() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("a");
        tokio::fs::create_dir_all(&p).await.unwrap();
        assert!(assert_same_filesystem(dir.path(), &p).await.is_ok());
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn assert_same_filesystem_surfaces_io_error_when_missing() {
        let dir = tempdir().unwrap();
        let result = assert_same_filesystem(dir.path(), &dir.path().join("nonexistent")).await;
        assert!(matches!(result, Err(HardlinkFarmError::Io { .. })));
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn build_farm_creates_generation_with_marker_and_hardlinks() {
        let dir = tempdir().unwrap();
        let source_root = dir.path().join("source-store");
        let farm_root = dir.path().join("farm");
        let system_path = source_root.join("abc-system");
        let subdir = system_path.join("bin");
        tokio::fs::create_dir_all(&subdir).await.unwrap();
        tokio::fs::write(subdir.join("switch-to-configuration"), b"#!/bin/sh\n")
            .await
            .unwrap();
        let shared = source_root.join("dep");
        tokio::fs::create_dir_all(&shared).await.unwrap();
        tokio::fs::write(shared.join("data"), b"hello").await.unwrap();
        std::os::unix::fs::symlink("../dep/data", system_path.join("data-link")).unwrap();

        let generation_dir = build_farm(
            &farm_root,
            7,
            &[system_path.clone(), shared.clone()],
            &GenerationMarker {
                closure_hash: "sha256:test".to_owned(),
                d2b_version: "0.4.0".to_owned(),
                activated_at: "2026-05-29T09:00:00Z".to_owned(),
                vm: "corp-vm".to_owned(),
                generation_number: 7,
            },
        )
        .await
        .unwrap();

        let farm_binary = live_dir(&farm_root).join("abc-system/bin/switch-to-configuration");
        assert!(farm_binary.exists());
        assert_eq!(
            tokio::fs::metadata(&farm_binary).await.unwrap().ino(),
            tokio::fs::metadata(system_path.join("bin/switch-to-configuration"))
                .await
                .unwrap()
                .ino()
        );
        assert_eq!(
            tokio::fs::read_link(live_dir(&farm_root).join("abc-system/data-link"))
                .await
                .unwrap(),
            PathBuf::from("../dep/data")
        );
        let marker = read_generation_marker(&generation_dir).await.unwrap();
        assert_eq!(marker.generation_number, 7);
        assert_eq!(marker.vm, "corp-vm");
        assert!(generation_dir.join("store-paths").exists());
        assert!(generation_dir.join("system").exists());
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn live_marker_is_zero_length() {
        let dir = tempdir().unwrap();
        let source_root = dir.path().join("source-store");
        let farm_root = dir.path().join("farm");
        let pkg = source_root.join("abc-pkg");
        tokio::fs::create_dir_all(&pkg).await.unwrap();
        tokio::fs::write(pkg.join("payload"), b"data").await.unwrap();

        build_farm(&farm_root, 3, std::slice::from_ref(&pkg), &make_marker(3))
            .await
            .unwrap();

        // ADR 0027: the guest-served readiness marker carries no payload.
        let marker = live_dir(&farm_root).join(".d2b-marker-corp-vm");
        let meta = tokio::fs::metadata(&marker).await.expect("live marker planted");
        assert!(meta.is_file(), "marker is a regular file");
        assert_eq!(meta.len(), 0, "live readiness marker must be zero-length");
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn guest_meta_json_has_exact_allow_list() {
        let dir = tempdir().unwrap();
        let source_root = dir.path().join("source-store");
        let farm_root = dir.path().join("farm");
        let a = source_root.join("aaa-pkg");
        let b = source_root.join("bbb-pkg");
        tokio::fs::create_dir_all(&a).await.unwrap();
        tokio::fs::create_dir_all(&b).await.unwrap();
        tokio::fs::write(a.join("payload"), b"a").await.unwrap();
        tokio::fs::write(b.join("payload"), b"b").await.unwrap();

        let marker = GenerationMarker {
            closure_hash: "sha256:deadbeef".to_owned(),
            d2b_version: "0.4.0".to_owned(),
            activated_at: "2026-06-09T09:00:00Z".to_owned(),
            vm: "corp-vm".to_owned(),
            generation_number: 9,
        };
        let generation_dir =
            build_farm(&farm_root, 9, &[a.clone(), b.clone()], &marker).await.unwrap();

        let raw = tokio::fs::read_to_string(generation_dir.join("meta.json"))
            .await
            .expect("guest meta.json written");
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let obj = value.as_object().expect("meta.json is a JSON object");

        // ADR 0027: the key set must equal exactly the guest allow-list.
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "closure_count",
                "generation_id",
                "generation_token",
                "schema_version",
                "sync_status",
            ],
            "guest meta.json must expose exactly the allow-listed keys"
        );
        assert_eq!(
            obj["schema_version"],
            serde_json::json!(GUEST_META_SCHEMA_VERSION)
        );
        assert_eq!(obj["generation_id"], serde_json::json!("sha256:deadbeef"));
        assert_eq!(obj["generation_token"], serde_json::json!(9));
        assert_eq!(obj["sync_status"], serde_json::json!("ok"));
        assert_eq!(obj["closure_count"], serde_json::json!(2));

        // Round-trips through the typed independent serializer.
        let typed: GuestGenerationMeta = serde_json::from_str(&raw).unwrap();
        assert_eq!(typed.generation_id, "sha256:deadbeef");
        assert_eq!(typed.closure_count, 2);
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn build_farm_idempotent_for_same_closure() {
        let dir = tempdir().unwrap();
        let source_root = dir.path().join("source-store");
        let farm_root = dir.path().join("farm");
        let system_path = source_root.join("abc-system");
        tokio::fs::create_dir_all(&system_path).await.unwrap();
        tokio::fs::write(system_path.join("payload"), b"data").await.unwrap();
        let marker = GenerationMarker {
            closure_hash: "toplevel:abc-system".to_owned(),
            d2b_version: "0.4.0".to_owned(),
            activated_at: "2026-05-29T09:00:00Z".to_owned(),
            vm: "corp-vm".to_owned(),
            generation_number: 7,
        };
        // Building the same closure into the same generation twice is a
        // no-op-equivalent: the second call reuses the dir + marker.
        build_farm(&farm_root, 7, std::slice::from_ref(&system_path), &marker)
            .await
            .unwrap();
        build_farm(&farm_root, 7, std::slice::from_ref(&system_path), &marker)
            .await
            .unwrap();
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn build_farm_refuses_generation_collision() {
        let dir = tempdir().unwrap();
        let source_root = dir.path().join("source-store");
        let farm_root = dir.path().join("farm");
        // Two DISTINCT closures (different toplevel identity) that
        // collided onto the same u32 generation number.
        let closure_a = source_root.join("aaa-system");
        let closure_b = source_root.join("bbb-system");
        tokio::fs::create_dir_all(&closure_a).await.unwrap();
        tokio::fs::create_dir_all(&closure_b).await.unwrap();
        tokio::fs::write(closure_a.join("payload"), b"a").await.unwrap();
        tokio::fs::write(closure_b.join("payload"), b"b").await.unwrap();

        build_farm(
            &farm_root,
            42,
            std::slice::from_ref(&closure_a),
            &GenerationMarker {
                closure_hash: "toplevel:aaa-system".to_owned(),
                d2b_version: "0.4.0".to_owned(),
                activated_at: "2026-05-29T09:00:00Z".to_owned(),
                vm: "corp-vm".to_owned(),
                generation_number: 42,
            },
        )
        .await
        .unwrap();

        // Same generation number, different closure identity → refuse
        // fail-closed rather than union the two closures.
        let result = build_farm(
            &farm_root,
            42,
            std::slice::from_ref(&closure_b),
            &GenerationMarker {
                closure_hash: "toplevel:bbb-system".to_owned(),
                d2b_version: "0.4.0".to_owned(),
                activated_at: "2026-05-29T09:00:00Z".to_owned(),
                vm: "corp-vm".to_owned(),
                generation_number: 42,
            },
        )
        .await;
        assert!(matches!(
            result,
            Err(HardlinkFarmError::GenerationCollision { .. })
        ));
        // The original closure's store view is untouched.
        assert!(live_dir(&farm_root).join("aaa-system/payload").exists());
        assert!(!live_dir(&farm_root).join("bbb-system/payload").exists());
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn build_farm_rebuilds_markerless_partial_generation() {
        let dir = tempdir().unwrap();
        let source_root = dir.path().join("source-store");
        let farm_root = dir.path().join("farm");
        let closure = source_root.join("ccc-system");
        tokio::fs::create_dir_all(&closure).await.unwrap();
        tokio::fs::write(closure.join("payload"), b"c").await.unwrap();

        // Simulate a crashed earlier build: a populated generation dir
        // with leftover files but NO marker.json.
        let stale_dir = farm_root.join("generations").join("9");
        tokio::fs::create_dir_all(&stale_dir).await.unwrap();
        tokio::fs::write(stale_dir.join("leftover-from-crash"), b"stale")
            .await
            .unwrap();

        // Building a (different) closure into the same generation must
        // NOT union the stale leftovers: it rebuilds from scratch.
        build_farm(
            &farm_root,
            9,
            std::slice::from_ref(&closure),
            &GenerationMarker {
                closure_hash: "toplevel:ccc-system".to_owned(),
                d2b_version: "0.4.0".to_owned(),
                activated_at: "2026-05-29T09:00:00Z".to_owned(),
                vm: "corp-vm".to_owned(),
                generation_number: 9,
            },
        )
        .await
        .unwrap();

        assert!(live_dir(&farm_root).join("ccc-system/payload").exists());
        assert!(!stale_dir.join("leftover-from-crash").exists());
        let marker = read_generation_marker(&stale_dir).await.unwrap();
        assert_eq!(marker.closure_hash, "toplevel:ccc-system");
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn build_farm_preserves_symlink_target_for_new_live_path() {
        let dir = tempdir().unwrap();
        let source_root = dir.path().join("source-store");
        let farm_root = dir.path().join("farm");
        let system_path = source_root.join("alpha-system");
        tokio::fs::create_dir_all(system_path.join("bin")).await.unwrap();
        tokio::fs::write(
            system_path.join("bin/switch-to-configuration"),
            b"#!/bin/sh\n",
        )
        .await
        .unwrap();
        let dep_dir = source_root.join("dep");
        tokio::fs::create_dir_all(&dep_dir).await.unwrap();
        tokio::fs::write(dep_dir.join("real"), b"real").await.unwrap();
        tokio::fs::write(dep_dir.join("wrong"), b"wrong").await.unwrap();
        std::os::unix::fs::symlink("../dep/real", system_path.join("data-link")).unwrap();

        build_farm(
            &farm_root,
            8,
            std::slice::from_ref(&system_path),
            &GenerationMarker {
                closure_hash: "sha256:test".to_owned(),
                d2b_version: "0.4.0".to_owned(),
                activated_at: "2026-05-29T09:00:00Z".to_owned(),
                vm: "corp-vm".to_owned(),
                generation_number: 8,
            },
        )
        .await
        .unwrap();

        let live_system_dir = live_dir(&farm_root).join("alpha-system");
        assert_eq!(
            tokio::fs::read_link(live_system_dir.join("data-link"))
                .await
                .unwrap(),
            PathBuf::from("../dep/real")
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn build_farm_preserves_broken_symlink_target_for_new_live_path() {
        let dir = tempdir().unwrap();
        let source_root = dir.path().join("source-store");
        let farm_root = dir.path().join("farm");
        let system_path = source_root.join("beta-system");
        tokio::fs::create_dir_all(system_path.join("bin")).await.unwrap();
        tokio::fs::write(
            system_path.join("bin/switch-to-configuration"),
            b"#!/bin/sh\n",
        )
        .await
        .unwrap();
        std::os::unix::fs::symlink("missing-target", system_path.join("data-link")).unwrap();

        build_farm(
            &farm_root,
            9,
            std::slice::from_ref(&system_path),
            &GenerationMarker {
                closure_hash: "sha256:test".to_owned(),
                d2b_version: "0.4.0".to_owned(),
                activated_at: "2026-05-29T09:00:00Z".to_owned(),
                vm: "corp-vm".to_owned(),
                generation_number: 9,
            },
        )
        .await
        .unwrap();

        let live_system_dir = live_dir(&farm_root).join("beta-system");
        assert_eq!(
            tokio::fs::read_link(live_system_dir.join("data-link"))
                .await
                .unwrap(),
            PathBuf::from("missing-target")
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn marker_round_trip() {
        let dir = tempdir().unwrap();
        let gen_dir = dir.path().join("generations/1");
        write_generation_marker(&gen_dir, &make_marker(1)).await.unwrap();
        let read = read_generation_marker(&gen_dir).await.unwrap();
        assert_eq!(read, make_marker(1));
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn marker_missing_is_typed_error() {
        let dir = tempdir().unwrap();
        let gen_dir = dir.path().join("generations/2");
        tokio::fs::create_dir_all(&gen_dir).await.unwrap();
        // No marker written.
        let result = read_generation_marker(&gen_dir).await;
        assert!(matches!(
            result,
            Err(HardlinkFarmError::MarkerMissing { .. })
        ));
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn marker_unparseable_is_typed_error() {
        let dir = tempdir().unwrap();
        let gen_dir = dir.path().join("generations/3");
        tokio::fs::create_dir_all(&gen_dir).await.unwrap();
        tokio::fs::write(gen_dir.join("marker.json"), b"not json")
            .await
            .unwrap();
        let result = read_generation_marker(&gen_dir).await;
        assert!(matches!(
            result,
            Err(HardlinkFarmError::MarkerUnparseable { .. })
        ));
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn marker_rejects_unknown_fields() {
        let dir = tempdir().unwrap();
        let gen_dir = dir.path().join("generations/4");
        tokio::fs::create_dir_all(&gen_dir).await.unwrap();
        // Inject a marker with an extra field - deny_unknown_fields
        // makes this an unparseable error.
        let json = serde_json::json!({
            "closureHash": "sha256:abc",
            "d2bVersion": "0.4.0",
            "activatedAt": "2026-05-29T09:00:00Z",
            "vm": "corp-vm",
            "generationNumber": 4,
            "extraField": "rejected"
        });
        tokio::fs::write(gen_dir.join("marker.json"), serde_json::to_vec(&json).unwrap())
            .await
            .unwrap();
        let result = read_generation_marker(&gen_dir).await;
        assert!(matches!(
            result,
            Err(HardlinkFarmError::MarkerUnparseable { .. })
        ));
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn swap_current_creates_symlink_to_target_generation() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("store");
        tokio::fs::create_dir_all(&store).await.unwrap();
        build_generation(&store, 1).await;

        swap_current_symlink(&store, 1).await.unwrap();

        let current = store.join("current");
        assert!(current.exists());
        let target = tokio::fs::read_link(&current).await.unwrap();
        assert_eq!(target, PathBuf::from("generations/1"));
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn swap_current_overwrites_existing_symlink() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("store");
        tokio::fs::create_dir_all(&store).await.unwrap();
        build_generation(&store, 1).await;
        build_generation(&store, 2).await;

        swap_current_symlink(&store, 1).await.unwrap();
        swap_current_symlink(&store, 2).await.unwrap();

        let target = tokio::fs::read_link(store.join("current")).await.unwrap();
        assert_eq!(target, PathBuf::from("generations/2"));
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn swap_current_refuses_marker_less_generation() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("store");
        tokio::fs::create_dir_all(store.join("generations/5"))
            .await
            .unwrap();
        // No marker written.

        let result = swap_current_symlink(&store, 5).await;
        assert!(matches!(
            result,
            Err(HardlinkFarmError::MarkerMissing { .. })
        ));
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn swap_current_refuses_marker_with_wrong_generation_number() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("store");
        tokio::fs::create_dir_all(&store).await.unwrap();
        let gen_dir = store.join("generations/6");
        // Marker claims generationNumber = 99 but lives in dir "6".
        let mut bogus_marker = make_marker(99);
        bogus_marker.generation_number = 99;
        write_generation_marker(&gen_dir, &bogus_marker).await.unwrap();

        let result = swap_current_symlink(&store, 6).await;
        assert!(matches!(
            result,
            Err(HardlinkFarmError::MarkerUnparseable { .. })
        ));
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_removes_stale_swap_tmp() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("store");
        tokio::fs::create_dir_all(&store).await.unwrap();
        // Simulate a crashed swap: leave current.tmp behind.
        std::os::unix::fs::symlink("generations/1", store.join("current.tmp")).unwrap();
        // `.exists()` follows symlinks; for a dangling symlink it
        // returns false. Use `symlink_metadata` to check link
        // presence regardless of target.
        assert!(tokio::fs::symlink_metadata(store.join("current.tmp"))
            .await
            .is_ok());

        reconcile_stale_swap_tmp(&store).await.unwrap();
        assert!(tokio::fs::symlink_metadata(store.join("current.tmp"))
            .await
            .is_err());
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_is_idempotent() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("store");
        tokio::fs::create_dir_all(&store).await.unwrap();
        // No current.tmp present; reconcile should be a no-op.
        reconcile_stale_swap_tmp(&store).await.unwrap();
        // Call twice for idempotency.
        reconcile_stale_swap_tmp(&store).await.unwrap();
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn sweep_live_pool_keeps_retained_generations_and_removes_stale_entries() {
        let dir = tempdir().unwrap();
        let source_root = dir.path().join("source-store");
        let farm_root = dir.path().join("farm");
        let gen1_path = source_root.join("aaa-system");
        let gen2_path = source_root.join("bbb-system");
        let stale_path = live_dir(&farm_root).join("stale-system");
        tokio::fs::create_dir_all(&gen1_path).await.unwrap();
        tokio::fs::create_dir_all(&gen2_path).await.unwrap();
        tokio::fs::write(gen1_path.join("payload"), b"a").await.unwrap();
        tokio::fs::write(gen2_path.join("payload"), b"b").await.unwrap();

        build_farm(
            &farm_root,
            1,
            std::slice::from_ref(&gen1_path),
            &GenerationMarker {
                closure_hash: "toplevel:aaa-system".to_owned(),
                d2b_version: "0.4.0".to_owned(),
                activated_at: "2026-05-29T09:00:00Z".to_owned(),
                vm: "corp-vm".to_owned(),
                generation_number: 1,
            },
        )
        .await
        .unwrap();
        build_farm(
            &farm_root,
            2,
            std::slice::from_ref(&gen2_path),
            &GenerationMarker {
                closure_hash: "toplevel:bbb-system".to_owned(),
                d2b_version: "0.4.0".to_owned(),
                activated_at: "2026-05-29T09:00:00Z".to_owned(),
                vm: "corp-vm".to_owned(),
                generation_number: 2,
            },
        )
        .await
        .unwrap();
        tokio::fs::create_dir_all(&stale_path).await.unwrap();
        tokio::fs::write(stale_path.join("payload"), b"stale").await.unwrap();

        let removed = sweep_live_pool(&farm_root, &[2]).await.unwrap();
        assert_eq!(removed, 2);
        assert!(!live_dir(&farm_root).join("aaa-system").exists());
        assert!(live_dir(&farm_root).join("bbb-system/payload").exists());
        assert!(!stale_path.exists());
        assert!(live_dir(&farm_root).join(".d2b-marker-corp-vm").exists());
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn swap_current_cleans_up_stale_tmp_before_writing_new_one() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("store");
        tokio::fs::create_dir_all(&store).await.unwrap();
        build_generation(&store, 1).await;
        // Leave a stale tmp from a previous crashed swap.
        std::os::unix::fs::symlink("generations/99", store.join("current.tmp")).unwrap();

        swap_current_symlink(&store, 1).await.unwrap();
        let target = tokio::fs::read_link(store.join("current")).await.unwrap();
        assert_eq!(target, PathBuf::from("generations/1"));
    }

    #[test]
    fn marker_round_trip_serializable() {
        let m = make_marker(7);
        let json = serde_json::to_string(&m).unwrap();
        let parsed: GenerationMarker = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, m);
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn split_closure(root: &Path) -> Vec<PathBuf> {
        let store = root.join("source-store");
        let system = store.join("zzz-nixos-system-host");
        let dep = store.join("aaa-dep");
        std::fs::create_dir_all(system.join("bin")).unwrap();
        std::fs::write(system.join("bin/switch-to-configuration"), b"#!/bin/sh\n").unwrap();
        std::fs::create_dir_all(&dep).unwrap();
        std::fs::write(dep.join("data"), b"payload").unwrap();
        vec![system, dep]
    }

    #[test]
    fn generation_id_is_order_independent_and_collision_free() {
        let dir = tempdir().unwrap();
        let closure = split_closure(dir.path());
        let system = system_store_path(&closure).map(Path::to_path_buf);
        let forward = generation_id(&closure, system.as_deref());
        let mut reversed = closure.clone();
        reversed.reverse();
        let backward = generation_id(&reversed, system.as_deref());
        assert_eq!(forward, backward, "id is independent of input order");
        assert!(forward.starts_with("g-"), "id carries the g- prefix");

        // A different closure (extra path) yields a different id.
        let mut bigger = closure.clone();
        bigger.push(dir.path().join("source-store/ccc-extra"));
        let other = generation_id(&bigger, system.as_deref());
        assert_ne!(forward, other);
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn build_store_view_writes_split_tree_with_guest_host_split() {
        let dir = tempdir().unwrap();
        let farm = dir.path().join("farm");
        let closure = split_closure(dir.path());
        let system = system_store_path(&closure).map(Path::to_path_buf);
        let gid = generation_id(&closure, system.as_deref());
        let marker = GenerationMarker {
            closure_hash: "sha256:split".to_owned(),
            d2b_version: "0.4.0".to_owned(),
            activated_at: "2026-06-09T09:00:00Z".to_owned(),
            vm: "corp-vm".to_owned(),
            generation_number: 11,
        };

        let counts = build_store_view(&farm, &gid, &closure, &marker).await.unwrap();
        assert_eq!(counts.linked, 2);
        assert_eq!(counts.skipped, 0);

        // live/: flat pool of top-level basenames, inode-shared.
        let live_bin = farm.join("live/zzz-nixos-system-host/bin/switch-to-configuration");
        assert!(live_bin.exists());
        assert_eq!(
            tokio::fs::metadata(&live_bin).await.unwrap().ino(),
            tokio::fs::metadata(closure[0].join("bin/switch-to-configuration"))
                .await
                .unwrap()
                .ino()
        );

        // meta/generations/<id>/: guest-safe only.
        let meta_gen = meta_generation_dir(&farm, &gid);
        assert!(meta_gen.join("store-paths").exists());
        assert!(meta_gen.join("meta.json").exists());
        assert!(!meta_gen.join("system").exists());
        assert!(!meta_gen.join("marker.json").exists());

        // state/generations/<id>/: host-only.
        let state_gen = state_generation_dir(&farm, &gid);
        assert!(state_gen.join("marker.json").exists());
        assert!(state_gen.join("system").exists());
        assert!(state_gen.join("meta.json").exists());

        // gcroots/generation-<id> -> system store path.
        let gcroot = gcroots_dir(&farm).join(format!("generation-{gid}"));
        assert!(gcroot.exists());
        assert_eq!(
            tokio::fs::read_link(&gcroot).await.unwrap(),
            closure[0]
        );

        // build_store_view must NOT swap currents or plant the marker.
        assert!(read_state_current_id(&farm).await.is_none());
        assert!(read_meta_current_id(&farm).await.is_none());
        assert!(!live_dir(&farm).join(".d2b-marker-corp-vm").exists());

        // Guest meta.json key set is exactly the allow-list, generation_id
        // is the split key, and no host-only field leaks in.
        let guest: serde_json::Value =
            serde_json::from_slice(&tokio::fs::read(meta_gen.join("meta.json")).await.unwrap())
                .unwrap();
        let gobj = guest.as_object().unwrap();
        let mut keys: Vec<&str> = gobj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "closure_count",
                "generation_id",
                "generation_token",
                "schema_version",
                "sync_status",
            ]
        );
        assert_eq!(gobj["generation_id"], serde_json::json!(gid));
        assert_eq!(gobj["generation_token"], serde_json::json!(11));

        // Host meta.json carries the host-only fields.
        let host: HostGenerationMeta =
            serde_json::from_slice(&tokio::fs::read(state_gen.join("meta.json")).await.unwrap())
                .unwrap();
        assert_eq!(host.generation_id, gid);
        assert_eq!(host.generation_token, 11);
        assert_eq!(host.vm, "corp-vm");
        assert_eq!(host.linked_count, 2);
        assert_eq!(host.closure_count, 2);
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn hardlink_tree_preserves_directory_modes() {
        let dir = tempdir().unwrap();
        let closure = split_closure(dir.path());
        let source_top = &closure[0];
        let source_bin = source_top.join("bin");
        tokio::fs::set_permissions(source_top, std::fs::Permissions::from_mode(0o555))
            .await
            .unwrap();
        tokio::fs::set_permissions(&source_bin, std::fs::Permissions::from_mode(0o555))
            .await
            .unwrap();

        let dest_root = dir.path().join("dest");
        tokio::fs::create_dir_all(&dest_root).await.unwrap();
        let live_top = dest_root.join("zzz-nixos-system-host");
        hardlink_tree(source_top, &live_top).await.unwrap();

        let live_bin = live_top.join("bin");
        assert_eq!(
            tokio::fs::metadata(&live_top).await.unwrap().mode() & 0o7777,
            0o555
        );
        assert_eq!(
            tokio::fs::metadata(&live_bin).await.unwrap().mode() & 0o7777,
            0o555
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn split_publish_swaps_currents_and_plants_marker_last() {
        let dir = tempdir().unwrap();
        let farm = dir.path().join("farm");
        let closure = split_closure(dir.path());
        let system = system_store_path(&closure).map(Path::to_path_buf);
        let gid = generation_id(&closure, system.as_deref());
        let marker = GenerationMarker {
            closure_hash: "sha256:split".to_owned(),
            d2b_version: "0.4.0".to_owned(),
            activated_at: "2026-06-09T09:00:00Z".to_owned(),
            vm: "corp-vm".to_owned(),
            generation_number: 11,
        };
        build_store_view(&farm, &gid, &closure, &marker).await.unwrap();

        // Not ready until both currents + the live marker are published.
        assert!(!split_fast_path_ready(&farm, &gid, "corp-vm", &closure).await);

        let db_dump = dir.path().join("db.dump");
        tokio::fs::write(&db_dump, b"db").await.unwrap();
        write_meta_db_dump(&farm, &gid, &db_dump).await.unwrap();
        swap_state_current(&farm, &gid).await.unwrap();
        swap_meta_current(&farm, &gid).await.unwrap();
        plant_live_marker(&farm, "corp-vm").await.unwrap();

        assert_eq!(
            read_state_current_id(&farm).await.as_deref(),
            Some(gid.as_str())
        );
        assert_eq!(
            read_meta_current_id(&farm).await.as_deref(),
            Some(gid.as_str())
        );
        let live_marker = live_dir(&farm).join(".d2b-marker-corp-vm");
        assert!(live_marker.exists());
        assert_eq!(tokio::fs::metadata(&live_marker).await.unwrap().len(), 0);
        assert_eq!(read_generation_token(&farm, &gid).await, Some(11));

        // Now the fast path reports ready for the same closure.
        assert!(split_fast_path_ready(&farm, &gid, "corp-vm", &closure).await);
        // ...but not for a different VM name.
        assert!(!split_fast_path_ready(&farm, &gid, "other-vm", &closure).await);
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn build_store_view_refuses_generation_id_collision() {
        let dir = tempdir().unwrap();
        let farm = dir.path().join("farm");
        let closure = split_closure(dir.path());
        let system = system_store_path(&closure).map(Path::to_path_buf);
        let gid = generation_id(&closure, system.as_deref());
        let marker_a = GenerationMarker {
            closure_hash: "sha256:closure-a".to_owned(),
            d2b_version: "0.4.0".to_owned(),
            activated_at: "2026-06-09T09:00:00Z".to_owned(),
            vm: "corp-vm".to_owned(),
            generation_number: 11,
        };
        build_store_view(&farm, &gid, &closure, &marker_a).await.unwrap();

        // Reuse the same on-disk id with a DIFFERENT closure identity:
        // simulates a SHA-256 collision. Must refuse rather than union.
        let marker_b = GenerationMarker {
            closure_hash: "sha256:closure-b".to_owned(),
            ..marker_a.clone()
        };
        let err = build_store_view(&farm, &gid, &closure, &marker_b)
            .await
            .unwrap_err();
        assert!(matches!(err, HardlinkFarmError::GenerationCollision { .. }));

        // Same id + same closure identity stays idempotent.
        build_store_view(&farm, &gid, &closure, &marker_a)
            .await
            .expect("idempotent rebuild");
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn replace_live_top_level_paths_exchanges_existing_tree() {
        let dir = tempdir().unwrap();
        let farm = dir.path().join("farm");
        let closure = split_closure(dir.path());
        let system = system_store_path(&closure).map(Path::to_path_buf);
        let gid = generation_id(&closure, system.as_deref());
        let marker = GenerationMarker {
            closure_hash: "sha256:split".to_owned(),
            d2b_version: "0.4.0".to_owned(),
            activated_at: "2026-06-09T09:00:00Z".to_owned(),
            vm: "corp-vm".to_owned(),
            generation_number: 11,
        };
        build_store_view(&farm, &gid, &closure, &marker).await.unwrap();
        let live_pkg = live_dir(&farm).join("zzz-nixos-system-host");
        tokio::fs::remove_dir_all(&live_pkg).await.unwrap();
        tokio::fs::create_dir_all(&live_pkg).await.unwrap();
        tokio::fs::write(live_pkg.join("bin"), "drifted").await.unwrap();

        let counts =
            replace_live_top_level_paths(&farm, "repair-test", &closure).await.unwrap();
        assert_eq!(counts.linked, 2);
        assert_eq!(
            tokio::fs::read_to_string(live_pkg.join("bin/switch-to-configuration"))
                .await
                .unwrap(),
            "#!/bin/sh\n"
        );
        let mut farm_entries = tokio::fs::read_dir(&farm).await.unwrap();
        let mut has_repair_stage = false;
        while let Some(entry) = farm_entries.next_entry().await.unwrap() {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with("live.repair.stage.")
            {
                has_repair_stage = true;
                break;
            }
        }
        assert!(!has_repair_stage, "repair stage dir should be cleaned");
    }

    // -- sync.lock owner record ----------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn locked_lock_file(dir: &Path) -> std::fs::File {
        let path = sync_lock_path(dir);
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap()
    }

    #[test]
    fn sync_lock_owner_record_round_trips_and_verifies_live() {
        let dir = tempdir().unwrap();
        let file = locked_lock_file(dir.path());
        let written = SyncLockOwnerRecord::record_current_process(&file).unwrap();
        assert_eq!(written.schema, SYNC_LOCK_OWNER_SCHEMA);
        assert!(written.pid > 1);
        assert!(written.process_start_time_ticks > 0);
        assert_eq!(written.boot_id, current_boot_id().unwrap());

        let read = SyncLockOwnerRecord::read_locked(&file).unwrap();
        assert_eq!(read, written);
        read.verify_live().expect("own record is live");
        // The owner name is the kernel-reported comm, not a caller string.
        let (comm, ticks) = process_identity(std::process::id() as i32).unwrap();
        assert_eq!(read.owner, comm);
        assert_eq!(read.process_start_time_ticks, ticks);
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn sync_lock_owner_record_rewrite_leaves_no_trailing_bytes() {
        let dir = tempdir().unwrap();
        let file = locked_lock_file(dir.path());
        let long = SyncLockOwnerRecord {
            schema: SYNC_LOCK_OWNER_SCHEMA.to_owned(),
            owner: "x".repeat(64),
            pid: std::process::id() as i32,
            process_start_time_ticks: 1,
            boot_id: current_boot_id().unwrap(),
        };
        long.write_locked(&file).unwrap();
        let short = SyncLockOwnerRecord::record_current_process(&file).unwrap();
        let raw = std::fs::read(sync_lock_path(dir.path())).unwrap();
        assert_eq!(raw, short.to_bytes(), "no stale bytes from the longer record");
        assert_eq!(SyncLockOwnerRecord::read_locked(&file).unwrap(), short);
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn sync_lock_owner_record_refuses_foreign_and_stale_owners() {
        let dir = tempdir().unwrap();
        let file = locked_lock_file(dir.path());
        let mut record = SyncLockOwnerRecord::record_current_process(&file).unwrap();

        // An absent record (the file exists but carries no bytes) and a
        // non-record payload both stay malformed: no adoption evidence.
        std::fs::write(sync_lock_path(dir.path()), b"").unwrap();
        assert_eq!(
            SyncLockOwnerRecord::read_locked(&file).unwrap_err(),
            SyncLockOwnerError::Malformed
        );
        std::fs::write(sync_lock_path(dir.path()), b"not a record").unwrap();
        assert_eq!(
            SyncLockOwnerRecord::read_locked(&file).unwrap_err(),
            SyncLockOwnerError::Malformed
        );

        // A record from another boot is stale.
        record.boot_id = "00000000-0000-0000-0000-000000000000".to_owned();
        assert_eq!(
            record.verify_live().unwrap_err(),
            SyncLockOwnerError::ForeignBoot
        );

        // A record naming a live process that is not the recorded owner
        // (the kernel's comm disagrees) is refused.
        record.boot_id = current_boot_id().unwrap();
        record.owner = "not-the-broker".to_owned();
        assert_eq!(
            record.verify_live().unwrap_err(),
            SyncLockOwnerError::OwnerMismatch
        );

        // A record naming a process that does not exist is refused.
        record.owner = std::process::id().to_string();
        record.pid = i32::MAX;
        assert_eq!(
            record.verify_live().unwrap_err(),
            SyncLockOwnerError::ProcessGone
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn sync_lock_owner_record_caps_oversize_payloads() {
        let dir = tempdir().unwrap();
        let file = locked_lock_file(dir.path());
        std::fs::write(
            sync_lock_path(dir.path()),
            vec![b'a'; SYNC_LOCK_OWNER_MAX_BYTES + 1],
        )
        .unwrap();
        assert_eq!(
            SyncLockOwnerRecord::read_locked(&file).unwrap_err(),
            SyncLockOwnerError::Malformed
        );

        // The shape check also refuses an unbounded owner name.
        let mut record = SyncLockOwnerRecord::for_current_process().unwrap();
        record.owner = "x".repeat(65);
        assert_eq!(
            record.validate_shape().unwrap_err(),
            SyncLockOwnerError::Malformed
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn sync_lock_owner_record_reads_the_kernel_stat_shape() {
        // Field 2 (comm) is parenthesized and may itself contain spaces and
        // parentheses, so the parse splits at the last `)`; the kernel's
        // `/proc/<pid>/comm` is an independent read of the same field.
        let pid = std::process::id() as i32;
        let (owner, start_ticks) = process_identity(pid).unwrap();
        assert!(start_ticks > 0);
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap();
        assert_eq!(owner, comm.trim());
        assert_eq!(process_identity(pid).unwrap(), (owner, start_ticks));
    }
}