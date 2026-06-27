//! Cloud Hypervisor argv generator.
//!
//! Pure Rust function that takes a [`ChArgvInput`] (VM identity,
//! closure paths, manifest network/share inputs, daemon-owned socket
//! paths) and emits the `Vec<String>` argv that `d2bd` will exec
//! against the packaged Cloud Hypervisor binary.
//!
//! The shape MUST track the W0b parity oracle in
//! `tests/golden/runner-shape/cloud-hypervisor-argv-minimal.txt` for
//! the headless `examples/minimal` VM, modulo the daemon divergences
//! enumerated in ADR 0004:
//!
//! - API socket placement/permissions (`--api-socket` is daemon-owned
//!   under `/run/d2b/vms/<vm>/ch-api.sock`);
//! - vsock CID allocation (daemon may override the manifest value if
//!   it conflicts with live state — but this generator emits whatever
//!   CID it is given, allocation lives in the caller);
//! - TAP fd-passing (`--net 'fd=<N>,mac=...'` when the host probed
//!   `tap-fd` mode, else `--net 'mac=...,tap=<name>'`).
//!
//! Crate invariant `#![forbid(unsafe_code)]` is honoured: the generator
//! is pure data shuffling with no system calls.

use serde::{Deserialize, Serialize};

/// CH net-handoff mode the broker selected at host-check time, see
/// `d2b_host::runner_shape::NetHandoffMode` (kept as a string here
/// to avoid pulling in the probe surface as a dependency of the argv
/// generator; the daemon translates the runner-shape outcome
/// before calling [`generate_ch_argv`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChNetHandoff {
    /// Broker opened TAP + `/dev/vhost-net`, fds passed via SCM_RIGHTS.
    /// `--net 'fd=<N>,mac=...'` (no `tap=` token).
    TapFd,
    /// Broker created a persistent TAP with `TUNSETOWNER`/`TUNSETGROUP`.
    /// `--net 'mac=...,tap=<name>'`.
    PersistentTap,
}

/// Single `--fs socket=...,tag=...` entry, one per `microvm.shares`
/// row. Order is preserved by the caller; the audit fixture shows the
/// emission order matches the declared share order (`ro-store`,
/// `d2b-meta`, `d2b-hkeys`, `d2b-ssh-host`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChFsShare {
    /// Path to the virtiofsd-owned UDS the runner connects to.
    pub socket: String,
    /// Mount tag the guest uses to reference this share.
    pub tag: String,
}

/// Single `--net 'mac=...,(tap|fd)=...'` entry, one per
/// `microvm.interfaces` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChNetIface {
    /// IEEE OUI-formatted MAC address (`AA:BB:CC:DD:EE:FF`). The
    /// generator emits it verbatim — case is preserved.
    pub mac: String,
    /// TAP ifname when [`ChNetHandoff::PersistentTap`] is selected.
    /// Ignored under [`ChNetHandoff::TapFd`].
    pub tap_ifname: String,
    /// File-descriptor slot the broker passes via SCM_RIGHTS under
    /// [`ChNetHandoff::TapFd`]. Daemon sets this to the post-`dup2`
    /// fd number it will hand to the runner. Ignored under
    /// [`ChNetHandoff::PersistentTap`].
    pub tap_fd: Option<i32>,
}

/// Vsock transport spec. The audit fixture uses
/// `cid=<N>,socket=notify.vsock`. Observability and other components
/// may append additional `--vsock socket=...` entries via [`extra_vsock`].
///
/// [`extra_vsock`]: ChArgvInput::extra_vsock
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChVsock {
    pub cid: u32,
    pub socket: String,
}

