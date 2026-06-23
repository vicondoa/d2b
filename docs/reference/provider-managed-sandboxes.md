# Provider-managed sandboxes

**Diataxis category:** reference.

A provider-managed sandbox is a workload unit that a cloud provider
creates, manages, and destroys on behalf of nixling. The nixling daemon
routes typed operations to the provider API or to a provider-side nixling
agent and receives typed responses; it does not own a hypervisor or broker
for these nodes. The first implemented adapter is Azure Container Apps.

This page documents the capability matrix, supported operations, absent
capabilities, rate-limit/backoff/circuit behavior, credential boundary,
safe diagnostics rules, and error shapes for provider-managed sandboxes.

For nodes where nixling owns the full host stack (hypervisor, broker,
guest-control), see [remote full-host nodes](./remote-full-host-nodes.md).

---

## What a provider-managed sandbox is

A provider-managed sandbox is a named workload target in a realm whose
lifecycle (create, start, stop, destroy) is owned by an external provider
API rather than by a `nixling-priv-broker` running on a locally managed
host. From the daemon's perspective it is a node with a bounded positive
capability set derived from what that provider API or provider-side nixling
agent supports. The daemon never provisions, registers, or expects a full
host `nixlingd`, `nixling-priv-broker`, KVM subsystem, vsock channel, cgroup
subtree, namespace hierarchy, full-host lifecycle, or device-hotplug surface
on a provider-managed node. ADR 0039 defines one exception to the old
exec-only model: a provider-managed sandbox may advertise persistent shell
only when it runs a guestd-compatible nixling agent that exposes shell
control and terminal-v1 streams over the constellation peer transport.

This model is distinct from a **remote full-host node** (see
[remote full-host nodes](./remote-full-host-nodes.md)), which runs its
own `nixlingd` and full broker stack and is reached through an
authenticated peer transport session. The following table summarizes the
key differences:

| Dimension | Provider-managed sandbox | Remote full-host node |
| --- | --- | --- |
| Who owns lifecycle | Cloud provider API | Remote `nixlingd` + `nixling-priv-broker` |
| Broker presence | None | Full broker on the remote host |
| Guest-control / vsock | No vsock or raw guest-control tunnel; persistent-shell-capable sandboxes require a guestd-compatible agent over constellation peer transport. | Present |
| KVM / hypervisor | Absent | Present |
| Cgroup / namespace authority | Absent | Present (remote host) |
| systemd | Absent | Present (remote host) |
| Device hotplug | Absent | Present (remote host) |
| SSH fallback | Absent | Absent |
| Authentication surface | Workload/managed identity → provider API | Peer session authenticated principal |
| Capability source | Provider adapter capability declaration | Substrate provider report |
| Registry | Provider API is the source of truth | Daemon router state |

---

## Capability matrix — Azure Container Apps adapter

Capabilities are positive assertions. A capability absent from this
table is not supported; operations requiring it receive
`CapabilityDenied` and do not fall back.

| Capability | Azure Container Apps support | Notes |
| --- | --- | --- |
| `lifecycle` | Conditional | Advertised only when sandbox defaults are configured; create/start/stop/list map to the Azure Container Apps sandbox data plane. |
| `exec` | ✓ | Synchronous Azure Container Apps `executeShellCommand`; returns a derived execution id, not a durable guest-control session. |
| `persistent-shell` | No | Live ADR 0039 capability. The executeShellCommand-only adapter must not advertise persistent shell; support requires a guestd-compatible in-sandbox agent over the constellation peer transport. |
| `logs` | ✗ | No retained-log stream in this adapter. |
| `pty` | ✗ | No interactive TTY or stdio attachment. |
| `file-copy` | ✗ | No bounded file-copy API. |
| `port-forward` | ✗ | No generic tunnel or port-forward API. |
| `vsock` | ✗ | No guest-control vsock channel. |
| `virtiofs` | ✗ | No per-workload /nix/store hardlink farm or virtiofsd share. |
| `window-forwarding` / `display-streaming` | ✗ | No Wayland/virtio-gpu or video sidecar. |
| `clipboard` | ✗ | No clipboard bridge. |
| `audio-playback` / `audio-capture` | ✗ | No vhost-user-sound or PipeWire mediation. |
| `usb` | ✗ | No USBIP passthrough. |
| `hid` | ✗ | No HID device operations. |
| `gpu-accel` | ✗ | No local GPU acceleration surface. |
| `snapshots` | ✗ | No snapshot API in this adapter. |
| `hotplug` | ✗ | No device hotplug API. |
| `ephemeral-sessions` | ✗ | Azure Container Apps sandboxes are selected by workload labels, not ephemeral session slots in this adapter. |
| `provider-managed-isolation` | ✓ | Advertised so callers can distinguish Azure Container Apps from a full nixling host. |

