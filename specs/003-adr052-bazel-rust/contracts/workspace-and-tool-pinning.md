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
  `common --check_direct_dependencies=error` sits beside it; see the measured
  reason under the module lock below.
- `.bazelrc` contains **no** `startup` line.
- The Make and Rust wrapper supplies every startup option as an absolute path
  derived from the worktree.
- The wrapper supplies **byte-identical** startup options to `build`, `test`,
  `query`, `info`, `shutdown`, and `clean`, and from W2 to the single child
  each of `cargo xtask bazel-repin` and `cargo xtask bazel-module-refresh`
  spawns. This is what makes "shut down with the same
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
for each, and each has exactly one repository-owned regeneration command:
`cargo xtask bazel-repin --hub <name>` for a hub lock and
`cargo xtask bazel-module-refresh` for `MODULE.bazel.lock`. No committed lock
has two supported regeneration paths, and neither command can regenerate the
other's lock.

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

## The supported regeneration paths

Forbidding the repin controls without naming a supported way to regenerate a
committed lock is what produces `CARGO_BAZEL_REPIN=1 make ...` in somebody's
shell history. Every committed lock this migration adds therefore has exactly
one repository-owned regeneration command, and no lock has two.

### Hub locks: `cargo xtask bazel-repin --hub <name>`

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
quietly. The wave note records the invocation as a command shape with
`<worktree>` placeholders, never as a real absolute path; see the note on wave
notes below.

### Module lock: `cargo xtask bazel-module-refresh`

Two things about `--lockfile_mode=error` at Bazel 8.6.0 were measured rather
than assumed. The first shapes this command; the second shapes `.bazelrc`, and
is recorded here because the two are read together.

First, the mode does not fail merely because `MODULE.bazel` changed. It fails
when the resolution needs a registry file whose checksum the committed lock
does not carry, and it never rewrites the lock:

```text
ERROR: Error computing the main repository mapping: Missing checksum for
registry file https://bcr.bazel.build/modules/<name>/<version>/MODULE.bazel
not permitted with --lockfile_mode=error. Please run `bazel mod deps
--lockfile_mode=update` to update your lockfile.
```

Second, a direct-dependency version the resolution *can* absorb, because some
other module already pulled a higher one, produces a `WARNING` and exit zero
under `--lockfile_mode=error` alone. That is a check that degrades, so
`.bazelrc` also carries `common --check_direct_dependencies=error`. Without it
a hub can silently build against a module version nobody declared, and the
module lock records the resolved graph as if that were intended.

The refresh invocation is the one Bazel names, measured to change only
`MODULE.bazel.lock` and to be idempotent on a current tree:

```text
bazel mod deps --lockfile_mode=update
```

`bazel mod` rejects `--symlink_prefix`, so the options this child shares with
every other Bazel invocation are the startup options that select the server and
the output base, not every command option the build commands take. W0
re-measures the invocation against this repository's real module graph before
the remediation message ships, because the measurement above was taken on a
scratch module graph.

The wider problem is that the invocation Bazel names carries no startup options
at all. A contributor who copies that line runs a bare `bazel` against the
default output user root under the home directory: a second server on the
worktree, a second output base outside `.scratch/`, and both the
workspace-boundary rule and the bounded-scratch rule defeated in one paste. The
remediation this repository ships therefore names a repository-owned command
instead:

```text
cargo xtask bazel-module-refresh
```

Its contract, all of it enforcing:

- It takes no arguments. There is no mode selection, no target selection, and
  no way to ask it to update anything else.
- It issues the measured module-lock update invocation with the same absolute
  output user root and output base every other Bazel command in this contract
  uses. In W0 that derivation lives in the command because no wrapper exists
  yet; from W2 the command calls the one shared construction, and the
  startup-option identity test covers this child alongside the repin child.
- It records the digest of every committed derived artifact before the child
  runs and fails afterwards if any tracked file other than `MODULE.bazel.lock`
  changed. A changed `Cargo.lock`, a changed hub lock under `bazel/cargo/`, a
  changed `.bazelignore`, a changed generated `BUILD.bazel`, or a changed
  `bazel/generated/**` is a failure, not a result.
- It is idempotent. On a tree whose module lock is already current the correct
  outcome is exit zero with nothing changed. That is what makes it safe to
  re-run during review, and it is a hard requirement rather than a convenience,
  because the command is named in a refusal a contributor may well run twice.
- It refuses to start when the ambient environment carries `CARGO_BAZEL_REPIN`,
  `REPIN`, or `CARGO_BAZEL_REPIN_ONLY`, for the same reason `bazel-repin` does:
  the child it spawns would otherwise repin every hub as a side effect of a
  module-lock update.
- It is **not** a Make target, no workflow may invoke it, and the guard refuses
  any `Makefile` recipe or workflow step that names it.
- It is the exact remediation for module lock drift. There is no documented
  mode flip, no hand-edit path, and no second command that also updates
  `MODULE.bazel.lock`.

## One updater and one validator for the yanked snapshot

The committed lock-bounded yanked snapshot has two repository-owned commands,
and they do different things. That is why the drift recovery ends on the second
one rather than stopping at the first:

```text
cargo xtask bazel-yanked-refresh
cargo xtask bazel-yanked-check
```

