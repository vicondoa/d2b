//! Single-inode store-view posture helpers.
//!
//! StoreSync is allowed to posture broker-owned metadata inodes it creates
//! (`state/`, `gcroots/`, `sync.lock`, and integrity files). It must never
//! recurse into `live/`, because those package trees are hardlinked to
//! `/nix/store`.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use nix::unistd::{chown, Gid, Uid};
#[cfg(not(test))]
use nix::unistd::{Group, User};
use nixling_host::hardlink_farm;

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
    host_gid: Gid,
    runner_gid: Gid,
}

#[cfg(test)]
fn resolve_principals() -> Result<Principals, PostureError> {
    Ok(Principals {
        owner_uid: Uid::current(),
        host_gid: Gid::current(),
        runner_gid: Gid::current(),
    })
}

#[cfg(not(test))]
fn resolve_principals() -> Result<Principals, PostureError> {
    let owner_uid = User::from_name("nixlingd")
        .map_err(|err| PostureError {
            path: "nixlingd".to_owned(),
            detail: format!("lookup user: {err}"),
        })?
        .ok_or_else(|| PostureError {
            path: "nixlingd".to_owned(),
            detail: "user not found".to_owned(),
        })?
        .uid;
    let host_gid = Group::from_name("nixling")
        .map_err(|err| PostureError {
            path: "nixling".to_owned(),
            detail: format!("lookup group: {err}"),
        })?
        .ok_or_else(|| PostureError {
            path: "nixling".to_owned(),
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
        &hardlink_farm::live_dir(store_root).join(format!(".nixling-marker-{vm}")),
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
