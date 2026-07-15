use std::{
    ffi::OsStr,
    fmt,
    os::{
        fd::{AsFd, OwnedFd},
        unix::ffi::OsStrExt,
    },
    path::Path,
    sync::Arc,
};

use d2b_contracts::v2_state::ResourceId;
use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};

use crate::{Error, ErrorCode, Result};

const RESOLVE_SAFE: ResolveFlags = ResolveFlags::BENEATH
    .union(ResolveFlags::NO_SYMLINKS)
    .union(ResolveFlags::NO_MAGICLINKS)
    .union(ResolveFlags::NO_XDEV);

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LeafName(String);

impl LeafName {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let bytes = value.as_bytes();
        if bytes.is_empty()
            || bytes.len() > 255
            || value == "."
            || value == ".."
            || !bytes
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(Error::Code(ErrorCode::PathRejected));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for LeafName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LeafName([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RelativePath {
    components: Vec<LeafName>,
    rendered: String,
}

impl RelativePath {
    pub fn from_components(
        components: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self> {
        let components = components
            .into_iter()
            .map(|component| LeafName::parse(component.into()))
            .collect::<Result<Vec<_>>>()?;
        if components.is_empty() {
            return Err(Error::Code(ErrorCode::PathRejected));
        }
        let rendered = components
            .iter()
            .map(LeafName::as_str)
            .collect::<Vec<_>>()
            .join("/");
        Ok(Self {
            components,
            rendered,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.rendered
    }

    pub fn leaf(&self) -> &LeafName {
        self.components
            .last()
            .expect("non-empty path is established by constructor")
    }
}

impl fmt::Debug for RelativePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RelativePath([redacted])")
    }
}

#[derive(Clone)]
pub struct AnchoredDir {
    fd: Arc<OwnedFd>,
}

impl fmt::Debug for AnchoredDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AnchoredDir")
    }
}

impl AnchoredDir {
    pub fn open_trusted(path: &Path) -> Result<Self> {
        if !path.is_absolute() || path.as_os_str().as_bytes().contains(&0) {
            return Err(Error::Code(ErrorCode::PathRejected));
        }
        let fd = rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|error| Error::io(ErrorCode::PathRejected, error))?;
        Ok(Self { fd: Arc::new(fd) })
    }

    pub fn from_owned_fd(fd: OwnedFd) -> Result<Self> {
        let stat = rustix::fs::fstat(&fd).map_err(|error| Error::io(ErrorCode::Io, error))?;
        if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::Directory {
            return Err(Error::Code(ErrorCode::PathRejected));
        }
        let flags =
            rustix::fs::fcntl_getfd(&fd).map_err(|error| Error::io(ErrorCode::Io, error))?;
        if !flags.contains(rustix::io::FdFlags::CLOEXEC) {
            return Err(Error::Code(ErrorCode::PathRejected));
        }
        Ok(Self { fd: Arc::new(fd) })
    }

    pub(crate) fn open_beneath(
        &self,
        path: &RelativePath,
        flags: OFlags,
        mode: Mode,
    ) -> Result<OwnedFd> {
        openat2(
            self.fd.as_ref(),
            path.as_str(),
            flags | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            mode,
            RESOLVE_SAFE,
        )
        .map_err(|error| {
            let code = if error == rustix::io::Errno::NOENT {
                ErrorCode::Missing
            } else if error == rustix::io::Errno::EXIST {
                ErrorCode::AlreadyExists
            } else {
                ErrorCode::PathRejected
            };
            Error::io(code, error)
        })
    }

    pub(crate) fn fd(&self) -> impl AsFd + '_ {
        self.fd.as_ref().as_fd()
    }
}

#[derive(Clone)]
pub struct AnchoredResource {
    pub resource_id: ResourceId,
    pub directory: AnchoredDir,
    pub leaf: LeafName,
}

impl fmt::Debug for AnchoredResource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnchoredResource")
            .field("resource_id", &self.resource_id)
            .finish_non_exhaustive()
    }
}

impl AnchoredResource {
    pub fn new(resource_id: ResourceId, directory: &AnchoredDir, leaf: LeafName) -> Self {
        Self {
            resource_id,
            directory: directory.clone(),
            leaf,
        }
    }

    pub(crate) fn leaf_os(&self) -> &OsStr {
        OsStr::new(self.leaf.as_str())
    }
}
