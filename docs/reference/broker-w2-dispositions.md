# Broker request dispositions

This table tracks the broker request dispositions for the current
`OperationAuthz.operation` enum plus the daemon-only `Hello`
handshake. Current broker behavior is described in
[`privileges.md`](./privileges.md). The table has exactly one
`compile-time-only` broker row (`PrepareSwtpmDir`, a `SpawnRunner`
side-effect audit operation that never reaches the wire dispatcher).

| Variant | Disposition | Note | Target |
| --- | --- | --- | --- |
| ApplyNftables | promoted-live | Resolves the trusted bundle nftables intent and applies or destroys the managed nftables batch. | live in production broker |
| ApplyNmUnmanaged | promoted-live | Resolves the trusted NetworkManager unmanaged intent and reconciles the managed config block. | live in production broker |
| ApplyRoute | promoted-live | Resolves the trusted route intent and applies or removes the host route. | live in production broker |
| ApplySysctl | promoted-live | Resolves the trusted sysctl intent and applies or removes the host sysctl value. | live in production broker |
| BindMountFromHardlinkFarm | promoted-live | Resolves the per-VM store-view intent, records the hardlink-farm source, and acknowledges the daemon-owned bind-mount step. | live in production broker |
| BindUnixSocket | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; sidecar socket binding is not implemented. | reserved |
| CreateOrReconcileUsersGroups | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; host account reconciliation is not implemented in the production dispatcher. | bootstrap-only |
| CreatePersistentTap | promoted-live | Creates or reconciles the VM TAP device through the live TAP handler and records the resulting ifnames. | live in production broker |
| CreateTapFd | promoted-live | Opens a TAP fd through the live TAP handler and returns it over `SCM_RIGHTS`. | live in production broker |
| DelegateCgroupV2 | promoted-live | Delegates the trusted cgroup v2 subtree and records the delegated scope. | live in production broker |
| DeregisterRunnerPidfd | promoted-live | Removes the runner pidfd registry entry idempotently and returns whether an entry was present. | live in production broker |
| DiskInit | promoted-live | Resolves trusted disk-init plans for the VM and creates, validates, or safely repairs disk images before runner spawn; ambiguous existing data fails closed. | live in production broker |
| ExportBrokerAudit | callable-read-only | Reads the append-only broker audit log, requires `caller_role: AdminUid { uid }`, and streams redacted lines back to `d2bd`. | live read-only callable |
| GuestControlSign | callable-read-only | Computes the per-VM guest-control auth tag over the bound transcript; returns only the transcript-bound MAC tag. | guest-control live callable |
| Hello | callable-read-only | Daemon-only handshake; returns `HelloOk` with the broker capability list. | live read-only callable |
| InjectSecretById | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; secret write paths are not implemented. | future work |
| LaunchMinijailChild | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; privileged child launch is not implemented. | future work |
| ModprobeIfAllowed | promoted-live | Resolves the trusted module policy, checks the host module posture, and runs the live modprobe handler when allowed. | live in production broker |
| OpenCgroupDir | promoted-live | Opens the trusted cgroup directory and returns the fd over `SCM_RIGHTS`. | live in production broker |
| OpenDevice | promoted-live | Opens a device allowed by the trusted device matrix and returns the fd over `SCM_RIGHTS`. | live in production broker |
| OpenFuse | promoted-live | Opens the allowed FUSE device path and returns the fd over `SCM_RIGHTS`. | live in production broker |
| OpenKvm | promoted-live | Opens the allowed KVM device path and returns the fd over `SCM_RIGHTS`. | live in production broker |
| OpenPidfd | promoted-live | Opens a runner pidfd, re-verifies the process start time, and returns the fd over `SCM_RIGHTS`. | live in production broker |
| OpenVhostNet | promoted-live | Opens the allowed vhost-net device path and returns the fd over `SCM_RIGHTS`. | live in production broker |
| OwnershipMatrixCheck | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; ownership-matrix preflight is not implemented. | future work |
| PauseBroker | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; broker admin pause controls are not implemented. | future work |
| PollChildReaped | promoted-live | Drains the broker's child-reap notification buffer and returns the pending notifications. | live in production broker |
| PrepareRuntimeDir | promoted-live | Resolves the trusted runtime-dir intent and prepares ownership/mode for the per-VM runtime directory. | live in production broker |
| PrepareStateDir | promoted-live | Resolves the trusted state-dir intent and prepares ownership/mode for the per-VM state directory. | live in production broker |
| PrepareStoreView | promoted-live | Resolves the per-VM store-view intent, builds the hardlink farm, and validates the generation marker. | live in production broker |
| PrepareSwtpmDir | compile-time-only | Audit operation emitted as a `SpawnRunner` side-effect for the `Swtpm` runner (issue #64); it provisions/hardens the persistent per-VM swtpm state dir and writes the identity-bound tamper marker inside the broker's pre-spawn step. It is not a standalone wire request, so it never reaches the wire dispatcher. | broker `SpawnRunner` side-effect |
| QemuMediaAttach | promoted-live | Resolves an enrolled qemu-media slot, passes the media fd to QEMU over QMP, and returns only redacted command labels. | live in production broker |
| QemuMediaBoot | promoted-live | Resolves the declared boot source, passes the media fd to QEMU over QMP, attaches the boot USB storage device, and continues the paused runner. | live in production broker |
| QemuMediaDetach | promoted-live | Resolves an enrolled qemu-media slot, removes the QMP device/block/fd nodes, and returns only redacted command labels. | live in production broker |
| QemuMediaEnroll | promoted-live | Enrolls a physical USB device into root-only qemu-media registry state and reconciles runtime automount-inhibition rules. | live in production broker |
| QemuMediaQueryStatus | promoted-live | Queries qemu-media QMP run state for lifecycle reconciliation; success polling is summarized by daemon lifecycle audit rather than per-poll broker audit records. | live in production broker |
| QemuMediaQuit | promoted-live | Requests clean qemu-media VMM exit over QMP after the guest has reached shutdown state. | live in production broker |
| QemuMediaRefreshRegistry | promoted-live | Rebuilds redacted daemon-readable qemu-media registry state and runtime automount-inhibition rules from the root-only persistent registry. | live in production broker |
| QemuMediaSystemPowerdown | promoted-live | Requests qemu-media guest shutdown through QMP `system_powerdown` before forced runner cleanup. | live in production broker |
| ReadSecretById | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; secret read paths are not implemented. | future work |
| ReconcileStorageScope | promoted-live | Resolves the trusted storage contract row and reconciles or validates the static storage scope without exposing raw paths. | live in production broker |
| ResumeBroker | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; broker admin resume controls are not implemented. | future work |
| RotateSecretById | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; secret rotation is not implemented. | future work |
| RunActivation | promoted-live | Resolves activation and store-view intents from the trusted bundle and runs the requested activation mode. | live in production broker |
| RunGc | promoted-live | Resolves the host GC intent from the trusted bundle and executes the typed store-prune policy. | live in production broker |
| RunHostInstall | promoted-live | Resolves the installer intent from the trusted bundle, writes installer artifacts, and can enable/start `d2bd.service`. | live in production broker |
| RunHostKeyTrust | promoted-live | Resolves the per-VM TOFU trust intent and atomically updates the managed known-hosts file. | live in production broker |
| RunKeysRotate | promoted-live | Resolves the managed per-VM key-rotation intent, runs `ssh-keygen`, and returns rotated key metadata. | live in production broker |
| RunMigrate | promoted-live | Resolves the host migration intent and writes per-VM migration markers. | live in production broker |
| RunRotateKnownHost | promoted-live | Resolves the per-VM known-host rotation intent and atomically removes or re-pins the managed entry. | live in production broker |
| SeedDnsmasqLease | promoted-live | Validates the VM against the trusted manifest, records the requested lease seed, and acknowledges the daemon-owned lease-file step. | live in production broker |
| SetBridgePortFlags | promoted-live | Reconciles trusted bridge-port isolation flags for the VM TAP. | live in production broker |
| SetSocketAcl | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; privileged socket ACL mutation is not implemented. | reserved |
| SetupMountNamespace | promoted-live | Prepares the per-VM mount-namespace staging root and store-view bind target. | live in production broker |
| SignalRunner | promoted-live | Looks up the runner's registered pidfd, sends the requested signal, and audits the live stop request. | live in production broker |
| SpawnRunner | promoted-live | Handles CH/virtiofsd/swtpm child process launch and `SCM_RIGHTS` pidfd handoff. | live in production broker |
| SshHostKeyPreflight | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; SSH host-key preflight is not implemented. | future work |
| StoreSync | promoted-live | Resolves the per-VM store-view intent, synchronizes the hardlink farm, and emits the terminal store-sync audit record. | live in production broker |
| StoreVerify | promoted-live | Verifies the per-VM store hardlink farm and optionally repairs drift through the store-sync path. | live in production broker |
| UpdateHostsFile | promoted-live | Resolves the trusted hosts-file intent and reconciles the managed `/etc/hosts` block. | live in production broker |
| UsbipBind | promoted-live | Resolves the trusted USBIP bind intent, enforces the allowlist, binds the device, and grants the backend ACL. | live in production broker |
| UsbipBindFirewallRule | promoted-live | Resolves the trusted USBIP firewall intent and reconciles the per-busid nftables carve-out. | live in production broker |
| UsbipExplicitBind | promoted-live | Binds an explicit present-busid device, acquires the per-busid lock, grants the per-device backend ACL, and audits redacted device identity without requiring a bundle allowlist. | live in production broker |
| UsbipExplicitFirewallRule | promoted-live | Reconciles the env-scoped nftables carve-out for an explicit present-busid attach while preserving currently active carve-outs. | live in production broker |
| UsbipProxyReconcile | promoted-live | Reconciles USBIP proxy lock expectations derived from trusted bundle intents. | live in production broker |
| UsbipUnbind | promoted-live | Resolves the current USBIP owner, revokes the backend ACL, and unbinds the device. | live in production broker |
| ValidateLockSpec | promoted-live | Resolves the trusted sync contract row and validates lock policy without mutating host state. | live in production broker |
| ValidateBundle | callable-read-only | Sole validation entry point; calls `d2b_core::manifest::validate_bundle` and logs only opaque metadata. | live read-only callable |
| SecurityKeyApplyUdevRules | stubbed-unimplemented | Writes broker-generated udev rules granting the `d2b-security-key` group ownership of the configured FIDO vendorId/productId/serial-matched hidraw nodes. No blanket hidraw access; a targeted audit event is recorded. | future work |
| SecurityKeyOpenDevice | stubbed-unimplemented | Resolves the stable device-label selector against the trusted bundle's security-key device table, checks sysfs presence and FIDO class, opens the exact hidraw node, and returns the fd via SCM_RIGHTS for the CTAP relay session. | future work |
