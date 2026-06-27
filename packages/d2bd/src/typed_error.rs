use std::path::PathBuf;

/// Closed enum of guest-control config-read failure classes. Each maps to a
/// distinct wire `kind` slug; the daemon never attaches a path, byte, or
/// guest-supplied string to the failure, so the public envelope is leak-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestControlReadErrorKind {
    /// Connect / CONNECT-ACK / handshake transport failure (incl. unreachable,
    /// old-generation listener, broker signer error).
    Transport,
    /// Authenticated handshake rejected (token / nonce / stale session).
    AuthFailed,
    /// Malformed or out-of-contract guest response.
    Protocol,
    /// The guest authenticated but does not advertise `ReadGuestFile`.
    CapabilityUnavailable,
    /// The guest config working copy does not exist.
    FileNotFound,
    /// The guest config exceeds the read cap.
    FileTooLarge,
    /// The resolved guest path was unsafe (symlink / non-regular / `..`).
    PathUnsafe,
    /// The guest denied the read (no path wired, or permission denied).
    ReadDenied,
    /// The probe deadline elapsed before a ready outcome.
    Timeout,
}

impl GuestControlReadErrorKind {
    pub fn wire_kind(self) -> &'static str {
        match self {
            Self::Transport => "guest-control-transport-unavailable",
            Self::AuthFailed => "guest-control-auth-failed",
            Self::Protocol => "guest-control-protocol-error",
            Self::CapabilityUnavailable => "guest-control-capability-unavailable",
            Self::FileNotFound => "guest-control-file-not-found",
            Self::FileTooLarge => "guest-control-file-too-large",
            Self::PathUnsafe => "guest-control-path-unsafe",
            Self::ReadDenied => "guest-control-read-denied",
            Self::Timeout => "guest-control-timeout",
        }
    }

    fn human_message(self) -> &'static str {
        match self {
            Self::Transport => "guest-control transport to the VM is unavailable",
            Self::AuthFailed => "guest-control authentication to the VM failed",
            Self::Protocol => "the guest returned a malformed guest-control response",
            Self::CapabilityUnavailable => {
                "the guest does not advertise the read-guest-file capability"
            }
            Self::FileNotFound => "the guest config working copy does not exist",
            Self::FileTooLarge => "the guest config exceeds the read size cap",
            Self::PathUnsafe => "the guest config path failed the guest-side safety check",
            Self::ReadDenied => "the guest denied the config read",
            Self::Timeout => "the guest-control config read timed out",
        }
    }

    fn remediation(self) -> &'static str {
        match self {
            Self::Transport | Self::Timeout => {
                "confirm the VM is running and guest-control-health is ready (`d2b vm status <vm>`), then retry"
            }
            Self::AuthFailed => {
                "the guest rejected the authenticated handshake; rotate the VM's guest-control material and restart the VM"
            }
            Self::Protocol => {
                "the guest-control protocol versions are skewed; rebuild the guest with a matching d2b generation"
            }
            Self::CapabilityUnavailable => {
                "rebuild the guest with the read-guest-file capability enabled (current d2b generation)"
            }
            Self::FileNotFound => {
                "create the editable guest config working copy inside the VM before syncing"
            }
            Self::FileTooLarge => "shrink the guest config below the read size cap before syncing",
            Self::PathUnsafe => {
                "ensure the guest config path is a regular file with no symlink or parent-escape component"
            }
            Self::ReadDenied => {
                "grant the guest-control reader access to the guest config path inside the VM"
            }
        }
    }
}

/// Closed enum of guest-control **exec** failure classes (establishment, per-op
/// proxy, and session-table reservation). Each maps to a distinct wire `kind`
/// slug and a CLI-meaningful exit code; the daemon never attaches argv, env,
/// output bytes, a session handle, or any guest-supplied string to the failure,
/// so the public envelope is leak-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestControlExecErrorKind {
    /// Connect / handshake transport failure (incl. unreachable, broker signer
    /// error).
    Transport,
    /// Authenticated handshake or per-op auth rejected (token / nonce / stale).
    Auth,
    /// Malformed or out-of-contract guest response, or an internal protocol
    /// violation (offset/control-seq mismatch).
    Protocol,
    /// The per-op or establishment deadline elapsed.
    Timeout,
    /// The guest does not advertise exec at all (old generation / exec-disabled
    /// build). No session, no SSH fallback.
    OldGeneration,
    /// The guest authenticated but does not advertise a required exec
    /// capability (e.g. `EXEC_TTY` for an interactive session).
    Capability,
    /// The guest authenticated but does not advertise detached exec support.
    DetachedUnavailable,
    /// A concurrent-session cap (global / per-uid / per-vm) was hit.
    SessionCapacity,
    /// The per-uid Start rate limit fired.
    RateLimited,
    /// The guest rejected the op because the authenticated session is stale.
    StaleSession,
    /// The requested detached exec id is not present on this guest boot.
    ExecNotFound,
    /// The requested detached exec id has expired or been reaped.
    ExecExpired,
    /// The command failed guest-side argv[0] validation.
    InvalidProgram,
    /// A deterministic guest-side op rejection (exec already exited, etc.).
    GuestError,
    /// Daemon-internal failure establishing or driving the session worker.
    Internal,
}

impl GuestControlExecErrorKind {
    pub fn wire_kind(self) -> &'static str {
        match self {
            Self::Transport => "guest-control-transport-unavailable",
            Self::Auth => "guest-control-auth-failed",
            Self::Protocol => "guest-control-protocol-error",
            Self::Timeout => "guest-control-timeout",
            Self::OldGeneration => "guest-control-unavailable-old-generation",
            Self::Capability => "guest-control-capability-unavailable",
            Self::DetachedUnavailable => "guest-control-exec-detached-unavailable",
            Self::SessionCapacity => "exec-session-capacity",
            Self::RateLimited => "exec-session-rate-limited",
            Self::StaleSession => "guest-control-stale-session",
            Self::ExecNotFound => "guest-control-exec-not-found",
            Self::ExecExpired => "guest-control-exec-expired",
            Self::InvalidProgram => "guest-control-invalid-program",
            Self::GuestError => "guest-control-exec-error",
            Self::Internal => "guest-control-exec-internal",
        }
    }

    /// Exit code surfaced in the public envelope. The CLI applies its own
    /// exec exit-code contract on top of the wire `kind`; these values are the
    /// fallback for a client that does not specialise exec handling.
    fn exit_code(self) -> u8 {
        match self {
            Self::Transport | Self::Timeout => 69,
            Self::OldGeneration | Self::Capability | Self::DetachedUnavailable => 70,
            Self::SessionCapacity | Self::RateLimited => 75,
            Self::Protocol | Self::GuestError | Self::ExecNotFound | Self::ExecExpired => 76,
            Self::InvalidProgram => 2,
            Self::Auth | Self::StaleSession => 77,
            Self::Internal => 42,
        }
    }

    fn human_message(self) -> &'static str {
        match self {
            Self::Transport => "guest-control transport to the VM is unavailable",
            Self::Auth => "guest-control authentication to the VM failed",
            Self::Protocol => "the guest returned a malformed guest-control exec response",
            Self::Timeout => "the guest-control exec operation timed out",
            Self::OldGeneration => "the VM generation does not support guest-control exec",
            Self::Capability => "the guest does not advertise a required exec capability",
            Self::DetachedUnavailable => "detached exec is unavailable for this VM",
            Self::SessionCapacity => "the exec session table is at capacity",
            Self::RateLimited => "exec session starts are rate limited for this caller",
            Self::StaleSession => "the guest-control exec session is stale",
            Self::ExecNotFound => "the requested detached exec was not found",
            Self::ExecExpired => "the requested detached exec has expired",
            Self::InvalidProgram => {
                "vm exec: command must be a program name or absolute path and must not start with '-'"
            }
            Self::GuestError => "the guest rejected the exec operation",
            Self::Internal => "the daemon failed to drive the exec session",
        }
    }

    fn remediation(self) -> &'static str {
        match self {
            Self::Transport | Self::Timeout => {
                "confirm the VM is running and guest-control-health is ready (`d2b vm status <vm>`), then retry"
            }
            Self::Auth => {
                "the guest rejected the authenticated handshake; rotate the VM's guest-control material and restart the VM"
            }
            Self::Protocol => {
                "the guest-control protocol versions are skewed; rebuild the guest with a matching d2b generation"
            }
            Self::OldGeneration => {
                "rebuild and switch the VM to the current d2b generation so guest-control exec is available; d2b does not fall back to SSH"
            }
            Self::Capability => {
                "ensure guest-control exec is enabled on the VM (`guest.exec.enable = true`) and the guest is rebuilt to the current d2b generation; an interactive session additionally requires the guest TTY capability"
            }
            Self::DetachedUnavailable => {
                "rebuild and restart the VM with detached exec support enabled so the guest advertises EXEC_DETACHED"
            }
            Self::SessionCapacity => {
                "wait for an in-flight exec session to finish or close an idle one, then retry"
            }
            Self::RateLimited => "reduce the rate of `d2b vm exec` invocations and retry",
            Self::StaleSession => {
                "retry after confirming the VM is still running; if the failure persists, restart the VM to refresh guest-control session state"
            }
            Self::ExecNotFound => {
                "run `d2b vm exec <vm> list` to find retained detached exec ids, then retry with a listed id"
            }
            Self::ExecExpired => {
                "the detached exec record has aged out of guest retention; start a new detached exec if you still need the command"
            }
            Self::InvalidProgram => {
                "pass the command after `--`; use a non-empty program name, a relative name resolved by the guest login shell, or an absolute path, and avoid leading '-'"
            }
            Self::GuestError => {
                "inspect the guest exec state; the command may have already exited or been cancelled"
            }
            Self::Internal => {
                "retry; if the failure persists inspect the daemon log for the typed exec-session record"
            }
        }
    }
}