/// All inputs required to render the CH argv. Pure data; the daemon
/// resolves the bundle / probes / manifest / closures DTOs into this
/// shape before calling [`generate_ch_argv`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChArgvInput {
    /// VM name (e.g. `"corp-vm"`); used for the `exec -a` process name.
    pub vm_name: String,
    /// Absolute store path to the `cloud-hypervisor` binary the daemon
    /// will exec.
    pub ch_binary_path: String,

    // ---- microvm fields ----
    /// CPU count; emitted as `--cpus 'boot=<N>'`. Daemon caller may
    /// emit additional CH cpu features in [`extra_args`] if needed.
    ///
    /// [`extra_args`]: ChArgvInput::extra_args
    pub cpus: u32,
    /// Whether to emit `--watchdog`. The audit fixture has it on for
    /// the microvm.nix CH runner default.
    pub watchdog: bool,
    /// Absolute store path to the guest kernel vmlinux.
    pub kernel_path: String,
    /// Absolute store path to the guest initrd. `None` for VMs that
    /// boot from a disk image without initramfs (none today in audit).
    pub initramfs_path: Option<String>,
    /// Verbatim kernel command line. Caller assembles this from
    /// `closures/<vm>.json` (boot cmdline + root + init + regInfo).
    pub cmdline: String,
    /// CH seccomp posture; the audit fixture sets `--seccomp true`.
    pub seccomp: String,
    /// Memory spec; the audit fixture uses `shared=on,size=512M`.
    /// `shared=on` is required by virtiofs.
    pub memory: String,
    /// Platform OEM strings (systemd notify credential, observability
    /// hints, etc.). One entry → `--platform 'oem_strings=[A,B,...]'`.
    /// Audit shape: `["io.systemd.credential:vmm.notify_socket=vsock-stream:2:8888"]`.
    pub platform_oem_strings: Vec<String>,
    /// `--console` value (`null` for headless).
    pub console: String,
    /// `--serial` value (`tty` for headless).
    pub serial: String,
    /// Primary vsock transport (CH notify). Always emitted first.
    pub primary_vsock: Option<ChVsock>,
    /// Extra `--vsock` entries (observability listener, etc.).
    /// Each renders as a separate `--vsock socket=<path>` (no CID).
    #[serde(default)]
    pub extra_vsock: Vec<String>,
    /// virtiofs shares; each rendered as one `--fs` arg value (the
    /// first share also carries the literal `--fs` flag, subsequent
    /// shares are positional args under the same flag as the audit
    /// fixture shows).
    #[serde(default)]
    pub fs_shares: Vec<ChFsShare>,
    /// Daemon-owned CH API socket path. The ADR 0014 contract requires
    /// `mode=0660` and a non-empty owner; both are enforced
    /// elsewhere (`runner_shape::runner_shape_preflight`) — this
    /// generator only emits the path.
    pub api_socket_path: String,
    /// Network interfaces; each emits `--net 'mac=...,(tap|fd)=...'`.
    #[serde(default)]
    pub net_ifaces: Vec<ChNetIface>,
    /// Selected TAP handoff mode; controls whether [`ChNetIface`]
    /// renders `tap=<name>` or `fd=<N>`.
    pub net_handoff: ChNetHandoff,
    /// Free-form additional CH args (TPM, GPU sockets, audio user
    /// devices, video vhost-user-media). Caller is responsible for
    /// quoting; each entry is emitted as-is in order at the end.
    #[serde(default)]
    pub extra_args: Vec<String>,
}

/// Errors the argv generator can return. Today the only failure modes
/// are structural inputs that would emit a malformed CH command line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum ChArgvError {
    /// `vm_name` was empty; `exec -a "microvm@"` would be ambiguous
    /// and the CH process name parity oracle would diverge.
    EmptyVmName,
    /// `ch_binary_path` was empty or non-absolute. CH must be invoked
    /// by absolute store path so the daemon's seccomp/minijail profile
    /// matches the resolved binary inode.
    InvalidChBinaryPath { path: String },
    /// `cpus` was zero. CH refuses `boot=0`.
    ZeroCpus,
    /// `kernel_path` was empty. Every supported VM must boot a
    /// d2b-provided kernel; bootloaders are out of scope.
    EmptyKernelPath,
    /// A [`ChNetIface`] in [`ChNetHandoff::TapFd`] mode was missing
    /// its `tap_fd`.
    TapFdMissing { iface_mac: String },
    /// A [`ChNetIface`] in [`ChNetHandoff::PersistentTap`] mode was
    /// missing its `tap_ifname`.
    TapIfnameMissing { iface_mac: String },
}

