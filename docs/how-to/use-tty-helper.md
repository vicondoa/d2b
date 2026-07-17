# Use the TTY runtime service

**Diataxis category:** how-to.

Use the configured unsafe-local terminal operation through d2b. The controller
resolves the configured command and opens an authenticated
`runtime-systemd-user` session; callers do not pass a shell path, uid, or
environment.

Before dispatch, provide one connected read-write terminal descriptor carrying
the request's attachment index. Keep `FD_CLOEXEC` set and bind the attachment
to the negotiated uid, session generation, and request id. The runtime rejects
ordinary files, inherited standard descriptors, and descriptors from another
request.

Cancel through the same authenticated session. Treat a teardown error as an
active or ambiguous operation and retry cancellation; do not start a host shell
or invoke the retired `--tty-exec` status protocol as a fallback.
