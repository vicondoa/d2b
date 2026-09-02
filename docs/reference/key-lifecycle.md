# Guest key lifecycle

**Diataxis category:** reference.

d2b keeps operator identity, Guest host-key trust, and Guest runtime
credentials separate. Private keys are host state owned by the configured
identity owner; they are never placed in a public Resource, manifest, schema,
or CLI response.

## Managed identity

The host may declare additional authorized public keys through
`d2b.site.userAuthorizedKeys`. The Guest activation Provider combines those
keys with its managed public key inside the Guest execution context. The
daemon resolves the target as `Guest/<name>` in the selected Zone.

```bash
d2b activation keys list --zone work
d2b activation keys show Guest/work-app --zone work
d2b activation keys rotate Guest/work-app --zone work --apply
```

Rotation is a daemon/broker operation with exact Guest UID, Provider
generation, and revision fencing. Operator-supplied keys remain owned by the
consumer and are not silently replaced.

## Host-key trust

Trust operations resolve a Guest Resource and update the host's bounded
known-hosts state through the broker. They do not accept a raw path or
identity file from the public request:

```bash
d2b activation trust Guest/work-app --zone work --apply
d2b activation rotate-known-host Guest/work-app --zone work --apply
```

The broker audits key rotation and trust changes with redacted Guest and
operation identity. Failed, stale, or unauthorized requests leave the old
trust state unchanged.

## Security

Do not wipe persistent TPM or credential state to repair SSH trust. TPM
identity is owned by its Provider and a replaced state directory fails closed.
Keep backups encrypted and access-controlled. Use `d2b audit --json` and
`d2b guest status` for bounded verification.
