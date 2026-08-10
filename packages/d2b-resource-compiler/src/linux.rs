//! Linux production adapter for the Provider artifact seams.
//!
//! The adapter anchors the selected output once, resolves every child with
//! `openat2(2)`, verifies the descriptor with `fstat(2)`, and keeps readable
//! and executable authority in separate Rust types. The only intentionally
//! unsafe operation is the final `execveat(2)` call; its pointers are built
//! from owned, NUL-free `CString` values immediately before the call.

use std::{
    ffi::{CString, OsString},
    fmt,
    mem::MaybeUninit,
    os::{
        fd::OwnedFd,
        unix::ffi::{OsStrExt, OsStringExt},
    },
    path::Path,
};

use d2b_core::provider_artifact::{
    AnchoredDir, Argv, Envp, ExecutableFile, LaunchError, LayoutDir, LayoutError, LayoutPath,
    ProcessLauncher, ReadableFile,
};
use rustix::{
    cstr,
    fs::{
        FileType, Mode, OFlags, RawDir, RawMode, ResolveFlags, SeekFrom, fstat, open, openat2, seek,
    },
    io::{Errno, read},
    runtime::execveat,
};
use sha2::{Digest, Sha256};

const RESOLVE: ResolveFlags =
    ResolveFlags::BENEATH.union(ResolveFlags::NO_SYMLINKS.union(ResolveFlags::NO_MAGICLINKS));
const READ_FLAGS: OFlags = OFlags::RDONLY.union(OFlags::NONBLOCK.union(OFlags::CLOEXEC));
const EXEC_FLAGS: OFlags = OFlags::PATH.union(OFlags::CLOEXEC);
const ANCHOR_FLAGS: OFlags =
    OFlags::PATH.union(OFlags::DIRECTORY.union(OFlags::CLOEXEC.union(OFlags::NOFOLLOW)));
const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY.union(OFlags::DIRECTORY.union(OFlags::CLOEXEC));

/// Failure while opening the selected artifact output itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorError {
    /// The selected output was absent.
    Absent,
    /// The selected output was not a directory.
    NotDirectory,
    /// The selected output was a symlink or escaped the requested root.
    Refused,
    /// The kernel did not provide an expected directory descriptor.
    Io,
}

impl fmt::Display for AnchorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Absent => "selected Provider output is absent",
            Self::NotDirectory => "selected Provider output is not a directory",
            Self::Refused => "selected Provider output resolution was refused",
            Self::Io => "selected Provider output could not be opened",
        })
    }
}

impl std::error::Error for AnchorError {}

/// An anchored Provider output directory.
pub struct LinuxAnchoredDir {
    fd: OwnedFd,
}

impl fmt::Debug for LinuxAnchoredDir {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LinuxAnchoredDir(<redacted>)")
    }
}

impl LinuxAnchoredDir {
    /// Open and anchor one selected output.
    pub fn open(path: &Path) -> Result<Self, AnchorError> {
        if !path.is_absolute() {
            return Err(AnchorError::Refused);
        }
        let fd = open(path, ANCHOR_FLAGS, Mode::empty()).map_err(anchor_error)?;
        let stat = fstat(&fd).map_err(|_| AnchorError::Io)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
            return Err(AnchorError::NotDirectory);
        }
        Ok(Self { fd })
    }

    /// Return the resolve mask used by all child opens.
    pub const fn resolve_flags() -> ResolveFlags {
        RESOLVE
    }

    /// Return the read-mode flags used by the compiler.
    pub const fn readable_flags() -> OFlags {
        READ_FLAGS
    }

    /// Return the execute-mode flags used by the launcher.
    pub const fn executable_flags() -> OFlags {
        EXEC_FLAGS
    }
}

impl AnchoredDir for LinuxAnchoredDir {
    type Readable = LinuxReadableFile;
    type Executable = LinuxExecutable;

    fn open_readable(&self, path: LayoutPath) -> Result<Self::Readable, LayoutError> {
        let path_string = validate_layout_path(path.as_str())?;
        let fd = openat2(&self.fd, path_string, READ_FLAGS, Mode::empty(), RESOLVE)
            .map_err(layout_error)?;
        let stat = fstat(&fd).map_err(|_| LayoutError::NotRegular)?;
        ensure_regular(&stat)?;
        if path.as_str().starts_with("bin/") && !has_execute_bit(stat.st_mode) {
            return Err(LayoutError::NotExecutable);
        }
        Ok(LinuxReadableFile {
            fd,
            length: u64::try_from(stat.st_size).unwrap_or(u64::MAX),
            mode: stat.st_mode,
        })
    }

    fn open_executable(&self, path: LayoutPath) -> Result<Self::Executable, LayoutError> {
        let path_string = validate_layout_path(path.as_str())?;
        let fd = openat2(&self.fd, path_string, EXEC_FLAGS, Mode::empty(), RESOLVE)
            .map_err(layout_error)?;
        let stat = fstat(&fd).map_err(|_| LayoutError::NotRegular)?;
        ensure_regular(&stat)?;
        Ok(LinuxExecutable { fd })
    }

