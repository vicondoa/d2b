# `processes.json` schema reference

`processes.json` is the private supervision artifact. It encodes the per-VM process DAG, typed readiness predicates, cgroup placement, restart policy, and minijail profile references that `d2bd` uses to launch and reconcile roles.

Producer: `nixos-modules/manifest-processes.nix` emits this artifact; `packages/d2b-core` parses it.

Schema: [`processes.json`](./processes.json) (forward reference; generated with `cargo xtask gen-schemas`).

## Top-level fields

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `schemaVersion` | string | yes | Artifact schema version. This schema emits `v1`. |
| `vms` | array | yes | Per-VM process graphs. |
| `profileCatalog` | string | yes | Artifact path or bundle key for `minijail-profile.json`. |
| `globalInvariants` | object | yes | v0.4.0 invariants that apply across all VM graphs. |

## Per-VM graph fields

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `vmName` | string | yes | VM name matching the public manifest entry. |
| `env` | string or null | yes | Env name, null only for legacy env-less VMs. |
| `roles` | array | yes | Role nodes in the DAG. |
| `edges` | array | yes | Dependency edges between role IDs. |
| `readiness` | array | yes | Ordered readiness predicates that gate dependent roles. |

## Required DAG order

Every VM graph is a typed DAG with this semantic order:

1. `hostReconcile`
2. `storeVirtiofsPreflight`
3. `swtpmPreStartFlush` when TPM is enabled
4. `virtiofsd`
5. optional `video`, `gpu`, and `audio` sidecars
6. `cloudHypervisorRunner`
7. `vsockRelay` roles when observability or notify relays are enabled
8. `guestSshReadiness`

The graph may omit feature-disabled optional roles, but it must not
weaken the ordering of roles that are present. This is the daemon-facing
form of [ADR 0004](../../../adr/0004-cloud-hypervisor-runner-shape.md).

## Role fields

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `id` | string | yes | Stable role ID unique within the VM graph. |
| `role` | enum | yes | Role kind such as `hostReconcile`, `virtiofsd`, `swtpm`, `gpu`, `video`, `audio`, `cloudHypervisor`, `vsockRelay`, or `guestSshReadiness`. |
| `argv` | array | yes | Executable and arguments after Nix evaluation. Command strings are not passed to the broker. |
| `environment` | object | yes | Explicit environment variables. Inherited ambient environment is not part of the contract. |
| `workingDirectory` | string | yes | Daemon-owned runtime directory for the role. |
| `minijailProfileId` | string | yes | Key into `minijail-profile.json`. |
| `uid` / `gid` | integer | yes | Non-root steady-state identity unless a bounded `requiresStartRoot` exception applies. |
| `capabilities` | array | yes | Declared Linux capabilities for the role; broad or undeclared sets are rejected. |
| `namespaces` | object | yes | Namespace policy reference: mount, pid, net, ipc, uts, user. |
| `seccompPolicy` | string | yes | Policy ID or path reference. This schema stores references only, not syscall allowlists. |
| `mountPolicy` | object | yes | Readonly/readwrite bind set and propagation rules. |
| `cgroup` | object | yes | Per-VM and per-role cgroup placement under the delegated d2b subtree. |
| `restartPolicy` | object | yes | Explicit crash/restart behavior. Running VMs are not restarted automatically for config drift. |
| `preStart` | array | yes | Typed pre-start hooks, including TPM flush where applicable. |

## Readiness predicates

| Predicate | Description |
| --- | --- |
| `api-socket-info` | Cloud Hypervisor API socket exists, has daemon-only ownership, and answers the expected API query. |
| `vsock-notify` | CH Unix-socket-backed vsock notify path is bound and ready. |
| `unix-socket-exists` | A sidecar socket exists after ownership/path validation. |
| `tcp-port` | Guest or sidecar TCP endpoint is reachable where the model still requires TCP. |
| `command` | Bounded diagnostic command owned by the daemon role, not a broker shell escape. |
| component-specific | Typed checks for `swtpm`, `virtiofsd`, audio/video/GPU helpers, and SSH. |

## Minijail metadata

`processes.json` carries typed minijail metadata per role: `uid`, `gid`,
`capabilities`, `namespaces`, `seccompPolicy`, `mountPolicy`, and
`cgroup` placement. It intentionally does **not** carry
kernel-version-specific syscall allowlists; those are generated and
reviewed in later runtime waves.

## v0.4.0 invariants encoded

| Invariant | Encoding |
| --- | --- |
| swtpm stale-session flush | TPM roles include a `preStart` hook equivalent to `swtpm_ioctl -i` boot and shutdown. |
| Per-VM audit pipeline | Audit/log roles and paths are explicit role metadata. |
| USBIP gating | USBIP helper roles appear only when the VM and env policy enable them. |
| TPM ownership migration | TPM ownership metadata is present, but running VMs are not mutated by bundle generation. |
| No config-change auto-restart | Role `restartPolicy` distinguishes crash handling from drift application. |
