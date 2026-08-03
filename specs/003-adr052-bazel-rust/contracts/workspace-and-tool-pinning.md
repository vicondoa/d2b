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
  `query`, `info`, `shutdown`, and `clean`. This is what makes "shut down with
  the same startup options" enforceable: startup options select the server and
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
  policy assertion proves it.
- No `.bazelrc` line and no wrapper argument sets
  `@rules_rust//rust/toolchain/channel`. The flag has universal scope, so
  setting it compiles every first-party crate on nightly while the gate stays
  green. A guard fails closed on it.
- `--test_output=streamed` is forbidden during any measured run, because it
  silently makes every test exclusive.

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
version bump. A `rules_rust` bump reopens review of the three hand-written
fragments the coverage map lists, because each tracks upstream internals.
