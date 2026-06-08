# Broker W2 dispositions

This table records the W2 broker request surface. It includes every broker-facing CamelCase operation from `docs/reference/schemas/v1/privileges.json` plus the daemon-only `Hello` handshake so the dispatcher stays closed and auditable.

W2 has no `compile-time-only` broker rows.

| Variant | Disposition | W2 implementation note | Target wave for promotion |
| --- | --- | --- | --- |
| Hello | callable-read-only | Daemon-only handshake; returns `HelloOk` with the W2 broker capability list. | W2 |
| ValidateBundle | callable-read-only | Sole validation entry point; calls `nixling_core::manifest::validate_bundle` and logs only opaque metadata. | W2 |
| ExportBrokerAudit | callable-read-only | Reads the append-only broker audit log, requires `caller_role: AdminUid { uid }`, and streams redacted lines back to `nixlingd`. | W2 |
| ApplyNftables | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; no nft syscall or host mutation is attempted. | W3 |
| ApplyNmUnmanaged | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; the NetworkManager unmanaged file remains daemon-planned only in W2. | W3 |
| ApplyRoute | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; no route reconciliation runs in W2. | W3 |
| ApplySysctl | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; no sysctl writes run in W2. | W3 |
| BindUnixSocket | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; sidecar socket binding waits for the high-risk runner waves. | W5 |
| CreateOrReconcileUsersGroups | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; no host account mutation runs in W2. | W3 |
| CreatePersistentTap | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; TAP creation is deferred with the host-prepare wave. | W3 |
| CreateTapFd | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; the read-only fd carve-out stays deferred past W2. | W3 |
| DelegateCgroupV2 | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; no cgroup delegation runs in W2. | W3 |
| InjectSecretById | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; secret write paths stay dark in W2. | W8 |
| LaunchMinijailChild | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; privileged child launch waits for the sandbox/sidecar wave. | W5 |
| ModprobeIfAllowed | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; kernel module mutation is deferred. | W3 |
| OpenCgroupDir | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; no broker-exported cgroup fd path is callable in W2. | W3 |
| OpenDevice | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; generic device opens stay out of W2. | W3 |
| OpenFuse | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; FUSE fd export waits for later store/runtime work. | W3 |
| OpenKvm | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; KVM fd export remains deferred. | W3 |
| OpenPidfd | stubbed-unimplemented | W4-fu: daemon-side reconcile-and-adopt. Returns `BrokerError::Unimplemented{target_wave: "W4-fu"}`. When promoted to live, broker calls `pidfd_open(pid)` AND re-verifies `/proc/<pid>/stat` field 22 atomically; mismatch surfaces a typed pidfd-race error. | W4-fu |
| OpenVhostNet | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; vhost-net fd export remains deferred. | W3 |
| PauseBroker | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; broker admin pause/resume controls are not yet live. | W4 |
| PrepareRuntimeDir | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; no runtime dir creation or ownership mutation runs in W2. | W3 |
| PrepareStateDir | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; no state dir creation or ownership mutation runs in W2. | W3 |
| PrepareStoreView | promoted-live | Production broker now resolves the per-VM store-view intent, builds the hardlink farm, and validates the generation marker; the legacy W2/bootstrap dispatcher remains stubbed. | W7 |
| ReadSecretById | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; secret read paths stay dark in W2. | W8 |
| ResumeBroker | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; broker admin pause/resume controls are not yet live. | W4 |
| RotateSecretById | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; secret rotation waits for the trust wave. | W8 |
| RunHostInstall | promoted-live | Production broker now resolves the installer intent from the trusted bundle, writes installer artifacts, and can enable/start `nixlingd.service`; the legacy W2/bootstrap dispatcher remains stubbed. | W15 |
| RunMigrate | promoted-live | Production broker now resolves the host migration intent from the trusted bundle and writes per-VM migration markers under `/var/lib/nixling/migrate/<vm>.json`; the legacy W2/bootstrap dispatcher remains stubbed. | W15 |
| RunActivation | promoted-live | Production broker now resolves activation + store-view intents from the trusted bundle, prepares the store view and mount namespace, and runs the requested `switch` / `boot` / `test` / `rollback` activation script; the legacy W2/bootstrap dispatcher remains stubbed. | W14 |
| RunGc | promoted-live | Production broker now resolves the host GC intent from the trusted bundle and executes the typed store-prune policy; the legacy W2/bootstrap dispatcher remains stubbed. | W14 |
| RunKeysRotate | promoted-live | Production broker now resolves the managed per-VM key-rotation intent from the trusted bundle, runs `ssh-keygen`, and returns the rotated key metadata; the legacy W2/bootstrap dispatcher remains stubbed. | W14 |
| RunHostKeyTrust | promoted-live | Production broker now resolves the per-VM TOFU trust intent from the trusted bundle and atomically updates the managed known-hosts file; the legacy W2/bootstrap dispatcher remains stubbed. | W14 |
| RunRotateKnownHost | promoted-live | Production broker now resolves the per-VM known-host rotation intent from the trusted bundle and atomically removes/re-pins the managed entry; the legacy W2/bootstrap dispatcher remains stubbed. | W14 |
| SetBridgePortFlags | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; bridge-port mutation is deferred with network reconcile. | W3 |
| SetSocketAcl | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; privileged socket ACL mutation waits for the sidecar wave. | W5 |
| SetupMountNamespace | promoted-live | Production broker now prepares the per-VM mount-namespace staging root and store-view bind target; the legacy W2/bootstrap dispatcher remains stubbed. | W7 |
| SignalRunner | promoted-live | Production broker now looks up the runner's registered pidfd, sends the requested signal with `pidfd_send_signal(2)`, and audits the live stop request; the legacy W2/bootstrap dispatcher remains stubbed. | W4-fu |
| UpdateHostsFile | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; `/etc/hosts` mutation is deferred past the W2 read-only surface. | W3 |
| UsbipBind | stubbed-unknown-operation | W3fu1 H1 (rust-1): W6 USBIP live device routing is **out of W3 scope** per plan §"W3 broker variant additions"; refuses with `BrokerError::UnknownOperation` (not `Unimplemented`) so the W3 broker audit shape records `unknown-operation`. | W6 |
| UsbipBindFirewallRule | stubbed-unimplemented | Returns `BrokerError::Unimplemented`; W3 prep landed the wire-stable skeleton only — scope s3 wires the per-busid firewall-rule reconcile in its scope commit. | W3 |
| UsbipProxyReconcile | stubbed-unknown-operation | W3fu1 H1 (rust-1): W6 USBIP proxy reconcile is **out of W3 scope** per plan §"W3 broker variant additions"; refuses with `BrokerError::UnknownOperation` (not `Unimplemented`) so the W3 broker audit shape records `unknown-operation`. | W6 |
| UsbipUnbind | stubbed-unknown-operation | W3fu1 H1 (rust-1): W6 USBIP live device routing is **out of W3 scope** per plan §"W3 broker variant additions"; refuses with `BrokerError::UnknownOperation` (not `Unimplemented`) so the W3 broker audit shape records `unknown-operation`. | W6 |
| SpawnRunner | stubbed-unimplemented | W4-H5: wire-stable opaque-ID variant; broker dispatcher returns `BrokerError::Unimplemented{ target_wave: "W4-fu" }` until the W4-fu broker-side spawn implementation lands (CH/virtiofsd/swtpm child process + SCM_RIGHTS pidfd handoff). | W4-fu |
