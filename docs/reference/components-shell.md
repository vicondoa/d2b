# Shell Provider

**Diataxis category:** reference.

Named shells are `ShellSession` Resources owned by the Zone and attached to a
Host or Guest execution target. The shell Provider owns the authenticated
ProcessAttachClient stream, terminal fd, bounded output cursors, and cleanup.

```bash
d2b shell open Guest/work-app --name build
d2b shell attach ShellSession/build
d2b shell status ShellSession/build --zone work
d2b shell detach ShellSession/build --zone work --apply
d2b shell kill ShellSession/build --zone work --apply
```

Shell names are bounded ASCII identifiers. The shell path never carries
credentials, host paths, argv, environment, or a private runtime locator.
Unsafe-local shells run only as the verified requesting UID.
