//! Single-inode store-view posture helpers.
//!
//! StoreSync is allowed to posture broker-owned metadata inodes it creates
//! (`state/`, `gcroots/`, `sync.lock`, and integrity files) plus the
//! broker-created levels above the per-VM state dir that the daemon must
//! walk to reach the farm. The per-VM state dir itself is the daemon's
//! ownership-matrix root and is never touched. It must never recurse into
//! `live/`, because those package trees are hardlinked to `/nix/store`.
//!
//! The per-level posture this module applies is not restated here: it is read
//! from the declared contract in `state-posture-contract.json` (the
//! `guest-store-view` tree), which the Nix provisioning and the live
//! `tests/host-integration/state-posture-contract.nix` validation read too.
//! That file also names the anchor-open rule this module's ancestor walk
//! obeys: an anchor component is opened `O_PATH` (the consumer needs search on
//! the parent, never read on the component), and only a leaf the consumer owns
//! is opened `O_RDONLY`. Do not reintroduce read-on-traversal-only opens.

use std::fs::OpenOptions;
use std::os::fd::AsFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use d2b_host::hardlink_farm;
use nix::unistd::{Gid, Uid, chown};
#[cfg(not(test))]
use nix::unistd::{Group, User};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PostureError {
    pub path: String,
    pub detail: String,
}

impl std::fmt::Display for PostureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.detail)
    }
}

impl std::error::Error for PostureError {}

#[derive(Debug, Clone, Copy)]
struct Principals {
    owner_uid: Uid,
    /// The daemon principal's primary group: the group every ancestor the
    /// daemon must traverse is postured for.
    daemon_gid: Gid,
    host_gid: Gid,
    runner_gid: Gid,
}

#[cfg(test)]
fn resolve_principals() -> Result<Principals, PostureError> {
    Ok(Principals {
        owner_uid: Uid::current(),
        daemon_gid: Gid::current(),
        host_gid: Gid::current(),
        runner_gid: Gid::current(),
    })
}

#[cfg(not(test))]
fn resolve_principals() -> Result<Principals, PostureError> {
    let daemon = User::from_name("d2bd")
        .map_err(|err| PostureError {
            path: "d2bd".to_owned(),
            detail: format!("lookup user: {err}"),
        })?
        .ok_or_else(|| PostureError {
            path: "d2bd".to_owned(),
            detail: "user not found".to_owned(),
        })?;
    let owner_uid = daemon.uid;
    let daemon_gid = daemon.gid;
    let host_gid = Group::from_name("d2b")
        .map_err(|err| PostureError {
            path: "d2b".to_owned(),
            detail: format!("lookup group: {err}"),
        })?
        .ok_or_else(|| PostureError {
            path: "d2b".to_owned(),
            detail: "group not found".to_owned(),
        })?
        .gid;
    let runner_gid = Group::from_name("users")
        .map_err(|err| PostureError {
            path: "users".to_owned(),
            detail: format!("lookup group: {err}"),
        })?
        .ok_or_else(|| PostureError {
            path: "users".to_owned(),
            detail: "group not found".to_owned(),
        })?
        .gid;
    Ok(Principals {
        owner_uid,
        daemon_gid,
        host_gid,
        runner_gid,
    })
}

#[derive(Debug, Clone, Copy)]
enum PathKind {
    Dir,
    File,
}

/// The declared host posture contract, embedded so the running binary cannot
/// drift from the checked-in declaration. Only the `guest-store-view` tree is
/// applied here; the other trees are provisioned by `nixos-modules/**` and
/// validated live by `tests/host-integration/state-posture-contract.nix`.
const STATE_POSTURE_CONTRACT: &str = include_str!("state-posture-contract.json");

const STORE_VIEW_TREE_ID: &str = "guest-store-view";

#[derive(Debug, serde::Deserialize)]
struct ContractFile {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    trees: Vec<ContractTree>,
}

#[derive(Debug, serde::Deserialize)]
struct ContractTree {
    id: String,
    levels: Vec<ContractLevel>,
}