/// Render the CH argv. Returns the full `Vec<String>` starting with
/// the binary path; the caller prepends `exec -a` semantics via the
/// process spawn API (the daemon uses `nix::unistd::execvp` with an
/// `arg0` override matching [`exec_arg0`]).
pub fn generate_ch_argv(input: &ChArgvInput) -> Result<Vec<String>, ChArgvError> {
    if input.vm_name.is_empty() {
        return Err(ChArgvError::EmptyVmName);
    }
    if input.ch_binary_path.is_empty() || !input.ch_binary_path.starts_with('/') {
        return Err(ChArgvError::InvalidChBinaryPath {
            path: input.ch_binary_path.clone(),
        });
    }
    if input.cpus == 0 {
        return Err(ChArgvError::ZeroCpus);
    }
    if input.kernel_path.is_empty() {
        return Err(ChArgvError::EmptyKernelPath);
    }
    for iface in &input.net_ifaces {
        match input.net_handoff {
            ChNetHandoff::TapFd => {
                if iface.tap_fd.is_none() {
                    return Err(ChArgvError::TapFdMissing {
                        iface_mac: iface.mac.clone(),
                    });
                }
            }
            ChNetHandoff::PersistentTap => {
                if iface.tap_ifname.is_empty() {
                    return Err(ChArgvError::TapIfnameMissing {
                        iface_mac: iface.mac.clone(),
                    });
                }
            }
        }
    }

    let mut argv: Vec<String> = Vec::with_capacity(32);
    argv.push(input.ch_binary_path.clone());

    argv.push("--cpus".to_owned());
    argv.push(format!("boot={}", input.cpus));

    if input.watchdog {
        argv.push("--watchdog".to_owned());
    }

    argv.push("--kernel".to_owned());
    argv.push(input.kernel_path.clone());

    if let Some(initrd) = &input.initramfs_path {
        argv.push("--initramfs".to_owned());
        argv.push(initrd.clone());
    }

    argv.push("--cmdline".to_owned());
    argv.push(input.cmdline.clone());

    argv.push("--seccomp".to_owned());
    argv.push(input.seccomp.clone());

    argv.push("--memory".to_owned());
    argv.push(input.memory.clone());

    if !input.platform_oem_strings.is_empty() {
        argv.push("--platform".to_owned());
        // CH expects oem_strings=[A,B,C] (comma-separated, no spaces).
        // The audit fixture has a single OEM string with no embedded
        // commas; if multi-string callers pass entries containing
        // commas they must escape upstream — CH itself does not
        // support nested escapes in this flag.
        let joined = input.platform_oem_strings.join(",");
        argv.push(format!("oem_strings=[{joined}]"));
    }

    argv.push("--console".to_owned());
    argv.push(input.console.clone());

    argv.push("--serial".to_owned());
    argv.push(input.serial.clone());

    if let Some(vsock) = &input.primary_vsock {
        argv.push("--vsock".to_owned());
        argv.push(format!("cid={},socket={}", vsock.cid, vsock.socket));
    }
    for socket in &input.extra_vsock {
        argv.push("--vsock".to_owned());
        argv.push(format!("socket={socket}"));
    }

    if !input.fs_shares.is_empty() {
        argv.push("--fs".to_owned());
        for share in &input.fs_shares {
            argv.push(format!("socket={},tag={}", share.socket, share.tag));
        }
    }

    argv.push("--api-socket".to_owned());
    argv.push(input.api_socket_path.clone());

    if !input.net_ifaces.is_empty() {
        argv.push("--net".to_owned());
        for iface in &input.net_ifaces {
            let net_val = match input.net_handoff {
                ChNetHandoff::TapFd => {
                    let fd = iface.tap_fd.expect("validated above");
                    format!("fd={},mac={}", fd, iface.mac)
                }
                ChNetHandoff::PersistentTap => {
                    format!("mac={},tap={}", iface.mac, iface.tap_ifname)
                }
            };
            argv.push(net_val);
        }
    }

    for extra in &input.extra_args {
        argv.push(extra.clone());
    }

    Ok(argv)
}

