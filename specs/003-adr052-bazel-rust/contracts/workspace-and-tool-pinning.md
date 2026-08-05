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
  derived from the worktree. From W2 there is exactly one construction that
  derives them, in `packages/d2b-bazel-support/src/startup.rs`, and the
  wrapper, `cargo xtask bazel-repin`, and `cargo xtask bazel-module-refresh`
  all call it. That module is in the neutral support crate rather than in the
  runner precisely so `xtask` can call it without an
  `xtask -> d2b-bazel-runner` dependency, which
  `tests/unit/meta/w0-dep-direction.sh` refuses.
- The wrapper supplies **byte-identical** startup options to `build`, `test`,
  `query`, `info`, `shutdown`, and `clean`, and from W2 to the single child
  each of `cargo xtask bazel-repin` and `cargo xtask bazel-module-refresh`
  spawns. This is what makes "shut down with the same
  startup options" enforceable: startup options select the server and
  output base, so a mismatched shutdown starts a second server and leaves the
  live one owning the tree.
- A mutation that perturbs one invocation's startup options must fail closed.

## Crate dependency direction

This migration adds three internal crates, and their edges are a closed set:

```text
packages/d2b-bazel-support/   <- packages/d2b-bazel-runner/
                              <- packages/d2b-test-locator/
                              <- packages/xtask/            (from W2)
```

- `d2b-bazel-support` is neutral. It declares no workspace member and no
  `d2b`-prefixed crate as a dependency of any kind. It holds what more than one
  consumer needs: the `FileSystem` boundary, the `RunfilesView` boundary, and
  the one absolute startup-option construction.
- `d2b-bazel-runner` and `d2b-test-locator` declare `d2b-bazel-support` and no
  other first-party crate. In particular they do not declare each other.
- `xtask` declares `d2b-bazel-support` from W2 and never declares
  `d2b-bazel-runner` or `d2b-test-locator`, in any dependency kind, so a
  dev-dependency under `packages/xtask/tests/` cannot smuggle the edge back.
  `xtask` generates the build targets the runner's own graph is made of;
  depending on the runner inverts that.
- Only the runner, the locator, and `xtask` may declare `d2b-bazel-support` as
  a non-dev dependency, and no crate outside the runner declares
  `d2b-bazel-runner` at all. `d2b-test-locator` is deliberately outside that
  restriction: the W1 migration makes it a **dev-dependency** of every
  first-party crate whose tests locate a binary or a fixture.

W0 enforces this by extending `tests/unit/meta/w0-dep-direction.sh`, the
repository's existing crate-granular direction gate, which
`tests/test-policy.sh` and `tests/static.sh` both run. That gate resolves
dependencies with `cargo metadata --no-deps`, so a `package =` rename, a
workspace-inherited dependency, and a target-specific dependency are all
visible to it and would all be invisible to a manifest-text scan; it already
fails closed when the resolver cannot run. The extension carries a
required-crate list naming all three and refuses when any of them is absent
from the resolver's member set, rather than falling through that gate's
existing "not a workspace member yet" skip, because a silent skip on a
misspelled crate name is the one way a direction gate passes while enforcing
nothing. That assertion is satisfiable only on the integrated tree, where all
three are members. Extending an existing gate adds
no new top-level shell gate, no Layer-1 job, and no required context, so
FR-053 holds.

The planted negatives are an `xtask -> d2b-bazel-runner` edge and a
first-party edge out of the support crate. Both are added, observed refused,
and reverted during W0 integrated validation, not inside a single scope
worktree, because the required-crate list cannot be satisfied before
integration.

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
  calls the one shared construction in
  `packages/d2b-bazel-support/src/startup.rs` instead, and the startup-option
  identity
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
  yet; from W2 the command calls the one shared construction in
  `packages/d2b-bazel-support/src/startup.rs`, and the
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
  module-lock update. That refusal carries its own recovery row rather than
  sharing one with `bazel-repin`, because the remedy has to end on the command
  the contributor was actually running, and a shared row would have to name a
  `--hub` value this command does not have and cannot infer.
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

### The index boundary the refresh calls through

`bazel-yanked-refresh` is the only command in this contract that opens a
socket, so it is the only one whose failure modes cannot be reached by
arranging local state. A test that wants to see what the command does with a
partial index answer would otherwise have to reach the live index, which makes
the test a network dependency, non-deterministic, and unavailable to anyone
reviewing offline. The refresh therefore calls the index through a trait rather
than through a client it constructs inline:

