# TTY one-shot runtime service

**Diataxis category:** reference.

The TTY one-shot adapter runs only behind an authenticated
`runtime-systemd-user` ComponentSession. The authenticated peer uid, runtime
process uid, and current uid must be the same non-root uid. Requests cannot
select another uid.

The adapter accepts one read-write terminal descriptor. The descriptor must be
bound to the authenticated request, have `FD_CLOEXEC`, identify a character
terminal, and pass terminal attribute inspection. Missing, duplicate,
cross-request, cross-generation, non-terminal, or non-CLOEXEC descriptors are
rejected before process creation.

The runtime receives only explicitly resolved, bounded argv and a bounded
systemd user-manager environment. `argv[0]` must be absolute. Empty values,
NULs, duplicate environment keys, malformed entries, and oversized payloads
fail closed. The adapter never reads the host passwd shell and never inherits
the helper process environment.

Each accepted request owns exactly one transient scope under the authenticated
user manager's `app.slice`. Scope name, invocation identity, cgroup leaf,
request id, session generation, and uid are verified before the scope becomes
active. Cancellation and service teardown target only that verified scope. A
failed teardown remains active and returns an error; it is never reported as
cancelled.

The inherited-stdin and one-byte stdout status protocol is disabled. There is
no direct `exec`, host-shell, helper-protocol, or legacy fallback.
