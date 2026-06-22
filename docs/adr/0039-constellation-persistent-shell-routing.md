# ADR 0039: Constellation persistent shell routing

- Status: Accepted
- Date: 2026-06-22
- Related: ADR 0028 (guest control plane over virtio-vsock), ADR 0029
  (framework SSH to typed guest-control RPCs), ADR 0030 (guest exec runs as
  the workload user), ADR 0031 (bare commands and detached workload-user
  exec), ADR 0032 (nixling v2 constellation control plane), ADR 0037 (local
  hypervisor runtime seam), ADR 0038 (persistent named guest shell sessions)

## Context

ADR 0032 defines constellation routing between local hosts, gateway guests,
remote full-host nodes, and provider-managed sandboxes. Its remote model is
semantic: daemons and provider agents exchange operation and stream frames, not
raw daemon, broker, or guest-control wire bytes. ADR 0038 defines persistent
named guest shells on a local VM by reusing the guest-control terminal substrate
and isolating shpool behind a guest-side helper.

Those decisions need one routing rule before implementation continues. A
provider API such as Azure Container Apps `executeShellCommand` can run a
one-shot command, but it does not provide ADR 0038 persistence, attach/detach,
force attach, terminal-v1 flow control, shell event reconciliation, or
nixling-owned guest audit semantics. Treating that provider API as a special
persistent-shell channel would create a second shell contract and would bypass
ADR 0032 capability and stream authorization.

The initial ADR wave was documentation and contract reservation only. The
generated constellation core schema now includes the shell capability,
operation, stream, and bounded DTO roots, and the router/provider seams now
understand persistent-shell capability gating and provider guestd-compatible
agents. Daemon, guestd, provider-agent runtime behavior, and CLI gateway
routing remain staged behind later implementation waves.

## Decision

Persistent shells are a semantic nixling operation family that routes through
the ADR 0032 target resolver, operation router, capability model, and stream
mux. There is no provider-specific shell channel.

This semantic peer transport does not create a routable overlay network, flat
VPN, or L3/L4 network collapse. It carries authorized shell operations and
terminal streams only.

### Capability and operation reservation

The generated constellation schema includes a `persistent-shell` capability.
Runtime support still depends on the target node or provider agent advertising
that capability and implementing the corresponding shell operation family.

The shell operation family is:

| Operation | Mutates state | Required capability | Notes |
| --- | --- | --- | --- |
| `ShellList` | No | `persistent-shell` | Returns bounded shell summaries for the target workload. |
| `ShellAttach` | Yes | `persistent-shell` plus terminal stream support | Creates/adopts the named shell and opens one terminal-v1 `pty` stream. |
| `ShellDetach` | Yes | `persistent-shell` | Detaches a live or stale attach handle without killing the named shell. |
| `ShellKill` | Yes | `persistent-shell` | Terminates the named shell session. |

`ShellAttach`, `ShellDetach`, and `ShellKill` carry idempotency keys when routed
across a constellation peer session. Replays with the same request fingerprint
return the retained result or current compatible state; same-key different-body
requests fail closed with the existing idempotency conflict shape.

The `persistent-shell` capability is independent from `exec`, `logs`,
`file-copy`, display, audio, USB, and provider-managed isolation. A node may
support one-shot exec without persistent shell. Attach uses the existing
terminal-v1/`pty` stream machinery, but the stream is authorized by the shell
operation; callers do not open arbitrary PTY tunnels.

### Routing by target kind

Shell routing follows the same target placement rules as ADR 0032:

1. **Local or host-resident target.** The local `nixlingd` authorizes the
   caller, checks `persistent-shell`, and re-originates the shell RPC over the
   local authenticated guest-control channel to the workload's `guestd`.
2. **Remote full-host node.** The gateway or controller sends semantic
   `Shell*` operation frames over the constellation peer session to the remote
   node's `nixlingd`. The remote daemon authorizes again and re-originates the
   local guest-control shell RPC near the guest. Raw guest-control, vsock,
   broker frames, pidfds, and host paths are never forwarded through the
   gateway.
3. **Provider-managed sandbox.** A sandbox may advertise `persistent-shell`
   only when it runs a guestd-compatible nixling agent in or adjacent to the
   sandbox. That agent exposes the ADR 0038 shell control surface and
   terminal-v1 streams over the ADR 0032 peer transport. It need not be a full
   host daemon and it does not imply broker, KVM, systemd, vsock, virtiofs, or
   device authority.
4. **Provider API exec-only sandbox.** A provider whose only command surface is
   synchronous `executeShellCommand` or equivalent may advertise one-shot
   `exec` if its contract supports that, but it must not advertise
   `persistent-shell` or accept `Shell*` operations. Requests fail with typed
   `CapabilityDenied` or `UnsupportedFeature`; there is no fallback to polling
   exec, SSH, or a provider-native shell stream.

### Guestd-compatible provider agents

A persistent-shell-capable provider sandbox runs a guestd-compatible agent that
implements the nixling-owned shell service contract:

