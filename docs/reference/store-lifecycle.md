# Guest store lifecycle

**Diataxis category:** reference.

The Volume Provider gives every Guest a closure-only `/nix/store` view. The
host's complete store is never exposed. Store state is private and broker
owned; it is not a public Resource or CLI input.

## Layout

The current store view is rooted below the Zone/Guest state owner:

```text
store-view/live/       guest-readable closure hardlink pool
store-view/meta/       guest-readable generation metadata
store-view/state/      host-only generation and integrity state
store-view/gcroots/    host-only Nix GC roots
store-view/sync.lock   broker-private OFD lock
```

Only the selected immutable Guest closure is linked into `live/`. The Guest
sees no host-only state, credentials, broker handles, or unrelated store path.

## Safety invariants

1. Hardlinks stay on the same filesystem as the host Nix store.
2. The Volume Provider writes only d2b-owned anchored paths.
3. Generation pointers change through staged symlink plus atomic rename.
4. Readiness markers are written last.
5. Restart adoption precedes cleanup.
6. No recursive chmod, chown, or setfacl crosses the hardlink pool.

If ownership, type, marker, lock, or generation evidence is missing or
foreign, the broker fails closed. It never repairs by sweeping a directory or
overwriting a foreign marker.

## Activation and rollback

```bash
d2b activation switch Guest/work-app --zone work --dry-run
d2b activation switch Guest/work-app --zone work --apply
d2b activation rollback Guest/work-app --zone work --apply
d2b activation gc --apply
```

Activation and garbage collection are typed, audited, and fenced by Guest
identity, Provider generation, and revision. Rollback keeps the Guest's prior
retained generation; host NixOS generations are managed separately.

## Inspection

```bash
d2b guest status work-app --zone work
d2b host doctor --read-only
d2b audit --json
```

See [the Volume Provider docs](./store-virtiofs.md),
[the daemon lifecycle](../explanation/daemon-lifecycle.md), and
[ADR 0034](../adr/0034-storage-lifecycle-restart-and-synchronization.md).