    fn entries(&self, dir: LayoutDir) -> Result<Vec<OsString>, LayoutError> {
        let path_string = validate_layout_path(dir.as_str())?;
        let fd = openat2(
            &self.fd,
            path_string,
            DIRECTORY_FLAGS,
            Mode::empty(),
            RESOLVE,
        )
        .map_err(layout_error)?;
        let stat = fstat(&fd).map_err(|_| LayoutError::NotRegular)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
            return Err(LayoutError::NotRegular);
        }
        let mut buffer = vec![MaybeUninit::<u8>::uninit(); 64 * 1024];
        let mut entries = Vec::new();
        let mut directory = RawDir::new(&fd, &mut buffer);
        while let Some(entry) = directory.next() {
            let entry = entry.map_err(|_| LayoutError::NoDevice)?;
            let bytes = entry.file_name().to_bytes();
            if bytes != b"." && bytes != b".." {
                entries.push(OsString::from_vec(bytes.to_owned()));
            }
        }
        Ok(entries)
    }
}

/// A readable, descriptor-verified regular file.
pub struct LinuxReadableFile {
    fd: OwnedFd,
    length: u64,
    mode: RawMode,
}

impl fmt::Debug for LinuxReadableFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LinuxReadableFile")
            .field("length", &self.length)
            .field("mode", &"redacted")
            .finish_non_exhaustive()
    }
}

impl LinuxReadableFile {
    /// Return the mode captured by the same `fstat` as regular-file checking.
    pub const fn mode(&self) -> RawMode {
        self.mode
    }
}

impl ReadableFile for LinuxReadableFile {
    fn len(&self) -> u64 {
        self.length
    }

    fn read_prefix(&mut self, out: &mut [u8]) -> Result<usize, LayoutError> {
        let mut total = 0;
        while total < out.len() {
            let count = read(&self.fd, &mut out[total..]).map_err(|_| LayoutError::NoDevice)?;
            if count == 0 {
                break;
            }
            total += count;
        }
        Ok(total)
    }

    fn read_to_digest(self) -> Result<[u8; 32], LayoutError> {
        seek(&self.fd, SeekFrom::Start(0)).map_err(|_| LayoutError::NoDevice)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let count = read(&self.fd, &mut buffer).map_err(|_| LayoutError::NoDevice)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        Ok(hasher.finalize().into())
    }
}

/// An executable-only descriptor.
pub struct LinuxExecutable {
    fd: OwnedFd,
}

impl fmt::Debug for LinuxExecutable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LinuxExecutable(<redacted>)")
    }
}

impl ExecutableFile for LinuxExecutable {}

/// The production `execveat(AT_EMPTY_PATH)` launcher.
#[derive(Debug, Default, Clone, Copy)]
pub struct LinuxProcessLauncher;

impl ProcessLauncher for LinuxProcessLauncher {
    type Executable = LinuxExecutable;

