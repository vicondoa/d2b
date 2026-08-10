# v3 replacement contracts for desktop clients

**Diataxis category:** reference.

**Contract version:** d2b 3.0 (v3) replacement surface.

**Document revision:** 1.

**Publication status:** Published early for companion adaptation. No preview
artifact is published.

This page is the actionable replacement contract for desktop clients that
consume d2b's public CLI, public daemon socket, launcher metadata, or
presentation artifacts. It publishes the interface shape and rejection rules
without publishing a binary, package, cache entry, or other consumable
intermediate release.

## Resolving the release constraint

The release blocker and the no-intermediate-artifact rule are both retained:

- d2b 3.0 remains blocked until every companion in the
  [companion inventory](./companion-contracts.md) is compatible and has been
  exercised against the release candidate on a live host.
- No preview build, preview tag, binary archive, Nix substituter output, or
  other intermediate release artifact is published.
- The mitigation is contract publication: companion maintainers can implement
  and test their adapters now against this page, the committed DTOs, and the
  generated schemas. They do not need a d2b binary to learn the replacement
  wire shape.
- If a companion cannot adapt from the published contracts, the correct
  response is to hold the 3.0 release. Publishing an unannounced preview or
  treating contract publication as compatibility evidence is not a substitute.

This is the intended resolution of the FR-039 and FR-045 tension. Contract
publication is early adaptation input, not a release and not a compatibility
waiver.

## Source precedence

Use these sources in this order when implementing a client:

1. the generated schema or typed DTO for the surface being consumed;
2. the matching reference page linked below; and
3. the human CLI examples only for presentation.

The v3 ResourceType schemas are in
[`schemas/v3/`](./schemas/v3/). The public command behavior is documented in
[`cli-contract.md`](./cli-contract.md), and the public/private artifact
boundary is documented in [`manifest-bundle.md`](./manifest-bundle.md).
When prose and a committed, passing implementation differ, the implementation
is authoritative and the discrepancy must be reported rather than hidden by a
client fallback.

## Public daemon transport

Desktop clients that need the daemon API use the public Unix socket. They do
not read the private bundle or root-owned runtime state.

| Property | Contract |
| --- | --- |
| Socket | `/run/d2b/public.sock`, an AF_UNIX `SOCK_SEQPACKET` socket. |
| Frame | A 4-byte little-endian unsigned body length followed by one UTF-8 JSON object. The length excludes the prefix. |
| Maximum body | 1 MiB. Reject a larger frame before decoding its JSON. |
| First message | A `type: "hello"` object with `clientVersion` as a semantic-version range and `supportedFeatures` as feature names. |
| Successful reply | `type: "helloOk"` with `serverVersion`, `selectedVersion`, and the negotiated `capabilities`. |
| Rejected reply | `type: "helloRejected"` with a typed error and a reason such as version mismatch or capability negotiation failure. |

The client must use the `selectedVersion` and returned feature list from
`helloOk`. It must not infer support from a package version or assume that a
missing capability will fall back to an older transport. A client that cannot
negotiate the required version or feature fails visibly and remains usable for
unrelated local functions.

The feature names currently used by the public daemon are:

- `typed-errors`
- `status-check-bridges`
- `export-broker-audit`
- `configured-launch-v1`
- `unsafe-local-provider-v1`

Only advertise a feature the client implements. Workload operations require
the relevant negotiated feature set; an unsupported request returns a typed
refusal rather than silently changing transport.

After the handshake, request frames use a type discriminator. The operation
name and its arguments are closed, camel-case JSON DTOs:

```json
{
  "type": "resourceRequest",
  "service": "d2b.zone.v3",
  "method": "List",
  "zoneRef": "Zone/local-root",
  "resourceType": "shell-terminal.d2bus.org.ShellSession",
  "executionRef": "Guest/corp-vm"
}
```

Responses use the matching response discriminator, operation name, and
`result`. Errors use `type: "error"` and a typed error envelope. Unknown
fields and malformed frames are refusals, not an invitation to guess a
legacy shape.

## Persistent shell replacement

The shell client must:

1. address a `Host/<name>` or `Guest/<name>` execution reference when opening;
2. address a qualified `shell-terminal.d2bus.org.ShellSession/<name>` for
   attach, status, detach, and kill;
3. open terminal I/O through the authenticated ProcessAttachClient named
   stream;
4. use the shell name grammar `^[a-z][a-z0-9-]{0,62}$`; and
5. render `Attached`, `Detached`, `Killed`, `PoolUnavailable`,
   `FeatureDisabled`, and `OutputGap` as distinct states.

`Create` and `Attach` own a long-lived named stream. Subsequent stdin, output,
resize, and close messages remain on that stream. Management requests are
ordinary Resource requests and never reuse the retired `type: "shell"`
envelope. Output chunks carry bounded base64 data and an offset, so a client
must advance from the returned cursor rather than replaying an unknown range.

Persistent shell operations are admin-only. There is no SSH, host-shell,
per-VM service, or broker-operation fallback for a refused shell request.

## Workload and launcher replacement

Launcher clients use provider-neutral workload operations and canonical
targets:

```text
<workload>.<realm>[.<ancestor>...].d2b
```

The first label is the workload and the remaining labels are the realm path,
most-specific first. The target is an identifier, not a DNS name, IP address,
SSH address, vsock address, or physical-node selector. Use
[the realm access resolver contract](./realm-access-resolver.md) for aliases,
ambiguity handling, and capability preflight.