- `packages/xtask/src/bazel_yanked.rs` declares `trait YankedIndex`. Its
  surface is exactly what the refresh needs and nothing more: the revision of
  the index it observed, and the yanked state of one `(name, version)` key.
- The same file carries `IndexClient`, the single networked implementation of
  that trait. It is the only site in the repository permitted to open a socket
  on behalf of this command, and it holds no snapshot-shaping logic, so a
  reviewer reading the refresh reads no transport code.
- The refresh is written against `YankedIndex` and receives its implementation
  from the command-line routing seam. Its unit tests supply an in-process fake
  that returns canned responses: an all-clear index, an index reporting a
  yanked version, a response omitting a key the locks declare, a response
  carrying a key no lock declares, a response with no revision, a transport
  failure, and a malformed payload. No unit test opens a socket, resolves a
  name, or reaches the live index.
- `bazel-yanked-check` never names `YankedIndex` and never constructs
  `IndexClient`. The offline validator is offline by construction rather than
  by discipline, and a structural guard asserts that neither name appears on
  its path.

What a fake cannot prove is that `IndexClient` speaks correctly to the real
index. That is measured separately and explicitly: the contributor who runs the
reviewed networked refresh produces a snapshot whose diff against the committed
one, together with the index revision it recorded, is the observation, and the
offline check that follows is the verdict. The wave that commits the snapshot
records that observation in its wave notes as a command shape and a revision,
never as a live assertion the gate repeats.

## Actionable failure contract

Every row's text is the operator-facing recovery string the refusal must
contain, quoted exactly. This column holds strings and nothing else; every
observation about the strings is below the table.

| Failure | Exact recovery text |
| --- | --- |
| Stale Bazel-side hub lock | Run `cargo xtask bazel-repin --hub <hub>`, review and commit the regenerated lock under `bazel/cargo/`, then rerun the failed command. |
| `bazel-repin` changed tracked files other than the named hub's lock | Commit or restore the listed repository-relative paths, then run `cargo xtask bazel-repin --hub <hub>`. |
| `bazel-repin` refused an ambient `CARGO_BAZEL_REPIN`, `REPIN`, or `CARGO_BAZEL_REPIN_ONLY` | Unset `CARGO_BAZEL_REPIN`, `REPIN`, and `CARGO_BAZEL_REPIN_ONLY`, then run `cargo xtask bazel-repin --hub <hub>`. |
| `bazel-module-refresh` refused an ambient `CARGO_BAZEL_REPIN`, `REPIN`, or `CARGO_BAZEL_REPIN_ONLY` | Unset `CARGO_BAZEL_REPIN`, `REPIN`, and `CARGO_BAZEL_REPIN_ONLY`, then run `cargo xtask bazel-module-refresh`. |
| Generated BUILD, governed-source, `.bazelignore`, harness-free or doctest census, or hermeticity-inventory drift | Run `cargo xtask gen-bazel`, review and commit the generated diff, then run `cargo xtask gen-bazel --check`. |
| Module lock drift | Run `cargo xtask bazel-module-refresh`, review and commit `MODULE.bazel.lock`, then rerun the failed command. |
| `bazel-module-refresh` changed tracked files other than `MODULE.bazel.lock` | Commit or restore the listed repository-relative paths, then run `cargo xtask bazel-module-refresh`. |
| Yanked snapshot key-set drift | Run `cargo xtask bazel-yanked-refresh`, review and commit `bazel/supply_chain/yanked-snapshot.json`, then run `cargo xtask bazel-yanked-check`. |

Observations about that table, none of which belong in any string in it:

- `<hub>` is the only substitution any of these strings takes, and it appears
  only in the three rows whose remedy is `bazel-repin`. The refusal prints the
  refused hub name in its place; the literal five characters never reach an
  operator. No other row is a template: each names one command with the exact
  arguments a contributor is meant to type.
- No refusal echoes an environment value. Each ambient-control row names the
  three variables because a contributor has to unset them by name; what they
  were set to is never printed. There are two such rows and not one because the
  remedy differs: the row a `bazel-repin` refusal emits ends on
  `bazel-repin --hub <hub>`, and the row a `bazel-module-refresh` refusal emits
  ends on `bazel-module-refresh`. Collapsing them would send a contributor who
  was updating the module lock off to repin a hub they never named.
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
  generator, stale-hub-lock, both ambient-control, and both unrelated-change
  rows, because it emits those strings itself. It carries no test for a message
  it does not emit.
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

