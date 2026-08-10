# Guest-control persistent shell protocol

> Diataxis: reference. Guest-control shell RPC and terminal transport contract.

Persistent shell RPCs live on the authenticated guest-control ttRPC service.
They are available only when guestd advertises the shell capabilities for the
running generation.

## RPCs

| RPC | Purpose |
| --- | --- |
| `ShellAttach` | Create or attach to a named persistent shell session. |
| `ShellList` | Return the configured default name and known sessions. |
| `ShellDetach` | Detach a named session's live client without killing the shell. |
| `ShellKill` | Terminate a named shell session. |
| `ShellCloseAttach` | Close the current owner attachment. |
| `TerminalWriteStdin` | Write terminal bytes to an attached shell session. |
| `TerminalReadOutput` | Read merged PTY output for an attached shell session. |
| `TerminalCloseStdin` | Close stdin for a terminal session. |
| `TerminalTtyWinResize` | Forward a terminal resize. |

Shell terminal I/O uses the terminal-generic metadata:

```text
TerminalRequestMetadata {
  common,
  session_id,
  guest_boot_id,
  kind = SHELL
}
```

The existing exec-named RPCs remain wire-compatible; persistent shells use the
terminal-generic methods to avoid duplicating the streaming vocabulary.

## Host Resource and stream mirror

The host daemon exposes persistent shells as qualified
`shell-terminal.d2bus.org.ShellSession` resources:

- `Create` opens a named session for a `Host/<name>` or `Guest/<name>`
  execution reference;
- `Attach` opens the existing session's authenticated named stream;
- `List`, `Status`, `Detach`, and `Kill` are Resource requests;
- stdin, output, resize, cancellation, and close are named-stream messages.

The retired `PublicRequest::Shell`, `ShellOp`, and `ShellOpResponse` socket
family is rejected. Provider policy, uid, argv, environment, cwd, and host
paths never enter the Resource request.

## Local discovery contract

Local desktop clients such as `d2b-wlterm` do not need to scrape human CLI
output. They list ShellSession resources for an explicit Host or Guest
execution reference and use the typed status projection for session state.

The current stable list payload is intentionally minimal:

- `defaultName`;
- `sessions[].name`;
- `sessions[].state`;
- `sessions[].attached`;
- `sessions[].isDefault`.

That is sufficient for local VM/session discovery and for rendering an
open/stop control surface. The contract does not currently expose attached
owner identity, session handles, last activity timestamps, terminal titles, or
cwd. Those fields are reserved until they can be proven non-leaky and useful;
clients should treat their absence as intentional rather than falling back to
logs, metrics, audit records, or terminal output scraping.

If ShellSession list or attach returns an error, clients surface the closed
Resource error class such as `unsupported-capability`,
`resource-provider-unavailable`, `resource-not-found`, `backpressure`, or
`timeout`. Clients do not retry into a different provider or transport.

## States and close causes

Shell session states are closed enum values:

- `attached`;
- `detached`;
- `killed`;
- `pool-unavailable`;
- `feature-disabled`;
- `output-gap`.

Close causes are:

- `client-detach`;
- `evicted-by-force`;
- `evicted-by-admin-detach`;
- `killed-by-admin`;
- `pool-unavailable`;
- `output-gap`.

Stale guest boot, guestd instance, or shell-pool epoch mismatches fail closed
rather than silently binding a client to an unrelated session.

## Stream behavior

Persistent shells are PTY-backed. Output is merged stdout-style terminal output;
stderr reads are unsupported. Ctrl-C, Ctrl-D, Ctrl-\, and other terminal control
bytes are sent in-band. Resize is the out-of-band terminal control operation.

## Redaction contract

DTO `Debug` implementations redact shell names where broad debug output does not
need them, and always redact terminal session handles and terminal byte payloads.
Metrics labels and daemon audit fields never contain raw terminal bytes, helper
stderr/stdout, argv, env, paths, or raw terminal handles. The daemon audit stream
uses a fixed shell correlation digest where it needs to relate attach, detach, or
kill actions.