For the cross-provider display and virtual I/O capability split, see
[display and virtual I/O capabilities](./display-io-capabilities.md).

---

## Supported operations

The Azure Container Apps adapter routes the following provider operations. All others are
refused with `UnsupportedFeature` before contacting the provider API.

| Operation | Behavior |
| --- | --- |
| `list` | Lists sandboxes selected by deterministic `nixling-workload` / realm labels and maps provider state to `WorkloadSummary`. |
| `create` | Ensures a workload sandbox exists, creating/reusing the disk image and sandbox through the Azure Container Apps data plane. |
| `start` | Ensures the sandbox exists and resumes it when idle. |
| `stop` | Resolves the workload alias to a sandbox and posts Azure Container Apps stop; already-absent/already-stopped is success. |
| `exec` | Runs synchronous `executeShellCommand` against the selected sandbox. Command bytes are opaque payload and are not logged or audited as metadata. This is not persistent shell. |

Gateway/router layers own idempotency for mutating operations. The Azure Container Apps
provider itself uses deterministic workload labels to discover upstream
state before creating or retrying mutating lifecycle calls.

Persistent shell support is a separate provider trait surface. A
guestd-compatible sandbox that advertises `persistent-shell` handles
`ShellList`, `ShellAttach`, `ShellDetach`, and `ShellKill` through a
`PersistentShellProvider`-style seam and binds attach to an authorized
`shell-pty` stream. This seam is not `WorkloadProvider::exec`, not durable
execution, and not a provider-native shell channel. The current Azure Container
Apps execute-only adapter does not implement it and must continue to return
typed capability denials for `Shell*` operations.

---

## guestd-compatible provider-agent bootstrap contract

A provider-managed sandbox may advertise `persistent-shell` only after its
provider reports a complete, non-secret guestd-compatible bootstrap contract:

1. The sandbox image places the guestd-compatible nixling agent binary in the
   image under provider control.
2. Auth bootstrap material is short-lived and relay-scoped; long-lived realm,
   provider, and Relay rule credentials remain gateway-side only.
3. The agent learns only an ADR 0032 peer-transport rendezvous, not a raw
   guest-control/vsock endpoint and not a provider-specific shell channel.
4. The sandbox has a workload identity suitable for acquiring its scoped relay
   sender material.
5. The persistent-shell helper is available in the sandbox image.
6. The agent reports bounded effective shell limits (`maxSessions` 1–256,
   `maxAttached` 1–64, and `maxAttached <= maxSessions`).
7. Health and capability advertisement come from the in-sandbox agent, with
   generation metadata for the guest boot, guestd instance, and shell daemon.

If any prerequisite is absent or malformed, the provider advertises no
`persistent-shell` capability. The current Azure Container Apps
`executeShellCommand` adapter intentionally reports the fail-closed bootstrap
shape: it may advertise `exec` and provider-managed isolation, but it does not
claim persistent shell until an ACA image can boot the guestd-compatible agent.

---

## Absent capabilities (explicit non-scope)

The following items are outside the provider-managed sandbox model and
are never routed through this adapter. Requests for them fail closed
with `UnsupportedFeature` or `CapabilityDenied`; there are no fallbacks.

- **No broker operation forwarding.** The adapter never forwards raw
  `nixling-priv-broker` frames to the container runtime.
- **No raw guest-control or vsock frames.** The current
  executeShellCommand-only Azure Container Apps adapter has no guestd instance
  and no vsock channel to attach or tunnel. Future persistent-shell-capable
  provider sandboxes must use a guestd-compatible nixling agent over the ADR
  0032 peer transport; they still do not expose raw guest-control frames or a
  provider-specific shell channel.
- **No exec-to-shell fallback.** `executeShellCommand`,
  `WorkloadProvider::exec`, and durable execution are one-shot/durable exec
  surfaces, not ADR 0039 persistent shell. A sandbox without a
  guestd-compatible agent and `persistent-shell` capability refuses `Shell*`
  operations with `CapabilityDenied` or `UnsupportedFeature`.
- **No pidfd or fd passing.** No file descriptors are exchanged with
  the container runtime.
- **No SSH fallback.** No SSH session is opened when the provider API
  is unavailable or when no exec surface is present.
- **No full-host registration.** The adapter does not register the Azure Container Apps
  environment as a full nixling host; it does not run `nixlingd`,
  install packages, or execute `nixling host prepare`.
- **No generic container tunnel.** Raw container exec, Azure Container
  Apps debug proxy, or any other tunnel endpoint is outside scope. The
  adapter uses only the Azure Container Apps management API surfaces listed above.
- **No device hotplug.** Storage attachment, GPU assignment, and device
  tree mutations are outside scope.
- **No cgroup or namespace authority.** The provider runtime owns the
  container lifecycle; nixling does not read, write, or delegate any
  cgroup subtree for these workloads.

