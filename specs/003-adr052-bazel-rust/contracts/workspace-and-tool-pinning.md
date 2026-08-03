# Workspace Boundary and Tool Pinning Contract

This contract covers the parts of the build configuration that are easy to get
wrong in ways that stay green: where startup options live, what the workspace
boundary is, how the four dependency hubs are locked, and which tool
acquisitions are permitted.

## Startup options come from the wrapper

`%workspace%` is expanded only for rc `import`/`try-import` paths and for a
small set of options the Java side resolves. `--output_user_root` and
`--output_base` are startup options parsed by the client, so a
`startup --output_user_root=%workspace%/.scratch/bazel` line creates a literal
`%workspace%` directory. Therefore:

- `.bazelrc` contains **only** `common`, `build`, `test`, and `build:<config>`
  lines. `common --lockfile_mode=error` is valid there, because `common`
  applies to every command that supports an option and is ignored elsewhere.
- `.bazelrc` contains **no** `startup` line.
- The Make and Rust wrapper supplies every startup option as an absolute path
  derived from the worktree.
- The wrapper supplies **byte-identical** startup options to `build`, `test`,
  `query`, `info`, `shutdown`, and `clean`, and from W2 to the single child
  `cargo xtask bazel-repin` spawns. This is what makes "shut down with the same
  startup options" enforceable: startup options select the server and
  output base, so a mismatched shutdown starts a second server and leaves the
  live one owning the tree.
- A mutation that perturbs one invocation's startup options must fail closed.

## Workspace boundary

With the output user root under `.scratch/bazel/`, the output base, every
external repository, and the convenience symlinks live inside the source tree.
Bazel does not automatically exclude a real directory under the workspace from
package loading or from `glob()`, and the worktree carries many Cargo output
directories. Therefore:

- `.bazelignore` is a generated, drift-checked artifact. It covers `.scratch/`
  and every Cargo output directory any workspace or tool in the worktree
  creates, including the broker's deterministic sibling output directories and
  the standalone tool and proof workspaces.
- The wrapper passes an absolute `--symlink_prefix` pointing beneath
  `.scratch/`, so no convenience link is created at the repository root.
- A drift mutation that drops one directory from `.bazelignore` must fail
  closed. A generator that emits an empty list is invalid.

## Four hubs, four Bazel-side locks

| Hub | Cargo lock | Bazel-side lock |
| --- | --- | --- |
| main | `packages/Cargo.lock` | required |
| broker | `packages/d2b-priv-broker/Cargo.lock` | required |
| guest | `packages/d2b-guest-shell-runner/Cargo.lock` | required |
| walker | `tests/tools/no-bash-ast-walker/Cargo.lock` | required |

- Every `crate.from_cargo` tag sets `lockfile = ...`. Omitting it makes repin
  unconditional and silently removes the drift guard, so a missing attribute is
  a fail-closed condition rather than a style issue.
- Every tag also sets `cargo_lockfile = ...` and
  `skip_cargo_lockfile_overwrite = True`. The first is required for the
  extension to report itself reproducible, which is what
  `--lockfile_mode=error` actually constrains. The second defaults to false at
  `rules_rust` 0.73.0, and at that default a repin writes the plain
  `Cargo.lock` back, which would make the Bazel side a second dependency
  authority. Both are fail-closed conditions.
- Folding the walker's dependencies into the main hub is forbidden: it
  re-resolves those crates and destroys the `--locked` equivalence the
  migration preserves.
- `packages/Cargo.guest.lock` is a generator input and a cache-key input. It is
  **not** a hub, because no Rust gate surface builds against it.

## Two lock mechanisms, kept separate

- `MODULE.bazel.lock` under `common --lockfile_mode=error` pins the module
  graph, including the transitive registry modules the Rust rules bring in.
  This is the mechanism that pins them; the module file alone does not.
- Each hub's committed Bazel-side lock is the `--locked` equivalent for
  crates, enforced by the dependency generator's own staleness check, which
  fails with a named remediation.

Neither substitutes for the other. A drift mutation must be proven separately
for each.

## Forbidden controls

- `CARGO_BAZEL_REPIN`, `REPIN`, and `CARGO_BAZEL_REPIN_ONLY` are never set in
  the Make wrapper environment or in any continuous-integration environment. A
  policy assertion proves it. The assertion matches the variable names, not a
  particular value: at 0.73.0 any value outside `false`, `no`, `0`, and `off`
  is truthy, so a guard keyed on `=1` is already wrong.
