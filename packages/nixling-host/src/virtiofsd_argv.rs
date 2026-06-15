//! virtiofsd argv generator.
//!
//! Pure Rust function that emits one virtiofsd argv per
//! `microvm.shares` entry. The W0b runner-shape audit shows that the
//! microvm.nix-generated virtiofsd wrappers all share the same flag
//! envelope:
//!
//! ```text
//! virtiofsd \
//!   --socket-path=<vm>-virtiofs-<tag>.sock \
//!   --socket-group=kvm \
//!   --shared-dir=<host-path> \
//!   --thread-pool-size=$(nproc) \
//!   --posix-acl --xattr \
//!   --cache=auto \
//!   --inode-file-handles=prefer
//! ```
//!
//! …plus an opportunistic `--rlimit-nofile 1048576` only when the
//! process is running as uid 0. The daemon path does not run virtiofsd
//! as root — the broker spawns each instance under a
//! per-VM virtiofsd uid/gid — so the rlimit clause is omitted by
//! default; callers that genuinely need it (raised broker carve-out)
//! can pass it through [`VirtiofsdArgvInput::extra_args`].
//!
//! Crate invariant `#![forbid(unsafe_code)]` is honoured.

use serde::{Deserialize, Serialize};

/// virtiofsd cache mode. Mirrors the upstream
/// `--cache={auto,always,never}` flag. The W0b audit shows `auto`
/// as the microvm.nix default; nixling keeps the same default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VirtiofsdCacheMode {
    Auto,
    Always,
    Never,
}

impl VirtiofsdCacheMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        }
    }
}

/// virtiofsd inode-file-handles policy. Mirrors the upstream
/// `--inode-file-handles={prefer,mandatory,never}` flag. The W0b audit
/// shows `prefer` as the microvm.nix default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VirtiofsdInodeFileHandles {
    Prefer,
    Mandatory,
    Never,
}

impl VirtiofsdInodeFileHandles {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prefer => "prefer",
            Self::Mandatory => "mandatory",
            Self::Never => "never",
        }
    }
}

/// All inputs required to render one virtiofsd argv. One instance per
/// `microvm.shares` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtiofsdArgvInput {
    /// Absolute store path to the `virtiofsd` binary.
    pub virtiofsd_binary_path: String,
    /// VM name, used only by [`exec_arg0`] for the daemon-side process
    /// name. The flag set itself does not embed the VM name.
    pub vm_name: String,
    /// Mount tag the guest references (matches the CH `--fs tag=`).
    /// Used only by [`exec_arg0`] / the share argv generator caller.
    pub share_tag: String,
    /// `--socket-path` value. The audit uses a runner-cwd-relative
    /// filename (`<vm>-virtiofs-<tag>.sock`); the daemon uses an absolute
    /// path under `/run/nixling/vms/<vm>/`. Either shape is
    /// honoured — the generator emits the string verbatim.
    pub socket_path: String,
    /// `--socket-group` owner. Audit fixture pins `kvm`; the
    /// daemon-owned broker may move this to a dedicated
    /// `nixling-virtiofs` group per the ADR 0003 minijail split.
    pub socket_group: String,
    /// `--shared-dir` host path. Absolute store path or daemon-owned
    /// state-dir path depending on the share role.
    pub shared_dir: String,
    /// `--thread-pool-size` value. Audit fixture sets `$(nproc)`;
    /// daemon caller resolves the actual integer at spawn time.
    pub thread_pool_size: u32,
    /// Emit `--posix-acl`. Audit fixture has it on.
    pub posix_acl: bool,
    /// Emit `--xattr`. Audit fixture has it on.
    pub xattr: bool,
    /// `--cache` mode.
    pub cache: VirtiofsdCacheMode,
    /// `--inode-file-handles` policy.
    pub inode_file_handles: VirtiofsdInodeFileHandles,
    /// Optional `--readonly`. Audit fixture turns this on for the
    /// `ro-store` share only; other shares are read/write.
    #[serde(default)]
    pub readonly: bool,
    /// Free-form additional virtiofsd args (e.g. `--rlimit-nofile`,
    /// `--allow-direct-io`, `--sandbox=chroot`). Caller is responsible
    /// for quoting; each entry is emitted as-is in order at the end.
    #[serde(default)]
    pub extra_args: Vec<String>,
}

