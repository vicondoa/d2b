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
use std::sync::atomic::{AtomicU64, Ordering};

/// `O_NOFOLLOW` for Linux (the only guest target). Octal `0o400000`; identical
/// on x86_64 and aarch64.
#[cfg(target_os = "linux")]
pub(crate) const O_NOFOLLOW: i32 = 0o0_400_000;
#[cfg(not(target_os = "linux"))]
pub(crate) const O_NOFOLLOW: i32 = 0;

/// Every per-slot file holds privileged exec metadata/output; create it
/// `root:root 0600` explicitly rather than relying on the process umask.
pub(crate) const FILE_MODE_0600: u32 = 0o600;

/// Monotonic counter making each atomic-write temp name unique within a
/// process, so two concurrent writers to the same target (e.g. two concurrent
/// `ExecCancel`s racing the `cancel` sentinel) never collide on a shared
/// `<name>.tmp` file.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

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
    // Unique per write: `<pid>.<seq>` so concurrent writers never share a temp.
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    name.push(format!(".tmp.{}.{seq}", std::process::id()));
    target.with_file_name(name)
}

/// Atomically replace `target` with `bytes` (temp -> fsync -> rename -> dir
/// fsync). The temp file is created `O_NOFOLLOW|O_CREAT|O_EXCL` with an
/// explicit `0600` mode (never umask-derived) and a unique name; any failure
/// after creation removes the temp so a dirty slot is never left behind.
pub fn atomic_write(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = tmp_path(target);
    let write_result = (|| {
        let tmp_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(FILE_MODE_0600)
            .custom_flags(O_NOFOLLOW)
            .open(&tmp)?;
        tmp_file.write_all_at(bytes, 0)?;
        tmp_file.set_len(bytes.len() as u64)?;
        tmp_file.sync_all()?;
        std::fs::rename(&tmp, target)?;
        fsync_parent_dir(target)
    })();
    if write_result.is_err() {
        // Best-effort: the rename consumes `tmp` on success, so a leftover only
        // exists on the error path.
        let _ = std::fs::remove_file(&tmp);
    }
    write_result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

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
        // The temp file is gone after a successful rename (unique-named, so we
        // assert no `.tmp.*` siblings linger).
        let leftover = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains(".tmp."));
        assert!(!leftover, "no temp file lingers after a successful write");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn atomic_write_creates_files_mode_0600() {
        use std::os::unix::fs::MetadataExt;
        let dir = scratch();
        let target = dir.join("record");
        atomic_write(&target, b"payload").unwrap();
        let mode = std::fs::metadata(&target).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o600, "data files are created 0600, not umask-derived");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn concurrent_writes_to_same_target_never_collide_on_temp() {
        // Two threads writing the same target concurrently must both succeed:
        // the temp name is unique per write, so neither clobbers the other's
        // temp (the concurrent-ExecCancel `cancel.tmp` collision).
        let dir = scratch();
        let target = Arc::new(dir.join("cancel"));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let target = Arc::clone(&target);
            handles.push(std::thread::spawn(move || {
                for _ in 0..50 {
                    atomic_write(&target, b"1").expect("concurrent atomic_write");
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(read_file_nofollow(&target).unwrap(), b"1");
        let leftover = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains(".tmp."));
        assert!(!leftover, "no temp files linger after concurrent writes");
        std::fs::remove_dir_all(&dir).ok();
    }
}