#[derive(Debug, serde::Deserialize)]
struct ContractLevel {
    path: String,
    kind: String,
    owner: String,
    group: String,
    mode: String,
    posture: String,
}

/// One `guest-store-view` contract row with its principals and mode resolved
/// against the live host.
#[derive(Debug, Clone)]
struct ResolvedLevel {
    /// Declared path relative to the tree root, `<vm>` substituted.
    relative: String,
    kind: PathKind,
    mode: u32,
    owner: Uid,
    group: Gid,
}

impl Principals {
    /// Resolve one contract group name. The contract names host principals
    /// symbolically; the mapping is closed on purpose - a new group name in
    /// the declaration must be taught here before posture can apply it.
    fn group_named(&self, name: &str) -> Option<Gid> {
        match name {
            "d2bd" => Some(self.daemon_gid),
            "d2b" => Some(self.host_gid),
            "users" => Some(self.runner_gid),
            _ => None,
        }
    }
}

fn contract_error(detail: String) -> PostureError {
    PostureError {
        path: "state-posture-contract.json".to_owned(),
        detail,
    }
}

/// Resolve the declared `guest-store-view` rows against the live principals.
///
/// Fail closed on anything posture cannot apply verbatim: an unknown schema,
/// a missing tree, a non-`exact` posture, an unknown kind/owner/group, a
/// non-octal mode, or a path that is not a plain relative descendant.
fn contract_store_view_levels(
    principals: &Principals,
    vm: &str,
) -> Result<Vec<ResolvedLevel>, PostureError> {
    let contract: ContractFile = serde_json::from_str(STATE_POSTURE_CONTRACT)
        .map_err(|err| contract_error(format!("parse: {err}")))?;
    if contract.schema_version != 1 {
        return Err(contract_error(format!(
            "unsupported schemaVersion {}",
            contract.schema_version
        )));
    }
    let tree = contract
        .trees
        .iter()
        .find(|tree| tree.id == STORE_VIEW_TREE_ID)
        .ok_or_else(|| contract_error(format!("tree `{STORE_VIEW_TREE_ID}` missing")))?;
    tree.levels
        .iter()
        .map(|level| {
            if level.posture != "exact" {
                return Err(contract_error(format!(
                    "{}: posture `{}` is not `exact`",
                    level.path, level.posture
                )));
            }
            if level.owner != "d2bd" {
                return Err(contract_error(format!(
                    "{}: unsupported owner `{}`",
                    level.path, level.owner
                )));
            }
            let kind = match level.kind.as_str() {
                "dir" => PathKind::Dir,
                "file" => PathKind::File,
                other => {
                    return Err(contract_error(format!(
                        "{}: unsupported kind `{other}`",
                        level.path
                    )));
                }
            };
            let group = principals.group_named(&level.group).ok_or_else(|| {
                contract_error(format!(
                    "{}: unsupported group `{}`",
                    level.path, level.group
                ))
            })?;
            let mode = u32::from_str_radix(&level.mode, 8).map_err(|err| {
                contract_error(format!("{}: mode `{}`: {err}", level.path, level.mode))
            })?;
            if level.path != "." {
                let relative = Path::new(&level.path);
                if relative.is_absolute()
                    || relative
                        .components()
                        .any(|component| !matches!(component, std::path::Component::Normal(_)))
                {
                    return Err(contract_error(format!(
                        "{}: not a plain relative descendant",
                        level.path
                    )));
                }
            }
            Ok(ResolvedLevel {
                relative: level.path.replace("<vm>", vm),
                kind,
                mode,
                owner: principals.owner_uid,
                group,
            })
        })
        .collect()
}

/// Resolve one declared `guest-store-view` row by its contract path (`"."`
/// for the tree root), with `<vm>` substituted.
fn contract_store_view_level(
    principals: &Principals,
    declared_path: &str,
    vm: &str,
) -> Result<ResolvedLevel, PostureError> {
    let wanted = declared_path.replace("<vm>", vm);
    contract_store_view_levels(principals, vm)?
        .into_iter()
        .find(|level| level.relative == wanted)
        .ok_or_else(|| contract_error(format!("row `{declared_path}` missing")))
}