/// Errors the virtiofsd argv generator can return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum VirtiofsdArgvError {
    /// `virtiofsd_binary_path` was empty or non-absolute.
    InvalidVirtiofsdBinaryPath { path: String },
    /// `vm_name` was empty (would corrupt the `exec -a` process name).
    EmptyVmName,
    /// `share_tag` was empty (would corrupt the `exec -a` process name
    /// and break the CH `--fs tag=` cross-reference).
    EmptyShareTag,
    /// `socket_path` was empty.
    EmptySocketPath,
    /// `socket_group` was empty (CH refuses to connect to a UDS with
    /// no group owner under the ADR-0003 minijail).
    EmptySocketGroup,
    /// `shared_dir` was empty.
    EmptySharedDir,
    /// `thread_pool_size` was zero. virtiofsd refuses zero.
    ZeroThreadPoolSize,
}

/// Render the virtiofsd argv. Returns the full `Vec<String>` starting
/// with the binary path. Caller pairs with [`exec_arg0`] when invoking
/// the spawn API.
pub fn generate_virtiofsd_argv(
    input: &VirtiofsdArgvInput,
) -> Result<Vec<String>, VirtiofsdArgvError> {
    if input.virtiofsd_binary_path.is_empty() || !input.virtiofsd_binary_path.starts_with('/') {
        return Err(VirtiofsdArgvError::InvalidVirtiofsdBinaryPath {
            path: input.virtiofsd_binary_path.clone(),
        });
    }
    if input.vm_name.is_empty() {
        return Err(VirtiofsdArgvError::EmptyVmName);
    }
    if input.share_tag.is_empty() {
        return Err(VirtiofsdArgvError::EmptyShareTag);
    }
    if input.socket_path.is_empty() {
        return Err(VirtiofsdArgvError::EmptySocketPath);
    }
    if input.socket_group.is_empty() {
        return Err(VirtiofsdArgvError::EmptySocketGroup);
    }
    if input.shared_dir.is_empty() {
        return Err(VirtiofsdArgvError::EmptySharedDir);
    }
    if input.thread_pool_size == 0 {
        return Err(VirtiofsdArgvError::ZeroThreadPoolSize);
    }

    let mut argv: Vec<String> = Vec::with_capacity(16);
    argv.push(input.virtiofsd_binary_path.clone());

    // virtiofsd accepts both `--flag value` and `--flag=value`; the W0b
    // audit shows the `=` form for socket-path/socket-group/shared-dir.
    // We track the audit fixture to keep parity diff visualization
    // stable.
    argv.push(format!("--socket-path={}", input.socket_path));
    argv.push(format!("--socket-group={}", input.socket_group));
    argv.push(format!("--shared-dir={}", input.shared_dir));
    argv.push(format!("--thread-pool-size={}", input.thread_pool_size));

    if input.posix_acl {
        argv.push("--posix-acl".to_owned());
    }
    if input.xattr {
        argv.push("--xattr".to_owned());
    }

    argv.push(format!("--cache={}", input.cache.as_str()));
    argv.push(format!(
        "--inode-file-handles={}",
        input.inode_file_handles.as_str()
    ));

    if input.readonly {
        argv.push("--readonly".to_owned());
    }

    for extra in &input.extra_args {
        argv.push(extra.clone());
    }

    Ok(argv)
}

