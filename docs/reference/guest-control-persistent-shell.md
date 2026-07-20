# Guest-control persistent shell

**Diataxis category:** reference.

Persistent guest shells use the authenticated `d2b.guest.v2` `OpenShell`
operation. Terminal traffic uses a bounded ComponentSession named stream; the
retired specialized shell and terminal RPCs are not accepted.

The guest owns shell policy, PTY state, and lifecycle. A disconnect detaches
the client but does not kill the persistent shell. Attach authorizes a fresh
stream on the current session generation. Kill targets only the exact
guest-verified shell record. Ambiguous adoption, stale generations, unknown
streams, and policy mismatch fail closed.

Shell names, handles, terminal bytes, argv, environment, working directories,
PIDs, and unit names must not enter logs, metrics, audit records, or error
details. The host cannot open a direct compositor, SSH, legacy guest-control,
or chunked-unary fallback.

The public daemon API may expose bounded configured shell metadata. That
projection is not authentication or repair authority; all shell actions still
cross an authenticated guest ComponentSession.