/// Closed enum of guest-control **shell** failure classes. The daemon never
/// attaches shell names, session handles, terminal bytes, or guest-supplied
/// strings to this error; the enum is the only public payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestControlShellErrorKind {
    Transport,
    Auth,
    Protocol,
    Timeout,
    Capability,
    StaleSession,
    Capacity,
    AlreadyAttached,
    NotFound,
    OutputGap,
    GuestError,
    Internal,
}

impl GuestControlShellErrorKind {
    pub fn wire_kind(self) -> &'static str {
        match self {
            Self::Transport => "guest-control-shell-transport-unavailable",
            Self::Auth => "guest-control-shell-auth-failed",
            Self::Protocol => "guest-control-shell-protocol-error",
            Self::Timeout => "guest-control-shell-timeout",
            Self::Capability => "guest-control-shell-capability-unavailable",
            Self::StaleSession => "guest-control-shell-stale-session",
            Self::Capacity => "guest-control-shell-capacity",
            Self::AlreadyAttached => "guest-control-shell-already-attached",
            Self::NotFound => "guest-control-shell-not-found",
            Self::OutputGap => "guest-control-shell-output-gap",
            Self::GuestError => "guest-control-shell-error",
            Self::Internal => "guest-control-shell-internal",
        }
    }

    fn exit_code(self) -> u8 {
        match self {
            Self::Transport | Self::Timeout => 69,
            Self::Capability => 70,
            Self::Capacity | Self::AlreadyAttached => 75,
            Self::Protocol | Self::NotFound | Self::OutputGap | Self::GuestError => 76,
            Self::Auth | Self::StaleSession => 77,
            Self::Internal => 42,
        }
    }

    fn human_message(self) -> &'static str {
        match self {
            Self::Transport => "guest-control shell transport to the VM is unavailable",
            Self::Auth => "guest-control shell authentication to the VM failed",
            Self::Protocol => "the guest returned a malformed guest-control shell response",
            Self::Timeout => "the guest-control shell operation timed out",
            Self::Capability => "the guest does not advertise a required shell capability",
            Self::StaleSession => "the guest-control shell session is stale",
            Self::Capacity => "the persistent shell session table is at capacity",
            Self::AlreadyAttached => "the persistent shell is already attached",
            Self::NotFound => "the persistent shell session was not found",
            Self::OutputGap => "the persistent shell output stream has a gap",
            Self::GuestError => "the guest rejected the shell operation",
            Self::Internal => "the daemon failed to drive the shell session",
        }
    }

    fn remediation(self) -> &'static str {
        match self {
            Self::Transport | Self::Timeout => {
                "confirm the VM is running and guest-control-health is ready (`d2b vm status <vm>`), then retry"
            }
            Self::Auth => {
                "the guest rejected the authenticated handshake; rotate the VM's guest-control material and restart the VM"
            }
            Self::Protocol => {
                "the guest-control protocol versions are skewed; rebuild the guest with a matching d2b generation"
            }
            Self::Capability => {
                "enable persistent guest shell support for the VM and rebuild/restart the guest so it advertises shell capabilities"
            }
            Self::StaleSession => {
                "reattach to the shell after confirming the VM is still running; if the failure persists, restart the VM"
            }
            Self::Capacity => "detach or kill an existing persistent shell session, then retry",
            Self::AlreadyAttached => {
                "reattach with the force flag or detach the existing owner first"
            }
            Self::NotFound => {
                "list persistent shell sessions for the VM and retry with a listed name"
            }
            Self::OutputGap => {
                "reattach to redraw the persistent shell; terminal output before the gap is no longer available"
            }
            Self::GuestError => "inspect guestd shell state and retry the shell operation",
            Self::Internal => {
                "retry; if the failure persists inspect the daemon log for the typed shell-session record"
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum TypedError {
    AuthzNotALauncher {
        peer_uid: u32,
    },
    AuthzNotAdmin {
        verb: String,
    },
    AuthzAuditRequiresAdmin,
    InternalAlreadyRunning {
        path: PathBuf,
    },
    InternalBrokerUnavailable {
        path: PathBuf,
        detail: String,
    },
    /// The privileged broker round trip (connect + write + read) did not
    /// complete before the caller's single absolute deadline. Distinct
    /// from [`Self::InternalBrokerUnavailable`] (a fast connect/transport
    /// failure) so a genuine deadline exhaustion can be surfaced as a
    /// timeout end to end — the guest-control signer maps this to
    /// [`crate::guest_control_health::GuestControlHealthError::Timeout`]
    /// (slug `guest-control-timeout`) instead of collapsing it into a
    /// generic signer/transport failure.
    InternalBrokerTimeout {
        path: PathBuf,
    },
    InternalConfig {
        detail: String,
    },
    InternalIo {
        context: String,
        detail: String,
    },
    InternalLockParentInvalid {
        path: PathBuf,
        detail: String,
    },
    GatewayDisplayUnavailable {
        detail: String,
    },
    WireVersionMismatch {
        client_range: String,
        accepted_range: String,
    },
    WireUnknownField {
        detail: String,
    },
    WireIfNameInvalid {
        detail: String,
    },
    WireFrameTooLarge {
        declared: usize,
    },
    WireInvalidFrame {
        detail: String,
    },
    WireBadHello {
        detail: String,
    },
    WireUnsupportedRequest {
        request_type: String,
    },
    GuestShellDisabled,
    BundleTampered {
        path: PathBuf,
        reason: String,
    },
    /// Refusal raised by the VM-start preflight when a per-VM state
    /// subdirectory has drifted from
    /// the typed ownership matrix declared in
    /// `nixos-modules/options-ownership-matrix.nix`. `path` points
    /// at the first drifted entry; `drift_reason` is the full
    /// operator-facing summary built by
    /// `ownership_preflight::render_drift_message`.
    OwnershipMatrixDrift {
        vm: String,
        path: PathBuf,
        drift_reason: String,
    },
    /// Refusal raised by the VM-start preflight when
    /// `/var/lib/d2b/vms/<vm>/sshd-host-keys`
    /// or one of its `ssh_host_*_key` leaves has drifted from the
    /// canonical posture (directory mode/owner enforced separately by
    /// `ownership_preflight`; each key file must be a non-symlink
    /// regular file owned `root:root` with mode `0o0400`).
    /// `drift` is the short, single-line reason rendered by
    /// [`crate::ssh_host_key_preflight::SshdHostKeyDrift::reason`].
    SshdHostKeyDrift {
        vm: String,
        path: PathBuf,
        drift: String,
    },
    /// Refusal raised by the VM-start preflight for `sys-<env>-net` VMs
    /// when the on-disk
    /// dnsmasq.conf hash diverges from the bundle's expectation.
    /// `env` is the env scope (e.g. `corp`, `personal`, `obs`);
    /// `expected` and `actual` are 64-char lowercase SHA-256 hex
    /// digests. The mismatch indicates the bundle was updated but
    /// the dnsmasq render step did not rerun — rebuild the bundle
    /// (or re-run the host singleton that renders dnsmasq.conf)
    /// and retry.
    BundleDnsmasqDrift {
        vm: String,
        env: String,
        path: PathBuf,
        expected: String,
        actual: String,
        reason: String,
    },
    /// Daemon refusal raised on startup when one or more REQUIRED kernel
    /// modules (see
    /// [`crate::kernel_module_check`]) are neither loaded into
    /// `/proc/modules` nor detected built-in. `missing` is the
    /// stable, comma-separated list of module names (KVM
    /// alternatives rendered as `kvm_intel|kvm_amd`).
    HostKernelModulesMissing {
        missing: String,
    },
    /// Typed annotation raised when the broker-spawned
    /// `RunnerRole::OtelHostBridge`
    /// runner did not satisfy the readiness gate
    /// (pidfd registration + obs vsock host socket present, the
    /// proxy for "socket accept succeeded + first OTLP forward
    /// acknowledged") before the configured deadline.
    ///
    /// Default behaviour on hit: VM-start still succeeds with a
    /// degraded-mode annotation so the obs VM itself is left
    /// running; the typed error is only surfaced as the public
    /// envelope when `D2B_OTEL_BRIDGE_READINESS_STRICT=1`. See
    /// `docs/reference/otel-host-bridge-readiness.md`.
    OtelHostBridgeReadinessTimeout {
        vm: String,
        elapsed_ms: u128,
    },
    /// Returned when a caller needs to surface persisted net-route
    /// preflight degradation. Read-only verbs (`status`,
    /// `host doctor --read-only`, `audit`) stay available. Focused
    /// recovery is `d2b host reconcile --network --apply`.
    NetRoutePreflightDegraded {
        consecutive_failures: u32,
        failed_envs: Vec<String>,
    },
    /// Returned by the per-busid USBIP state machine when any step in
    /// the canonical order
    /// (`modprobe → lock → withhold → firewall → backend → bind →
    /// proxy`) fails. `busid` is the host-side bus identifier
    /// (e.g. `1-2`); `step` is the typed
    /// [`crate::usbip_state_machine::UsbipBusidStep`] that blew
    /// up; `reason` is the short executor-supplied detail.
    /// Carries exit code 67 so operators can correlate the
    /// failure to the per-busid bring-up path (not the broader
    /// kernel-module / route-degraded surfaces).
    UsbipStepFailed {
        busid: String,
        step: crate::usbip_state_machine::UsbipBusidStep,
        reason: String,
    },
    /// Explicit-attach: the requested busid is not present in
    /// `/sys/bus/usb/devices/`. Sysfs presence is the fail-closed gate
    /// for explicit attach; absent busids are rejected before any broker
    /// call or firewall mutation. Exit code 67 (same as other USB errors).
    UsbipBusidNotPresent {
        busid: String,
        verb: String,
    },
    /// Explicit-attach: another active daemon claim already holds
    /// the exclusive OFD lock for the requested busid. The daemon
    /// reads the lock file before any broker call and rejects the
    /// conflicting explicit attach fail-closed. Exit code 67.
    UsbipExplicitClaimConflict {
        busid: String,
        owner_vm: String,
        verb: String,
    },
    /// Returned by USB detach/revocation when the daemon cannot prove an exact
    /// VM→proxy flow tuple for the target busid that can be terminated with
    /// targeted conntrack deletion and/or TCP socket kill. The daemon must
    /// refuse instead of killing the shared per-env proxy listener and
    /// disrupting unrelated same-env streams.
    UsbipRevocationNotIsolated {
        vm: String,
        busid: String,
        reason: crate::usbip_reconcile_state::UsbipRevocationFlowFailure,
    },
    /// The selected VM runtime does not implement a capability required by the
    /// requested verb. Payload is limited to public VM/runtime/capability names;
    /// no guest data, argv, paths, or registry identities are included.
    RuntimeCapabilityUnsupported {
        vm: String,
        runtime_kind: String,
        capability: String,
        verb: String,
    },
    /// Authenticated guest-control config read failed. The closed-enum `kind`
    /// is the ONLY payload — never a path, byte, or guest-supplied string — so
    /// the public envelope cannot leak guest content.
    GuestControlReadFailed {
        kind: GuestControlReadErrorKind,
    },
    /// Authenticated guest-control **exec** failed (establishment, per-op proxy,
    /// or session-table reservation). The closed-enum `kind` is the ONLY
    /// payload — never argv, env, output, a session handle, or a guest string.
    GuestControlExecFailed {
        kind: GuestControlExecErrorKind,
    },
    /// Authenticated guest-control **shell** failed. Closed-enum kind only; no
    /// shell name, session handle, terminal bytes, or guest string.
    GuestControlShellFailed {
        kind: GuestControlShellErrorKind,
    },
    /// The daemon refused a new connection because the bounded in-flight
    /// connection-handler pool is saturated. Returned immediately
    /// (non-blocking) from the accept path so a burst of clients cannot
    /// stall the accept loop. Transient: the caller should retry shortly.
    DaemonBusy,
}

/// Classify the detail string for a lock-parent validation failure into
/// a deterministic, path-free public description.
fn redacted_lock_parent_reason(detail: &str) -> &'static str {
    if detail.contains("symlink") {
        "parent directory must not be a symlink"
    } else if detail.contains("not a directory") {
        "parent path is not a directory"
    } else if detail.contains("no parent") {
        "lock path has no parent directory"
    } else if detail.contains("uid") || detail.contains("gid") || detail.contains("mode") {
        "parent directory ownership or mode is incorrect"
    } else {
        "lock parent validation failed"
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorEnvelope {
    pub kind: String,
    pub exit_code: u8,
    pub message: String,
    pub remediation: String,
}

fn redact_path_like_tokens(detail: &str) -> String {
    detail
        .split_whitespace()
        .map(|token| if token.contains('/') { "<path>" } else { token })
        .collect::<Vec<_>>()
        .join(" ")
}

fn public_usb_busid(busid: &str) -> &str {
    if d2b_contracts::usbip::validate_bus_id(busid).is_ok() {
        busid
    } else {
        "<invalid-busid>"
    }
}

impl TypedError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::AuthzNotALauncher { .. } => "authz-not-a-launcher",
            Self::AuthzNotAdmin { .. } => "authz-not-admin",
            Self::AuthzAuditRequiresAdmin => "authz-audit-requires-admin",
            Self::InternalAlreadyRunning { .. } => "internal-already-running",
            Self::InternalBrokerUnavailable { .. } => "internal-broker-unavailable",
            Self::InternalBrokerTimeout { .. } => "internal-broker-timeout",
            Self::InternalConfig { .. } => "internal-config-invalid",
            Self::InternalIo { .. } => "internal-io",
            Self::InternalLockParentInvalid { .. } => "internal-lock-parent-invalid",
            Self::GatewayDisplayUnavailable { .. } => "gateway-display-unavailable",
            Self::WireVersionMismatch { .. } => "wire-version-mismatch",
            Self::WireUnknownField { .. } => "wire-unknown-field",
            Self::WireIfNameInvalid { .. } => "wire-ifname-invalid",
            Self::WireFrameTooLarge { .. } => "wire-frame-too-large",
            Self::WireInvalidFrame { .. } => "wire-invalid-frame",
            Self::WireBadHello { .. } => "wire-bad-hello",
            Self::WireUnsupportedRequest { .. } => "wire-unsupported-request",
            Self::GuestShellDisabled => "guest-shell-disabled",
            Self::BundleTampered { .. } => "bundle-tampered",
            Self::OwnershipMatrixDrift { .. } => "ownership-matrix-drift",
            Self::SshdHostKeyDrift { .. } => "sshd-host-key-drift",
            Self::BundleDnsmasqDrift { .. } => "bundle-dnsmasq-drift",
            Self::HostKernelModulesMissing { .. } => "host-kernel-modules-missing",
            Self::OtelHostBridgeReadinessTimeout { .. } => "otel-host-bridge-readiness-timeout",
            Self::NetRoutePreflightDegraded { .. } => "net-route-preflight-degraded",
            Self::UsbipStepFailed { .. } => "usbip-step-failed",
            Self::UsbipBusidNotPresent { .. } => "usbip-busid-not-present",
            Self::UsbipExplicitClaimConflict { .. } => "usbip-explicit-claim-conflict",
            Self::UsbipRevocationNotIsolated { .. } => "usbip-revocation-not-isolated",
            Self::RuntimeCapabilityUnsupported { .. } => "runtime-capability-unsupported",
            Self::GuestControlReadFailed { kind } => kind.wire_kind(),
            Self::GuestControlExecFailed { kind } => kind.wire_kind(),
            Self::GuestControlShellFailed { kind } => kind.wire_kind(),
            Self::DaemonBusy => "daemon-busy",
        }
    }

    pub fn exit_code(&self) -> u8 {
        match self {
            Self::AuthzNotALauncher { .. } => 31,
            Self::AuthzNotAdmin { .. } => 75,
            Self::AuthzAuditRequiresAdmin => 32,
            Self::InternalAlreadyRunning { .. } => 41,
            Self::InternalBrokerUnavailable { .. }
            | Self::InternalBrokerTimeout { .. }
            | Self::InternalConfig { .. }
            | Self::InternalIo { .. }
            | Self::InternalLockParentInvalid { .. }
            | Self::GatewayDisplayUnavailable { .. } => 42,
            Self::WireUnknownField { .. } => 51,
            Self::WireVersionMismatch { .. } => 52,
            Self::WireIfNameInvalid { .. } => 53,
            Self::WireFrameTooLarge { .. }
            | Self::WireInvalidFrame { .. }
            | Self::WireBadHello { .. }
            | Self::WireUnsupportedRequest { .. } => 54,
            Self::GuestShellDisabled => 70,
            Self::BundleTampered { .. } => 60,
            Self::OwnershipMatrixDrift { .. } => 61,
            Self::SshdHostKeyDrift { .. } => 62,
            Self::BundleDnsmasqDrift { .. } => 63,
            Self::HostKernelModulesMissing { .. } => 64,
            Self::OtelHostBridgeReadinessTimeout { .. } => 65,
            // Net-route degraded mode shares the kind class with
            // otel-host-bridge-readiness (operator-only mode) but
            // gets its own exit code 66 so operators can correlate the
            // failure to the network preflight (not the obs bridge).
            Self::NetRoutePreflightDegraded { .. } => 66,
            // Per-busid USBIP bring-up failure. Distinct exit code so
            // operators can grep for
            // it across hosts independently of the kernel-module
            // (64), otel-bridge (65), or net-route-degraded (66)
            // adjacent surfaces.
            Self::UsbipStepFailed { .. } => 67,
            // Explicit-attach pre-flight rejections use exit code 67
            // (same class as other USB errors) so operators can correlate them
            // to the USBIP surface.
            Self::UsbipBusidNotPresent { .. } => 67,
            Self::UsbipExplicitClaimConflict { .. } => 67,
            Self::UsbipRevocationNotIsolated { .. } => 67,
            Self::RuntimeCapabilityUnsupported { .. } => 70,
            // Guest-control config read failures share one exit code; the
            // distinct `kind` slug carries the sub-class.
            Self::GuestControlReadFailed { .. } => 70,
            Self::GuestControlExecFailed { kind } => kind.exit_code(),
            Self::GuestControlShellFailed { kind } => kind.exit_code(),
            // Shares the EX_TEMPFAIL-class exit code with the other
            // transient back-pressure refusals (session-capacity,
            // rate-limited): a retry may succeed.
            Self::DaemonBusy => 75,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::AuthzNotALauncher { peer_uid } => {
                format!("peer uid {peer_uid} is not in d2b.site.launcherUsers")
            }
            Self::AuthzNotAdmin { verb } => {
                format!("{verb} requires an admin role from d2b.site.adminUsers")
            }
            Self::AuthzAuditRequiresAdmin => {
                "audit requires an admin role from d2b.site.adminUsers".to_owned()
            }
            Self::InternalAlreadyRunning { .. } => "daemon lock is already held".to_owned(),
            Self::InternalBrokerUnavailable { .. } => {
                "could not reach the privileged broker socket".to_owned()
            }
            Self::InternalBrokerTimeout { .. } => {
                "the privileged broker round trip exceeded its deadline".to_owned()
            }
            Self::InternalConfig { .. } => "invalid daemon configuration".to_owned(),
            Self::InternalIo { .. } => "internal I/O failure".to_owned(),
            Self::InternalLockParentInvalid { detail, .. } => {
                format!(
                    "lock parent failed validation: {}",
                    redacted_lock_parent_reason(detail)
                )
            }
            Self::GatewayDisplayUnavailable { detail } => {
                format!(
                    "gateway display unavailable: {}",
                    redact_path_like_tokens(detail)
                )
            }
            Self::WireVersionMismatch {
                client_range,
                accepted_range,
            } => format!(
                "client version range {client_range} does not match server range {accepted_range}"
            ),
            Self::WireUnknownField { detail } => {
                format!("request contains an unknown field: {detail}")
            }
            Self::WireIfNameInvalid { .. } => "invalid network interface name".to_owned(),
            Self::WireFrameTooLarge { declared } => {
                format!("frame length {declared} exceeds the 1 MiB limit")
            }
            Self::WireInvalidFrame { detail } => format!("invalid frame: {detail}"),
            Self::WireBadHello { detail } => format!("invalid hello handshake: {detail}"),
            Self::WireUnsupportedRequest { request_type } => {
                format!("unsupported request type {request_type}")
            }
            Self::GuestShellDisabled => {
                "persistent guest shell is not available for this VM".to_owned()
            }
            Self::BundleTampered { reason, .. } => {
                format!("bundle tamper detected: {reason}")
            }
            Self::OwnershipMatrixDrift { drift_reason, .. } => drift_reason.clone(),
            Self::SshdHostKeyDrift { vm, drift, .. } => {
                format!("vm '{vm}' refused: sshd host key drift: {drift}")
            }
            Self::BundleDnsmasqDrift { vm, reason, .. } => {
                format!("vm '{vm}' refused: {reason}")
            }
            Self::HostKernelModulesMissing { missing } => {
                format!("daemon refused to start: required kernel modules not loaded: {missing}")
            }
            Self::NetRoutePreflightDegraded {
                consecutive_failures,
                failed_envs,
            } => {
                let envs = if failed_envs.is_empty() {
                    "(no env-specific data)".to_owned()
                } else {
                    failed_envs.join(", ")
                };
                format!(
                    "net-route preflight has {consecutive_failures} consecutive failures (envs: {envs}); run network reconcile or inspect bridge state"
                )
            }
            Self::OtelHostBridgeReadinessTimeout { vm, elapsed_ms } => {
                format!(
                    "vm '{vm}' started but OtelHostBridge readiness gate did not close in {elapsed_ms}ms; observability is degraded"
                )
            }
            Self::UsbipStepFailed {
                busid,
                step,
                reason,
            } => {
                let busid = public_usb_busid(busid);
                let kind = crate::usbip_reconcile_state::classify_usbip_step_failure(
                    step.as_str(),
                    reason,
                );
                format!(
                    "usbip busid '{busid}' refused at step '{step}': {}",
                    kind.summary()
                )
            }
            Self::UsbipBusidNotPresent { busid, verb } => {
                let busid = public_usb_busid(busid);
                format!(
                    "{verb}: busid '{busid}' is not present in sysfs; device is absent or not plugged in"
                )
            }
            Self::UsbipExplicitClaimConflict {
                busid,
                owner_vm,
                verb,
            } => {
                let busid = public_usb_busid(busid);
                let owner_vm = public_usb_busid(owner_vm);
                format!(
                    "{verb}: busid '{busid}' is already claimed by vm '{owner_vm}'; detach it first"
                )
            }
            Self::UsbipRevocationNotIsolated { vm, busid, reason } => {
                let busid = public_usb_busid(busid);
                format!(
                    "usb detach for vm '{vm}' busid '{busid}' refused: {}",
                    reason.summary()
                )
            }
            Self::RuntimeCapabilityUnsupported {
                vm,
                runtime_kind,
                capability,
                verb,
            } => {
                format!(
                    "vm '{vm}' uses runtime '{runtime_kind}', which does not support capability '{capability}' required by '{verb}'"
                )
            }
            Self::GuestControlReadFailed { kind } => kind.human_message().to_owned(),
            Self::GuestControlExecFailed { kind } => kind.human_message().to_owned(),
            Self::GuestControlShellFailed { kind } => kind.human_message().to_owned(),
            Self::DaemonBusy => "the daemon is at its in-flight connection limit".to_owned(),
        }
    }

    pub fn remediation(&self) -> String {
        match self {
            Self::AuthzNotALauncher { .. } => {
                "add the caller to d2b.site.launcherUsers or connect with an allowed launcher user"
                    .to_owned()
            }
            Self::AuthzNotAdmin { verb } => {
                format!("add the caller to d2b.site.adminUsers to use {verb}")
            }
            Self::AuthzAuditRequiresAdmin => {
                "add the caller to d2b.site.adminUsers to use audit".to_owned()
            }
            Self::InternalAlreadyRunning { .. } => {
                "stop the existing d2bd instance or remove the stale OFD lock holder"
                    .to_owned()
            }
            Self::InternalBrokerUnavailable { .. } => {
                "start d2b-priv-broker or disable audit requests until the broker is available"
                    .to_owned()
            }
            Self::InternalBrokerTimeout { .. } => {
                "check that d2b-priv-broker is responsive (not backlogged or half-open) and retry; the round trip exceeded the caller's deadline"
                    .to_owned()
            }
            Self::InternalConfig { .. } => {
                "fix /etc/d2b/daemon-config.json or pass an explicit test config"
                    .to_owned()
            }
            Self::InternalIo { .. } | Self::InternalLockParentInvalid { .. } => {
                "repair the daemon runtime directory ownership, mode, or symlink posture and retry"
                    .to_owned()
            }
            Self::GatewayDisplayUnavailable { detail } => {
                let redacted = redact_path_like_tokens(detail);
                if detail.contains("host-held gateway credentials") {
                    "enroll inside gateway then retry".to_owned()
                } else if detail.contains("mode 0600") {
                    format!("repair the operator Waypipe receiver socket permissions and retry: {redacted}")
                } else if detail.contains("waypipeSocket") {
                    format!("repair d2b.gateways.<realm>.display.waypipeSocket and retry: {redacted}")
                } else {
                    format!("repair the gateway display configuration and retry: {redacted}")
                }
            }
            Self::WireVersionMismatch { .. } => {
                "use a client whose SemverRange includes the daemon's selected version"
                    .to_owned()
            }
            Self::WireUnknownField { .. } => {
                "remove unknown fields; daemon request decoding is deny_unknown_fields"
                    .to_owned()
            }
            Self::WireIfNameInvalid { .. } => {
                "send an interface name shorter than IFNAMSIZ and matching [A-Za-z0-9_-]+"
                    .to_owned()
            }
            Self::WireFrameTooLarge { .. }
            | Self::WireInvalidFrame { .. }
            | Self::WireBadHello { .. }
            | Self::WireUnsupportedRequest { .. } => {
                "resend a valid framed JSON request that matches the documented daemon wire shape"
                    .to_owned()
            }
            Self::GuestShellDisabled => {
                "enable d2b.vms.<vm>.guest.shell when the shell runtime is available, rebuild the guest, and retry"
                    .to_owned()
            }
            Self::BundleTampered { .. } => {
                "rebuild the bundle from a trusted source (nixos-rebuild switch) and verify ownership root:d2bd 0640; refuse to run mutating verbs until the bundle is restored".to_owned()
            }
            Self::OwnershipMatrixDrift { .. } => {
                "reconcile per-VM state ownership against d2b.daemon.perVmStateOwnershipMatrix; see docs/reference/per-vm-state-ownership.md. Recovery: nixos-rebuild switch (re-runs the host-activation chown), or manually chown/chmod the listed entries. NEVER run a recursive ownership/ACL op across /var/lib/d2b/vms/<vm>/store/ — its inodes are shared with /nix/store via the hardlink farm.".to_owned()
            }
            Self::SshdHostKeyDrift { .. } => {
                "regenerate or chown/chmod the per-VM sshd host keys so each ssh_host_*_key under /var/lib/d2b/vms/<vm>/sshd-host-keys is a regular file owned root:root with mode 0400 (no symlinks); see docs/reference/ssh-host-key-preflight.md. Recovery: nixos-rebuild switch (re-runs the host-activation key sync), or remove the offending key and let d2b keys rotate <vm> reprovision it.".to_owned()
            }
            Self::BundleDnsmasqDrift { .. } => {
                "re-render the per-env dnsmasq.conf so it matches the trusted bundle's hosts_intent + route_intent + nft_intent, then retry the net VM start. Recovery: nixos-rebuild switch (re-runs the dnsmasq render host singleton) and verify the file at /var/lib/d2b/dnsmasq/<env>.conf is owned by the daemon and matches the bundle. See docs/reference/net-vm-bundle-gate.md.".to_owned()
            }
            Self::HostKernelModulesMissing { .. } => {
                "load the listed kernel modules with `modprobe <name>` (or via `boot.kernelModules` in the NixOS host config) and restart d2bd. KVM alternatives display as `kvm_intel|kvm_amd` — load whichever matches the host CPU. See docs/reference/kernel-module-check.md for the full required-vs-optional matrix and per-feature remediation.".to_owned()
            }
            Self::OtelHostBridgeReadinessTimeout { .. } => {
                "check that the OtelHostBridge runner is healthy: `d2b host doctor` reports its pidfd liveness and last-relay-flush timestamp. If the runner is missing, the broker SpawnRunner for `RunnerRole::OtelHostBridge` failed — inspect the broker audit log. If the vsock host socket does not exist, the obs VM cannot accept OTLP from workload VMs; restart the obs VM. To raise the deadline set `D2B_OTEL_BRIDGE_READINESS_TIMEOUT_MS=<ms>`; to fail-closed instead of degrading set `D2B_OTEL_BRIDGE_READINESS_STRICT=1`. See docs/reference/otel-host-bridge-readiness.md.".to_owned()
            }
            Self::NetRoutePreflightDegraded { .. } => {
                "`d2b host reconcile --network --apply` re-runs the per-env nftables / route / sysctl reconcile through the broker without starting any VM and clears the net-route preflight history on success. Read-only verbs (`status`, `host doctor --read-only`, `audit`) remain available. See docs/explanation/host-prepare.md § \"Net-route preflight & network reconcile\".".to_owned()
            }
            Self::UsbipStepFailed {
                busid,
                step,
                reason,
            } => {
                let busid = public_usb_busid(busid);
                let kind = crate::usbip_reconcile_state::classify_usbip_step_failure(
                    step.as_str(),
                    reason,
                );
                format!(
                    "For busid '{busid}', {}. Run `d2b usb probe`, fix the reported USB posture, then retry the lifecycle verb.",
                    kind.remediation()
                )
            }
            Self::UsbipBusidNotPresent { busid, .. } => {
                let busid = public_usb_busid(busid);
                format!(
                    "Plug in the USB device at busid '{busid}', verify it appears in `lsusb`, then retry the attach. Use `d2b usb probe` to inspect current device availability."
                )
            }
            Self::UsbipExplicitClaimConflict {
                busid, owner_vm, ..
            } => {
                let busid = public_usb_busid(busid);
                let owner_vm = public_usb_busid(owner_vm);
                format!(
                    "Run `d2b usb detach {owner_vm} {busid} --apply` to release the existing claim, then retry the attach."
                )
            }
            Self::UsbipRevocationNotIsolated { vm, busid, reason } => {
                let busid = public_usb_busid(busid);
                format!(
                    "For busid '{busid}', {}. Then retry `d2b usb detach {vm} {busid} --apply`.",
                    reason.remediation()
                )
            }
            Self::RuntimeCapabilityUnsupported {
                runtime_kind,
                capability,
                ..
            } => {
                format!(
                    "use a VM/runtime that supports '{capability}', or use the qemu-media lifecycle/media verbs that are available for runtime '{runtime_kind}'"
                )
            }
            Self::GuestControlReadFailed { kind } => kind.remediation().to_owned(),
            Self::GuestControlExecFailed { kind } => kind.remediation().to_owned(),
            Self::GuestControlShellFailed { kind } => kind.remediation().to_owned(),
            Self::DaemonBusy => {
                "the daemon is briefly at capacity; retry the command shortly".to_owned()
            }
        }
    }

    pub fn to_envelope(&self) -> ErrorEnvelope {
        self.log_raw_detail();
        ErrorEnvelope {
            kind: self.kind().to_owned(),
            exit_code: self.exit_code(),
            message: self.message(),
            remediation: self.remediation(),
        }
    }

    /// Convenience for dispatcher call sites that already return
    /// `Result<serde_json::Value, TypedError>`. Renders the envelope
    /// as a JSON object so the caller can return `Ok(value)` for a
    /// fail-closed-but-handled refusal.
    pub fn to_envelope_value(&self) -> serde_json::Value {
        serde_json::to_value(self.to_envelope())
            .unwrap_or_else(|_| serde_json::json!({"kind": self.kind()}))
    }

    /// Log the full unredacted detail (paths, errno strings, raw config
    /// text) so operators can debug from daemon logs.  The public
    /// envelope returned to clients intentionally omits this context.
    fn log_raw_detail(&self) {
        match self {
            Self::InternalAlreadyRunning { path } => {
                tracing::error!(
                    kind = self.kind(),
                    path = %path.display(),
                    "daemon lock is already held"
                );
            }
            Self::InternalBrokerUnavailable { path, detail } => {
                tracing::error!(
                    kind = self.kind(),
                    path = %path.display(),
                    detail = %detail,
                    "could not reach broker socket"
                );
            }
            Self::InternalBrokerTimeout { path } => {
                tracing::error!(
                    kind = self.kind(),
                    path = %path.display(),
                    "broker round trip exceeded its deadline"
                );
            }
            Self::InternalConfig { detail } => {
                tracing::error!(
                    kind = self.kind(),
                    detail = %detail,
                    "invalid daemon configuration"
                );
            }
            Self::InternalIo { context, detail } => {
                tracing::error!(
                    kind = self.kind(),
                    context = %context,
                    detail = %detail,
                    "internal I/O failure"
                );
            }
            Self::InternalLockParentInvalid { path, detail } => {
                tracing::error!(
                    kind = self.kind(),
                    path = %path.display(),
                    detail = %detail,
                    "lock parent failed validation"
                );
            }
            Self::WireIfNameInvalid { detail } => {
                tracing::warn!(
                    kind = self.kind(),
                    detail = %detail,
                    "invalid network interface name"
                );
            }
            Self::BundleTampered { path, reason } => {
                tracing::error!(
                    kind = self.kind(),
                    path = %path.display(),
                    reason = %reason,
                    "bundle tamper-resistance check failed"
                );
            }
            Self::UsbipStepFailed {
                busid,
                step,
                reason,
            } => {
                let busid = public_usb_busid(busid);
                let reason_kind = crate::usbip_reconcile_state::classify_usbip_step_failure(
                    step.as_str(),
                    reason,
                );
                tracing::warn!(
                    kind = self.kind(),
                    busid = %busid,
                    step = %step,
                    reason_kind = reason_kind.telemetry_label(),
                    "usbip step failed"
                );
            }
            Self::UsbipBusidNotPresent { busid, verb } => {
                let busid = public_usb_busid(busid);
                tracing::warn!(
                    kind = self.kind(),
                    busid = %busid,
                    verb = %verb,
                    "usbip explicit attach rejected: busid not present in sysfs"
                );
            }
            Self::UsbipExplicitClaimConflict {
                busid,
                owner_vm,
                verb,
            } => {
                let busid = public_usb_busid(busid);
                let owner_vm = public_usb_busid(owner_vm);
                tracing::warn!(
                    kind = self.kind(),
                    busid = %busid,
                    owner_vm = %owner_vm,
                    verb = %verb,
                    "usbip explicit attach rejected: active claim conflict"
                );
            }
            Self::UsbipRevocationNotIsolated { vm, busid, reason } => {
                let busid = public_usb_busid(busid);
                tracing::warn!(
                    kind = self.kind(),
                    vm = %vm,
                    busid = %busid,
                    reason = reason.telemetry_label(),
                    "usbip revocation refused because selected stream is not isolated"
                );
            }
            // Remaining variants already carry only safe values in
            // their public messages (UIDs, version ranges, frame
            // sizes, field names) — no extra logging needed.
            _ => {}
        }
    }

    pub fn hello_rejected_reason(&self) -> &'static str {
        match self {
            Self::WireVersionMismatch { .. } => "versionMismatch",
            Self::WireUnknownField { .. }
            | Self::WireIfNameInvalid { .. }
            | Self::WireFrameTooLarge { .. }
            | Self::WireInvalidFrame { .. }
            | Self::WireBadHello { .. }
            | Self::WireUnsupportedRequest { .. }
            | Self::GuestShellDisabled => "internalError",
            Self::AuthzNotALauncher { .. }
            | Self::AuthzNotAdmin { .. }
            | Self::AuthzAuditRequiresAdmin
            | Self::InternalAlreadyRunning { .. }
            | Self::InternalBrokerUnavailable { .. }
            | Self::InternalBrokerTimeout { .. }
            | Self::InternalConfig { .. }
            | Self::InternalIo { .. }
            | Self::InternalLockParentInvalid { .. }
            | Self::GatewayDisplayUnavailable { .. }
            | Self::BundleTampered { .. }
            | Self::OwnershipMatrixDrift { .. }
            | Self::SshdHostKeyDrift { .. }
            | Self::BundleDnsmasqDrift { .. }
            | Self::HostKernelModulesMissing { .. }
            | Self::OtelHostBridgeReadinessTimeout { .. }
            | Self::NetRoutePreflightDegraded { .. }
            | Self::UsbipStepFailed { .. }
            | Self::UsbipBusidNotPresent { .. }
            | Self::UsbipExplicitClaimConflict { .. }
            | Self::UsbipRevocationNotIsolated { .. }
            | Self::RuntimeCapabilityUnsupported { .. }
            | Self::GuestControlReadFailed { .. }
            | Self::GuestControlExecFailed { .. }
            | Self::GuestControlShellFailed { .. }
            | Self::DaemonBusy => "internalError",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;
    use std::path::PathBuf;

    /// A path-like substring: at least two slash-separated segments.
    fn path_regex() -> Regex {
        Regex::new(r"(/[a-zA-Z][a-zA-Z0-9_.-]*){2,}").unwrap()
    }

    fn assert_no_path_leak(variant_name: &str, message: &str) {
        let re = path_regex();
        assert!(
            !re.is_match(message),
            "{variant_name}: public message leaks a host path: {message:?}"
        );
    }

    #[test]
    fn internal_already_running_redacted() {
        let err = TypedError::InternalAlreadyRunning {
            path: PathBuf::from("/run/d2b/daemon.lock"),
        };
        assert_eq!(err.kind(), "internal-already-running");
        assert_no_path_leak("InternalAlreadyRunning", &err.message());
    }

    #[test]
    fn internal_broker_unavailable_redacted() {
        let err = TypedError::InternalBrokerUnavailable {
            path: PathBuf::from("/run/d2b/priv.sock"),
            detail: "Connection refused (os error 111)".to_owned(),
        };
        assert_eq!(err.kind(), "internal-broker-unavailable");
        assert_no_path_leak("InternalBrokerUnavailable", &err.message());
    }

    #[test]
    fn internal_config_redacted() {
        let err = TypedError::InternalConfig {
            detail: "/etc/d2b/daemon-config.json: missing field `serverVersion`".to_owned(),
        };
        assert_eq!(err.kind(), "internal-config-invalid");
        assert_no_path_leak("InternalConfig", &err.message());
    }

    #[test]
    fn internal_io_redacted() {
        let err = TypedError::InternalIo {
            context: format!("read {}", "/home/paydro/secrets/key.pem"),
            detail: "No such file or directory (os error 2)".to_owned(),
        };
        assert_eq!(err.kind(), "internal-io");
        assert_no_path_leak("InternalIo", &err.message());
    }

    #[test]
    fn internal_lock_parent_invalid_redacted() {
        let err = TypedError::InternalLockParentInvalid {
            path: PathBuf::from("/tmp/d2b-test/fake-parent"),
            detail: "parent directory must not be a symlink".to_owned(),
        };
        assert_eq!(err.kind(), "internal-lock-parent-invalid");
        assert_no_path_leak("InternalLockParentInvalid", &err.message());
    }

    #[test]
    fn gateway_display_unavailable_preserves_actionable_detail_without_paths() {
        let err = TypedError::GatewayDisplayUnavailable {
            detail: "waypipeSocket /run/user/1000/wpc.sock must have mode 0600; current mode is 0o660; run chmod 0600 on the socket path".to_owned(),
        };
        assert_eq!(err.kind(), "gateway-display-unavailable");
        assert!(err.message().contains("waypipeSocket"));
        assert!(err.message().contains("mode 0600"));
        assert!(err.remediation().contains("mode 0600"));
        assert_no_path_leak("GatewayDisplayUnavailable", &err.message());
        assert_no_path_leak("GatewayDisplayUnavailable", &err.remediation());
    }

    #[test]
    fn guest_shell_disabled_is_typed_and_actionable() {
        let err = TypedError::GuestShellDisabled;
        assert_eq!(err.kind(), "guest-shell-disabled");
        assert_eq!(err.exit_code(), 70);
        assert!(err.message().contains("persistent guest shell"));
        assert!(err.remediation().contains("guest.shell"));
        assert_no_path_leak("GuestShellDisabled", &err.message());
        assert_no_path_leak("GuestShellDisabled", &err.remediation());
    }

    #[test]
    fn usbip_step_failed_public_envelope_redacts_sensitive_values() {
        let err = TypedError::UsbipStepFailed {
            busid: "1-2.4".to_owned(),
            step: crate::usbip_state_machine::UsbipBusidStep::Bind,
            reason: "raw stderr for /sys/devices/pci0000:00/usb1/1-2/1-2.4 traceparent=0123456789abcdef0123456789abcdef timed out".to_owned(),
        };

        let envelope = err.to_envelope();
        assert_eq!(envelope.kind, "usbip-step-failed");
        assert_eq!(envelope.exit_code, 67);
        assert!(envelope.message.contains("1-2.4"));
        assert!(envelope.remediation.contains("1-2.4"));
        assert!(envelope.message.contains("bind"));
        assert!(envelope.message.contains("timed out"));
        assert!(envelope.remediation.contains("d2b usb probe"));
        for surface in [&envelope.message, &envelope.remediation] {
            for forbidden in [
                "/sys/",
                "stderr",
                "traceparent",
                "0123456789abcdef0123456789abcdef",
            ] {
                assert!(
                    !surface.contains(forbidden),
                    "usbip envelope leaked {forbidden:?}: {surface:?}"
                );
            }
        }

        let malformed_busid = TypedError::UsbipStepFailed {
            busid: "/sys/devices/pci0000:00/usb1/1-2".to_owned(),
            step: crate::usbip_state_machine::UsbipBusidStep::Bind,
            reason: "invalid persisted claim".to_owned(),
        }
        .to_envelope();
        assert!(malformed_busid.message.contains("<invalid-busid>"));
        assert!(malformed_busid.remediation.contains("<invalid-busid>"));
        assert_no_path_leak("UsbipStepFailed", &malformed_busid.message);
        assert_no_path_leak("UsbipStepFailed", &malformed_busid.remediation);
    }

    #[test]
    fn usbip_revocation_not_isolated_is_fail_secure_and_actionable() {
        let err = TypedError::UsbipRevocationNotIsolated {
            vm: "work-vm".to_owned(),
            busid: "1-2.4".to_owned(),
            reason: crate::usbip_reconcile_state::UsbipRevocationFlowFailure::SharedListeningSocket,
        };

        let envelope = err.to_envelope();
        assert_eq!(envelope.kind, "usbip-revocation-not-isolated");
        assert_eq!(envelope.exit_code, 67);
        assert!(envelope.message.contains("work-vm"));
        assert!(envelope.message.contains("1-2.4"));
        assert!(envelope.remediation.contains("1-2.4"));
        assert!(
            envelope
                .message
                .contains("shared per-env USBIP proxy listener")
        );
        assert!(envelope.remediation.contains("stop the VM"));
        assert!(
            envelope
                .remediation
                .contains("d2b usb detach work-vm 1-2.4 --apply")
        );
        assert_no_path_leak("UsbipRevocationNotIsolated", &envelope.message);
        assert_no_path_leak("UsbipRevocationNotIsolated", &envelope.remediation);

        let malformed_busid = TypedError::UsbipRevocationNotIsolated {
            vm: "work-vm".to_owned(),
            busid: "/sys/devices/pci0000:00/usb1/1-2".to_owned(),
            reason: crate::usbip_reconcile_state::UsbipRevocationFlowFailure::MissingExactTuple,
        }
        .to_envelope();
        assert!(malformed_busid.message.contains("<invalid-busid>"));
        assert!(malformed_busid.remediation.contains("<invalid-busid>"));
        assert_no_path_leak("UsbipRevocationNotIsolated", &malformed_busid.message);
        assert_no_path_leak("UsbipRevocationNotIsolated", &malformed_busid.remediation);
    }

    #[test]
    fn wire_ifname_invalid_redacted() {
        let err = TypedError::WireIfNameInvalid {
            detail: "eth0/../../../etc/shadow".to_owned(),
        };
        assert_eq!(err.kind(), "wire-ifname-invalid");
        assert_no_path_leak("WireIfNameInvalid", &err.message());
    }

    #[test]
    fn guest_control_read_failed_kinds_are_distinct_and_leak_free() {
        let kinds = [
            (
                GuestControlReadErrorKind::Transport,
                "guest-control-transport-unavailable",
            ),
            (
                GuestControlReadErrorKind::AuthFailed,
                "guest-control-auth-failed",
            ),
            (
                GuestControlReadErrorKind::Protocol,
                "guest-control-protocol-error",
            ),
            (
                GuestControlReadErrorKind::CapabilityUnavailable,
                "guest-control-capability-unavailable",
            ),
            (
                GuestControlReadErrorKind::FileNotFound,
                "guest-control-file-not-found",
            ),
            (
                GuestControlReadErrorKind::FileTooLarge,
                "guest-control-file-too-large",
            ),
            (
                GuestControlReadErrorKind::PathUnsafe,
                "guest-control-path-unsafe",
            ),
            (
                GuestControlReadErrorKind::ReadDenied,
                "guest-control-read-denied",
            ),
            (GuestControlReadErrorKind::Timeout, "guest-control-timeout"),
        ];
        for (kind, slug) in kinds {
            let err = TypedError::GuestControlReadFailed { kind };
            assert_eq!(err.kind(), slug);
            assert_eq!(err.exit_code(), 70);
            // Neither the human message nor the remediation may leak a host or
            // guest path / byte / string.
            assert_no_path_leak(slug, &err.message());
            assert_no_path_leak(slug, &err.remediation());
        }
    }

    #[test]
    fn guest_control_exec_failed_kinds_are_leak_free() {
        // Every exec failure kind (including the serde/protocol and
        // transport classes) must surface a non-empty, leak-free public
        // message + remediation — no host path, argv, env, output bytes, or
        // session handle. The daemon never attaches guest-supplied content to
        // a `GuestControlExecFailed` envelope, so iterating the closed enum is
        // sufficient sentinel coverage for the failure path.
        let kinds = [
            GuestControlExecErrorKind::Transport,
            GuestControlExecErrorKind::Auth,
            GuestControlExecErrorKind::Protocol,
            GuestControlExecErrorKind::Timeout,
            GuestControlExecErrorKind::OldGeneration,
            GuestControlExecErrorKind::Capability,
            GuestControlExecErrorKind::DetachedUnavailable,
            GuestControlExecErrorKind::SessionCapacity,
            GuestControlExecErrorKind::RateLimited,
            GuestControlExecErrorKind::StaleSession,
            GuestControlExecErrorKind::ExecNotFound,
            GuestControlExecErrorKind::ExecExpired,
            GuestControlExecErrorKind::InvalidProgram,
            GuestControlExecErrorKind::GuestError,
            GuestControlExecErrorKind::Internal,
        ];
        for kind in kinds {
            let err = TypedError::GuestControlExecFailed { kind };
            let slug = err.kind();
            assert!(
                slug.starts_with("guest-control-") || slug.starts_with("exec-session-"),
                "slug={slug} does not use a guest-control / exec-session prefix"
            );
            assert!(!err.message().is_empty(), "kind={slug} message empty");
            assert!(
                !err.remediation().is_empty(),
                "kind={slug} remediation empty"
            );
            assert_no_path_leak(slug, &err.message());
            assert_no_path_leak(slug, &err.remediation());
            // The public envelope must never carry guest-supplied tokens.
            for surface in [err.message(), err.remediation()] {
                for forbidden in ["argv", "stdout", "stderr", "session=", "handle="] {
                    assert!(
                        !surface.contains(forbidden),
                        "kind={slug} leaks {forbidden:?}: {surface:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn guest_control_shell_failed_kinds_are_leak_free() {
        let kinds = [
            GuestControlShellErrorKind::Transport,
            GuestControlShellErrorKind::Auth,
            GuestControlShellErrorKind::Protocol,
            GuestControlShellErrorKind::Timeout,
            GuestControlShellErrorKind::Capability,
            GuestControlShellErrorKind::StaleSession,
            GuestControlShellErrorKind::Capacity,
            GuestControlShellErrorKind::AlreadyAttached,
            GuestControlShellErrorKind::NotFound,
            GuestControlShellErrorKind::OutputGap,
            GuestControlShellErrorKind::GuestError,
            GuestControlShellErrorKind::Internal,
        ];
        for kind in kinds {
            let err = TypedError::GuestControlShellFailed { kind };
            let slug = err.kind();
            assert!(
                slug.starts_with("guest-control-shell-"),
                "slug={slug} does not use guest-control-shell prefix"
            );
            assert!(!err.message().is_empty(), "kind={slug} message empty");
            assert!(
                !err.remediation().is_empty(),
                "kind={slug} remediation empty"
            );
            assert_no_path_leak(slug, &err.message());
            assert_no_path_leak(slug, &err.remediation());
            for surface in [err.message(), err.remediation()] {
                for forbidden in ["stdout", "stderr", "session=", "handle=", "shell="] {
                    assert!(
                        !surface.contains(forbidden),
                        "kind={slug} leaks {forbidden:?}: {surface:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn envelope_kind_matches_expected_discriminant() {
        let cases: Vec<(TypedError, &str)> = vec![
            (
                TypedError::AuthzNotALauncher { peer_uid: 1000 },
                "authz-not-a-launcher",
            ),
            (
                TypedError::AuthzNotAdmin {
                    verb: "switch".to_owned(),
                },
                "authz-not-admin",
            ),
            (
                TypedError::AuthzAuditRequiresAdmin,
                "authz-audit-requires-admin",
            ),
            (
                TypedError::InternalAlreadyRunning {
                    path: PathBuf::from("/etc/nixos/foo.nix"),
                },
                "internal-already-running",
            ),
            (
                TypedError::InternalBrokerUnavailable {
                    path: PathBuf::from("/run/d2b/priv.sock"),
                    detail: "refused".to_owned(),
                },
                "internal-broker-unavailable",
            ),
            (
                TypedError::InternalConfig {
                    detail: "bad".to_owned(),
                },
                "internal-config-invalid",
            ),
            (
                TypedError::InternalIo {
                    context: "read /etc/nixos/foo.nix".to_owned(),
                    detail: "ENOENT".to_owned(),
                },
                "internal-io",
            ),
            (
                TypedError::InternalLockParentInvalid {
                    path: PathBuf::from("/home/paydro/.config/test"),
                    detail: "not a directory".to_owned(),
                },
                "internal-lock-parent-invalid",
            ),
            (
                TypedError::WireVersionMismatch {
                    client_range: ">=0.4.0".to_owned(),
                    accepted_range: ">=0.5.0".to_owned(),
                },
                "wire-version-mismatch",
            ),
            (
                TypedError::WireUnknownField {
                    detail: "field x".to_owned(),
                },
                "wire-unknown-field",
            ),
            (
                TypedError::WireIfNameInvalid {
                    detail: "too long".to_owned(),
                },
                "wire-ifname-invalid",
            ),
            (
                TypedError::WireFrameTooLarge {
                    declared: 2_000_000,
                },
                "wire-frame-too-large",
            ),
            (
                TypedError::WireInvalidFrame {
                    detail: "bad".to_owned(),
                },
                "wire-invalid-frame",
            ),
            (
                TypedError::WireBadHello {
                    detail: "missing type".to_owned(),
                },
                "wire-bad-hello",
            ),
            (
                TypedError::WireUnsupportedRequest {
                    request_type: "foo".to_owned(),
                },
                "wire-unsupported-request",
            ),
            (
                TypedError::OwnershipMatrixDrift {
                    vm: "vm1".to_owned(),
                    path: PathBuf::from("/var/lib/d2b/vms/vm1/state"),
                    drift_reason: "drift".to_owned(),
                },
                "ownership-matrix-drift",
            ),
            (
                TypedError::SshdHostKeyDrift {
                    vm: "vm1".to_owned(),
                    path: PathBuf::from("/var/lib/d2b/vms/vm1/sshd-host-keys/ssh_host_ed25519_key"),
                    drift: "ssh host key mode 644 != expected 400".to_owned(),
                },
                "sshd-host-key-drift",
            ),
            (
                TypedError::BundleDnsmasqDrift {
                    vm: "sys-work-net".to_owned(),
                    env: "work".to_owned(),
                    path: PathBuf::from("/var/lib/d2b/dnsmasq/work.conf"),
                    expected: "a".repeat(64),
                    actual: "b".repeat(64),
                    reason: "dnsmasq.conf hash for env 'work' diverges from bundle expectation"
                        .to_owned(),
                },
                "bundle-dnsmasq-drift",
            ),
            (
                TypedError::HostKernelModulesMissing {
                    missing: "kvm_intel|kvm_amd, vhost_net".to_owned(),
                },
                "host-kernel-modules-missing",
            ),
        ];
        for (err, expected_kind) in &cases {
            assert_eq!(err.kind(), *expected_kind, "kind mismatch for {err:?}");
            let envelope = err.to_envelope();
            assert_eq!(envelope.kind, *expected_kind);
        }
    }

    #[test]
    fn sshd_host_key_drift_envelope_shape() {
        let err = TypedError::SshdHostKeyDrift {
            vm: "vm1".to_owned(),
            path: PathBuf::from("/var/lib/d2b/vms/vm1/sshd-host-keys/ssh_host_ed25519_key"),
            drift: "ssh host key mode 644 != expected 400".to_owned(),
        };
        assert_eq!(err.kind(), "sshd-host-key-drift");
        assert_eq!(err.exit_code(), 62);
        let env = err.to_envelope();
        assert_eq!(env.exit_code, 62);
        assert!(env.message.contains("vm1"));
        assert!(env.message.contains("400"));
        assert!(env.remediation.contains("regenerate"));
    }

    #[test]
    fn bundle_dnsmasq_drift_envelope_shape() {
        let err = TypedError::BundleDnsmasqDrift {
            vm: "sys-work-net".to_owned(),
            env: "work".to_owned(),
            path: PathBuf::from("/var/lib/d2b/dnsmasq/work.conf"),
            expected: "a".repeat(64),
            actual: "b".repeat(64),
            reason: "dnsmasq.conf hash for env 'work' diverges from bundle expectation \
                (expected aaaa, actual bbbb); rebuild required"
                .to_owned(),
        };
        assert_eq!(err.kind(), "bundle-dnsmasq-drift");
        assert_eq!(err.exit_code(), 63);
        let envelope = err.to_envelope();
        assert_eq!(envelope.exit_code, 63);
        assert!(envelope.message.contains("sys-work-net"));
        assert!(envelope.message.contains("work"));
        assert!(envelope.message.contains("rebuild required"));
        assert!(envelope.remediation.contains("re-render"));
        assert!(envelope.remediation.contains("dnsmasq"));
    }
}