fn resolved_path(store_root: &Path, level: &ResolvedLevel) -> PathBuf {
    if level.relative == "." {
        store_root.to_path_buf()
    } else {
        store_root.join(&level.relative)
    }
}

pub(crate) fn posture_store_view_matrix_paths(
    store_root: &Path,
    vm: &str,
) -> Result<(), PostureError> {
    posture_store_view_matrix_paths_with(store_root, vm, resolve_principals()?)
}

/// [`posture_store_view_matrix_paths`] with the host principals supplied by
/// the caller.
///
/// Split out for the ownership regression tests: `cfg(test)`'s
/// [`resolve_principals`] collapses every principal onto the test process,
/// which would turn the ancestor walk's group grant into a silent no-op no
/// gid assertion could observe.
///
/// The tree's own levels come from the declared contract, so a level cannot
/// exist in the code without being declared (or vice versa).
fn posture_store_view_matrix_paths_with(
    store_root: &Path,
    vm: &str,
    principals: Principals,
) -> Result<(), PostureError> {
    posture_daemon_traverse_ancestors(store_root, principals.daemon_gid)?;
    for level in contract_store_view_levels(&principals, vm)? {
        let path = resolved_path(store_root, &level);
        posture_existing(&path, level.kind, level.mode, level.owner, level.group)?;
    }
    Ok(())
}

/// Posture every ancestor strictly above the per-VM state dir (the farm
/// root's parent, i.e. the daemon's ownership-matrix root) up to the first
/// world-traversable directory, so the daemon's group can search it.
///
/// The daemon reaches `<state-root>/zones/<zone>/guests/<guest>/store-view`
/// through an anchored walk that needs search (`--x`) permission on each
/// ancestor. Those ancestors are created by the broker, so their traversal
/// grant is owned here with the rest of the store-view matrix instead of
/// depending on the broker's umask, its group, or spawn-time ACLs that are
/// only established later. The posture adds group search only: existing mode
/// bits are preserved and group write is never granted. A directory the
/// daemon's group can already search (group search bit set and owned by that
/// group, or world-searchable) is left untouched, and the walk stops at the
/// first world-searchable ancestor - the directories above it already grant
/// search to every principal.
///
/// The per-VM state dir is never postured: it is the daemon's ownership-matrix
/// root (`d2bd:users 03770`, see `d2bd-runtime/src/ownership_preflight.rs`),
/// the daemon owns it (so search is already granted), and the fail-closed
/// matrix preflight refuses the next VM start on any ownership change to it.
///
/// This is the broker side of the contract's `anchor-open` rule
/// (`state-posture-contract.json`): the consumer that walks this chain opens
/// every anchor component `O_PATH` (`packages/d2bd/src/resource_plane_v3.rs`,
/// `open_anchored_directory`), so search is the only right the chain must
/// grant - never read and never write. A future edit must not reintroduce a
/// read-on-traversal-only open here or at that walk.
fn posture_daemon_traverse_ancestors(
    store_root: &Path,
    daemon_gid: Gid,
) -> Result<(), PostureError> {
    use std::os::unix::fs::MetadataExt as _;
    // The farm root is `<state-dir>/store-view`, so its parent is the per-VM
    // state dir - the daemon's ownership-matrix root. The daemon owns that
    // directory (search is already granted), and the fail-closed matrix
    // preflight refuses the next VM start the moment its group or mode moves
    // (the trap `ops/state_dir.rs` documents for the same directory). The
    // walk therefore starts at the matrix root's parent: it never stats,
    // chowns or chmods the matrix root or anything below it.
    let matrix_root = store_root.parent().unwrap_or(store_root);
    for directory in matrix_root.ancestors().skip(1) {
        let meta = match std::fs::symlink_metadata(directory) {
            Ok(meta) => meta,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(io_error(directory, format!("stat: {err}"))),
        };
        if meta.file_type().is_symlink() {
            return Err(io_error(directory, "ancestor is a symlink".to_owned()));
        }
        if !meta.is_dir() {
            return Err(io_error(directory, "expected directory".to_owned()));
        }
        let mode = meta.mode() & 0o7777;
        if mode & 0o001 != 0 {
            // World-searchable: this directory and every ancestor above it
            // already grant search to every principal, so the posture stops
            // here and leaves the unowned levels alone.
            break;
        }
        if meta.gid() == daemon_gid.as_raw() && mode & 0o010 != 0 {
            continue;
        }
        chown(directory, None, Some(daemon_gid))
            .map_err(|err| io_error(directory, format!("chown: {err}")))?;
        if mode & 0o010 == 0 {
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(mode | 0o010))
                .map_err(|err| io_error(directory, format!("chmod group-traverse: {err}")))?;
        }
    }
    Ok(())
}

