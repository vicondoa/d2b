//! Single-inode store-view posture helpers.
//!
//! StoreSync is allowed to posture broker-owned metadata inodes it creates
//! (`state/`, `gcroots/`, `sync.lock`, and integrity files) plus the
//! ancestor chain the daemon must walk to reach the farm (below
//! `<state-root>/zones`). It must never recurse into `live/`, because those
//! package trees are hardlinked to `/nix/store`.

use std::fs::OpenOptions;
use std::os::fd::AsFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

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

pub(crate) fn posture_store_view_matrix_paths(
    store_root: &Path,
    vm: &str,
) -> Result<(), PostureError> {
    let principals = resolve_principals()?;
    posture_daemon_traverse_ancestors(store_root, principals.daemon_gid)?;
    posture_existing(
        store_root,
        PathKind::Dir,
        0o755,
        principals.owner_uid,
        principals.runner_gid,
    )?;
    posture_existing(
        &hardlink_farm::live_dir(store_root),
        PathKind::Dir,
        0o755,
        principals.owner_uid,
        principals.runner_gid,
    )?;
    posture_existing(
        &hardlink_farm::meta_dir(store_root),
        PathKind::Dir,
        0o755,
        principals.owner_uid,
        principals.runner_gid,
    )?;
    posture_existing(
        &hardlink_farm::meta_dir(store_root).join("generations"),
        PathKind::Dir,
        0o755,
        principals.owner_uid,
        principals.runner_gid,
    )?;
    posture_existing(
        &hardlink_farm::live_dir(store_root).join(format!(".d2b-marker-{vm}")),
        PathKind::File,
        0o644,
        principals.owner_uid,
        principals.runner_gid,
    )?;
    posture_existing(
        &hardlink_farm::state_dir(store_root),
        PathKind::Dir,
        0o750,
        principals.owner_uid,
        principals.host_gid,
    )?;
    posture_existing(
        &hardlink_farm::state_dir(store_root).join("generations"),
        PathKind::Dir,
        0o750,
        principals.owner_uid,
        principals.host_gid,
    )?;
    posture_existing(
        &hardlink_farm::gcroots_dir(store_root),
        PathKind::Dir,
        0o750,
        principals.owner_uid,
        principals.host_gid,
    )?;
    posture_existing(
        &hardlink_farm::sync_lock_path(store_root),
        PathKind::File,
        0o600,
        principals.owner_uid,
        principals.host_gid,
    )?;
    posture_existing(
        &hardlink_farm::state_dir(store_root).join("integrity-unknown.json"),
        PathKind::File,
        0o640,
        principals.owner_uid,
        principals.host_gid,
    )?;
    Ok(())
}

/// Posture every ancestor between the farm root and the first
/// world-traversable directory so the daemon's group can search it.
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
fn posture_daemon_traverse_ancestors(
    store_root: &Path,
    daemon_gid: Gid,
) -> Result<(), PostureError> {
    use std::os::unix::fs::MetadataExt as _;
    for directory in store_root.ancestors().skip(1) {
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
    let live = hardlink_farm::live_dir(store_root);
    let marker = live.join(format!(".d2b-marker-{vm}"));
    let tmp = live.join(format!(".d2b-marker-{vm}.tmp"));
    let _ = std::fs::remove_file(&tmp);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&tmp)
        .map_err(|err| io_error(&tmp, format!("create marker tmp: {err}")))?;
    crate::sys::path_safe::fchown(
        file.as_fd(),
        Some(principals.owner_uid.as_raw()),
        Some(principals.runner_gid.as_raw()),
    )
    .map_err(|err| io_error(&tmp, format!("fchown marker tmp: {err}")))?;
    crate::sys::path_safe::fchmod(file.as_fd(), 0o644)
        .map_err(|err| io_error(&tmp, format!("fchmod marker tmp: {err}")))?;
    file.sync_all()
        .map_err(|err| io_error(&tmp, format!("fsync marker tmp: {err}")))?;
    drop(file);
    std::fs::rename(&tmp, &marker)
        .map_err(|err| io_error(&marker, format!("rename marker tmp: {err}")))?;
    if let Ok(dir) = std::fs::File::open(&live) {
        let _ = dir.sync_all();
    }
    posture_existing(
        &marker,
        PathKind::File,
        0o644,
        principals.owner_uid,
        principals.runner_gid,
    )
}

pub(crate) fn posture_host_only_file(path: &Path) -> Result<(), PostureError> {
    let principals = resolve_principals()?;
    posture_existing(
        path,
        PathKind::File,
        0o640,
        principals.owner_uid,
        principals.host_gid,
    )
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
    /// farm root plus the ancestors below `zones`, innermost first.
    fn farm_chain(root: &Path) -> (PathBuf, Vec<PathBuf>) {
        let farm = root
            .join("zones")
            .join("work")
            .join("guests")
            .join("acceptance-guest")
            .join("store-view");
        std::fs::create_dir_all(&farm).expect("create farm chain");
        let ancestors = vec![
            root.join("zones").join("work").join("guests").join("acceptance-guest"),
            root.join("zones").join("work").join("guests"),
            root.join("zones").join("work"),
            root.join("zones"),
        ];
        (farm, ancestors)
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
        // The broker creates the chain under UMask=0027; the worst shape it
        // can land in is owner-only, which the daemon's group cannot search.
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
        // The farm root itself keeps the matrix posture: daemon-owned and
        // readable by the runner group, never writable by it.
        assert_eq!(mode_of(&farm), 0o755);
        assert_eq!(gid_of(&farm), daemon_gid);
    }

    #[test]
    fn matrix_posture_leaves_an_already_searchable_chain_untouched() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (farm, ancestors) = farm_chain(dir.path());
        let (guest_dir, guests_dir, zone_dir, zones_dir) = (
            ancestors[0].clone(),
            ancestors[1].clone(),
            ancestors[2].clone(),
            ancestors[3].clone(),
        );
        // `guests` is already searchable by the daemon's group; `zone` is
        // world-searchable, so the posture must stop there and leave every
        // directory above untouched.
        set_mode(&guest_dir, 0o700);
        set_mode(&guests_dir, 0o750);
        set_mode(&zone_dir, 0o755);
        set_mode(&zones_dir, 0o700);

        posture_store_view_matrix_paths(&farm, "acceptance-guest").expect("posture");

        assert_eq!(mode_of(&guest_dir), 0o710, "the denied ancestor is postured");
        assert_eq!(mode_of(&guests_dir), 0o750, "searchable ancestor untouched");
        assert_eq!(mode_of(&zone_dir), 0o755, "world-searchable ancestor untouched");
        assert_eq!(mode_of(&zones_dir), 0o700, "walk stops above world-search");
    }
}