---

## Rate-limit, retry, and circuit-breaker behavior

### Provider-layer retry metadata

The Azure Container Apps adapter tracks retry context internally. Retry hint fields
(suggested delay, retry-after header values, attempt counts) are part of
the provider layer's internal retry state and are **not** forwarded as
fields on `ConstellationError`. The public `ConstellationError` schema is
unchanged. Callers should inspect the `ErrorKind` and
the bounded `message` to determine whether a retry is appropriate.

### Azure Container Apps 429 and retry-after handling

When the Azure Container Apps management API responds with HTTP 429 (Too Many Requests),
the adapter:

1. Reads the `Retry-After` response header (seconds or HTTP date form).
2. Converts it to bounded provider-layer `RetryHint` metadata and opens
   the shared circuit breaker for the upstream.
3. Returns `Backpressure` with a bounded message indicating that the
   provider rate limit was exceeded. The provider does not sleep inside
   tests or blindly retry side-effecting operations.

The raw `Retry-After` value and endpoint details are not recorded in
errors or audit records. Low-cardinality telemetry records only bounded
retry-hint and applied-backoff duration buckets.

### Circuit breaker

The adapter uses a circuit breaker shared across all provider instances
targeting the same upstream: endpoint, subscription, resource group, and
sandbox group.
The circuit transitions through three states:

| State | Behavior |
| --- | --- |
| **Closed** | Operations reach the provider API normally. Failures are counted against the trip threshold. |
| **Open** | Operations fail immediately with `Backpressure`. The error message includes the remaining open duration and notes that the circuit is open. No requests are sent to the provider API. |
| **Probe in flight** | One probe request is allowed through after the open window expires. Success closes the circuit; failure extends the open window. Concurrent probes are denied with `Backpressure`. |

Probe attempts have a bounded timeout; if a probe is dropped or remains
in-flight past that timeout, the circuit reopens. Repeated transient
failures use bounded exponential backoff with jitter, capped by provider
configuration, so retries do not synchronize into a thundering herd.
Concurrent 429 responses from the same admitted closed-window request
batch can extend an already-open circuit when the later response carries
a longer bounded retry window.

When the circuit is open, the provider error message carries the
remaining open duration in bounded form (for example:
`"provider circuit breaker open (retry after 14000 ms)"`). The state and the
remaining duration are the only circuit details exposed in the error
surface; internal trip counts and thresholds are not forwarded.

Circuit state is shared across provider instances for the same upstream
so that one degraded provider instance does not shed load onto a sibling
instance pointing at the same Azure Container Apps endpoint.

---

## Credential boundary

Provider-managed sandboxes enforce a strict workload/managed identity
boundary:

- **In production deployments**, the adapter authenticates to the Azure Container Apps
  management API with a workload identity credential configured through
  the gateway's sealed credential envelope first, then falls back to the
  managed identity assigned to the gateway guest VM. Ambient developer
  credentials (Azure CLI cached tokens, client-secret environment
  variables, developer-toolchain credential chains) are not used and are
  not present in the production credential resolution order.
- **Non-production / local-validation contexts** may inject a credential
  explicitly in local dev/live-smoke tooling (for example, the live smoke
  example injects Azure CLI). This is not a runtime fallback and is not part of
  the production credential resolution order.
- Managed identity client IDs are declared as non-secret gateway
  configuration (subscription, resource group, sandbox group, region,
  managed-identity client ID). They are not treated as secret material
  and may appear in non-secret configuration sections.
- Relay Send bearer tokens minted by the gateway for sandbox sender
  connections are short-lived and scoped to the Relay namespace. They
  are never stored in the Azure Container Apps environment and are not written to logs,
  audit records, or error messages.
- Long-lived Relay rule keys and any credential whose loss would grant
  durable access are always gateway-side only and are never passed to a
  provider-managed sandbox.

---

## Diagnostics redaction

The adapter surfaces Azure REST API error details through a strict
allowlist. The following rules govern what may appear in
provider error messages, structured log spans, and audit records:

### Allowlisted fields

| Field | Constraint |
| --- | --- |
| `error.code` | Included after bounding/sanitization when Azure provides an allowlisted value (for example `AuthorizationFailed`, `RevisionProvisioningFailed`, `QuotaExceeded`). Allowlisted values are emitted with case-stable canonical spelling; non-allowlisted codes are mapped to the literal `unknown`. |
| `error.message` | Included in sanitized form: length-bounded, control characters stripped, no embedded JSON objects, no URLs, no UUIDs, no subscription IDs, and no internal diagnostic detail. If sanitization leaves an empty string, the field is omitted. |
| `x-ms-correlation-request-id` | Included verbatim when present. This is an opaque Azure-side correlation token with no operational secret value. |
| HTTP status code | Included as an integer (for example `429`, `503`). |

