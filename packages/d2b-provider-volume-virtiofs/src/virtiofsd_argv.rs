//! FD-based virtiofsd argv generation.
//!
//! The shared directory is represented by an inherited descriptor, and the
//! socket path is a private adapter value.  No free-form argument channel is
//! exposed.

use std::fmt;

use crate::socket_path::PrivateSocketPath;

/// Closed cache modes accepted by virtiofsd.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtiofsdCacheMode {
    /// Let virtiofsd choose its normal cache behavior.
    Auto,
    /// Always cache file contents.
    Always,
    /// Never cache file contents.
    Never,
}

impl VirtiofsdCacheMode {
    /// Return the command-line spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        }
    }
}

/// A resolved socket group, kept separate from authored resource data.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SocketGroup(u32);

impl SocketGroup {
    /// Construct a resolved group identity.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the resolved numeric group for the effect adapter.
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Debug for SocketGroup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SocketGroup(<redacted>)")
    }
}

/// Inputs accepted by the effect-boundary argv renderer.
#[derive(Clone, PartialEq, Eq)]
pub struct VirtiofsdArgvInput {
    /// Absolute path to the trusted virtiofsd binary.
    pub virtiofsd_binary_path: String,
    /// Private derived socket path.
    pub socket_path: PrivateSocketPath,
    /// Inherited Volume-view descriptor number.
    pub shared_dir_fd: i32,
    /// Resolved socket group, or `None` for broker defaulting.
    pub socket_group: Option<SocketGroup>,
    /// Bounded worker thread count.
    pub thread_pool_size: u32,
    /// Whether to enable POSIX ACL support.
    pub posix_acl: bool,
    /// Whether to enable xattr support.
    pub xattr: bool,
    /// Cache mode.
    pub cache: VirtiofsdCacheMode,
    /// Whether to serve the view read-only.
    pub readonly: bool,
}

impl fmt::Debug for VirtiofsdArgvInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VirtiofsdArgvInput")
            .field("socket_path", &self.socket_path)
            .field("shared_dir_fd", &"<redacted>")
            .field("socket_group", &self.socket_group)
            .field("thread_pool_size", &self.thread_pool_size)
            .field("posix_acl", &self.posix_acl)
            .field("xattr", &self.xattr)
            .field("cache", &self.cache)
            .field("readonly", &self.readonly)
            .finish_non_exhaustive()
    }
}

/// Closed argv rendering failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtiofsdArgvError {
    /// The binary path is not absolute.
    InvalidBinaryPath,
    /// The inherited descriptor number is invalid.
    InvalidSharedDirectoryFd,
    /// The private socket path exceeds the kernel limit.
    SocketPathTooLong,
    /// No worker threads were requested.
    ZeroThreadPoolSize,
}

impl VirtiofsdArgvError {
    /// Return the stable path-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidBinaryPath => "virtiofsd-binary-path-invalid",
            Self::InvalidSharedDirectoryFd => "virtiofsd-shared-directory-fd-invalid",
            Self::SocketPathTooLong => "virtiofsd-socket-path-too-long",
            Self::ZeroThreadPoolSize => "virtiofsd-thread-pool-zero",
        }
    }
}

impl fmt::Display for VirtiofsdArgvError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for VirtiofsdArgvError {}

/// Render the complete fixed virtiofsd argv.
pub fn generate_virtiofsd_argv(
    input: &VirtiofsdArgvInput,
) -> Result<Vec<String>, VirtiofsdArgvError> {
    if input.virtiofsd_binary_path.is_empty() || !input.virtiofsd_binary_path.starts_with('/') {
        return Err(VirtiofsdArgvError::InvalidBinaryPath);
    }
    if input.shared_dir_fd < 0 {
        return Err(VirtiofsdArgvError::InvalidSharedDirectoryFd);
    }
    if input.socket_path.byte_len() > super::MAX_SOCKET_PATH_BYTES {
        return Err(VirtiofsdArgvError::SocketPathTooLong);
    }
    if input.thread_pool_size == 0 {
        return Err(VirtiofsdArgvError::ZeroThreadPoolSize);
    }

    let mut argv = vec![
        input.virtiofsd_binary_path.clone(),
        format!("--socket-path={}", input.socket_path.as_private_str()),
    ];
    if let Some(group) = input.socket_group {
        argv.push(format!("--socket-group={}", group.get()));
    }
    argv.push(format!(
        "--shared-dir=/proc/self/fd/{}",
        input.shared_dir_fd
    ));
    argv.push(format!("--thread-pool-size={}", input.thread_pool_size));
    if input.posix_acl {
        argv.push("--posix-acl".to_owned());
    }
    if input.xattr {
        argv.push("--xattr".to_owned());
    }
    argv.push(format!("--cache={}", input.cache.as_str()));
    argv.push("--sandbox=chroot".to_owned());
    argv.push("--inode-file-handles=never".to_owned());
    if input.readonly {
        argv.push("--readonly".to_owned());
    }
    Ok(argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::ResourceRef;
    use d2b_contracts::v3::execution_policy::BoundedToken;

    fn input() -> VirtiofsdArgvInput {
        let socket_path = PrivateSocketPath::derive(
            "/run/d2b",
            &BoundedToken::parse("dev").unwrap(),
            &ResourceRef::parse("Volume/work-state").unwrap(),
            &ResourceRef::parse("Guest/work-vm").unwrap(),
        )
        .unwrap();
        VirtiofsdArgvInput {
            virtiofsd_binary_path: "/nix/store/example-virtiofsd/bin/virtiofsd".to_owned(),
            socket_path,
            shared_dir_fd: 17,
            socket_group: Some(SocketGroup::new(100)),
            thread_pool_size: 4,
            posix_acl: true,
            xattr: true,
            cache: VirtiofsdCacheMode::Auto,
            readonly: true,
        }
    }

    #[test]
    fn argv_uses_the_inherited_volume_fd_and_fixed_sandbox_flags() {
        let argv = generate_virtiofsd_argv(&input()).unwrap();
        assert!(
            argv.iter()
                .any(|arg| arg == "--shared-dir=/proc/self/fd/17")
        );
        assert!(argv.iter().any(|arg| arg == "--sandbox=chroot"));
        assert!(argv.iter().any(|arg| arg == "--inode-file-handles=never"));
        assert!(argv.iter().any(|arg| arg == "--readonly"));
        assert!(
            !argv
                .iter()
                .any(|arg| arg.starts_with("--shared-dir=/nix/store"))
        );
    }

    #[test]
    fn no_extra_argument_channel_or_host_store_path_is_admitted() {
        let argv = generate_virtiofsd_argv(&input()).unwrap();
        assert!(argv.iter().all(|arg| !arg.starts_with("--extra")));
        let known = [
            "--socket-path=",
            "--socket-group=",
            "--shared-dir=/proc/self/fd/",
            "--thread-pool-size=",
            "--posix-acl",
            "--xattr",
            "--cache=",
            "--sandbox=chroot",
            "--inode-file-handles=never",
            "--readonly",
        ];
        assert!(
            argv.iter()
                .skip(1)
                .all(|arg| { known.iter().any(|prefix| arg.starts_with(prefix)) })
        );
    }

    #[test]
    fn argv_input_debug_redacts_private_binary_and_socket_values() {
        let rendered = format!("{:?}", input());
        assert!(!rendered.contains("/nix/store"));
        assert!(!rendered.contains(".sock"));
        assert!(!rendered.contains("shared_dir_fd: 17"));
        assert!(!rendered.contains("SocketGroup(100)"));
        assert!(rendered.contains("VirtiofsdArgvInput"));
    }
}
