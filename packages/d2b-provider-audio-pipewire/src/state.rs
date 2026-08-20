//! Provider-owned durable audio state and OFD-locked migration I/O.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt as _;
use std::os::unix::io::AsRawFd as _;
use std::path::Path;

use nix::fcntl::{FcntlArg, fcntl};

use crate::{AudioPolicyError, AudioPolicyState, parse_audio_state};

// ── Lock path ────────────────────────────────────────────────────────────────

/// Path of the per-VM OFD lock file.
pub fn audio_lock_path(locks_dir: &Path, vm: &str) -> std::path::PathBuf {
    locks_dir.join(format!("audio-{vm}.lock"))
}

/// Path of the per-VM audio-state file.
pub fn audio_state_path(state_dir: &Path) -> std::path::PathBuf {
    state_dir.join("state/audio-state.json")
}

// ── OFD lock helpers ─────────────────────────────────────────────────────────

/// Acquire a Linux OFD lock on `fd`.
///
/// `exclusive = true`  → F_OFD_SETLKW write-lock (blocking).
/// `exclusive = false` → F_OFD_SETLKW read-lock  (blocking).
///
/// The file descriptor must have been opened with `O_CLOEXEC` so exec'd
/// children do not inherit the lock.
fn ofd_lock(fd: std::os::unix::io::RawFd, exclusive: bool) -> io::Result<()> {
    let ltype = if exclusive {
        libc::F_WRLCK as libc::c_short
    } else {
        libc::F_RDLCK as libc::c_short
    };
    let fl = libc::flock {
        l_type: ltype,
        l_whence: libc::SEEK_SET as libc::c_short,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    fcntl(fd, FcntlArg::F_OFD_SETLKW(&fl))
        .map(|_| ())
        .map_err(|e| io::Error::from_raw_os_error(e as i32))
}

/// Unlock an OFD lock held on `fd`.
///
/// Uses `F_OFD_SETLK` (non-blocking) rather than `F_OFD_SETLKW`: unlocking
/// never needs to wait and using the blocking variant is incorrect for the
/// release path.
fn ofd_unlock(fd: std::os::unix::io::RawFd) -> io::Result<()> {
    let fl = libc::flock {
        l_type: libc::F_UNLCK as libc::c_short,
        l_whence: libc::SEEK_SET as libc::c_short,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    fcntl(fd, FcntlArg::F_OFD_SETLK(&fl))
        .map(|_| ())
        .map_err(|e| io::Error::from_raw_os_error(e as i32))
}

/// RAII guard that releases an OFD lock when dropped.
struct OfdLockGuard {
    fd: std::os::unix::io::RawFd,
}

impl Drop for OfdLockGuard {
    fn drop(&mut self) {
        let _ = ofd_unlock(self.fd);
    }
}

pub struct AudioStateLock {
    _guard: OfdLockGuard,
    _file: File,
}

pub fn acquire_audio_state_lock(
    lock_path: &Path,
    exclusive: bool,
) -> Result<AudioStateLock, AudioStateIoError> {
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(lock_path)
        .map_err(AudioStateIoError::LockOpen)?;
    let fd = lock_file.as_raw_fd();
    ofd_lock(fd, exclusive).map_err(AudioStateIoError::LockAcquire)?;
    Ok(AudioStateLock {
        _guard: OfdLockGuard { fd },
        _file: lock_file,
    })
}

// ── Audio state I/O ──────────────────────────────────────────────────────────

/// Error from audio state file I/O.
#[derive(Debug)]
pub enum AudioStateIoError {
    LockOpen(io::Error),
    LockAcquire(io::Error),
    StateRead(io::Error),
    StateParse(AudioPolicyError),
    TempFile(io::Error),
    TempWrite(io::Error),
    TempSync(io::Error),
    AtomicRename(io::Error),
}

impl std::fmt::Display for AudioStateIoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LockOpen(e) => write!(f, "open audio lock file: {e}"),
            Self::LockAcquire(e) => write!(f, "acquire audio OFD lock: {e}"),
            Self::StateRead(e) => write!(f, "read audio state file: {e}"),
            Self::StateParse(e) => write!(f, "parse audio state: {e}"),
            Self::TempFile(e) => write!(f, "create audio state temp file: {e}"),
            Self::TempWrite(e) => write!(f, "write audio state temp file: {e}"),
            Self::TempSync(e) => write!(f, "sync audio state temp file: {e}"),
            Self::AtomicRename(e) => write!(f, "atomic rename audio state: {e}"),
        }
    }
}

/// Read the current audio state under a shared OFD lock.
///
/// Opens `lock_path` with `O_RDONLY|O_CLOEXEC|O_CREAT` (the lock file is
/// pre-created by systemd-tmpfiles, but we tolerate it being absent during
/// tests). Acquires a shared lock, reads and parses the state file, then
/// releases the lock.
///
/// Returns `AudioPolicyState::default_v2()` when the state file is absent.
pub fn read_audio_state_locked(
    lock_path: &Path,
    state_path: &Path,
) -> Result<AudioPolicyState, AudioStateIoError> {
    let _lock = acquire_audio_state_lock(lock_path, false)?;
    read_audio_state_unlocked(state_path)
}

pub fn read_audio_state_unlocked(state_path: &Path) -> Result<AudioPolicyState, AudioStateIoError> {
    let bytes = match std::fs::read(state_path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(AudioPolicyState::default_v2());
        }
        Err(e) => return Err(AudioStateIoError::StateRead(e)),
    };

    parse_audio_state(&bytes).map_err(AudioStateIoError::StateParse)
}

/// Write a new audio state atomically under an exclusive OFD lock.
///
/// The write path:
/// 1. Open the lock file and acquire an exclusive OFD lock.
/// 2. Serialize the new state to v2 JSON.
/// 3. Write to a `.tmp` file in the same directory (ensuring same-fs rename).
/// 4. `fsync` the temp file.
/// 5. `rename` temp → state file (atomic on the same fs).
/// 6. Release the lock via the RAII guard.
pub fn write_audio_state_locked(
    lock_path: &Path,
    state_path: &Path,
    state: &AudioPolicyState,
) -> Result<(), AudioStateIoError> {
    let _lock = acquire_audio_state_lock(lock_path, true)?;
    write_audio_state_unlocked(state_path, state)
}

pub fn write_audio_state_unlocked(
    state_path: &Path,
    state: &AudioPolicyState,
) -> Result<(), AudioStateIoError> {
    use std::io::Write as _;
    let bytes = state.to_v2_bytes().map_err(AudioStateIoError::StateParse)?;

    // Place the temp file in the same directory to guarantee same-filesystem
    // rename (hardlinks cannot cross mount points).
    let parent = state_path.parent().unwrap_or(Path::new("."));
    let tmp_path = parent.join("audio-state.json.tmp");

    let mut tmp_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(&tmp_path)
        .map_err(AudioStateIoError::TempFile)?;

    tmp_file
        .write_all(&bytes)
        .map_err(AudioStateIoError::TempWrite)?;

    // Ensure the data reaches stable storage before rename.
    tmp_file.sync_data().map_err(AudioStateIoError::TempSync)?;
    drop(tmp_file);

    std::fs::rename(&tmp_path, state_path).map_err(AudioStateIoError::AtomicRename)?;

    Ok(())
}