pub(crate) fn plant_live_marker_with_matrix_posture(
    store_root: &Path,
    vm: &str,
) -> Result<(), PostureError> {
    let principals = resolve_principals()?;
    let level = contract_store_view_level(&principals, "live/.d2b-marker-<vm>", vm)?;
    let live = hardlink_farm::live_dir(store_root);
    let marker = live.join(format!(".d2b-marker-{vm}"));
    let tmp = live.join(format!(".d2b-marker-{vm}.tmp"));
    let _ = std::fs::remove_file(&tmp);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(level.mode)
        .open(&tmp)
        .map_err(|err| io_error(&tmp, format!("create marker tmp: {err}")))?;
    crate::sys::path_safe::fchown(
        file.as_fd(),
        Some(level.owner.as_raw()),
        Some(level.group.as_raw()),
    )
    .map_err(|err| io_error(&tmp, format!("fchown marker tmp: {err}")))?;
    crate::sys::path_safe::fchmod(file.as_fd(), level.mode)
        .map_err(|err| io_error(&tmp, format!("fchmod marker tmp: {err}")))?;
    file.sync_all()
        .map_err(|err| io_error(&tmp, format!("fsync marker tmp: {err}")))?;
    drop(file);
    std::fs::rename(&tmp, &marker)
        .map_err(|err| io_error(&marker, format!("rename marker tmp: {err}")))?;
    if let Ok(dir) = std::fs::File::open(&live) {
        let _ = dir.sync_all();
    }
    posture_existing(&marker, level.kind, level.mode, level.owner, level.group)
}

/// Posture the broker's host-only integrity record
/// (`state/integrity-unknown.json`) from its declared contract row.
pub(crate) fn posture_host_only_file(path: &Path) -> Result<(), PostureError> {
    let principals = resolve_principals()?;
    let level = contract_store_view_level(&principals, "state/integrity-unknown.json", "")?;
    posture_existing(path, level.kind, level.mode, level.owner, level.group)
}

fn posture_existing(
    path: &Path,
    kind: PathKind,
    mode: u32,
    uid: Uid,
    gid: Gid,
) -> Result<(), PostureError> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(io_error(path, format!("stat: {err}"))),
    };
    if meta.file_type().is_symlink() {
        return Err(io_error(path, "leaf is a symlink".to_owned()));
    }
    match kind {
        PathKind::Dir if !meta.is_dir() => {
            return Err(io_error(path, "expected directory".to_owned()));
        }
        PathKind::File if !meta.is_file() => {
            return Err(io_error(path, "expected regular file".to_owned()));
        }
        _ => {}
    }
    chown(path, Some(uid), Some(gid)).map_err(|err| io_error(path, format!("chown: {err}")))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|err| io_error(path, format!("chmod {mode:o}: {err}")))?;
    Ok(())
}

