# Broker disposition snapshot

This table preserves the original broker request snapshot derived from
`docs/reference/schemas/v1/privileges.json` plus the daemon-only
`Hello` handshake, so the historical dispatcher surface stays
closed and auditable.

Current broker behavior is described in
[`privileges.md`](./privileges.md). This snapshot has no
`compile-time-only` broker rows.

| Variant | Disposition | Snapshot note | Current status |
| --- | --- | --- | --- |
| Hello | callable-read-only | Daemon-only handshake; returns `HelloOk` with the broker capability list. | legacy snapshot callable |
| ValidateBundle | callable-read-only | Sole validation entry point; calls `nixling_core::manifest::validate_bundle` and logs only opaque metadata. | legacy snapshot callable |
| ExportBrokerAudit | callable-read-only | Reads the append-only broker audit log, requires `caller_role: AdminUid { uid }`, and streams redacted lines back to `nixlingd`. | legacy snapshot callable |
| GuestControlSign | callable-read-only | Computes the per-VM guest-control auth tag (HMAC-SHA256 over the bound transcript of `vmId`, `role`, `purpose`, host/guest nonces, peer CID, and capabilities hash); returns only the transcript-bound MAC tag, never host state, the per-VM token, or the raw transcript. | guest-control live callable |
| ApplyNftables | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; no nft syscall or host mutation is attempted in this snapshot. | live in production broker |
| ApplyNmUnmanaged | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; this snapshot does not manage the NetworkManager unmanaged file. | live in production broker |
| ApplyRoute | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; no route reconciliation runs in this snapshot. | live in production broker |
| ApplySysctl | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; no sysctl writes run in this snapshot. | live in production broker |
| BindUnixSocket | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; sidecar socket binding is outside this snapshot. | reserved |
| CreateOrReconcileUsersGroups | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; no host account mutation runs in this snapshot. | bootstrap-only |
| CreatePersistentTap | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; TAP creation is outside this snapshot. | live in production broker |
| CreateTapFd | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; the read-only fd carve-out is outside this snapshot. | live in production broker |
| DelegateCgroupV2 | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; no cgroup delegation runs in this snapshot. | live in production broker |
| InjectSecretById | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; secret write paths are not implemented. | future work |
| LaunchMinijailChild | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; privileged child launch is not implemented. | future work |
| ModprobeIfAllowed | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; kernel module mutation is outside this snapshot. | live in production broker |
| OpenCgroupDir | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; no broker-exported cgroup fd path is callable in this snapshot. | live in production broker |
| OpenDevice | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; generic device opens stay out of this snapshot. | live in production broker |
| OpenFuse | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; FUSE fd export stays out of this snapshot. | live in production broker |
| OpenKvm | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; KVM fd export stays out of this snapshot. | live in production broker |
| OpenPidfd | stubbed-unimplemented | Returns `BrokerError::Unimplemented`. In the live broker, `OpenPidfd` calls `pidfd_open(pid)` and re-verifies `/proc/<pid>/stat` field 22 atomically; mismatch surfaces a typed pidfd-race error. | live in production broker |
| OpenVhostNet | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; vhost-net fd export stays out of this snapshot. | live in production broker |
| PauseBroker | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; broker admin pause/resume controls are not implemented. | future work |
| PrepareRuntimeDir | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; no runtime dir creation or ownership mutation runs in this snapshot. | live in production broker |
| PrepareStateDir | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; no state dir creation or ownership mutation runs in this snapshot. | live in production broker |
| PrepareStoreView | promoted-live | The production broker resolves the per-VM store-view intent, builds the hardlink farm, and validates the generation marker; the legacy snapshot dispatcher remains stubbed. | live in production broker |
| ReadSecretById | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; secret read paths are not implemented. | future work |
| ResumeBroker | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; broker admin pause/resume controls are not implemented. | future work |
| RotateSecretById | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; secret rotation is not implemented. | future work |
| RunHostInstall | promoted-live | The production broker resolves the installer intent from the trusted bundle, writes installer artifacts, and can enable/start `nixlingd.service`; the legacy snapshot dispatcher remains stubbed. | live in production broker |
| RunMigrate | promoted-live | The production broker resolves the host migration intent from the trusted bundle and writes per-VM migration markers under `/var/lib/nixling/migrate/<vm>.json`; the legacy snapshot dispatcher remains stubbed. | live in production broker |
| RunActivation | promoted-live | The production broker resolves activation + store-view intents from the trusted bundle, prepares the store view and mount namespace, and runs the requested `switch` / `boot` / `test` / `rollback` activation script; the legacy snapshot dispatcher remains stubbed. | live in production broker |
| RunGc | promoted-live | The production broker resolves the host GC intent from the trusted bundle and executes the typed store-prune policy; the legacy snapshot dispatcher remains stubbed. | live in production broker |
| RunKeysRotate | promoted-live | The production broker resolves the managed per-VM key-rotation intent from the trusted bundle, runs `ssh-keygen`, and returns the rotated key metadata; the legacy snapshot dispatcher remains stubbed. | live in production broker |
| RunHostKeyTrust | promoted-live | The production broker resolves the per-VM TOFU trust intent from the trusted bundle and atomically updates the managed known-hosts file; the legacy snapshot dispatcher remains stubbed. | live in production broker |
| RunRotateKnownHost | promoted-live | The production broker resolves the per-VM known-host rotation intent from the trusted bundle and atomically removes/re-pins the managed entry; the legacy snapshot dispatcher remains stubbed. | live in production broker |
| SetBridgePortFlags | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; bridge-port mutation is outside this snapshot. | live in production broker |
| SetSocketAcl | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; privileged socket ACL mutation is outside this snapshot. | reserved |
| SetupMountNamespace | promoted-live | The production broker prepares the per-VM mount-namespace staging root and store-view bind target; the legacy snapshot dispatcher remains stubbed. | live in production broker |
| SignalRunner | promoted-live | The production broker looks up the runner's registered pidfd, sends the requested signal with `pidfd_send_signal(2)`, and audits the live stop request; the legacy snapshot dispatcher remains stubbed. | live in production broker |
| UpdateHostsFile | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; `/etc/hosts` mutation is outside this snapshot. | live in production broker |
| UsbipBind | stubbed-unknown-operation | USBIP live device routing is outside the current broker surface. The snapshot refuses with `BrokerError::UnknownOperation` (not `Unimplemented`) so the audit shape records `unknown-operation`. | future work |
| UsbipBindFirewallRule | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; the snapshot keeps this variant stubbed even though the production broker now reconciles the per-busid firewall carve-out. | live in production broker |
| UsbipProxyReconcile | stubbed-unknown-operation | USBIP proxy reconcile is outside the current broker surface. The snapshot refuses with `BrokerError::UnknownOperation` (not `Unimplemented`) so the audit shape records `unknown-operation`. | future work |
| UsbipUnbind | stubbed-unknown-operation | USBIP live device routing is outside the current broker surface. The snapshot refuses with `BrokerError::UnknownOperation` (not `Unimplemented`) so the audit shape records `unknown-operation`. | future work |
| SpawnRunner | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; the production broker handles CH/virtiofsd/swtpm child process launch and SCM_RIGHTS pidfd handoff. | live in production broker |
