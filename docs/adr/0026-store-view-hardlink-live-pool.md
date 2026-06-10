# ADR 0026: Hardlink-backed store-view live pool

## Status

Proposed.

## Context

nixling currently has two per-VM store mechanisms in flight:

- legacy `<stateDir>/<vm>/store`, a flat hardlink farm that virtiofsd serves as the guest lower store;
- daemon-native `<stateDir>/<vm>/store-view/generations/<N>`, a generational hardlink farm built by the broker but not yet served by virtiofsd.

The legacy flat farm is fast for live switches because it skips top-level store paths that already exist, but it has weaker generation semantics and is still wired through old readiness/metadata assumptions. The daemon-native generation tree has stronger marker/current semantics, but materializing a full hardlink tree per generation repeats directory-entry work and can be slower for large VM closures.

The system must keep the guest lower store restricted to the VM closure, preserve guest writable `/nix/store` behavior through overlayfs, avoid mutating host `/nix/store` inodes, and support live workload VM updates without restarting virtiofsd.

## Decision

Use hard links only and make `store-view` the canonical store mechanism. The Rust daemon/broker path is the only store-view writer. NixOS activation may create directories and enforce posture, but it must not build, sweep, or activate VM store-view closures.

The store-view layout is:

```text
<stateDir>/<vm>/store-view/
  live/
    <hash>-pkg/
    .nixling-marker-<vm>
  generations/
    <N>/
      system -> /nix/store/<hash>-nixos-system-<vm>-...
      store-paths
      db.dump
      marker.json
  current -> generations/<N>
  gcroots/
  sync.lock
```

`live/` is the flat hardlink pool served by virtiofsd. `generations/<N>/` is metadata-only. Synchronizing a generation hardlinks only missing top-level store paths into `live/`, writes generation metadata (`store-paths`, `db.dump`, `system`, `marker.json`, and `meta.json`), swaps `current`, and then performs best-effort cleanup only when the retained generation set is known.

Retained generations include the new current generation and a conservative rollback/running set. While a VM is running, cleanup must prefer retaining extra paths over deleting a path that the running guest might still reference.

During rollout, the legacy `<vm>/store` path may remain as a fallback artifact, but it is not a store-view writer. There must never be two writers against one persistent `store-view` tree.

## Consequences

Positive:

- avoids recursive hardlink materialization for every generation;
- preserves the live-switch-friendly stable served directory;
- separates generation metadata from the served hardlink pool;
- enables O(1) same-generation sync when metadata and live paths already exist;
- allows deprecating legacy `<vm>/store`.

Negative:

- `live/` can temporarily contain paths from retained older generations, so exact shrinkage is delayed until safe cleanup;
- generation metadata and live pool must stay consistent under crash/retry;
- ACL/posture code must explicitly handle `store-view/live` as a hardlink-farm carve-out.
- until running-generation retention is fully wired, cleanup may be deferred and `live/` may over-retain paths.

## Safety invariants

- No recursive `chmod`, `chown`, or `setfacl` may touch regular files under hardlink-backed store trees.
- `store-view/live` must be owned/postured so only nixling-controlled sync code can mutate directory entries.
- `current` must not point to a generation whose required top-level store paths are absent from `live`.
- Cleanup may remove only top-level `live` basenames not present in the retained-generation union.
- Guest Nix DB metadata must match paths visible through the lower or upper store.
- The metadata share exposed to the guest must not provide writable access to `live/`.
- Store-view writes are serialized by `store-view/sync.lock`.

## Validation requirements

- unit tests for marker/current behavior, same-generation fast path, live-pool sweep, and hardlink inode preservation;
- ownership-matrix tests for `store-view/live` carve-out;
- eval tests proving `ro-store` is routed to `store-view/live`;
- runtime validation on one small VM and one heavy VM;
- panel review for plan/design, security/ACL, Nix correctness, performance, and operations/recovery before merge.