- validates shell names and force/detach/kill semantics exactly as ADR 0038;
- runs shells as the sandbox workload identity, never as a provider control
  principal;
- uses shpool or a compatible in-sandbox persistence engine behind the
  nixling-owned shell API;
- preserves terminal-v1 cursor, resize, close, and slow-reader behavior;
- emits bounded shell event and status records for reconciliation;
- keeps terminal bytes, argv, env, cwd, provider endpoint details, and
  credential material out of audit, metrics, spans, and error metadata.

The provider may use its API to create, start, stop, or health-check the
sandbox and to inject bootstrap configuration. Once a shell attach is active,
interactive bytes flow only through authorized constellation streams between the
gateway/controller and the nixling agent. The provider API is not a shell data
plane.

### CLI and facade behavior

ADR 0039 expands ADR 0038's local `nixling shell <vm> ...` form to the public
`nixling shell <target> ...` facade. The first positional is interpreted as the
normal nixling target address. Implementations may stage support: a generation
that only supports local persistent shells may continue to reject gateway-backed
targets with the existing local usage error.
Once ADR 0039 routing is implemented, gateway-backed targets are forwarded
through the selected gateway exactly like other constellation operations and are
gated by the remote node or provider's `persistent-shell` capability.

`nixling realm enter` and `nixling realm run` remain debugging and scripting
escape hatches, not the persistent-shell transport. Operators should not have to
manually enter a gateway to use a supported remote shell target after this ADR
is implemented.

### Audit and observability

Gateway, remote-node, and provider-agent audit records use the same redaction
rules as ADR 0032 and ADR 0038. They may carry bounded target, principal,
operation kind, capability fingerprint, result, trace context, and the
ADR 0038-approved shell identifier or correlation digest where needed for
operator accountability. They never carry terminal bytes, command output,
shpool protocol bytes, argv, env, cwd, host paths, provider endpoints, relay
credentials, or provider access tokens.

Metrics use bounded labels only. Shell names, attach ids, session instance ids,
terminal stream ids, provider resource ids, and raw error bodies are not metric
labels.

### Documentation and implementation boundary

This ADR defines the cross-constellation contract. Implementation updates
generated schemas through the normal `xtask` path, adds capability and
operation DTOs in the owning crates, updates reference docs from generated
artifacts where applicable, and adds tests in the locations required by
`tests/AGENTS.md`. The required test strategy is: Type 2 tests for
constellation core, router, and guestd logic; Type 3 tests for CLI, daemon, and
guestd integration with mocks or loopback where possible; Type 6 drift and
generator checks for schema, CLI, proto, and docs artifacts; and Type 10 tests
only for real PAM, systemd, and PTY boundaries that Layer 1 cannot prove. No
top-level `tests/*.sh` gates are added for this feature.

## Consequences

- ADR 0038 persistent shells can become remote/provider-capable without
  exposing shpool, guest-control ttRPC, or provider-native shell APIs as public
  transport contracts.
- Azure Container Apps `executeShellCommand` remains useful for limited one-shot
  exec, but it is explicitly insufficient for persistent shell.
- Provider-managed sandboxes that want persistent shell must carry a nixling
  agent and advertise the new capability; this raises the bar for provider
  support but keeps authz, audit, and terminal semantics uniform.
- Existing local-shell-only generations may continue to reject gateway-backed
  `nixling shell` targets until the reserved routing contract is implemented.

## Alternatives considered

- **Map `nixling shell` to `executeShellCommand`.** Rejected because the provider
  call is synchronous and command-scoped. It cannot preserve named shell state,
  attach/detach, force attach, terminal-v1 backpressure, or shell event
  reconciliation.
- **Tunnel raw guest-control over the relay.** Rejected because ADR 0032 forbids
  WAN tunneling of local guest ttRPC. Remote effects must be re-originated by the
  node or provider agent that owns the workload boundary.
- **Add a provider-specific shell stream.** Rejected because it would duplicate
  ADR 0038 terminal behavior, create provider-specific audit gaps, and bypass
  the shared capability and stream authorization model.
- **Require full nixlingd and broker in every provider sandbox.** Rejected
  because provider-managed sandboxes do not own the host substrate. A
  guestd-compatible workload agent is enough for shell semantics and does not
  imply host mutation authority.

## References

- [ADR 0028](0028-guest-control-plane-over-vsock.md)
- [ADR 0029](0029-framework-ssh-to-typed-guest-rpc.md)
- [ADR 0030](0030-guest-exec-as-workload-user.md)
- [ADR 0031](0031-bare-command-and-detached-exec.md)
- [ADR 0032](0032-nixling-v2-constellation-control-plane.md)
- [ADR 0037](0037-local-hypervisor-runtime-seam.md)
- [ADR 0038](0038-persistent-guest-shell-sessions.md)
- [Constellation core reference](../reference/constellation-core.md)
- [Provider-managed sandboxes reference](../reference/provider-managed-sandboxes.md)
- [Remote full-host nodes reference](../reference/remote-full-host-nodes.md)
- [CLI contract](../reference/cli-contract.md)
