//! Shared, dependency-pure atomic file I/O primitives.
//!
//! Both the FileRing sidecar and the runner's `status`/`record`/`cancel`
//! writers need the same durable-replace contract: write a temp file in the
//! same directory, fsync it, rename it over the target, then fsync the parent
//! directory so the rename itself is durable. Symlink-safe opens use
//! `O_NOFOLLOW` on the final path component.

use std::fs::OpenOptions;
use std::io::{self, Read};
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// `O_NOFOLLOW` for Linux (the only guest target). Octal `0o400000`; identical
/// on x86_64 and aarch64.
#[cfg(target_os = "linux")]
pub(crate) const O_NOFOLLOW: i32 = 0o0_400_000;
#[cfg(not(target_os = "linux"))]
pub(crate) const O_NOFOLLOW: i32 = 0;

/// Open a file for reading, refusing to follow a final-component symlink.
pub fn open_read_nofollow(path: &Path) -> io::Result<std::fs::File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW)
        .open(path)
}

/// Read an entire small control file (status/record/sidecar/spec). The caller
/// is responsible for any size validation via the decoder's bounded fields.
pub fn read_file_nofollow(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = open_read_nofollow(path)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok(buf)
}

/// fsync the directory that contains `path` so a rename is durable.
pub fn fsync_parent_dir(path: &Path) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let handle = OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW)
        .open(dir)?;
    handle.sync_all()
}

fn tmp_path(target: &Path) -> PathBuf {
    let mut name = target
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    target.with_file_name(name)
}

/// Atomically replace `target` with `bytes` (temp -> fsync -> rename -> dir
/// fsync). The temp file is created `O_NOFOLLOW` and truncated.
pub fn atomic_write(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = tmp_path(target);
    {
        let tmp_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .custom_flags(O_NOFOLLOW)
            .open(&tmp)?;
        tmp_file.write_all_at(bytes, 0)?;
        tmp_file.set_len(bytes.len() as u64)?;
        tmp_file.sync_all()?;
    }
    std::fs::rename(&tmp, target)?;
    fsync_parent_dir(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let base = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let dir = base.join(format!(
            "atomicio-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn atomic_write_then_read_round_trips_and_replaces() {
        let dir = scratch();
        let target = dir.join("status");
        atomic_write(&target, b"first").unwrap();
        assert_eq!(read_file_nofollow(&target).unwrap(), b"first");
        // Replace with a shorter payload; no stale tail remains.
        atomic_write(&target, b"hi").unwrap();
        assert_eq!(read_file_nofollow(&target).unwrap(), b"hi");
        // The temp file is gone after a successful rename.
        assert!(!dir.join("status.tmp").exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