### Excluded fields

The following are never included in errors, logs, or audit records
emitted by this adapter:

- Full HTTP response bodies.
- Request or response endpoint URLs, hostnames, or path segments.
- Authorization headers, bearer tokens, or SAS tokens.
- Azure resource IDs, subscription IDs, resource group names, or
  workspace IDs.
- Azure Container Apps container image references or registry addresses.
- Operation payloads, container environment variable values, or command
  arguments.
- Workload stdout/stderr output.
- Internal retry attempt metadata, circuit-breaker state transitions,
  or trip thresholds.

These redaction rules apply uniformly to errors returned to callers, to
structured log spans, and to audit `OpAuditRecord` entries for
provider-layer operations.

---

## Error and remediation shapes

Errors from the provider-managed sandbox adapter use the standard
`ConstellationError` shape. The public `ConstellationError` schema is
unchanged; retry hint fields remain internal.

| Adapter reason | `ErrorKind` | Meaning | Remediation |
| --- | --- | --- | --- |
| `sandbox-not-found` | `ProviderAllocationFailed` | The target workload label has no matching Azure Container Apps sandbox where the operation requires one. | Check the workload label and Azure Container Apps configuration. |
| `sandbox-provision-failed` | `ProviderAllocationFailed` | Azure Container Apps reported a provisioning failure (`RevisionProvisioningFailed` or allowlisted equivalent). | Check Azure Container Apps activity log via Azure portal. |
| `quota-exceeded` | `ProviderAllocationFailed` or `Unauthorized` | Provider quota or authorization policy rejected the request. | Reduce concurrent workloads, request quota increase, or verify the managed identity role. |
| `rate-limited` | `Backpressure` | Azure Container Apps management API returned 429 and the retry ceiling was reached. | Wait for the indicated window and retry. |
| `circuit-open` | `Backpressure` | Circuit breaker is open for this upstream; message includes remaining open duration. | Wait for the duration in the error message before retrying. |
| `credential-acquisition-failed` | `AuthenticationFailed` | The gateway could not acquire a managed/workload identity token. | Verify explicit managed/workload identity configuration. |
| `upstream-authorization-failed` | `Unauthorized` | Azure Container Apps returned 403 for an otherwise formed request. | Verify the managed identity has the required Azure Container Apps data-plane role. |
| `unsupported-operation` | `UnsupportedFeature` | The operation kind is outside the Azure Container Apps adapter's scope. | Use a full-host node for operations requiring broker/guest-control/exec. |
| `capability-denied` | `CapabilityDenied` | The required capability is absent from the adapter's capability set. | See the capability matrix above. |
| `provider-unavailable` | `ProviderAllocationFailed` | Azure Container Apps management API is unreachable or returned an unrecoverable 5xx. | Check provider status and retry after the circuit window if one is reported. |

All provider error messages are bounded and comply with the
redaction rules above.

---

## Scope limitations

The following items are deferred and not currently supported by the Azure Container Apps
adapter:

- Interactive exec sessions or attached TTY to running containers.
- Persistent named shell sessions; ADR 0039 defines this for a
  guestd-compatible in-sandbox agent and explicitly excludes mapping it to
  `executeShellCommand`. The pure provider trait/DTO seam exists, but the
  Azure Container Apps runtime adapter does not implement it.
- Live stdio streaming (current support is polling-based log read only).
- Automatic workload image build or push from a local Nix store.
- Multi-region or multi-subscription Azure Container Apps targeting from a single
  provider instance.
- Automatic credential refresh without gateway guest restart.
- End-to-end display, audio, or USB forwarding to Azure Container Apps containers.

These limitations are documented here and not gated by runtime checks
beyond `UnsupportedFeature` responses. Operators evaluating production
use cases should verify that the operations they require are listed in
the capability matrix above.

---

## Cross-references

- [ADR 0039 - constellation persistent shell routing](../adr/0039-constellation-persistent-shell-routing.md) - the live core contract for persistent shells on remote/provider targets.
- [Remote full-host nodes](./remote-full-host-nodes.md) — the model
  for nodes that run their own `nixlingd`/broker/guest-control stack.
- [Azure Relay transport](./transport-azure-relay.md) — the Relay
  WebSocket transport used for sandbox sender connections.
- [Constellation core](./constellation-core.md) — typed error shapes,
  capability model, audit redaction, and idempotency contract.
- [Transport conformance matrix](./transport-conformance-matrix.md) —
  cross-transport capability and conformance requirements.
- [Host substrate providers](./host-substrate-providers.md) — discovery
  adapters for host-owned capability reporting (distinct from
  provider-managed nodes).
- [Privileges reference](./privileges.md) — broker op catalogue (not
  applicable to provider-managed nodes, which bypass the broker).