/// `arg0` the daemon must pass to `execvp` (or equivalent) so each
/// per-share virtiofsd shows up in `ps` as
/// `microvm-virtiofsd@<vm>-<tag>`. Pairs the supervisord program name
/// shape from the W0b audit (`virtiofsd-ro-store` etc.) with the
/// per-VM scope.
pub fn exec_arg0(input: &VirtiofsdArgvInput) -> Result<String, VirtiofsdArgvError> {
    if input.vm_name.is_empty() {
        return Err(VirtiofsdArgvError::EmptyVmName);
    }
    if input.share_tag.is_empty() {
        return Err(VirtiofsdArgvError::EmptyShareTag);
    }
    Ok(format!(
        "microvm-virtiofsd@{}-{}",
        input.vm_name, input.share_tag
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// W0b audit fixture: `ro-store` share for `corp-vm`. Audit shape:
    /// `--socket-path=corp-vm-virtiofs-ro-store.sock`,
    /// `--socket-group=kvm`, `--shared-dir=/nix/store`,
    /// `--thread-pool-size=$(nproc)`, `--posix-acl`, `--xattr`,
    /// `--cache=auto`, `--inode-file-handles=prefer`.
    fn audit_ro_store_input() -> VirtiofsdArgvInput {
        VirtiofsdArgvInput {
            virtiofsd_binary_path:
                "/nix/store/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA-virtiofsd-1.13.0/bin/virtiofsd"
                    .to_owned(),
            vm_name: "corp-vm".to_owned(),
            share_tag: "ro-store".to_owned(),
            socket_path: "corp-vm-virtiofs-ro-store.sock".to_owned(),
            socket_group: "kvm".to_owned(),
            shared_dir: "/nix/store".to_owned(),
            thread_pool_size: 4,
            posix_acl: true,
            xattr: true,
            cache: VirtiofsdCacheMode::Auto,
            inode_file_handles: VirtiofsdInodeFileHandles::Prefer,
            readonly: true,
            extra_args: Vec::new(),
        }
    }

    fn audit_nl_meta_input() -> VirtiofsdArgvInput {
        VirtiofsdArgvInput {
            share_tag: "nl-meta".to_owned(),
            socket_path: "corp-vm-virtiofs-nl-meta.sock".to_owned(),
            shared_dir: "/var/lib/nixling/vms/corp-vm/store-meta".to_owned(),
            readonly: false,
            ..audit_ro_store_input()
        }
    }

    #[test]
    fn audit_ro_store_parity() {
        let argv = generate_virtiofsd_argv(&audit_ro_store_input()).unwrap();
        // Binary path first.
        assert!(argv[0].ends_with("/virtiofsd"));
        let joined = argv.join(" ");
        assert!(joined.contains("--socket-path=corp-vm-virtiofs-ro-store.sock"));
        assert!(joined.contains("--socket-group=kvm"));
        assert!(joined.contains("--shared-dir=/nix/store"));
        assert!(joined.contains("--thread-pool-size=4"));
        assert!(joined.contains("--posix-acl"));
        assert!(joined.contains("--xattr"));
        assert!(joined.contains("--cache=auto"));
        assert!(joined.contains("--inode-file-handles=prefer"));
        assert!(joined.contains("--readonly"));
    }

    const VIRTIOFSD_ARGV_GOLDEN: &str =
        include_str!("../../../tests/golden/runner-shape/virtiofsd-argv-minimal.txt");

    fn golden_payload() -> String {
        VIRTIOFSD_ARGV_GOLDEN
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn audit_ro_store_snapshot_line() {
        let argv = generate_virtiofsd_argv(&audit_ro_store_input()).unwrap();
        let observed = argv.join(" ");
        let expected = golden_payload();
        assert_eq!(
            observed, expected,
            "virtiofsd argv drifted from tests/golden/runner-shape/virtiofsd-argv-minimal.txt"
        );
        println!("SNAPSHOT: {observed}");
    }

    #[test]
    fn audit_nl_meta_omits_readonly() {
        let argv = generate_virtiofsd_argv(&audit_nl_meta_input()).unwrap();
        let joined = argv.join(" ");
        assert!(joined.contains("--shared-dir=/var/lib/nixling/vms/corp-vm/store-meta"));
        assert!(!joined.contains("--readonly"));
    }

    #[test]
    fn exec_arg0_matches_audit_naming() {
        let arg0 = exec_arg0(&audit_ro_store_input()).unwrap();
        assert_eq!(arg0, "microvm-virtiofsd@corp-vm-ro-store");
    }

    #[test]
    fn exec_arg0_rejects_empty_vm_name() {
        let mut input = audit_ro_store_input();
        input.vm_name.clear();
        assert!(matches!(
            exec_arg0(&input),
            Err(VirtiofsdArgvError::EmptyVmName)
        ));
    }

    #[test]
    fn exec_arg0_rejects_empty_share_tag() {
        let mut input = audit_ro_store_input();
        input.share_tag.clear();
        assert!(matches!(
            exec_arg0(&input),
            Err(VirtiofsdArgvError::EmptyShareTag)
        ));
    }

    #[test]
    fn rejects_non_absolute_binary() {
        let mut input = audit_ro_store_input();
        input.virtiofsd_binary_path = "virtiofsd".to_owned();
        assert!(matches!(
            generate_virtiofsd_argv(&input),
            Err(VirtiofsdArgvError::InvalidVirtiofsdBinaryPath { .. })
        ));
    }

    #[test]
    fn rejects_empty_binary() {
        let mut input = audit_ro_store_input();
        input.virtiofsd_binary_path.clear();
        assert!(matches!(
            generate_virtiofsd_argv(&input),
            Err(VirtiofsdArgvError::InvalidVirtiofsdBinaryPath { .. })
        ));
    }

    #[test]
    fn rejects_empty_vm_name() {
        let mut input = audit_ro_store_input();
        input.vm_name.clear();
        assert!(matches!(
            generate_virtiofsd_argv(&input),
            Err(VirtiofsdArgvError::EmptyVmName)
        ));
    }

    #[test]
    fn rejects_empty_share_tag() {
        let mut input = audit_ro_store_input();
        input.share_tag.clear();
        assert!(matches!(
            generate_virtiofsd_argv(&input),
            Err(VirtiofsdArgvError::EmptyShareTag)
        ));
    }

    #[test]
    fn rejects_empty_socket_path() {
        let mut input = audit_ro_store_input();
        input.socket_path.clear();
        assert!(matches!(
            generate_virtiofsd_argv(&input),
            Err(VirtiofsdArgvError::EmptySocketPath)
        ));
    }

    #[test]
    fn rejects_empty_socket_group() {
        let mut input = audit_ro_store_input();
        input.socket_group.clear();
        assert!(matches!(
            generate_virtiofsd_argv(&input),
            Err(VirtiofsdArgvError::EmptySocketGroup)
        ));
    }

    #[test]
    fn rejects_empty_shared_dir() {
        let mut input = audit_ro_store_input();
        input.shared_dir.clear();
        assert!(matches!(
            generate_virtiofsd_argv(&input),
            Err(VirtiofsdArgvError::EmptySharedDir)
        ));
    }

    #[test]
    fn rejects_zero_thread_pool() {
        let mut input = audit_ro_store_input();
        input.thread_pool_size = 0;
        assert!(matches!(
            generate_virtiofsd_argv(&input),
            Err(VirtiofsdArgvError::ZeroThreadPoolSize)
        ));
    }

    #[test]
    fn cache_mode_string_round_trip() {
        let modes = [
            (VirtiofsdCacheMode::Auto, "auto"),
            (VirtiofsdCacheMode::Always, "always"),
            (VirtiofsdCacheMode::Never, "never"),
        ];
        for (mode, expected) in modes {
            assert_eq!(mode.as_str(), expected);
        }
    }

    #[test]
    fn inode_file_handles_string_round_trip() {
        let modes = [
            (VirtiofsdInodeFileHandles::Prefer, "prefer"),
            (VirtiofsdInodeFileHandles::Mandatory, "mandatory"),
            (VirtiofsdInodeFileHandles::Never, "never"),
        ];
        for (mode, expected) in modes {
            assert_eq!(mode.as_str(), expected);
        }
    }

    #[test]
    fn extra_args_emitted_in_order_at_end() {
        let mut input = audit_ro_store_input();
        input.extra_args = vec![
            "--allow-direct-io".to_owned(),
            "--sandbox=chroot".to_owned(),
        ];
        let argv = generate_virtiofsd_argv(&input).unwrap();
        let last_two = &argv[argv.len() - 2..];
        assert_eq!(last_two, &["--allow-direct-io", "--sandbox=chroot"]);
    }

    #[test]
    fn omits_optional_flags_when_disabled() {
        let mut input = audit_ro_store_input();
        input.posix_acl = false;
        input.xattr = false;
        input.readonly = false;
        let argv = generate_virtiofsd_argv(&input).unwrap();
        let joined = argv.join(" ");
        assert!(!joined.contains("--posix-acl"));
        assert!(!joined.contains("--xattr"));
        assert!(!joined.contains("--readonly"));
    }

    #[test]
    fn argv_is_round_trip_serializable() {
        let input = audit_ro_store_input();
        let json = serde_json::to_string(&input).unwrap();
        let parsed: VirtiofsdArgvInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, input);
    }

    #[test]
    fn all_four_audit_shares_render_independently() {
        // The W0b audit shows the supervisord config supervises four
        // virtiofsd programs for `corp-vm`: ro-store, nl-meta,
        // nl-hkeys, nl-ssh-host. Each should render an independent
        // argv with distinct socket-path / shared-dir.
        let shares = [
            (
                "ro-store",
                "corp-vm-virtiofs-ro-store.sock",
                "/nix/store",
                true,
            ),
            (
                "nl-meta",
                "corp-vm-virtiofs-nl-meta.sock",
                "/var/lib/nixling/vms/corp-vm/store-meta",
                false,
            ),
            (
                "nl-hkeys",
                "corp-vm-virtiofs-nl-hkeys.sock",
                "/var/lib/nixling/vms/corp-vm/host-keys",
                false,
            ),
            (
                "nl-ssh-host",
                "corp-vm-virtiofs-nl-ssh-host.sock",
                "/var/lib/nixling/vms/corp-vm/sshd-host-keys",
                false,
            ),
        ];
        for (tag, socket, dir, readonly) in shares {
            let input = VirtiofsdArgvInput {
                share_tag: tag.to_owned(),
                socket_path: socket.to_owned(),
                shared_dir: dir.to_owned(),
                readonly,
                ..audit_ro_store_input()
            };
            let argv = generate_virtiofsd_argv(&input).unwrap();
            let joined = argv.join(" ");
            assert!(joined.contains(&format!("--socket-path={socket}")));
            assert!(joined.contains(&format!("--shared-dir={dir}")));
            if readonly {
                assert!(joined.contains("--readonly"));
            } else {
                assert!(!joined.contains("--readonly"));
            }
        }
    }
}
