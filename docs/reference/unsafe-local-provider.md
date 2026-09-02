# Unsafe-local Provider

**Diataxis category:** reference.

Unsafe-local is an explicit, default-denied Host execution posture. It is a
Provider-backed `Host` Resource, not a Guest, not a second Zone hierarchy, and
not a way to bypass the daemon or broker.

## Contract

The Provider runs only as the authenticated requesting UID. Its Resource
specification contains semantic policy and typed references; it never exposes
configured argv, host paths, credentials, namespace IDs, or a caller-selected
systemd unit.

Unsafe-local has no VM isolation. Public status and audit must state that
posture explicitly so a consumer cannot mistake it for a Guest boundary.

## Shells

Unsafe-local shell sessions use the same `ShellSession` Resource lifecycle as
Guest shells:

```bash
d2b shell open Host/tools --name terminal
d2b shell attach ShellSession/terminal
d2b shell kill ShellSession/terminal --apply
```

The helper validates the requester UID again, accepts one validated terminal
fd, bounds output with cursors, and cleans only the exact verified session.
Cross-UID execution, broad same-UID cleanup, root services, and direct
compositor fallback are forbidden.

## Verification

```bash
d2b host status --json
d2b resource list Host --zone local-root
d2b shell status ShellSession/terminal --zone local-root
```

Missing Provider capability, stale session evidence, or an unavailable helper
is a typed failure. The CLI does not retry through SSH or a Guest path.
