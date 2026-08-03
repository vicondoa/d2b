# Cache and Workflow Permission Boundaries

## Shadow

- The workflow is non-required and outside `V3_PR_GATE_WORKFLOWS`.
- It restores and saves no Bazel cache.
- Qualification and measurement draw only from qualification records, which are
  `push` events on `refs/heads/v3` produced by merged pull requests. See
  `shadow-promotion-evidence.md`.
- Pull-request runs stay path-filtered and diagnostic. They produce no record.
- PR-reachable jobs request only `contents: read`.
- No PR-reachable job requests `actions: write`.
- No direct, indirect, post-step, or unknown cache writer is reachable.
- Checkout uses `persist-credentials: false`.
- Cache service credentials never enter a `run:` environment.

## Promotion

Only two cache kinds exist:

| Kind | Maximum | Notes |
| --- | --- | --- |
| Action/disk | 4 GiB | Trimmed synchronously before measurement and save. |
| Repository/download | 1 GiB | Separate entry and key. |

The output base is forbidden.

### Bound key inputs

A change to any of these must produce a different key rather than a subtly
stale cache:

- `.bazelversion`, `MODULE.bazel`, `MODULE.bazel.lock`, `.bazelrc`;
- both `rust-toolchain.toml` files;
- all four hub Cargo locks, including
  `tests/tools/no-bash-ast-walker/Cargo.lock`;
- `packages/Cargo.guest.lock`;
- all four per-hub dependency-generator Bazel-side locks;
- the dependency generator binary's pinned URL and sha256;
- all deny configurations and the advisory-database pin;
- the committed yanked-state snapshot, which always exists;
- `.bazelignore`;
- the symlink-prefix and startup-option configuration;
- the third-party build-script annotation digest and the action-environment
  allowlist;
- the generated BUILD tree digest.

Primary keys include a unique successful protected-`v3` run ID. Restore
prefixes omit the run ID and the commit SHA. Any change to the
action-environment allowlist invalidates the entire action cache and is
reviewed against the 4 GiB budget in the same change.

PR jobs restore read-only. Exactly one protected-`v3` job may publish. Cache
actions alone receive cache credentials; Bazel and repository or third-party
code do not.

## Trimming is synchronous and on demand

Bazel's built-in disk-cache collection runs asynchronously in the server while
it idles. A job that proceeds directly to a size measurement, or that shuts the
server down first as the cleanup contract requires, can observe an untrimmed
cache and then correctly refuse to publish, permanently. That is the deadlock
the size rule exists to prevent.

1. Run the explicit on-demand collector as a named step: the upstream
   `//src/tools/diskcache:gc` tool at the pinned Bazel version, or a pinned
   repository-owned equivalent.
2. Observe its completion before any size measurement.
3. Only then measure.

Idle-delay-based collection and the size refusal remain secondary mechanism and
backstop, not the primary mechanism. A Bazel version bump reopens this design
review rather than being an ordinary version bump.

## Maintenance ordering

1. Stop retired Cargo cache writes.
2. Enumerate cache entries with complete pagination.
3. Reject failed queries, incomplete pages, and ambiguous prefix matches.
4. Delete only committed authorized retired prefixes and superseded Bazel
   generations beyond retention.
5. Run the synchronous collector and observe completion.
6. Requery usage and require existing use plus planned snapshots at most 8 GiB.
7. Immediately before save, requery and enforce the same bound.
8. Publish from the one authorized writer.

Unauthorized entries are not deleted. Failure names the entry key and headroom
shortfall, not credentials. The maintenance verdict is separate from
`test-rust` in both directions.

## Policy fixtures

Fixtures must reject `actions/cache` with a saving post-step,
`actions/cache/save`, a saving `Swatinem/rust-cache`, unknown writers, a
missing promoted deadline control, and PR `actions: write` at both job and
workflow level. A compliant restore-only PR job alongside a writer restricted
to pushes on protected `v3` must pass.

Two structural assertions are implementation deliverables of the promotion
change rather than existing checks, and this contract does not claim the
current Cargo workflow already satisfies them:

- no `pull_request`-reachable job requests `actions: write`;
- every promoted Bazel Rust job sets the in-band deadline control.