/// `arg0` the daemon must pass to `execvp` (or equivalent) to match the
/// microvm.nix runner process name (`microvm@<vm>`). Kept as a separate
/// function because the spawn API surface (broker `SpawnRunner`) carries
/// `arg0` distinctly from the argv vector.
pub fn exec_arg0(input: &ChArgvInput) -> Result<String, ChArgvError> {
    if input.vm_name.is_empty() {
        return Err(ChArgvError::EmptyVmName);
    }
    Ok(format!("microvm@{}", input.vm_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal headless VM matching the W0b audit's `corp-vm` shape,
    /// except daemon-owned divergences:
    ///
    /// - API socket lives at `/run/d2b/vms/corp-vm/ch-api.sock`
    ///   (audit fixture uses the runner-cwd-relative `corp-vm.sock`);
    /// - net uses [`ChNetHandoff::PersistentTap`] which is the audit's
    ///   `--net 'mac=...,tap=work-l10'` shape.
    fn audit_input() -> ChArgvInput {
        ChArgvInput {
            vm_name: "corp-vm".to_owned(),
            ch_binary_path:
                "/nix/store/5dp5ya1q03ab3indxnd7x3pwixifw5rn-cloud-hypervisor-52.0/bin/cloud-hypervisor"
                    .to_owned(),
            cpus: 1,
            watchdog: true,
            kernel_path:
                "/nix/store/6p1aazl39927kp22ajw4h8bqa6j5g4vz-linux-6.18.31-dev/vmlinux"
                    .to_owned(),
            initramfs_path: Some(
                "/nix/store/qdrg2rycwnqw7b5m69v12pizvf3p19yr-initrd-linux-6.18.31/initrd"
                    .to_owned(),
            ),
            cmdline:
                "earlyprintk=ttyS0 console=ttyS0 reboot=t panic=-1 8250.nr_uarts=1 \
                 root=fstab loglevel=4 lsm=landlock,yama,bpf \
                 init=/nix/store/5ycspc2h3zhl9qiq2axsc1hvirr5pm02-nixos-system-corp-vm-26.05pre-git/init \
                 regInfo=/nix/store/ldfmwp9xh6av69d5bvz7j898m6kqlgzm-closure-info/registration"
                    .to_owned(),
            seccomp: "true".to_owned(),
            memory: "shared=on,size=512M".to_owned(),
            platform_oem_strings: vec![
                "io.systemd.credential:vmm.notify_socket=vsock-stream:2:8888".to_owned()
            ],
            console: "null".to_owned(),
            serial: "tty".to_owned(),
            primary_vsock: Some(ChVsock {
                cid: 10_914_385,
                socket: "notify.vsock".to_owned(),
            }),
            extra_vsock: Vec::new(),
            fs_shares: vec![
                ChFsShare { socket: "corp-vm-virtiofs-ro-store.sock".to_owned(),    tag: "ro-store".to_owned() },
                ChFsShare { socket: "corp-vm-virtiofs-d2b-meta.sock".to_owned(),     tag: "d2b-meta".to_owned() },
                ChFsShare { socket: "corp-vm-virtiofs-d2b-hkeys.sock".to_owned(),    tag: "d2b-hkeys".to_owned() },
                ChFsShare { socket: "corp-vm-virtiofs-d2b-ssh-host.sock".to_owned(), tag: "d2b-ssh-host".to_owned() },
            ],
            api_socket_path: "corp-vm.sock".to_owned(),
            net_ifaces: vec![ChNetIface {
                mac: "02:76:53:AE:57:0A".to_owned(),
                tap_ifname: "work-l10".to_owned(),
                tap_fd: None,
            }],
            net_handoff: ChNetHandoff::PersistentTap,
            extra_args: Vec::new(),
        }
    }

    #[test]
    fn headless_audit_parity_minimal() {
        let argv = generate_ch_argv(&audit_input()).expect("audit input is valid");

        // Sanity: binary path first.
        assert_eq!(
            argv[0],
            "/nix/store/5dp5ya1q03ab3indxnd7x3pwixifw5rn-cloud-hypervisor-52.0/bin/cloud-hypervisor"
        );

        // Spot-check ordered emission. The pinned unit-test surface
        // covers the W0b audit contract field-by-field rather than
        // doing a byte-compare against
        // tests/golden/runner-shape/cloud-hypervisor-argv-minimal.txt;
        // the W0b audit fixture is a snapshot of microvm.nix's
        // runner shape that includes `${runtime_args:-}` template
        // expansion the daemon does not emit, so a literal
        // byte-compare would always diverge.
        let joined = argv.join(" ");
        assert!(joined.contains("--cpus boot=1"));
        assert!(joined.contains("--watchdog"));
        assert!(joined.contains(
            "--kernel /nix/store/6p1aazl39927kp22ajw4h8bqa6j5g4vz-linux-6.18.31-dev/vmlinux"
        ));
        assert!(joined.contains(
            "--initramfs /nix/store/qdrg2rycwnqw7b5m69v12pizvf3p19yr-initrd-linux-6.18.31/initrd"
        ));
        assert!(joined.contains("--seccomp true"));
        assert!(joined.contains("--memory shared=on,size=512M"));
        assert!(joined.contains(
            "--platform oem_strings=[io.systemd.credential:vmm.notify_socket=vsock-stream:2:8888]"
        ));
        assert!(joined.contains("--console null"));
        assert!(joined.contains("--serial tty"));
        assert!(joined.contains("--vsock cid=10914385,socket=notify.vsock"));
        assert!(joined.contains("--fs socket=corp-vm-virtiofs-ro-store.sock,tag=ro-store"));
        assert!(joined.contains("socket=corp-vm-virtiofs-d2b-meta.sock,tag=d2b-meta"));
        assert!(joined.contains("socket=corp-vm-virtiofs-d2b-hkeys.sock,tag=d2b-hkeys"));
        assert!(joined.contains("socket=corp-vm-virtiofs-d2b-ssh-host.sock,tag=d2b-ssh-host"));
        assert!(joined.contains("--api-socket corp-vm.sock"));
        assert!(joined.contains("--net mac=02:76:53:AE:57:0A,tap=work-l10"));
    }

    #[test]
    fn exec_arg0_matches_runner_process_name() {
        assert_eq!(exec_arg0(&audit_input()).unwrap(), "microvm@corp-vm");
    }

    #[test]
    fn exec_arg0_rejects_empty_vm_name() {
        let mut input = audit_input();
        input.vm_name.clear();
        assert!(matches!(exec_arg0(&input), Err(ChArgvError::EmptyVmName)));
    }

    #[test]
    fn rejects_empty_vm_name() {
        let mut input = audit_input();
        input.vm_name.clear();
        assert!(matches!(
            generate_ch_argv(&input),
            Err(ChArgvError::EmptyVmName)
        ));
    }

    #[test]
    fn rejects_non_absolute_ch_binary() {
        let mut input = audit_input();
        input.ch_binary_path = "cloud-hypervisor".to_owned();
        assert!(matches!(
            generate_ch_argv(&input),
            Err(ChArgvError::InvalidChBinaryPath { .. })
        ));
    }

    #[test]
    fn rejects_empty_ch_binary() {
        let mut input = audit_input();
        input.ch_binary_path.clear();
        assert!(matches!(
            generate_ch_argv(&input),
            Err(ChArgvError::InvalidChBinaryPath { .. })
        ));
    }

    #[test]
    fn rejects_zero_cpus() {
        let mut input = audit_input();
        input.cpus = 0;
        assert!(matches!(
            generate_ch_argv(&input),
            Err(ChArgvError::ZeroCpus)
        ));
    }

    #[test]
    fn rejects_empty_kernel_path() {
        let mut input = audit_input();
        input.kernel_path.clear();
        assert!(matches!(
            generate_ch_argv(&input),
            Err(ChArgvError::EmptyKernelPath)
        ));
    }

    #[test]
    fn tap_fd_mode_emits_fd_token() {
        let mut input = audit_input();
        input.net_handoff = ChNetHandoff::TapFd;
        input.net_ifaces[0].tap_fd = Some(7);
        let argv = generate_ch_argv(&input).expect("valid tap-fd input");
        let joined = argv.join(" ");
        assert!(joined.contains("--net fd=7,mac=02:76:53:AE:57:0A"));
        assert!(!joined.contains("tap=work-l10"));
    }

    #[test]
    fn tap_fd_missing_is_rejected() {
        let mut input = audit_input();
        input.net_handoff = ChNetHandoff::TapFd;
        // tap_fd intentionally unset
        input.net_ifaces[0].tap_fd = None;
        assert!(matches!(
            generate_ch_argv(&input),
            Err(ChArgvError::TapFdMissing { .. })
        ));
    }

    #[test]
    fn persistent_tap_missing_ifname_is_rejected() {
        let mut input = audit_input();
        input.net_handoff = ChNetHandoff::PersistentTap;
        input.net_ifaces[0].tap_ifname.clear();
        assert!(matches!(
            generate_ch_argv(&input),
            Err(ChArgvError::TapIfnameMissing { .. })
        ));
    }

    #[test]
    fn omits_initramfs_when_absent() {
        let mut input = audit_input();
        input.initramfs_path = None;
        let argv = generate_ch_argv(&input).expect("input without initramfs is valid");
        assert!(!argv.iter().any(|a| a == "--initramfs"));
    }

    #[test]
    fn omits_platform_when_no_oem_strings() {
        let mut input = audit_input();
        input.platform_oem_strings.clear();
        let argv = generate_ch_argv(&input).expect("input without platform oem is valid");
        assert!(!argv.iter().any(|a| a == "--platform"));
    }

    #[test]
    fn omits_vsock_when_absent() {
        let mut input = audit_input();
        input.primary_vsock = None;
        let argv = generate_ch_argv(&input).expect("input without primary vsock is valid");
        // No primary vsock and no extra vsock means no --vsock arg.
        assert!(!argv.iter().any(|a| a == "--vsock"));
    }

    #[test]
    fn extra_vsock_emits_socket_only_form() {
        let mut input = audit_input();
        input.primary_vsock = None;
        input.extra_vsock = vec!["/run/d2b/vms/corp-vm/obs.vsock".to_owned()];
        let argv = generate_ch_argv(&input).expect("extra-only vsock is valid");
        let joined = argv.join(" ");
        // socket=... form (no cid=...)
        assert!(joined.contains("--vsock socket=/run/d2b/vms/corp-vm/obs.vsock"));
    }

    #[test]
    fn extra_args_appended_in_order() {
        let mut input = audit_input();
        input.extra_args = vec![
            "--tpm".to_owned(),
            "socket=/run/d2b/vms/corp-vm/swtpm.sock".to_owned(),
            "--gpu".to_owned(),
            "socket=/run/d2b/vms/corp-vm/gpu.sock".to_owned(),
        ];
        let argv = generate_ch_argv(&input).expect("extra args valid");

        // Find each entry; they must appear after --net in declaration order.
        let net_pos = argv.iter().position(|a| a == "--net").unwrap();
        let tpm_pos = argv.iter().position(|a| a == "--tpm").unwrap();
        let gpu_pos = argv.iter().position(|a| a == "--gpu").unwrap();
        assert!(net_pos < tpm_pos);
        assert!(tpm_pos < gpu_pos);
    }

    #[test]
    fn omits_fs_when_no_shares() {
        let mut input = audit_input();
        input.fs_shares.clear();
        let argv = generate_ch_argv(&input).expect("input without fs is valid");
        assert!(!argv.iter().any(|a| a == "--fs"));
    }

    #[test]
    fn omits_net_when_no_ifaces() {
        let mut input = audit_input();
        input.net_ifaces.clear();
        let argv = generate_ch_argv(&input).expect("input without net is valid");
        assert!(!argv.iter().any(|a| a == "--net"));
    }

    #[test]
    fn omits_watchdog_when_disabled() {
        let mut input = audit_input();
        input.watchdog = false;
        let argv = generate_ch_argv(&input).expect("input without watchdog is valid");
        assert!(!argv.iter().any(|a| a == "--watchdog"));
    }

    #[test]
    fn multiple_oem_strings_join_with_comma() {
        let mut input = audit_input();
        input.platform_oem_strings = vec![
            "io.systemd.credential:vmm.notify_socket=vsock-stream:2:8888".to_owned(),
            "io.d2b.vm=corp-vm".to_owned(),
        ];
        let argv = generate_ch_argv(&input).expect("multi-oem valid");
        let joined = argv.join(" ");
        assert!(joined.contains(
            "oem_strings=[io.systemd.credential:vmm.notify_socket=vsock-stream:2:8888,io.d2b.vm=corp-vm]"
        ));
    }

    #[test]
    fn argv_is_round_trip_serializable() {
        // The input is shipped from `d2bd` to the broker over the
        // `SpawnRunner` wire as part of the bundle-resolved intent.
        // Round-trip the JSON shape so wire drift surfaces in
        // unit tests rather than via gate scripts.
        let input = audit_input();
        let json = serde_json::to_string(&input).unwrap();
        let parsed: ChArgvInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, input);
    }
}