Two later notes obey the same rule. The wave that commits the yanked snapshot
records the reviewed networked refresh the same way, as a command shape plus
the index revision it observed, which is the only part of that run worth
carrying forward. The wave that consolidates every startup option into one
construction records the resulting invocation as a shape with `<worktree>`
placeholders only, because a consolidated construction is exactly the note a
later reader is most likely to copy verbatim.

The rule is enforced by a type-5 policy lint, not by a scan a validation task
performs by hand. W0 lands it in
`packages/d2b-contract-tests/tests/policy_docs.rs`. It enumerates every entry
of `specs/003-adr052-bazel-rust/wave-notes/` and refuses an empty corpus, any
entry that is not a readable regular file named `w<digits>.md`, any line that
still holds a `/`-rooted path token once every `<worktree>`-rooted path and
every `http` or `https` scheme-and-authority prefix has been removed, and any
line carrying the worktree's own absolute path, or that path stripped of its
leading slash, as a bare substring. A `<worktree>`-rooted path is the exact
literal `<worktree>` followed by `/`-separated segments, each an ordinary
segment or a further angle-bracket placeholder, so
`<worktree>/.scratch/bazel/<base>/execroot` is consumed whole. `<worktree>` is
allowed in exactly that spelling; `<WORKTREE>` and `<worktree-root>` are not.
The scheme allowlist is exactly `http` and `https`, so a `file:` URI is refused
rather than parsed, because a `file:` URI is an absolute path wearing a scheme.
No real absolute path is allowlisted, `/dev/null` included: a note records a
shape, not a transcript.

### The lint reads the corpus through the shared filesystem boundary

The lint does not call `std::fs::read_dir`, does not call
`std::fs::read_to_string`, and never builds a path by concatenating a
`DirEntry` onto a parent. It reads the corpus through the same `FileSystem`
trait in `packages/d2b-bazel-support/src/fsops.rs` that cleanup, the result
writer, the locator, and the topology provider checks use, reached as a
**dev-dependency only**, which keeps the support crate neutral and keeps
`packages/d2b-contract-tests` off the non-dev consumer list the
dependency-direction gate pins.

- The notes directory is opened once as an anchored close-on-exec directory
  descriptor beneath the repository root, with
  `RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS`.
- Entries are enumerated from that descriptor as **raw names**, never as
  reconstructed paths and never as decoded text. A name is a
  `std::ffi::OsString` holding exactly the bytes the enumeration returned,
  because a Linux directory entry is any NUL-free, `/`-free byte string and
  `String` cannot hold every name the corpus may contain. The kernel's `d_type`
  is advisory and is not trusted for
  classification, because it is `DT_UNKNOWN` on some filesystems; the
  authoritative answer is the errno the descriptor-relative open or read
  returns.
- The enumerated names are **sorted by unsigned byte order over
  `OsStr::as_bytes()` before any entry is
  opened**, and both the returned entry sequence and every one-based position
  the renderer can fall back to derive from that sorted sequence. Directory
  order is not an order. Measured on the reference host, the same seven note
  names enumerate as `w2 w0 w1 w11 w3 w10 w9` on ext4 and as
  `w3 w11 w1 w0 w2 w10 w9` on tmpfs, so an unsorted position label names a
  different entry in CI than it does locally and neither run is reproducible
  from the other's message. Byte order is used rather than a locale collation
  because it is total over raw directory-entry bytes and identical everywhere,
  and totality is what a corpus containing a name no collation is defined over
  actually needs. The key is `as_bytes()` rather than `OsString`'s own `Ord`,
  which the standard library does not promise equals the raw-byte relation on
  every target; measured here the two agree, and naming the bytes is what stops
  that agreement from being load-bearing.
- Each entry is opened descriptor-relative from that same anchor under the same
  resolve policy and read through the boundary. Nothing reopens the directory,
  and no absolute path is ever constructed.
- The strict policy binds on both of the boundary's routes. Where the boundary
  takes its forced component-walk route it carries `O_NOFOLLOW` on the leaf as
  well as on every intermediate component, because the policy parameter decides
  the leaf. A walk route that exempted the leaf would follow a symlinked note
  out of the directory this lint polices, which is the escape the lint exists
  to refuse, performed by the lint. `runner-environment.md` states that rule
  once for every call site.