- No `.bazelrc` line and no wrapper argument sets
  `@rules_rust//rust/toolchain/channel`. The flag has universal scope, so
  setting it compiles every first-party crate on nightly while the gate stays
  green. A guard fails closed on it.
- `--test_output=streamed` is forbidden during any measured run, because it
  silently makes every test exclusive.

## The one supported regeneration path

Forbidding the repin controls without naming a supported way to regenerate a
committed lock is what produces `CARGO_BAZEL_REPIN=1 make ...` in somebody's
shell history. Regeneration is therefore a repository-owned command:

```text
cargo xtask bazel-repin --hub <main|broker|guest|walker>
```

Its contract, all of it enforcing:

- `--hub` is mandatory and must name one of exactly those four hubs. Anything
  else is refused by name; there is no all-hubs mode.
- The command refuses to start when the ambient environment already carries
  `CARGO_BAZEL_REPIN`, `REPIN`, or `CARGO_BAZEL_REPIN_ONLY`, so it cannot be
  used to launder a control a contributor exported.
- It sets `CARGO_BAZEL_REPIN` and `CARGO_BAZEL_REPIN_ONLY=<hub>` **only** on
  the child process it spawns, through the process builder, never with a
  process-global mutation. `CARGO_BAZEL_REPIN_ONLY` is an exact-match
  comma-delimited hub allowlist at 0.73.0, so the scoping is enforced by the
  substrate and not only by the command. `packages/xtask/src/bazel.rs` is the
  only repository site that may make that assignment; the guard is written
  against the assignment form, because the guard file itself necessarily
  contains all three literal names.
- It passes the same absolute output user root, output base, and
  `--symlink_prefix` the rest of this contract fixes. In W0 that derivation
  lives in the command, because no wrapper exists yet; from W2 the command
  calls the one shared construction instead, and the startup-option identity
  test covers the repin child alongside every other command.
- It records the digest of every committed derived artifact before the child
  runs and fails afterwards if any tracked file other than the named hub's
  Bazel-side lock changed. A changed `Cargo.lock`, `MODULE.bazel`,
  `MODULE.bazel.lock`, `.bazelignore`, generated `BUILD.bazel`, or
  `bazel/generated/**` is a failure, not a result.
- It reports whether that lock changed, and does not treat "unchanged" as an
  error. On a tree whose lock is already current, the correct outcome is exit
  zero with nothing changed, which is what makes the command safe to re-run in
  validation. The separate assertion that the command is invoking the right
  thing belongs where inputs actually moved: the run immediately after a
  workspace-membership change must report a changed lock, and a wrong
  invocation is caught there rather than by punishing idempotence.
- It is **not** a Make target and no workflow may invoke it. The approved
  target policy needs no exception because there is nothing to except.

The invocation the child issues is measured in W0 and recorded in the wave
notes, not copied from upstream prose: the 0.73.0 docstring recommends the
WORKSPACE-era `bazel sync --only=<hub>` while the bzlmod `regen_command`
default is `bazel mod show_repo`, which repins nothing. Because the command
reports rather than assumes, a wrong invocation shows up as "no hub lock
changed" on the one run where a change is required, instead of succeeding
quietly.

## Tool acquisition

- Bazel is `bazel_8` (8.6.0) from the pinned nixpkgs, reached through the dev
  shell. `.bazelversion` records `8.6.0` and the wrapper fails closed when
  `bazel --version` disagrees.
- The dev shell exposes `bazel_8` and `bazel-buildtools` before any Bazel
  target lands. Bazelisk is not required, because it is not on the gate path.
- The dependency generator binary is consumed from its registry-pinned URL and
  sha256. The non-reproducible source-bootstrap fallback, which needs a host
  `cargo` and is not marked reproducible, is refused by a structural guard. Its
  URL and sha256 are cache-key inputs and its download is charged to the cold
  profile.
- Repository-rule fetch is permitted and is always pinned, by URL plus checksum
  or by git rev. No action in the Rust gate opens a network socket.

## Version-bump review

Any Bazel version bump reopens the disk-cache garbage-collection design review
in `cache-workflow-boundaries.md`, because that design depends on
`--experimental_*` flags and on an upstream tool label. It is not an ordinary
version bump. A `rules_rust` bump reopens review of the hand-written
fragments the coverage map lists, because each tracks upstream internals.
