# Use persistent shells

Persistent shells are `ShellSession` Resources owned by a Zone. Use
`d2b exec run` for one-off commands and a named shell when you need to
disconnect and reconnect.

## Open a shell

```bash
d2b shell open Guest/work-app --zone work --name build
d2b shell attach ShellSession/build --zone work
```

The execution target must be a typed `Host/<name>` or `Guest/<name>`
ResourceRef. Shell names are bounded ASCII identifiers and are scoped to the
Zone session.

## Inspect and manage

```bash
d2b shell list --zone work
d2b shell status ShellSession/build --zone work
d2b shell detach ShellSession/build --zone work --apply
d2b shell kill ShellSession/build --zone work --apply
```

Detach is idempotent. Kill requests the shell Provider to close the exact
session and its Process resources; it does not perform broad cleanup.

## Security

Guest shells use an authenticated ComponentSession and ProcessAttachClient.
Unsafe-local shells use the exact verified requester UID and provide no VM
isolation. Neither path exposes a host path, credential, argv, environment,
terminal handle, or private runtime locator. A stale session or lost
ComponentSession is a typed failure, not a reason to retry through SSH.

See [the shell Provider reference](../reference/components-shell.md) and
[the CLI contract](../reference/cli-contract.md).