That policy is not decoration; it is what makes the four failure states
distinguishable, and each was measured against this exact flag set on a plain
committed directory: a regular note opens; a symlink, valid or dangling, is
refused `ELOOP`; a committed subdirectory opens and its read returns `EISDIR`;
a `..` escape is refused `EXDEV`; and a mode `0000` entry is refused `EACCES`.
A `read_to_string` over a concatenated `DirEntry` path would follow a symlink
out of the directory and read whatever it pointed at, and would resolve the
directory a second time between enumeration and read. It is the same
check-then-use shape the provider rules refuse, applied to the guard that
polices leaks.

### Two violation shapes, two remedies, one corpus error

A refusal is a shipped operator-facing message, and the remedy has to match
what actually went wrong. One violation type carrying an optional line and an
optional error can render a line remedy for a permission denial. The type is
therefore split, and the split is structural:

| Shape | Carries | Renders |
| --- | --- | --- |
| `PathLeak` | note label and the one-based line, and nothing else | the label, the line, then the single remedy: rewrite the path as a `<worktree>`-rooted shape or drop it |
| `ReadError` | note label and the preserved `std::io::Error` | the label, the error, then the single file-level remedy: fix the entry's permissions or remove the invalid entry |

The label is the entry name when the name renders and passes the lint's own
rules, and the entry's one-based position in the sorted enumeration when it
does not. "Renders" is a check, not an assumption: the entry name is an
`OsString`, and `Name` is constructed only where `OsStr::to_str()` returned
`Some`.

`PathLeak` has no error field to be `None` and `ReadError` has no line field to
be zero, so neither of the two impossible states is expressible and no
validator has to re-derive the invariant. Neither renders the offending token,
and neither renders an absolute path: FR-029 already forbids a refusal message
from carrying one, and a refusal that echoes the leaked token is one. A lint
refusal is republished into the `test-fixture-contracts` output, into panel
comments, and into PR bodies, so a message that echoes the leaked absolute path
has copied it into three artifacts the note never reached, performed by the
guard that caught it. `w1.md:37` plus the remedy is sufficient, because the
contributor has the file.

`ReadError` preserves the boundary's error unchanged in every case that has
one: `EACCES` for a permission denial, `EISDIR` for a committed subdirectory,
`ELOOP` for a symlink or a dangling symlink, and `ErrorKind::InvalidData` for
non-UTF-8 content. The single exception is an entry whose name does not match
`w<digits>.md`, which is never opened and so has no errno; that one case
constructs `ErrorKind::InvalidInput` with a fixed message naming the required
name shape, and a test asserts that this is the only constructed error by
requiring the exact `raw_os_error` for the other four.

An unreadable or empty corpus is not a violation of a note, because there is no
note to name. It is a corpus-level error the enumerator returns instead of a
list: `Unreadable` carries the real errno from the anchored open or the entry
enumeration, so a directory the lint cannot read is a fail-closed refusal
rather than a corpus that merely looks empty, and `Empty` carries no error
because none occurred. Each renders its own remedy, restore
`specs/003-adr052-bazel-rust/wave-notes/` and its permissions, or add this
wave's note under that directory, and cross-rendering any of the
four remedies is a test failure.

**Both corpus errors name the directory, and they name it as the fixed
repository-relative literal `specs/003-adr052-bazel-rust/wave-notes/`.** A
corpus error names no entry, so without the directory the remedy is
"restore the notes directory" with no way to tell which one. The literal is
compile-time text and is never the rendered form of the path the enumerator
actually opened. That is not a style preference; it is forced. The enumerator
resolves the corpus beneath `repo_root()`, so rendering what it opened would
print the contributor's worktree, which is an absolute path FR-029 forbids in a
refusal, and the lint's own self-application case would then catch its own
message. The literal survives the lint's rules by construction: every `/` in it
is preceded by an ordinary path character, so none of them is a `/`-rooted
token, and it contains no worktree substring on any machine. The two remedy
sentences stay distinguishable even though both carry the literal, so the
per-variant assertion is on the whole remedy sentence and never on the shared
directory text.