fn io_error(path: &Path, detail: String) -> PostureError {
    PostureError {
        path: path.display().to_string(),
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt as _;
    use std::path::PathBuf;

    /// Build the broker-shaped farm path
    /// `<root>/zones/work/guests/acceptance-guest/store-view` and return the
    /// farm root plus the broker-created levels above the per-VM state dir,
    /// innermost first. The state dir itself - the daemon's ownership-matrix
    /// root - is `farm.parent()`.
    fn farm_chain(root: &Path) -> (PathBuf, Vec<PathBuf>) {
        let farm = root
            .join("zones")
            .join("work")
            .join("guests")
            .join("acceptance-guest")
            .join("store-view");
        std::fs::create_dir_all(&farm).expect("create farm chain");
        let ancestors = vec![
            root.join("zones").join("work").join("guests"),
            root.join("zones").join("work"),
            root.join("zones"),
        ];
        (farm, ancestors)
    }

    /// The per-VM state dir: the daemon's ownership-matrix root.
    fn matrix_root(farm: &Path) -> PathBuf {
        farm.parent()
            .expect("the farm root's parent is the per-VM state dir")
            .to_path_buf()
    }

    /// The principals `cfg(test)` resolves: every principal is the test
    /// process itself.
    fn test_principals() -> Principals {
        resolve_principals().expect("test principals resolve")
    }

    /// A daemon primary group that is not the test process's own group, so a
    /// regression that re-groups the matrix root cannot hide behind
    /// `cfg(test)`'s current-process principals.
    ///
    /// A supplementary group of the test process is preferred: the chown then
    /// applies exactly as it does for the privileged production broker, so the
    /// regression surfaces as a flipped gid. Without one any distinct id still
    /// surfaces it - the chown fails closed with `EPERM`.
    fn foreign_daemon_gid() -> Gid {
        let current = Gid::current();
        nix::unistd::getgroups()
            .unwrap_or_default()
            .into_iter()
            .find(|gid| *gid != current)
            .unwrap_or_else(|| Gid::from_raw(current.as_raw().wrapping_add(1)))
    }

    fn set_mode(path: &Path, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
    }

    fn mode_of(path: &Path) -> u32 {
        std::fs::symlink_metadata(path).expect("stat").mode() & 0o7777
    }

    fn gid_of(path: &Path) -> u32 {
        std::fs::symlink_metadata(path).expect("stat").gid()
    }

    #[test]
    fn matrix_posture_makes_the_ancestor_chain_group_traversable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (farm, ancestors) = farm_chain(dir.path());
        let matrix = matrix_root(&farm);
        // The broker creates the chain under UMask=0027; the worst shape it
        // can land in is owner-only, which the daemon's group cannot search.
        set_mode(&matrix, 0o700);
        for ancestor in &ancestors {
            set_mode(ancestor, 0o700);
        }

        posture_store_view_matrix_paths(&farm, "acceptance-guest").expect("posture");

        let daemon_gid = Gid::current().as_raw();
        for ancestor in &ancestors {
            assert_eq!(
                gid_of(ancestor),
                daemon_gid,
                "ancestor {} must belong to the daemon's group",
                ancestor.display()
            );
            let mode = mode_of(ancestor);
            assert_ne!(
                mode & 0o010,
                0,
                "ancestor {} must be group-searchable",
                ancestor.display()
            );
            assert_eq!(
                mode & 0o040,
                0,
                "ancestor {} must stay traversal-only (no group read)",
                ancestor.display()
            );
            assert_eq!(
                mode & 0o022,
                0,
                "ancestor {} must never be group/other writable",
                ancestor.display()
            );
        }
        // The farm root's parent is the daemon's ownership-matrix root: the
        // daemon already owns it, and the fail-closed matrix preflight refuses
        // the next VM start on any ownership change to it, so the walk must
        // leave it exactly as it found it.
        assert_eq!(
            mode_of(&matrix),
            0o700,
            "the matrix root must keep its mode"
        );
        assert_eq!(
            gid_of(&matrix),
            daemon_gid,
            "the matrix root must keep its group"
        );
        // The farm root itself keeps the matrix posture: daemon-owned and
        // readable by the runner group, never writable by it.
        assert_eq!(mode_of(&farm), 0o755);
        assert_eq!(gid_of(&farm), daemon_gid);
    }

    #[test]
    fn matrix_posture_leaves_an_already_searchable_chain_untouched() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (farm, ancestors) = farm_chain(dir.path());
        let (guests_dir, zone_dir, zones_dir) = (
            ancestors[0].clone(),
            ancestors[1].clone(),
            ancestors[2].clone(),
        );
        let matrix = matrix_root(&farm);
        let matrix_gid = gid_of(&matrix);
        // `guests` is already searchable by the daemon's group; the zone is
        // world-searchable, so the posture must stop there and leave every
        // directory above untouched.
        set_mode(&matrix, 0o3770);
        set_mode(&guests_dir, 0o750);
        set_mode(&zone_dir, 0o755);
        set_mode(&zones_dir, 0o700);

        posture_store_view_matrix_paths(&farm, "acceptance-guest").expect("posture");

        assert_eq!(mode_of(&guests_dir), 0o750, "searchable ancestor untouched");
        assert_eq!(mode_of(&zone_dir), 0o755, "world-searchable ancestor untouched");
        assert_eq!(mode_of(&zones_dir), 0o700, "walk stops above world-search");
        assert_eq!(gid_of(&matrix), matrix_gid, "the matrix root keeps its group");
        assert_eq!(mode_of(&matrix), 0o3770, "the matrix root keeps `03770`");
    }

    /// Regression: the walk used to re-group the farm root's parent - the
    /// daemon's ownership-matrix root, declared `d2bd:users 03770` - to the
    /// daemon principal's primary group. StoreSync runs immediately before the
    /// fail-closed ownership preflight on every VM start, so the sync itself
    /// tripped the drift check and `OwnershipMatrixDrift` refused the start.
    #[test]
    fn matrix_posture_leaves_the_matrix_root_ownership_untouched() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (farm, ancestors) = farm_chain(dir.path());
        let matrix = matrix_root(&farm);
        // The declared row (`ownership_preflight.rs` CANONICAL_MATRIX ".").
        set_mode(&matrix, 0o3770);
        // World-searchable above the matrix root: the walk stops at the first
        // such level, so the matrix root is the only directory it can mutate.
        for ancestor in &ancestors {
            set_mode(ancestor, 0o755);
        }
        let matrix_gid = gid_of(&matrix);

        posture_store_view_matrix_paths_with(
            &farm,
            "acceptance-guest",
            Principals {
                daemon_gid: foreign_daemon_gid(),
                ..test_principals()
            },
        )
        .expect("posture");

        assert_eq!(
            gid_of(&matrix),
            matrix_gid,
            "the matrix root must keep the group its matrix row declares"
        );
        assert_eq!(
            mode_of(&matrix),
            0o3770,
            "the matrix root must keep `03770`"
        );
    }

    /// The posture applied to every materialized contract row equals the row
    /// `state-posture-contract.json` declares. If a row's mode is edited in the
    /// declaration without the live posture moving (or vice versa), this fails.
    #[test]
    fn every_declared_store_view_row_is_the_posture_applied() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (farm, _ancestors) = farm_chain(dir.path());
        let principals = test_principals();
        let rows = contract_store_view_levels(&principals, "acceptance-guest")
            .expect("contract rows resolve");

        // Materialize every declared row so posture has something to stamp;
        // posture deliberately no-ops on absent levels.
        for level in &rows {
            let path = resolved_path(&farm, level);
            match level.kind {
                PathKind::Dir => std::fs::create_dir_all(&path),
                PathKind::File => {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)
                            .expect("declared file parent materializes");
                    }
                    std::fs::write(&path, b"")
                }
            }
            .unwrap_or_else(|err| {
                panic!("materialize declared row {}: {err}", path.display())
            });
        }

        posture_store_view_matrix_paths(&farm, "acceptance-guest").expect("posture");

        for level in &rows {
            let path = resolved_path(&farm, level);
            assert_eq!(
                mode_of(&path),
                level.mode,
                "{} must carry its declared mode",
                path.display()
            );
        }
    }
}