    fn exec_from(
        &self,
        file: Self::Executable,
        argv: &Argv,
        envp: &Envp,
    ) -> Result<std::convert::Infallible, LaunchError> {
        let argv = argv
            .as_slice()
            .iter()
            .map(|argument| CString::new(argument.as_bytes()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| LaunchError::FormatRejected)?;
        if argv.is_empty() {
            return Err(LaunchError::FormatRejected);
        }
        let envp = envp
            .as_slice()
            .iter()
            .map(|entry| CString::new(entry.as_bytes()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| LaunchError::FormatRejected)?;
        let argv_pointers: Vec<*const u8> = argv
            .iter()
            .map(|argument| argument.as_ptr().cast())
            .chain(std::iter::once(std::ptr::null()))
            .collect();
        let envp_pointers: Vec<*const u8> = envp
            .iter()
            .map(|entry| entry.as_ptr().cast())
            .chain(std::iter::once(std::ptr::null()))
            .collect();
        // SAFETY: every pointer points into an owned CString kept alive until
        // the call returns, both vectors are NUL-terminated, and `file` is an
        // owned descriptor opened with O_PATH|O_CLOEXEC and fstat-verified as
        // a regular file immediately before reaching this type.
        let error = unsafe {
            execveat(
                &file.fd,
                cstr!(""),
                argv_pointers.as_ptr(),
                envp_pointers.as_ptr(),
                rustix::fs::AtFlags::EMPTY_PATH,
            )
        };
        Err(match error {
            Errno::NOEXEC => LaunchError::FormatRejected,
            Errno::ACCESS => LaunchError::PermissionDenied,
            Errno::NOENT => LaunchError::InterpreterUnresolvable,
            _ => LaunchError::FormatRejected,
        })
    }
}

fn anchor_error(error: Errno) -> AnchorError {
    match error {
        Errno::NOENT => AnchorError::Absent,
        Errno::LOOP | Errno::XDEV | Errno::INVAL => AnchorError::Refused,
        Errno::NOTDIR => AnchorError::NotDirectory,
        _ => AnchorError::Io,
    }
}

fn layout_error(error: Errno) -> LayoutError {
    match error {
        Errno::NOENT => LayoutError::Absent,
        Errno::LOOP => LayoutError::SymlinkRefused,
        Errno::XDEV => LayoutError::NotBeneath,
        Errno::NXIO => LayoutError::NoDevice,
        Errno::NOTDIR => LayoutError::NotRegular,
        _ => LayoutError::NotRegular,
    }
}

fn validate_layout_path(path: &str) -> Result<&str, LayoutError> {
    if path.is_empty()
        || path.starts_with('/')
        || path
            .split('/')
            .any(|component| component.is_empty() || component == ".")
        || path.split('/').any(|component| component == "..")
        || path.as_bytes().contains(&0)
    {
        return Err(LayoutError::NotBeneath);
    }
    Ok(path)
}

fn ensure_regular(stat: &rustix::fs::Stat) -> Result<(), LayoutError> {
    if FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile {
        Ok(())
    } else {
        Err(LayoutError::NotRegular)
    }
}

fn has_execute_bit(mode: RawMode) -> bool {
    let mode = Mode::from_raw_mode(mode);
    mode.intersects(Mode::XUSR | Mode::XGRP | Mode::XOTH)
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsString,
        fs,
        io::Write,
        os::unix::{fs::PermissionsExt, fs::symlink},
        path::Path,
    };

    use tempfile::tempdir;

    use super::*;

    fn write_file(path: &Path, bytes: &[u8], mode: u32) {
        let mut file = fs::File::create(path).expect("create fixture");
        file.write_all(bytes).expect("write fixture");
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("set mode");
    }

    #[test]
    fn anchored_read_rejects_symlink_and_escape() {
        let root = tempdir().expect("temporary root");
        fs::create_dir_all(root.path().join("share/d2b/provider")).expect("metadata directory");
        write_file(
            &root.path().join("share/d2b/provider/config-schema.json"),
            br#"{"type":"object"}"#,
            0o644,
        );
        symlink(
            "config-schema.json",
            root.path().join("share/d2b/provider/link"),
        )
        .expect("same-output link");
        let outside = tempdir().expect("outside root");
        write_file(&outside.path().join("outside"), b"outside", 0o644);
        symlink(
            outside.path().join("outside"),
            root.path().join("share/d2b/provider/escape"),
        )
        .expect("escape link");

        let anchor = LinuxAnchoredDir::open(root.path()).expect("anchor");
        assert_eq!(
            anchor
                .open_readable(LayoutPath::new("share/d2b/provider/link"))
                .unwrap_err(),
            LayoutError::SymlinkRefused
        );
        assert_eq!(
            anchor
                .open_readable(LayoutPath::new("share/d2b/provider/escape"))
                .unwrap_err(),
            LayoutError::SymlinkRefused
        );
        assert_eq!(
            anchor
                .open_readable(LayoutPath::new(
                    "share/d2b/provider/../provider/config-schema.json"
                ))
                .unwrap_err(),
            LayoutError::NotBeneath
        );
    }

    #[test]
    fn anchored_read_checks_regular_file_and_execute_mode() {
        let root = tempdir().expect("temporary root");
        fs::create_dir(root.path().join("bin")).expect("bin directory");
        write_file(&root.path().join("bin/program"), b"not elf", 0o644);
        let anchor = LinuxAnchoredDir::open(root.path()).expect("anchor");
        assert_eq!(
            anchor
                .open_readable(LayoutPath::new("bin/program"))
                .unwrap_err(),
            LayoutError::NotExecutable
        );
        fs::set_permissions(
            root.path().join("bin/program"),
            fs::Permissions::from_mode(0o755),
        )
        .expect("set execute mode");
        let mut file = anchor
            .open_readable(LayoutPath::new("bin/program"))
            .expect("readable executable");
        let mut prefix = [0_u8; 4];
        assert_eq!(file.read_prefix(&mut prefix).unwrap(), 4);
        assert_eq!(&prefix, b"not ");
        assert_eq!(file.mode() & 0o111, 0o111);
    }

    #[test]
    fn anchored_entries_are_relative_to_the_open_directory() {
        let root = tempdir().expect("temporary root");
        fs::create_dir(root.path().join("bin")).expect("bin directory");
        write_file(&root.path().join("bin/a"), b"a", 0o755);
        write_file(&root.path().join("bin/b"), b"b", 0o755);
        let anchor = LinuxAnchoredDir::open(root.path()).expect("anchor");
        let mut entries = anchor.entries(LayoutDir::new("bin")).unwrap();
        entries.sort();
        assert_eq!(entries, vec![OsString::from("a"), OsString::from("b")]);
    }
}