The entry name is the only borrowed text any refusal prints, and it is checked
before it is printed. The renderer first asks `OsStr::to_str()` for a rendering
at all, then runs that `&str` through the same
`/`-rooted-token and worktree-substring rules the note lines face; a name whose
`to_str()` is `None`, or one that fails those rules, is replaced by the entry's
one-based position in the **sorted**
enumeration, and the refusal says so. Without the check, the self-application
test below would hold only for entry names that happen to be clean, which is a
coincidence rather than a property. Without the sort, the position would be a
different entry on ext4 than on tmpfs, and a refusal a contributor cannot
reproduce is a refusal that gets ignored.

**Holding the name as `OsString` is what keeps the corpus rule fail-closed.**
The name-shape rule is what makes anything other than a `w<digits>.md` note in
that directory a refusal, and it runs over the raw bytes: `w\xff9.md` fails
`^w[0-9]+\.md$` because `0xff` is not a digit, so the entry is refused
`ErrorKind::InvalidInput` without ever being opened and is labelled by
position. A `String` name field cannot reach that state. It leaves the
enumerator two options and both are defects. Dropping the entry removes it from
the corpus, so the one entry a contributor could not have named by accident is
the one entry the name-shape refusal never sees. Lossy conversion keeps the
entry and corrupts the label: measured, the distinct raw names `w\xff9.md` and
`w\xfe9.md` produce identical lossy text, so two entries share one rendered
label and one sort key and the tie falls back to directory order; and raw
`w\x80.md` sorts before the valid UTF-8 name `w\xc3\xa9.md` while their lossy
forms sort the other way, because `U+FFFD` encodes to `0xEF 0xBF 0xBD`. The
second measurement is the one that matters most: a lossy sort moves the
position label of entries that were never broken.

The scanner proves all of this about itself: one test runs every rendered
refusal and every rendered corpus error, from every planted input, back through
the same path-token and worktree-substring rules and requires no violation, so
a change that adds the token to a message, or that renders the resolved corpus
path instead of the fixed literal, fails the lint that added it. A second test
asserts, per variant, that the correct remedy is present and that each of the
other three remedies is absent, so a refactor cannot borrow a remedy across
shapes. A third supplies one corpus in two different enumeration orders and
requires the returned entry sequence, the violation order, and every assigned
position to be identical, with the removed sort planted as the mutation it must
reject. A fourth requires each corpus error to carry the fixed
repository-relative directory literal and no absolute path. A fifth supplies a
corpus mixing raw names that are not valid UTF-8 with a valid non-ASCII one and
requires the sorted sequence, the violation order, and every assigned position
to follow the raw-byte relation, with a sort over the lossy rendering planted
as the mutation it must reject. A sixth requires an entry whose raw name has no
UTF-8 rendering to be refused `ErrorKind::InvalidInput` without being opened
and labelled by position, with both the dropped-entry and the
lossy-name-as-label mutations planted and required to fail.

The enumeration API returns a corpus `Result` at the outer level and a
`std::io::Result` per entry carrying exactly what the boundary read returned,
beside an `OsString` name carrying exactly what the enumeration returned.
It never collapses a failed read to `None` or to an empty string, and it never
drops or launders a name it cannot decode. Keeping the
real `std::io::Error` is also what stops a later refactor from mapping every
failure onto the same benign-looking value; keeping the raw name is what stops
one from mapping every unrepresentable entry onto absence.

The lint proves it can refuse before its pass is treated as evidence, and it
does so against injected corpora rather than files written into the notes
directory: the enumeration negatives are supplied states of the `FileSystem`
fake, and the line-rule negatives are in-test note bodies. The planted set is a
generic absolute path belonging to no machine in the run, an empty corpus, an
unreadable directory, a worktree substring with no leading slash, a non-note
entry name beside an entry whose read failed with `EACCES`, a symlinked entry
refused `ELOOP`, a subdirectory entry refused `EISDIR`, non-UTF-8 content, an
entry name that itself carries a leak, an entry whose raw name is not valid
UTF-8, a second such name that differs from the first only in bytes a lossy
conversion collapses, an invalid raw name ordered against a valid non-ASCII
one, and the two near-miss placeholder
spellings plus the `file:` URI.
The one lane that executes it is
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`, because
`tests/test-rust.sh` excludes `d2b-contract-tests` from every workspace leaf
and `tests/test-policy.sh` names seven contract-test binaries that do not
include `policy_docs`. No new gate or shell script is added.

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
