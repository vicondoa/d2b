# Historical host isolation inspection

**Diataxis category:** historical reference.

This page preserves the predecessor host/Realm isolation workflow. It is not
a current command or configuration surface.

Use the current Zone and broker inspection commands instead:

```bash
d2b host check --json
d2b host doctor --read-only
d2b zone list
d2b guest list --zone local-root
d2b audit --json
```

The current control plane has exactly `d2bd.service`,
`d2b-broker.socket`, and `d2b-broker.service`. It does not use a
Gateway daemon or a name-only host lookup.
