# Cache and Workflow Permission Boundaries

## Shadow

- Workflow is non-required and outside `V3_PR_GATE_WORKFLOWS`.
- It restores and saves no Bazel cache.
- Qualification uses the five most recent qualifying cold shadow runs for PRs
  merged into protected `v3`.
- PR-reachable jobs request only `contents: read`.
- No PR-reachable job requests `actions: write`.
- No direct, indirect, post-step, or unknown cache writer is reachable.
- Checkout uses `persist-credentials: false`.
- Cache service credentials never enter a `run:` environment.

## Promotion

Only two cache kinds exist:

| Kind | Maximum | Notes |
| --- | --- | --- |
| Action/disk | 4 GiB | Trim before save, zero idle GC delay. |
| Repository/download | 1 GiB | Separate entry and key. |

Output base is forbidden. Keys bind `.bazelversion`, `MODULE.bazel`,
`MODULE.bazel.lock`, `.bazelrc`, both toolchain pins, all Cargo locks including
guest lock, all deny files, advisory pin, and generated BUILD digest. Primary
keys include a unique successful protected-`v3` run ID. Restore prefixes omit
run ID and commit SHA.

PR jobs restore read-only. Exactly one protected-`v3` job may publish. Cache
actions alone receive cache credentials; Bazel and repository/third-party code
do not.

## Maintenance ordering

1. Stop retired Cargo cache writes.
2. Enumerate cache entries with complete pagination.
3. Reject failed queries, incomplete pages, and ambiguous prefix matches.
4. Delete only committed authorized retired prefixes and superseded Bazel
   generations beyond retention.
5. Requery usage and require existing use plus planned snapshots at most
   8 GiB.
6. Immediately before save, requery and enforce the same bound.
7. Publish from the one authorized writer.

Unauthorized entries are not deleted. Failure names the entry key and
headroom shortfall, not credentials. Maintenance verdict is separate from
`test-rust` in both directions.

Policy fixtures must reject `actions/cache` with a saving post-step,
`actions/cache/save`, saving `Swatinem/rust-cache`, unknown writers, missing
promoted deadline, and PR `actions: write` at job and workflow level. A
compliant restore-only PR plus protected-`v3` writer fixture must pass.