- `bazel-yanked-refresh` is the explicit reviewed **networked** update. It
  reaches the index, rewrites `bazel/supply_chain/yanked-snapshot.json` with one
  entry per `(name, version)` in the three committed locks together with the
  index revision it observed, writes nothing else, and is never reachable from a
  Make target or a workflow. A person reviews and commits its output.
- `bazel-yanked-check` is the **offline** exact key-set validator. It reads the
  committed snapshot and the three committed locks, proves `(name, version)`
  key-set equality in both directions, opens no socket, and writes nothing.
  It is the single implementation of that comparison and the single site of its
  message: the three Bazel supply-chain carriers execute the built validator
  binary with the snapshot and the three locks as declared inputs, and a
  contributor runs the same command in a shell and reads the same bytes.

A refreshed snapshot nobody verified is the failure this split exists to
prevent. A refresh that is reviewed but leaves a key the locks no longer
declare still fails the gate, and the contributor finds that out in their own
shell rather than in continuous integration.

## Actionable failure contract

Every row's text is the operator-facing recovery string the refusal must
contain, quoted exactly. This column holds strings and nothing else; every
observation about the strings is below the table.

| Failure | Exact recovery text |
| --- | --- |
| Stale Bazel-side hub lock | Run `cargo xtask bazel-repin --hub <hub>`, review and commit the regenerated lock under `bazel/cargo/`, then rerun the failed command. |
| `bazel-repin` changed tracked files other than the named hub's lock | Commit or restore the listed repository-relative paths, then run `cargo xtask bazel-repin --hub <hub>`. |
| Ambient `CARGO_BAZEL_REPIN`, `REPIN`, or `CARGO_BAZEL_REPIN_ONLY` | Unset `CARGO_BAZEL_REPIN`, `REPIN`, and `CARGO_BAZEL_REPIN_ONLY`, then run `cargo xtask bazel-repin --hub <hub>`. |
| Generated BUILD, governed-source, `.bazelignore`, harness-free or doctest census, or hermeticity-inventory drift | Run `cargo xtask gen-bazel`, review and commit the generated diff, then run `cargo xtask gen-bazel --check`. |
| Module lock drift | Run `cargo xtask bazel-module-refresh`, review and commit `MODULE.bazel.lock`, then rerun the failed command. |
| `bazel-module-refresh` changed tracked files other than `MODULE.bazel.lock` | Commit or restore the listed repository-relative paths, then run `cargo xtask bazel-module-refresh`. |
| Yanked snapshot key-set drift | Run `cargo xtask bazel-yanked-refresh`, review and commit `bazel/supply_chain/yanked-snapshot.json`, then run `cargo xtask bazel-yanked-check`. |

Observations about that table, none of which belong in any string in it:

- `<hub>` is the only substitution any of these strings takes. The refusal
  prints the refused hub name in its place; the literal five characters never
  reach an operator.
- No refusal echoes an environment value. The ambient-control row names the
  three variables because a contributor has to unset them by name; what they
  were set to is never printed.
- No refusal prints an absolute path. The two unrelated-change rows list the
  paths that changed and list them repository-relative, because that message is
  read in review as often as in the shell that produced it, and because an
  absolute path is a local value like any other.
- No string carries a wave, phase, round, or finding marker. These are shipped
  operator-facing bytes, and the process-marker ban applies to them in full.
- The module-lock refusal is surfaced by this repository, not by Bazel. Bazel's
  own `--lockfile_mode=error` diagnostic stands as it is; the repository adds
  its recovery line beside that diagnostic, from `bazel-repin` and
  `bazel-module-refresh` in W0 and from every command the Make wrapper issues
  from W2.
- The module-lock update invocation `bazel-module-refresh` issues is
  `bazel mod deps --lockfile_mode=update`, measured at Bazel 8.6.0 to change
  only `MODULE.bazel.lock` and to change nothing on a second run. W0
  re-measures it against this repository's module graph before the message
  ships.

Tests follow the emitter, not the table:

- `packages/xtask/src/bazel.rs` carries the table-driven message test for the
  generator, stale-hub-lock, ambient-control, and both unrelated-change rows,
  because it emits those strings itself. It carries no test for a message it
  does not emit.
- The module-lock row is proved by an integration test that plants real module
  drift, runs the pinned Bazel under `--lockfile_mode=error`, and asserts the
  repository's recovery line beside the real upstream diagnostic. Asserting
  upstream text against a hand-written fixture would prove only that the
  fixture matches itself, and would keep passing after an upstream wording
  change that broke the contributor's actual experience.
- The yanked row is proved with the offline validator that emits it.

Each of those tests triggers its row, asserts the exact string, asserts that no
other row's remedy appears, and plants an absolute path and an environment
value to prove neither survives into the message.

## Wave notes record shapes, not local values

W0 records two measured invocations in its wave notes: the one that repins
exactly one hub, and the one that updates the module lock. Both are recorded as
command **shapes** with placeholders, for example
`<worktree>/.scratch/bazel` for the output user root, and never as the real
absolute path of the worktree the measurement happened to run in. A wave note
is read by every later wave and quoted into review comments; a real home
directory path in one is a local value that has escaped, in the same way an
echoed environment value would be. The redaction rule that governs refusal text
governs the notes that describe how those refusals were measured.

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