`type: "workload"` requests expose:

- `list` with an optional realm selector;
- `status` for one canonical target; and
- `launcherExec` with a target, an item id, and an operation id.

The public workload summary contains provider-neutral state, execution and
display posture, capability tokens, `launcherItems`, and `defaultItemId`.
Each launcher item contains an id, display name, optional icon, `type`
(`exec` or `shell`), graphical flag, and capability tokens. It never contains
argv, environment, cwd, uid, or private proxy paths. A launcher should select
an item id and send the canonical target; it must not reconstruct private
execution data.

The launcher metadata artifact is
`/etc/d2b/realm-workloads-launcher-v2.json` with `schemaVersion: "v2"`.
It is daemon-owned and mode `0640`; authorized clients consume its public-safe
projection through the daemon API and must not open the private file directly.
Its `runtimeState` may be `contract-only` until the corresponding dispatch is
enabled. That state is a visible contract status, not permission to invent a
local argv fallback.

## UI colors and Wayland integration

The presentation contract consists of:

- `/etc/d2b/ui-colors.json`, whose top-level `version` is `1`;
- `/etc/d2b/ui-colors.css`, with GTK-compatible `@define-color` names; and
- the packaged `d2b-wayland-proxy` integration for graphics clients.

Colors identify host, environment, realm, workload, VM, and lifecycle state.
They are presentation metadata only and never authorize an operation. A
consumer must fail visibly but remain usable when a color artifact is absent
or malformed. It must not read private d2b state to recover a color. CSS
names use underscores, with hyphens in identifiers normalized to underscores.
See [the UI color contract](./ui-colors.md) and [the proxy warning
catalog](./wayland-proxy-warnings.md).

## Audio status replacement

The machine-readable companion surface is:

```text
d2b audio status --json
```

The public operation is `type: "audio"` with `op: "status"` and an `args.vms`
array. An empty array requests all accessible targets. A successful response
has `entries` and may have per-target `errors`; one misconfigured provider
does not hide the state of the other targets. Each entry carries:

- `vm`;
- `speaker` and `microphone` channel state, including `level` and `muted`;
- `providerKind`; and
- `enforcement`, such as `host-and-guest`, `host-only`, `guest-only`, or
  `unsupported`.

Clients must display the enforcement posture and any remediation instead of
assuming that a successful status query means guest enforcement is active.
The audio public wire DTO is also used by `d2b audio mic`, `speaker`, and
`off`; those operations remain daemon-mediated and do not grant a client
direct access to PipeWire or guest state.

## Security-key status and actions

The public security-key operations are:

- `d2b usb security-key status`;
- `d2b usb security-key sessions`; and
- `d2b usb security-key cancel`.

The status response is a bounded list of configured device reachability,
current lease, per-VM virtual-device state, and session state. Desktop
presentation uses `WlcontrolSkStatus` with `overall`, `active`, and
`recentTerminal`. Each active row is a `WlcontrolCeremonyRow`; its actions
are `WlcontrolAction` values containing `actionKey`, `label`, and
`sessionId`.

An action key is an opaque, pre-minted nonce-bearing value. Forward it
verbatim to the CLI or equivalent daemon request. The desktop client must not
open a hidraw device, alter a lease, or perform a privileged cancellation
itself. Missing or stale action keys are visible action failures, not a reason
to retry a different privileged path.

Until the daemon-side handlers are enabled, a status, sessions, or cancel
request may return the typed `not-yet-implemented` refusal. That response is
an interim implementation state, not a compatibility pass. Clients must render
the refusal without a privileged fallback, and the release candidate must
exercise the real status and action paths.

## Graceful stop semantics

When a client offers a stop action, normal `d2b vm stop --apply` (legacy form) asks a
supported local provider to shut the guest down and then performs the bounded
host cleanup. `--force` skips only that provider-aware graceful wait and uses
the standard cleanup path; it is not an immediate SIGKILL shortcut. Clients
must preserve this distinction in labels, confirmation, and status handling.

The lifecycle metadata is exposed in the runtime manifest and includes
`gracefulShutdown.enable` and an optional `timeoutSeconds`. See the [manifest
schema](./manifest-schema.md) for the provider capability and lifecycle
metadata.

## Clipboard picker boundary

The picker remains a separate protocol. It is version 1 newline-delimited
JSON over the inherited anonymous Unix `socketpair()` descriptor. The picker
must implement the bounded `ClientHello`, `OpenRequest`, `Select`, and
`Cancel` messages in [the picker protocol reference](./clipboard-picker-protocol.md).
It receives destination metadata, canonical targets, accent colors, and
filtered candidates, but not transfer file descriptors, clipboard authority,
or privileged host sockets.

## Adaptation checklist

Before a companion declares its adapter ready, its maintainer should be able
to answer yes to all of the following:

- The adapter uses the public socket and handshake rather than private bundle
  files.
- Every optional operation is gated by negotiated capabilities and the
  advertised runtime operation capability.
- Closed DTOs are decoded with the documented version and unknown fields are
  handled as a visible incompatibility.
- Canonical targets, opaque ids, session handles, and action keys are passed
  through without reinterpretation.
- A missing provider capability produces a useful disabled or degraded UI,
  not a legacy SSH, shell, or direct-device fallback.
- The adapter can run against a local contract fixture without requiring a
  published d2b artifact.
- Final compatibility is still tested by exercising the companion against the
  release candidate on a live host.

This checklist is adaptation guidance only. The final live-host result remains
the release-gate evidence.
