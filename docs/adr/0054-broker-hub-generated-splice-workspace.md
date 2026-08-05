# ADR 0054: A generated splice workspace for the privileged broker's Bazel dependency hub

- Status: Proposed
- Date: 2026-08-04
- Related: [ADR 0052](0052-bazel-rust-build-and-test.md) (Bazel as the Rust
  build and test scheduler), whose sections 2, 3, 4 and 5 and invariant 2 this
  record refines without reversing anything in them;
  [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md) (Rust toolchain,
  MSRV and supply-chain policy), whose independent per-lock `cargo-deny` and
  `cargo-audit` coverage is the reason the broker keeps its own lock;
  [ADR 0000](0000-repository-layout-and-rust-bootstrap.md) (repository
  layout), which this decision extends with one generated directory under
  `bazel/`; [ADR 0002](0002-non-root-daemon-and-privileged-broker.md) and
  [ADR 0015](0015-daemon-only-clean-break.md), which make
  `d2b-priv-broker` the audited privileged surface whose dependency closure
  this record refuses to dissolve.
- Scope: how the `broker` `crate_universe` hub declared in `MODULE.bazel`
  obtains a spliceable Cargo workspace, and what the repository-owned
  generator must emit for it. Build-graph shape only.
- Non-scope: the other three hubs, which are unchanged; the contents of the
  first-party `BUILD.bazel` files, which ADR 0052 section 4 already owns; any
  runtime, daemon, broker-op, or packaging behaviour. No entry in
  `docs/contributing/critical-subsystems.md` changes, because this decision
  touches build orchestration and not a runtime subsystem.
- Unblocks: Spec 003 wave W0, parked at commit `a3e7d68c` on
  `spec003-w0-planfix` with tasks T019 and T021 through T025 open.

## Context

ADR 0052 declares four `crate_universe` hubs, one per Cargo workspace in the
repository, and requires that `Cargo.toml` and the three `Cargo.lock` files
stay the authoritative dependency inputs. Three of those hubs work as written.
The fourth does not.

`packages/d2b-priv-broker` is a standalone Cargo workspace excluded from
`packages/Cargo.toml`. It path-depends on five crates that are members of the
main workspace: `d2b-contracts`, `d2b-core`, `d2b-host`, `d2b-realm-core` and
`d2b-realm-provider`. Cargo resolves that shape without complaint, because a
package's workspace is found by walking up from the package directory, so each
of the five inherits from `packages/Cargo.toml` even when the broker workspace
consumes it as a path dependency. `crate_universe` splicing relocates
manifests into a temporary tree, and every part of that arrangement breaks
under relocation.

Spec 003 W0 landed two experiments against this, both committed and both
refused by the substrate. Neither faked a lock or reached for a repin escape
hatch, which is why the wave parked rather than shipping something green and
wrong.

### Measured constraints that shape the decision

Measured on 2026-08-04 against Bazel 8.6.0 from the pinned nixpkgs and
`rules_rust` 0.73.0, using a synthetic reproduction of the repository's
workspace shape under `.scratch/` plus direct reads of the fetched
`rules_rust` source. Where a message is quoted it is the literal observed
output.

1. **Supplying only the broker manifest cannot see its path dependencies.**
   `packages/d2b-priv-broker/Cargo.toml` carries an empty `[workspace]` table,
   so `parent_workspace` returns the manifest itself and `SplicerKind::new`
   selects `SplicerKind::Workspace`. `splice_workspace` symlinks only the
   broker directory into the splice tree, so `path = "../d2b-contracts"`
   resolves to a sibling of the temporary directory and does not exist.
   Observed, for the reproduction's analogue of `d2b-contracts`:

   ```text
   error: failed to load manifest for dependency `liba`
   Caused by:
     failed to read `/tmp/liba/Cargo.toml`
   Caused by:
     No such file or directory (os error 2)
   Error: Failed to generate lockfile
   ```

   This is commit `e80ac1ef`'s shape.

2. **Supplying the path manifests alongside it is refused by name.**
   `SplicerKind::new` collects `parent_workspace` for every supplied manifest
   and bails when more than one distinct workspace root appears. The five
   first-party manifests resolve to `packages/Cargo.toml`; the broker manifest
   resolves to itself. Observed:

   ```text
   Error: When splicing manifests, manifests are not allowed to from from
   different workspaces. Saw manifests which belong to the following
   workspaces: <root>/pkgs/Cargo.toml, <root>/pkgs/broker/Cargo.toml
   ```

   This is commit `65fbe095`'s shape. The message's grammatical defect is
   upstream's and is quoted verbatim so a future reader can grep for it.

3. **The splice tree inherits the spliced directory's `Cargo.lock`.**
   `symlink_roots` links every top-level entry of the root manifest's
   directory into the splice tree, including `Cargo.lock`.
   `write_root_manifest` removes and rewrites `Cargo.toml` only. So the lock
   the spliced workspace sees is whatever sits beside the manifest that was
   spliced, not the `cargo_lockfile` attribute.

4. **`skip_cargo_lockfile_overwrite = True` removes the only writer.** With
   that attribute set, which ADR 0052 section 3 mandates for every hub,
   `cargo-bazel splice` loads the `cargo_lockfile` directly for rendering and
   takes the branch that never copies it into the splice tree and never runs
   `cargo fetch`. Every remaining Cargo invocation on the splice tree passes
   `--locked`: the `TreeResolver` feature pass and the final metadata dump.
   The consequence is sharp and is the constraint that eliminates most of the
   option space: **the spliced root directory's own `Cargo.lock` must already
   satisfy the spliced manifest.** When it does not, the observed failure is

   ```text
   Error: Failed to generate features
   Caused by:
       0: Failed to copy project with proc macro deps made direct
       1: Failed to run cargo metadata to list transitive proc macros
       2: `cargo metadata` exited with an error: error: cannot update the
          lock file <tmp>/Cargo.lock because --locked was passed to prevent
          this
   ```

5. **Without that attribute the authoritative lock is rewritten.** Measured by
   clearing it in the reproduction: the splice copied the authoritative lock
   into the splice tree, ran `cargo fetch`, re-resolved, and wrote the result
   back onto the source lock, which gained a package it did not have. This is
   the failure ADR 0052 section 3 already forbids; it is recorded here because
   this decision must not create a reason to relax it.

6. **Cargo does not resolve a non-member path dependency's
   dev-dependencies.** Measured both ways. In the real tree,
   `packages/d2b-priv-broker/Cargo.lock` contains no `regex` and no `ttrpc`,
   while `packages/d2b-core` dev-depends on `regex` and `packages/d2b-host`
   dev-depends on `ttrpc 0.9.0`. Making those crates members of a synthetic
   splice workspace therefore demands a lock that the authoritative broker
   lock is not, and constraint 4 turns that into a hard refusal. Any design
   that reaches the five first-party crates by membership must strip their
   dev-dependencies to stay equal to the lock.

7. **The five first-party crates inherit from `packages/Cargo.toml`.** Each
   uses `license.workspace = true`, `[lints] workspace = true`, and between
   two and six `<dep>.workspace = true` entries drawn from that file's
   `[workspace.dependencies]`. Relocated under any root that does not carry
   those tables, `cargo metadata` fails. Observed:

   ```text
   Caused by:
     error inheriting `license` from workspace root manifest's
     `workspace.package.license`
   Caused by:
     `workspace.package.license` was not defined
   ```

   Any design that splices the real manifests must also mirror three
   `[workspace.*]` tables.

8. **The main hub cannot serve the broker.** The main lock resolves 544
   packages, 490 of them from a registry; the broker lock resolves 117, 111
   from a registry. Twenty-seven of the broker's registry entries are absent
   from the main lock at the same version. Fourteen crate names are absent
   from the main lock entirely (`foldhash`, `id-arena`, `leb128fmt`,
   `prettyplease`, `unicode-xid`, `wasip3`, `wasm-encoder`, `wasm-metadata`,
   `wasmparser`, `wit-bindgen-core`, `wit-bindgen-rust`,
   `wit-bindgen-rust-macro`, `wit-component`, `wit-parser`), and thirty shared
   names carry different version sets, including `nix` at `0.29.0` against the
   main lock's `{0.26.4, 0.29.0, 0.31.3}` and `windows-sys` at
   `{0.59.0, 0.61.2}` against `{0.48.0, 0.52.0, 0.61.2}`. Eighty-four of the
   broker's 111 registry packages do exist in the main lock at the same
   version, so the four-hub design already pays for up to eighty-four
   duplicate compilations; that cost is ADR 0052's, not this record's.

9. **The problem is broker-specific.** `packages/d2b-guest-shell-runner` has
   one lock entry without a `source` field, its own; so does
   `tests/tools/no-bash-ast-walker`. Only the broker workspace has a
   path-dependency closure larger than one package. The blocker therefore
   needs one narrow mechanism, not a general one.

10. **There is no `rules_rust` to upgrade to.** `gh release list --repo
    bazelbuild/rules_rust` reports 0.73.0, released 2026-07-31, as the newest
    release. Every option that begins "upgrade past this" has nothing to
    upgrade to.

11. **A lock is a resolve, not a set of declarations.** `packages/d2b-core`
    declares `bolero = { version = "0.10", optional = true }` behind
    `fuzz = ["dep:bolero"]`. The main lock carries `bolero`; the broker lock
    does not, because nothing in the broker workspace activates that feature.
    A stub tree built from the declared dependency tables therefore resolves
    thirty packages the authoritative broker lock does not contain
    (`bolero` and its closure, including `addr2line`, `backtrace` and a second
    `syn` major version) and is refused with the constraint 4 message. Built
    instead from the edges the broker workspace's resolve realizes, the same
    six-package tree resolves clean:
    `cargo metadata --locked --offline` exits zero against a byte-identical
    copy of `packages/d2b-priv-broker/Cargo.lock`. Measured on the real tree,
    not on the reproduction.

12. **The broker workspace has exactly one member, and the broker suites test
    only that member.** `cargo metadata --manifest-path
    packages/d2b-priv-broker/Cargo.toml --format-version 1 --no-deps
    --offline` reports one entry in `workspace_members` and one in
    `workspace_default_members`, `d2b-priv-broker`, whose package declares one
    `lib` target, one `bin` target and thirteen `test` targets. The three
    `rust-broker-*` surfaces in ADR 0052 section 5 are
    `cargo test --workspace --manifest-path
    packages/d2b-priv-broker/Cargo.toml` under three feature sets
    (`tests/test-rust.sh` lines 506, 509 and 512), and `--workspace` on that
    manifest selects that one member. The five path-dependency crates' test
    targets are not built and not run by the broker suites today; they belong
    to `rust-main-workspace-tests`. Together with constraint 6 this fixes what
    a broker-hub first-party target may be: a test target for `d2b-core` needs
    `regex` and one for `d2b-host` needs `ttrpc`, and neither crate is in the
    authoritative broker lock at all.

13. **The hub's staleness check covers only the manifests the hub names, and
    the splice runs only once a repin is already required.** Read from the
    fetched 0.73.0 source. `determine_repin` in
    `crate_universe/private/generate_utils.bzl` calls `cargo-bazel query`,
    which recomputes `Digest::new` over the committed hub lock's own context,
    the generated config, a `SplicingMetadata`, and the `cargo`, `rustc` and
    `cargo-bazel` versions. `SplicingMetadata::try_from` reads and parses the
    file contents of exactly the manifests named in the hub's `manifests`
    attribute and nothing else, and `crate_universe/extensions.bzl` performs
    the splice inside `if repin:`. So for a hub whose `manifests` attribute is
    one generated workspace root, a stub manifest beneath that root is outside
    the digest, does not trigger a repin, and is not read at all on an
    ordinary build. The substrate does not notice that the generated tree
    moved. The corollary is the lever: what the attribute names is what the
    digest covers, so naming the stubs brings them inside it. Section 2 does
    that and section 5's fourth gate is what it buys.

Constraints 1 and 2 are the two committed experiments. Constraints 3, 4 and 6
together rule out every arrangement that splices the real first-party
manifests. Constraint 8 rules out serving the broker from the main hub or
mixing hubs. Constraint 10 rules out waiting. Constraint 11 fixes what the
generator must read. Constraint 12 fixes which targets a broker-hub first-party
variant may carry. Constraint 13 fixes what the hub's `manifests` attribute has
to name for generated-tree staleness to be catchable by the substrate at all.

### Drift noted, not corrected

`bazel/cargo/README.md`, committed on `spec003-w0-planfix`, states that the
`broker` hub's Cargo manifest is `packages/d2b-priv-broker/Cargo.toml`. Under
this decision the hub's `manifests` attribute points at the generated splice
root instead, while `cargo_lockfile` continues to point at the authoritative
broker lock. That file is a generated-artifact README owned by the Spec 003
foundation scope and is corrected there, in the wave that implements this
decision, rather than here.

## Decision

### 1. The broker stays a standalone Cargo workspace

`packages/d2b-priv-broker/Cargo.toml` and
`packages/d2b-priv-broker/Cargo.lock` are not edited by this decision, not by
the generator, and not by any Bazel command. The broker is not merged into
`packages/Cargo.toml`. Its 117-package lock is a security artifact: it is the
independently pinned, independently `cargo-deny`ed and `cargo-audit`ed
dependency closure of the only binary the framework runs as root, and ADR 0052
invariant 14 reports it under its own `rust-deny-broker` identifier. Dissolving
it into a 544-package resolve to make a build tool's splicer happy trades a
fail-closed supply-chain boundary for convenience, which is the wrong direction
for this repository.

The change this decision makes to `packages/` is nothing. It is additive.

### 2. The `broker` hub splices a generated workspace, not the authoritative one

`MODULE.bazel` changes one attribute:

```python
crate.from_cargo(
    name = "broker",
    manifests = [
        "//bazel/cargo/broker-workspace:Cargo.toml",
        "//bazel/cargo/broker-workspace:d2b-contracts/Cargo.toml",
        "//bazel/cargo/broker-workspace:d2b-core/Cargo.toml",
        "//bazel/cargo/broker-workspace:d2b-host/Cargo.toml",
        "//bazel/cargo/broker-workspace:d2b-priv-broker/Cargo.toml",
        "//bazel/cargo/broker-workspace:d2b-realm-core/Cargo.toml",
        "//bazel/cargo/broker-workspace:d2b-realm-provider/Cargo.toml",
    ],
    cargo_lockfile = "//packages/d2b-priv-broker:Cargo.lock",
    lockfile = "//bazel/cargo:broker.lock",
    skip_cargo_lockfile_overwrite = True,
)
```

`cargo_lockfile`, `lockfile` and `skip_cargo_lockfile_overwrite` are unchanged
from the committed declaration. The authoritative broker lock remains the
rendering authority: under constraint 4 it is the file `cargo-bazel` loads to
decide which crates the hub contains and at which versions. Only the
*resolution witness*, the manifest set `cargo metadata` reads, moves.

The attribute names the generated root **and every stub manifest beneath it**,
sorted. Splicing is unaffected: every listed manifest resolves to the same
generated root, so `SplicerKind::new` still selects `SplicerKind::Workspace`
and splices identically, printing one upstream `INFO` line saying the extra
entries can be removed (measured; the alternative this comes from is recorded
below). The reason to list them is constraint 13. `SplicingMetadata::try_from`
reads and parses exactly the manifests the attribute names, so listing the
stubs is what puts their content in the hub digest, and that digest is the only
mechanism in the system that compares `bazel/cargo/broker.lock` to the tree it
was rendered from. Section 5's fourth gate and section 6's one partial-failure
state both rest on it. The six stub labels carry a slash in the target name
because the subtree is a single Bazel package: one `BUILD.bazel` at the
generated root exports all seven manifests and the lock mirror, and no stub
directory is its own package. The list is generated with the tree, and
`cargo xtask gen-bazel --check` fails when it is not exactly the root plus
every emitted stub in sorted order.

The other three hubs are untouched.

### 3. What the generated splice workspace is

`bazel/cargo/broker-workspace/` is a tracked, generated directory containing
exactly:

- `Cargo.toml`, a workspace root whose `members` are the package names in the
  broker workspace's path-dependency closure, in sorted order, and whose
  `resolver` is the resolver version the authoritative broker workspace
  resolves under. That is `"3"` today, because
  `packages/d2b-priv-broker/Cargo.toml` is its own workspace root, sets no
  explicit `[workspace] resolver`, and declares `edition = "2024"`. The
  generator derives it and refuses rather than defaulting when it cannot.
  The root declares no
  `[workspace.package]`, `[workspace.dependencies]` or `[workspace.lints]`
  table, because the stubs below carry no inheritance to satisfy.
- One directory per member, named for the package, containing a stub
  `Cargo.toml` and an empty `src/lib.rs`. The stub carries the package's
  `name`, `version`, `edition`, `publish = false`, an explicit
  `[lib] path = "src/lib.rs"`, its `[features]` table, and its dependency
  tables. Target-conditional dependencies are
  reproduced under the same `[target.'<cfg>'.dependencies]` key, and
  `build-dependencies` under their own.
- `Cargo.lock`, a byte-identical copy of
  `packages/d2b-priv-broker/Cargo.lock`.
- `BUILD.bazel`, exporting the root manifest, all six stub manifests by their
  slash-bearing target names, and the lock mirror. There is one Bazel package
  for the whole subtree; no stub directory is its own package.

Three rules make the resolution witness equal the authoritative lock rather
than merely similar to it. Each was measured, and each was measured by first
getting it wrong.

- **The stubs carry the edges the resolve realizes, not the tables the
  manifests declare.** Every `workspace = true` inheritance is written out as
  the literal value. Every optional dependency the broker workspace's resolve
  does not activate is omitted, along with the `dep:` and `<name>/<feature>`
  entries in the feature table that would name it. This is constraint 11:
  built from the declared tables the tree drags in `bolero` and twenty-nine
  more packages the authoritative lock does not have; built from the resolve
  it is exact.
- **Dev-dependencies are carried for the workspace member and for nothing
  else.** `cargo metadata` on the broker workspace reports exactly one entry in
  `workspace_members`, `d2b-priv-broker`; the other five packages are path
  dependencies. The stub for the member reproduces its `[dev-dependencies]`;
  the five path-dependency stubs omit theirs. This is what constraint 6
  requires, and it is derived, not chosen: the generator reads the
  member set rather than hard-coding a name.
- **The stubs carry no source.** `src/lib.rs` is empty. The splice never
  compiles first-party code; it only resolves. First-party compilation is the
  job of the generated `BUILD.bazel` files under `packages/`.

The root manifest's first line is a generated marker naming
`cargo xtask gen-bazel` and stating that Cargo never builds this tree.

### 4. The generator owns it, and drift dies in `test-drift`

`cargo xtask gen-bazel` emits the whole directory as a pure function of
`cargo metadata --manifest-path packages/d2b-priv-broker/Cargo.toml --locked`
plus the authoritative lock file's bytes, and emits the `broker` hub's
`manifests` list in `MODULE.bazel` from the same closure.
`cargo xtask gen-bazel --check`
regenerates into a scratch tree and fails on any difference, exactly as it
already does for the first-party `BUILD.bazel` files and the governed-source
manifest under ADR 0052 section 4. It is wired into `test-drift`, which is
where every other `xtask gen-*` staleness is already caught.

The emission of the splice workspace is one function with two callers. Section
6's `cargo xtask bazel-repin --hub broker` calls it directly rather than
shelling out to `gen-bazel` or reading the tracked tree, so the two commands
cannot disagree about what the tree is, and a tree written by one is byte-equal
to what the other would write.

The generator fails closed, by name, in three cases a stub cannot represent:

- a package in the closure declares a `build` script or a `links` key, which a
  stub cannot reproduce and which ADR 0052 measured constraint 7 says does not
  exist today;
- a package's directory name and package name disagree, which would make a
  path dependency between two stubs unresolvable;
- a Cargo workspace other than the broker's resolves a lock with more than one
  entry lacking a `source` field while no generated splice workspace is
  declared for its hub. That is the extension point: the day
  `d2b-guest-shell-runner` grows a first-party path dependency, the generator
  refuses and names the remedy instead of silently producing a hub that omits
  it.

### 5. Four independent gates, layered on purpose

- **Byte equality.** `cargo xtask gen-bazel --check` in `test-drift` proves the
  generated tree is what the authoritative manifests currently imply. This is
  the only gate that catches a change which moves no resolved version: a new
  optional dependency, a changed `[features]` table, a changed feature default.
- **Lock equivalence, offline, on every pull request.** `test-drift` also runs
  `cargo metadata --manifest-path bazel/cargo/broker-workspace/Cargo.toml
  --locked --offline` and requires exit zero. This proves the generated tree's
  resolution is exactly the authoritative broker lock, needs no network and no
  Bazel, and runs on every pull request rather than only when someone repins.
- **The substrate's own refusal on the resolve.** A repin cannot produce a hub
  from a tree whose resolve differs from the lock, because constraint 4's
  `--locked` metadata pass refuses. Section 6 moves that refusal earlier: the
  repin runs the same offline `--locked` pass on the tree it just generated,
  before it writes anything and before Bazel exists in the picture. It is a
  backstop and not a gate: it fires only on a difference the resolve realizes,
  and only inside a repin, since by constraint 13 the splice itself runs inside
  `if repin:`.
- **The substrate's own refusal on the digest.** Because section 2's
  `manifests` attribute names every stub, each stub's parsed content is inside
  the hub digest. A committed `bazel/cargo/broker.lock` rendered from a stub
  tree the repository has since replaced therefore makes `determine_repin`
  report the lockfile stale, and Bazel fails closed with its own
  repin-required message at the next analysis of the hub, which is on the pull
  request. This is the only check in the set that compares the hub lock to the
  tree it was rendered from, and it is the net under section 6's one
  partial-failure state. It fires late by construction, which is why the
  alternatives section rejects it as a primary guard and why it is exactly
  right as a net: a residue left behind by a command that already failed
  cannot be caught by that command.

The first two are stated as a pair deliberately. Version drift and feature
drift are different failures and neither gate catches both. The last two are a
pair for the same reason: one compares the tree to the Cargo lock, the other
compares the Bazel-side lock to the tree, and no single check does both.

### 6. One command performs the mutation and synchronizes its own inputs

Changing a dependency of the broker or of any crate in its closure is:

1. edit the Cargo manifest;
2. `cargo xtask bazel-repin --hub broker`.

`cargo xtask gen-bazel` is still the owner of every generated artifact in the
repository, and it still has to run for the first-party `BUILD.bazel` files,
the governed-source manifest and the hub declaration in `MODULE.bazel`. It is
not a precondition of the repin. The repin generates the broker splice inputs
itself, from the same function, in the same run, so for the broker hub there is
no ordering between the two commands to get wrong. That is the point. Round 1
of this record enforced an order with a refusal, and an order that has to be
enforced is an order someone will be told to satisfy by running the command the
tool could have run itself.

**The closed sequence.** `cargo xtask bazel-repin --hub broker` performs
exactly these steps, in this order, and fails closed at each. Steps 1 through 5
write nothing outside the ignored scratch root.

1. **Snapshot.** Record the worktree state: `git status --porcelain` over the
   repository plus a content hash for every path it names. Step 9 retakes the
   same snapshot and subtracts, so the command's own change set is separable
   from whatever the contributor already had in flight.
2. **Generate into scratch.** Emit the broker splice workspace into a scratch
   directory under the repository's ignored `.scratch/` root, through the
   generator entry point of section 4, from
   `cargo metadata --manifest-path packages/d2b-priv-broker/Cargo.toml
   --locked` plus the authoritative lock's bytes. The tracked subtree is not an
   input to this step; it is only a destination.
3. **Validate the scratch tree before it may become the tracked tree.**
   Section 4's three named refusals run on it. Its `Cargo.lock` must be
   byte-identical to `packages/d2b-priv-broker/Cargo.lock`. And
   `cargo metadata --manifest-path <scratch>/Cargo.toml --locked --offline`
   must exit zero, which is section 5's second gate run pre-write: the
   generated tree's resolve is proved equal to the authoritative lock before
   anything is written and without a network or a Bazel process. Any failure
   names the reason and leaves the worktree byte-identical to how the command
   found it.
4. **Refuse ambient work rather than overwrite it.** The command may write only
   `bazel/cargo/broker-workspace/**` and `bazel/cargo/broker.lock`, so it
   inspects both before writing either. Every path under the generated subtree,
   tracked or untracked, must be byte-identical either to its committed content
   at `HEAD` or to what step 2 produced, and the subtree must hold no path
   outside the union of those two file sets. `bazel/cargo/broker.lock` must be
   byte-identical to its committed content at `HEAD`. Anything else is a local
   modification this command did not make and would destroy: a hand edit of a
   generated file, a half-applied patch, a conflicted merge, a lock left
   uncommitted by an earlier run. The command exits nonzero, lists every
   offending path repository-relative, names both remedies (`git restore` to
   discard the local change, or commit or stash it to keep it), and spawns no
   Bazel. The two permitted subtree states are exactly the two in which no
   contributor work exists to lose: the committed tree, and the tree this run
   would produce anyway. The authoritative broker manifest and lock are
   deliberately outside this check. They are the inputs the contributor is
   editing, and requiring them clean would refuse the case the command exists
   to serve.
5. **Refuse on the one input it may not write.** If the fresh generation
   implies a `manifests` list for the `broker` hub other than the one committed
   in `MODULE.bazel`, the command exits nonzero naming `MODULE.bazel` and
   `cargo xtask gen-bazel` as the remedy, still having written nothing. That
   list changes only when the broker's path-dependency closure gains or loses a
   package, which a version change and a feature change never do, so this
   refusal is off the ordinary path. It exists because `MODULE.bazel` is
   outside the permitted final path set of step 9 and because rendering a hub
   from a declaration the repository has since replaced is the failure this
   record spent round 1 refusing.
6. **Synchronize the subtree.** Replace `bazel/cargo/broker-workspace/` with
   the validated scratch result, including the lock mirror, so the tracked
   subtree is byte-equal to what `cargo xtask gen-bazel` would emit. Nothing
   under `packages/` is written, no other hub's inputs are written, and
   `MODULE.bazel` is not written.
7. **Spawn exactly one Bazel child.** ADR 0052 section 3's controls are
   unchanged: one child process, `CARGO_BAZEL_REPIN` and
   `CARGO_BAZEL_REPIN_ONLY=broker` set only in that child's environment and
   never process-globally, and the same absolute output user root, output base
   and symlink prefix the Make wrapper derives, so no second server starts.
8. **Hold the child to one file.** After the child exits, the only tracked file
   it changed, measured against the state step 6 left behind, must be
   `bazel/cargo/broker.lock`. This is ADR 0052's rule
   unweakened, and it keeps its own job: it is what catches a
   `skip_cargo_lockfile_overwrite` regression rewriting an authoritative
   `Cargo.lock` (constraint 5) or a second hub's lock moving.
9. **Hold the command to its permitted set.** The command's own change set,
   step 1's snapshot subtracted from the same snapshot retaken, must be a
   subset of `bazel/cargo/broker-workspace/**` plus `bazel/cargo/broker.lock`.
   Any other changed path fails the command and is listed repository-relative.
   Steps 8 and 9 are different checks: step 8 bounds what Bazel wrote, step 9
   bounds what the whole command wrote, and only step 9 sees a generator that
   scribbled outside its own subtree.

**Nothing here generates on a gate or a build path.** `make`, every workflow,
and every Bazel invocation on the gate path remain incapable of generating a
tracked artifact, and `cargo xtask gen-bazel --check` in `test-drift` is still
the fail-closed gate for a stale tracked tree, still naming
`cargo xtask gen-bazel` as its remedy. Self-synchronization is a property of
one explicit contributor mutation command that is not a Make target, that no
workflow may invoke, and that is the only place the three repin environment
names may appear as a process-environment assignment.

**When the Bazel child fails after the subtree is synchronized.** Steps 1
through 5 leave the worktree exactly as they found it, so every refusal above
is total. After step 6 there is one state this command can leave behind that it
did not find: `bazel/cargo/broker-workspace/**` current, and
`bazel/cargo/broker.lock` still describing the previous inputs. A Bazel child
that fails for its own reasons, a full disk, an interrupt, produces it. The
command exits nonzero and says exactly that, naming both paths and their
states.

It does not roll back. Rewriting the subtree it just proved correct would make
a run that did real work indistinguishable from one that never happened, would
discard the validated generation for no gain, and cannot be made safe against a
concurrent edit. It does not silently retry either. The recovery is one line in
the failure message: fix what the Bazel child reported and re-run
`cargo xtask bazel-repin --hub broker`. The second run regenerates the same
bytes, because step 2 is deterministic; finds the subtree already equal to them,
which is step 4's second permitted state, so step 4 passes and step 6 is a
no-op; and spawns the child again. Re-running is therefore always safe and is
never the wrong thing to do, which is the property a partial-failure recovery
has to have. If instead the contributor wants the whole change gone, the
recovery is the ordinary one, `git restore bazel/cargo/broker-workspace`, and
the command is what told them the subtree is the only thing it touched.

That residue is not invisible, which is the property round 1 bought with a
refusal and this section has to buy some other way. Section 5's fourth gate
buys it: with the stub manifests named in the hub's `manifests` attribute, a
`bazel/cargo/broker.lock` rendered from a stub tree the repository has since
replaced fails `determine_repin` at the next analysis of the hub, on the pull
request, with the substrate's own repin-required message. Committing the
residue does not get it past review.

**The ordering hazard is gone by construction, not by refusal.** Round 1
described the dangerous sequence: repin first, so the hub renders from the
previous generated tree; then `gen-bazel`, which brings the tree up to date;
then commit, with every gate green. That sequence no longer exists. The repin
does not read the tracked tree as an input at all. It produces the tree it
renders from, in the same run, from the same function `gen-bazel` uses, so
there is no window in which the hub lock is rendered from a tree the repository
has since replaced.

What round 1 got right, and this section keeps, is why that hazard needed a
guard at all: it is invisible to every other check. A feature-only change moves
no resolved version, so `gen-bazel --check` and the offline
`cargo metadata --locked` both pass on the end state and constraint 4 never
fires. That is why the guard could never be a post-run diff, and why section 5
now carries a fourth gate rather than three.

### 7. Two first-party library sets, and one place the tests live

The five crates in the broker's closure are compiled twice **as libraries**,
once against `@main//` for the main workspace and once against `@broker//` for
the broker's three feature suites. This is forced by constraint 8, not chosen:
the two locks resolve different versions of crates those five crates depend
on, so a single compilation would be wrong for one of the two consumers.

The generator emits the broker-hub variant beside the main-hub one in the same
Bazel package, suffixed `-broker`: `//packages/d2b-contracts:d2b-contracts` and
`//packages/d2b-contracts:d2b-contracts-broker`. The suffix is greppable, the
`crate_name` is unchanged in both, and both appear in the ADR 0052 section 5
coverage map so an unmapped variant fails analysis rather than being merely
absent.

**A `-broker` variant is a library target and nothing else.** It carries no
`rust_test`, no `rust_doc_test`, and belongs to no test suite. The tests of the
five closure crates stay on their main-workspace variants under
`rust-main-workspace-tests`, which is where they run today.

This falls out of constraint 6 rather than being a preference. A test target
for `d2b-core` needs `regex` and one for `d2b-host` needs `ttrpc`; neither is
in the authoritative broker lock, because Cargo does not resolve a non-member
path dependency's dev-dependencies. A `-broker` test target therefore demands
crates the broker hub does not contain and cannot be rendered from it at all.
The only two ways to obtain them would be to make the five crates members of
the generated workspace, which constraint 4 refuses because the authoritative
lock would no longer satisfy the tree, or to reach into `@main//` for the
missing dev-dependencies, which is exactly the version mixture the isolation
check below forbids.

It is also parity rather than a reduction, and that is constraint 12: the
broker workspace has one member, the three `rust-broker-*` surfaces are
`cargo test --workspace` on that manifest, and the five closure crates' test
targets are not built by those surfaces today. Rendering them under Bazel
would add a surface, not preserve one, and it would be a surface with no
identifier and no census row in the coverage map.

The rule the generator applies is derived, not hard-coded, and it is the same
membership rule section 3 applies to dev-dependencies: **a package compiled
against the broker hub carries test targets if and only if it is a member of
the broker Cargo workspace.** Today that is exactly `d2b-priv-broker`, whose
one lib, one bin and thirteen test targets its own lock supports, and which
needs no suffix because it has no main-hub variant. If the broker workspace
ever gains a member, that member's tests come with it and its stub keeps its
dev-dependencies, and nothing about the five path-dependency crates changes.

**Isolation is checked over first-party targets directly.** The generator
emits the two target sets it created, and the check reads those sets rather
than re-deriving them from label text:

- **B**, the broker set: every target of a package that is a member of the
  broker Cargo workspace, today `//packages/d2b-priv-broker:*`, plus every
  `-broker` variant it emitted.
- **M**, the main set: every other first-party Rust target in the repository.

Two conditions over Rust compile and link edges, meaning the `deps` and
`proc_macro_deps` a target declares, transitively:

- the first-party portion of `deps(B)` is a subset of `B`;
- the first-party portion of `deps(M)` is a subset of `M`.

In words: no target reachable from a `-broker` variant may depend on an
unsuffixed first-party variant, and no main-hub variant may depend on a
`-broker` variant. A runfiles or `data` edge from a main-workspace test to the
broker *binary* is not a compile edge and is not restricted: it spawns a
separate process rather than linking one lock's compilation into the other's
graph. Source-file edges are likewise out of scope, which is what the existing
policy tests that read `packages/d2b-priv-broker/src` take.

Both conditions are evaluated primarily over the generator's own emitted
compile-edge map, because both graphs are generated: every `deps` and
`proc_macro_deps` entry written for a target in B names a label in B or an
`@broker//` spoke, and every such entry written for a target in M names a
label in M or an `@main//` spoke. That check needs no Bazel server and is
total over the class of edge that matters. A `bazel query` over library kinds
confirms it against the analysed graph. Reading emitted sets rather than
matching `-broker$` over labels is deliberate: a first-party crate
legitimately named for the broker would match the pattern and a variant the
generator failed to emit would not, so a label regex is a heuristic where an
emitted set is a fact.

The third-party form of the same idea, that no target reachable from a
`-broker` variant depends on a `@main//` spoke and no main-hub target depends
on a `@broker//` spoke, is retained as a supplemental assertion. It is not the
proof and must not be the only check, because it detects a wrong first-party
edge only when the crate on the far side of that edge happens to have a
third-party dependency. Today all five do; `d2b-realm-core` has two, `serde`
and `schemars`. Nothing forbids a first-party crate with none, a new leaf crate
is the likely shape of one, and against that crate the spoke check silently
proves nothing while the direct check still names the edge. The spoke check
also names the wrong thing when it does fire: it reports a third-party alias
resolved from the wrong hub, not the first-party dependency that put it there.

## Consequences

**Cache keys.** ADR 0052 invariant 9 binds cache keys to all Cargo locks and
the generated BUILD tree digest. `bazel/cargo/broker-workspace/**` joins the
generated-tree digest. It does not become a separate key component, because it
is derived from inputs the key already binds; adding it to the digest is what
makes a stale generator visible as a cache miss rather than a wrong hit.

**Repository surface.** Fifteen new tracked files: one root manifest, six stub
manifests, six empty `src/lib.rs` files, one lock mirror, one `BUILD.bazel`.
Measured at 139 lines of manifest for the real closure. All generated, all
drift-checked, none authored.

**Repin surface.** `cargo xtask bazel-repin --hub broker` keeps every property
ADR 0052 section 3 gave it: an explicit hub from the closed four-hub set,
exactly one Bazel child, `CARGO_BAZEL_REPIN` and `CARGO_BAZEL_REPIN_ONLY`
scoped to that child's environment and forbidden everywhere else, the wrapper's
derived output user root and output base so no second server starts, no Make
target and no workflow reachability, and exit zero with nothing changed when
the lock is already current. Two things change, both narrow.

It generates and validates the broker splice inputs itself instead of reading
them, so the mutation is one command rather than an ordered pair, and the
generated tree is not an input the contributor has to have gotten right first.

And the post-run rule becomes two rules. The Bazel child is still held to
exactly one changed tracked file, the hub's Bazel-side lock; that rule is not
weakened, and it is the one that catches an authoritative `Cargo.lock` being
rewritten. The command as a whole is held to that file plus the one subtree it
synchronized, `bazel/cargo/broker-workspace/**`, and it fails listing any other
changed path repository-relative. Nothing else in ADR 0052 section 3 moves.

**A partial state this command can leave behind.** If the Bazel child fails
after step 6, the generated subtree is current and `bazel/cargo/broker.lock` is
not. The command reports it, does not roll back, and does not overwrite work it
did not make; recovery is to fix the reported Bazel failure and re-run the same
command, which is idempotent, or `git restore bazel/cargo/broker-workspace` to
drop the change entirely. The residue cannot merge quietly, because section 5's
fourth gate fails the hub's next analysis with the substrate's repin-required
message. This is a deliberate trade: round 1 had no such residue and paid for
it with a refusal that made the ordinary mutation a two-command ritual.

**A stub change now makes the hub demand a repin.** With the stubs inside the
hub digest, a broker dependency change makes the next Bazel analysis of the
`broker` hub fail closed until `cargo xtask bazel-repin --hub broker` has run.
The generated tree and `bazel/cargo/broker.lock` therefore move together in one
pull request or that pull request does not pass. The shadow lane, which never
sets a repin control, reports the state as a build failure carrying the
substrate's own remedy rather than rendering silently from a stale lock. The
cost is one command on a dependency change, which is the command the
contributor was going to run anyway.

**Build cost.** Five first-party libraries compile twice. Their test targets
compile once, against the main hub, exactly as they do today: section 7 makes
the `-broker` variants library-only, so this decision adds no test target
anywhere. The duplicate third-party compilation, up to eighty-four crates
shared between the main and broker locks at identical versions, is
pre-existing under the four-hub design and is neither created nor removed
here.

**Discoverability cost, and it is real.** There is now a directory that looks
like a Cargo workspace, resolves like the broker workspace, and is not the
broker workspace. Someone will run `cargo build` in it and it will succeed,
because empty stubs compile. Mitigations: the generated marker on line one of
the root manifest, a generated `.bazelignore` entry for
`bazel/cargo/broker-workspace/target/`, and `gen-bazel --check` refusing when
the marker is absent.

**ADR 0052.** Nothing is reversed and nothing is superseded. Invariant 2 holds
unchanged: `Cargo.toml`, the three `Cargo.lock` files and the two
`rust-toolchain.toml` files remain the authoritative dependency and toolchain
inputs, and a dependency change is still a Cargo-file edit followed by a
regeneration. The generated splice workspace is a derived artifact of exactly
that class, not a second place a dependency is declared. Section 4's generator
gains one output class and one attribute in `MODULE.bazel`, the `broker` hub's
`manifests` list, which names generated paths and declares no dependency, so
the `from_specs` prohibition is untouched. The four-hub set, the three
authoritative locks, and the ban on the source bootstrap and the repin
environment controls are all unchanged.

The section 3 repin command is refined in exactly two places, both narrow and
both stated in full in the repin-surface consequence above: it synchronizes the
broker hub's generated splice inputs before it spawns Bazel, and its
changed-path rule splits into a child-scoped rule, unchanged from ADR 0052, and
a command-scoped rule permitting that one file plus the one subtree the command
synchronized. Its required hub argument, its scoped child environment, its
single output base, its single child process, its exit-zero-when-current
behaviour and its absence from Make and continuous integration are unchanged.

### The specific failures this design makes possible

**An optional dependency the broker never activates.** Constraint 11 is this
failure caught early. A generator that reads declared dependency tables rather
than realized resolve edges pulls `bolero` and twenty-nine more packages into
the broker hub, none of which the authoritative lock pins and none of which
`cargo-deny` or `cargo-audit` sees under `rust-deny-broker`. It fails closed
today, at the constraint 4 `--locked` pass, which is why the implementation
checks below require the offline check to be demonstrated failing on exactly
this mutation rather than only passing.

**A stub that is stale in a way the lock cannot see.** Add an optional
dependency to `d2b-core` that nothing enables, or change a `[features]`
default. The resolved version set does not move, so
`cargo metadata --locked` on the generated tree still passes and the repin
still succeeds, while the broker hub renders a feature set Cargo would not
produce. The guard is that the tree is generated rather than authored:
`gen-bazel --check` regenerates from `cargo metadata` and byte-compares, so the
pull request that edited the manifest fails `test-drift`. This is why section 5
requires both gates and why removing the byte check because the lock check
"already covers it" is not permitted.

**A first-party dev-dependency that quietly diverges the two paths.** Add
`ttrpc` to `d2b-host`'s dev-dependencies, as already exists. Under Cargo the
broker's lock does not gain it, because a non-member path dependency's
dev-dependencies are not resolved. Under a design that made those crates
members it would, and the two paths would test different graphs. This design
refuses by construction: the stubs for path-dependency packages carry no
dev-dependencies, the `-broker` variants they render carry no test target at
all (section 7), and the offline `--locked` check proves the first half.

**A hub rendered from a dependency set the repository no longer has.** Enable a
feature on an existing dependency of `packages/d2b-priv-broker`, which moves no
resolved version, and get `bazel/cargo/broker.lock` rendered from the previous
stub tree. The committed hub lock then describes a manifest set the repository
no longer has, and it renders the broker's crates with a feature set Cargo
would not produce. Three of the four gates are structurally blind to it.
`gen-bazel --check` compares the tree to the manifests and they agree. The
offline `cargo metadata --locked` compares the tree to the lock and they agree.
Constraint 4 never fires, because the resolve did not move.

Round 1 reached this state by running the repin before the regeneration, and
guarded it by refusing. Under section 6 that sequence does not exist: the repin
generates the tree it renders from, in the same run, so the input can never be
one the repository has since replaced. What survives is a narrower path to the
same wrong artifact, a Bazel child that fails after the subtree is synchronized
and a contributor who commits instead of re-running. The guard is section 5's
fourth gate, the stub manifests inside the hub digest, which is the only check
that compares the hub lock to the tree and which fails the hub's next analysis
with the substrate's repin-required message.

**A mutation command that destroys work it did not make.** This is the failure
self-synchronization newly makes possible, and it is worth naming plainly: the
repin is now a writer, and a writer that runs `cargo xtask gen-bazel`'s
emission over a subtree can silently discard whatever was already there. The
realistic shapes are a contributor mid-review with a hand edit under
`bazel/cargo/broker-workspace/` they were about to revert, a half-applied patch
or a conflicted merge in that subtree, and an uncommitted
`bazel/cargo/broker.lock` from an earlier run being replaced by a run whose
result nobody has looked at. The guard is section 6 step 4, which reads before
it writes and permits exactly two states per path, the committed content and
the content this run would produce, so the only bytes the command can replace
are bytes it can reproduce or bytes that are already in git. Everything else is
listed repository-relative and refused. The reason it is a refusal rather than
a stash or a backup copy is that a command with one output set and no hidden
state is reviewable; a command that relocates a contributor's work somewhere is
one more place to look when something goes missing.

**The privileged binary linking a library the broker lock never described.**
Bind `//packages/d2b-priv-broker:d2b-priv-broker` to
`//packages/d2b-contracts:d2b-contracts` instead of to its `-broker` variant,
which is one wrong label in a generated file. The binary that runs as root then
contains third-party code resolved from the main lock, and the two surfaces
that exist to notice keep passing, because `rust-deny-broker` and
`rust-audit-broker` read `packages/d2b-priv-broker/Cargo.lock` and that lock is
still internally true about a graph the build no longer has. The guard is
section 7's first-party target-set check, which reports the offending
first-party edge by name. The supplemental spoke check would also fire here,
because `d2b-contracts` has third-party dependencies today, but it is not what
this rests on: it reports a `@main//` alias in the broker graph, and against a
first-party crate with no third-party dependencies it reports nothing.

**A hub that silently omits a first-party crate.** Give
`d2b-guest-shell-runner` a path dependency on `d2b-contracts`. Its hub would
splice as `SplicerKind::Workspace`, symlink only its own directory, and fail
exactly as constraint 1 shows, at repin time, possibly months later. The guard
is section 4's third refusal: the generator inspects every workspace's lock for
entries without a `source` field and refuses at `gen-bazel` time, on the pull
request that added the dependency.

## Alternatives considered

**Add the five path manifests to the `broker` hub.** Committed as `65fbe095`
and reverted by `e80ac1ef`. Refused by `SplicerKind::new` under constraint 2.
No attribute, ordering, or subsetting changes the outcome, because the refusal
is on the set of distinct `parent_workspace` results and `packages/Cargo.toml`
is unavoidably one of them.

**Keep the single broker manifest and fix it some other way.** Constraint 1.
The path dependencies are unreachable from the splice tree, and there is no
`crate_universe` attribute that reaches back out of it. `direct_packages`
joins the non-hermetic workspace directory onto declared paths, but it injects
into the synthetic root package, never into a member's own dependency table.

**Merge the broker into the main Cargo workspace.** This is the option that
makes the blocker disappear, and it is rejected on security grounds rather
than build grounds. It deletes the broker's independent 117-package lock, and
with it the independent `rust-deny-broker` and `rust-audit-broker` surfaces
that ADR 0009 and ADR 0052 invariant 14 name. It also changes what runs as
root: a merged resolve unifies each compatible range onto one version, and the
main lock's current pins for seven ranges the broker resolves alone differ from
the broker's, so the privileged binary would move to `tinyvec 1.11.0` from
`1.12.0`, `regex-syntax 0.8.11` from `0.8.10`, `smallvec 1.15.2` from
`1.15.1`, `memchr 2.8.2` from `2.8.0`, `log 0.4.32` from `0.4.30`,
`typenum 1.20.1` from `1.20.0`, and `syn 2.0.118` from `2.0.117`. It would
couple every future version of the privileged binary's dependency closure to
the requirements of fifty-six unprivileged crates. It would also
require amending ADR 0052 from four hubs to three and from three authoritative
locks to two. The blocker is a build-tool limitation; it does not get to
relocate a security boundary.

**Import the broker's third-party crates in its own hub but bind the
first-party path crates to main-hub Bazel targets.** Rejected on correctness.
The broker binary would link a version mixture that neither lock describes:
`d2b-host` compiled against `@main//:nix` while the broker crate compiles
against `@broker//:nix`, with the main hub's `nix` alias selecting from
`{0.26.4, 0.29.0, 0.31.3}` and the broker lock pinning `0.29.0`. Twenty-seven
of the broker's registry entries diverge (constraint 8), so this is not a
theoretical mixture. It also makes the broker's independent lock decorative,
which is the same loss as the merge with none of the simplification.

**Upgrade or patch `rules_rust`.** 0.73.0 is the newest release as of this
date (constraint 10), so there is nothing to upgrade to. Patching the splicer
to copy `[workspace.*]` tables from a member's real parent workspace into the
synthetic root would work, and it would put a repository fork of a third-party
build ruleset on the trusted build path. ADR 0052 section 3 pins
`rules_rust` to a single explicit Bazel Central Registry version and refuses
the non-reproducible source bootstrap for the same reason. A fork is worse
than either. Revisit only if upstream lands the capability in a release.

**Declare the broker's third-party dependencies manually through
`crate.from_specs`.** Rejected. It moves dependency declaration into
`MODULE.bazel`, which ADR 0052 invariant 2 forbids in as many words, and it
still needs a lock consistent with the synthetic manifest, so it pays the same
cost as the generated tree while losing the property that the tree is derived
from `cargo metadata` and byte-checkable against it.

**One synthetic package declaring the union of the closure's third-party
requirements.** Rejected as the more fragile of the two synthetic shapes. It
requires merging version requirements and feature sets across six crates,
which is a second resolver implementation in `xtask`, and it needs a fourth
lock-shaped file that is not byte-identical to any authoritative lock, so the
strongest available check degrades from byte equality to set comparison. The
per-crate stub tree is a per-package copy, not a merge, and keeps the lock
mirror byte-identical to the authority.

**Splice `packages/` with a sibling root manifest such as
`packages/Cargo.bazel-broker.toml`.** This selects `SplicerKind::Package`, puts
all six crates in reach, and resolves inheritance from the mirrored
`[workspace.*]` tables. It fails on constraint 3: `packages/Cargo.lock` is
symlinked into the splice tree as `Cargo.lock`, `skip_cargo_lockfile_overwrite
= True` never replaces it, and the `--locked` metadata pass reads the main
workspace's lock against a broker manifest and refuses. Measured, with the
constraint 4 message. It also requires deleting the broker manifest's
`[workspace]` table, since a nested workspace root among the members produces
`error: multiple workspace roots found in the same workspace`.

**Symlink the real crate directories under a generated splice root.** Measured
workable up to the point where it is not. It requires mirroring three
`[workspace.*]` tables (constraint 7), it drags the five crates'
dev-dependencies into the resolve so the authoritative lock no longer satisfies
it (constraint 6, and the tree needs `regex` and `ttrpc`, which the broker lock
does not contain, with no Cargo operation that could add them), and the
in-tree symlinks make Bazel define a second package for every symlinked crate
unless each is listed in `.bazelignore`. Three problems for one mechanism.

**Drop `skip_cargo_lockfile_overwrite` for the broker hub only.** This makes
the splice copy the authoritative lock in and `cargo fetch` re-resolve, which
solves constraint 3 by re-introducing constraint 5: measured, the source lock
is rewritten. ADR 0052 section 3 forbids it and is right to.

**Name every stub manifest in the hub's `manifests` attribute and let the
substrate detect the staleness.** Measured compatible, rejected as the primary
guard in round 1 of this record, and adopted in round 2 as the second net.
`SplicerKind::new` bails only when the supplied manifests
resolve to more than one distinct `parent_workspace`; every stub resolves to
the generated root, so listing the root plus its members still selects
`SplicerKind::Workspace`, splices identically, and prints an upstream `INFO`
line saying the extra entries can be removed. Each listed manifest's parsed
content then enters the hub digest (constraint 13), so a stub tree that does
not match the committed `bazel/cargo/broker.lock` makes `determine_repin` fail
closed.

It is rejected as the primary guard because of when it fires: the digest is
consulted the next time Bazel analyses the hub, which is after a wrong
`bazel/cargo/broker.lock` has already been written, and it cannot run inside
the repin command at all. A property that is an ordering between two
repository-owned commands is enforceable in the commands themselves, and
section 6 does better than enforce it, by removing the ordering. But late is
precisely the right time for a net under a command that has already failed:
section 6's one partial-failure state, subtree current and hub lock stale, is
by definition not catchable by the run that produced it, and the four gates of
section 5 otherwise contain nothing that compares the hub lock to the tree.
Section 2 therefore names the stubs. The cost is stated in the consequences: a
broker dependency change makes the hub demand a repin before it will analyse.

**Have the repin refuse a stale generated tree and require a separate
`cargo xtask gen-bazel` first.** This was round 1 of this record and it is
rejected. It is not wrong about the hazard, and the refusal it specifies is
implementable; it is wrong about the remedy. The contributor's response to
"your generated inputs are stale, run `cargo xtask gen-bazel`" is to run
`cargo xtask gen-bazel`, which the command could have run itself from the same
function with the same inputs, so the refusal buys no decision and no review,
only a second command. It also leaves the ordering it enforces intact and
therefore keeps a hazard alive to enforce against, where generating in-run
deletes the hazard: a run that produces the tree it renders from has no stale
input to have. What round 1 correctly established, and section 6 keeps, is that
the guard cannot be a post-run diff, that no gate or build path may generate,
and that the command must never silently repair state a contributor is looking
at. The last of those is why section 6 refuses ambient modification instead of
overwriting it: the round-1 objection, that a repin quietly fixing its own
inputs makes the state it should have refused unobservable, is answered by
bounding what may be replaced to bytes the run can reproduce or bytes already
in git, not by refusing to write.

**Roll back the synchronized subtree when the Bazel child fails.** Rejected.
It would rewrite a tree the command just proved correct against the
authoritative lock, in order to restore a tree it proved stale. It makes a run
that did real work indistinguishable from one that never started, which is the
opposite of what a failure report should leave behind. It cannot be made safe
against a concurrent edit arriving between the synchronization and the
rollback, and the rollback itself is a write the changed-path check would then
have to be taught to permit. The recorded consequence plus an idempotent
re-run is smaller and honest.

**Let a gate or the Make wrapper regenerate when it finds the tree stale.**
Rejected, and it is the shape this repository has refused before. A gate that
repairs its own input cannot fail on that input, so `test-drift` would report
green on a repository whose committed tree is stale, and the staleness would
first become visible wherever the regeneration did not run. `gen-bazel --check`
stays read-only and fail-closed, and the only two writers of the generated
splice workspace are contributor-invoked commands neither Make nor continuous
integration can reach.

## Invariants this decision creates

1. `packages/d2b-priv-broker/Cargo.toml` and
   `packages/d2b-priv-broker/Cargo.lock` are authoritative and are never
   written by a generator, a Bazel command, or a repin. The broker remains a
   Cargo workspace distinct from `packages/Cargo.toml`.
2. Every hub keeps `skip_cargo_lockfile_overwrite = True`,
   `cargo_lockfile` pointing at an authoritative Cargo lock, and `lockfile`
   pointing at a committed Bazel-side lock. The `broker` hub's `manifests`
   attribute is the only one that names generated manifests, and it names
   exactly the generated splice root plus the stub manifest of every package in
   the closure, sorted, so every stub is inside the hub digest. That list is
   emitted by `cargo xtask gen-bazel` and byte-checked by `--check`.
3. `bazel/cargo/broker-workspace/**` is generated in full by
   `cargo xtask gen-bazel` and is never hand-edited. Its `Cargo.lock` is
   byte-identical to `packages/d2b-priv-broker/Cargo.lock`.
4. The generated tree's resolution equals the authoritative broker lock. This
   is proved twice and both proofs are enforcing: byte equality through
   `cargo xtask gen-bazel --check`, and
   `cargo metadata --manifest-path bazel/cargo/broker-workspace/Cargo.toml
   --locked --offline` exiting zero. Removing either because the other exists
   is not authorized; they catch different drift. A third proof, the stub
   manifests inside the hub digest, binds `bazel/cargo/broker.lock` to the tree
   it was rendered from and is likewise not removable.
5. A stub carries dev-dependencies if and only if its package appears in the
   broker workspace's `workspace_members`, and carries an optional dependency
   if and only if the broker workspace's resolve activates it. The generator
   derives both from `cargo metadata` and hard-codes neither.
6. The generator refuses, by name and with a remedy, when a package in the
   closure declares `build` or `links`, when a package's directory name and
   package name disagree, or when any Cargo workspace resolves a lock with
   more than one `source`-less entry and has no generated splice workspace.
7. First-party crates in the broker's closure carry two Bazel library
   variants, and the `-broker` variant is a library target only: it carries no
   `rust_test`, no `rust_doc_test`, and belongs to no test suite. A package
   compiled against the broker hub carries test targets if and only if it is a
   member of the broker Cargo workspace, which today is exactly
   `d2b-priv-broker`. The five closure crates' tests stay on their
   main-workspace variants under `rust-main-workspace-tests`. Both library
   variants appear in the ADR 0052 section 5 coverage map.
8. `cargo xtask bazel-repin --hub broker` is a closed mutation that
   synchronizes its own generated inputs. In order, and writing nothing outside
   an ignored scratch root until the synchronization step: it regenerates the
   broker splice workspace into scratch through the same generator entry point
   `cargo xtask gen-bazel` calls, never reading the tracked subtree as an
   input; it validates that
   scratch tree, requiring section 4's refusals to pass, its lock mirror to be
   byte-identical to `packages/d2b-priv-broker/Cargo.lock`, and
   `cargo metadata --locked --offline` on it to exit zero; it refuses when any
   path under `bazel/cargo/broker-workspace/`, tracked or untracked, is neither
   its committed `HEAD` content nor the freshly generated content, or when the
   subtree holds a path outside the union of those two file sets, or when
   `bazel/cargo/broker.lock` differs from its committed `HEAD` content; it
   refuses when the committed `manifests` list for the hub is not what the
   fresh generation implies, naming `MODULE.bazel` and
   `cargo xtask gen-bazel`; and it then synchronizes only
   `bazel/cargo/broker-workspace/**` from the validated result. It writes
   nothing under `packages/`, no other hub's inputs, and not `MODULE.bazel`.
   Every refusal above names every offending path repository-relative and
   spawns no Bazel process, creates no output base, and places
   `CARGO_BAZEL_REPIN` and `CARGO_BAZEL_REPIN_ONLY` in no child environment.
9. That command then spawns exactly one Bazel child under ADR 0052 section 3's
   scoped controls and derived output root. Two changed-path rules bound the
   result. The child may change only `bazel/cargo/broker.lock`, which is ADR
   0052's rule unweakened. The command's own change set, computed by
   subtracting a pre-run worktree snapshot from the same snapshot retaken, must
   be a subset of `bazel/cargo/broker-workspace/**` plus
   `bazel/cargo/broker.lock`; any other changed path fails the command and is
   listed repository-relative. When the child fails after synchronization the
   command reports the resulting state, subtree current and hub lock unchanged,
   and never rolls the subtree back and never overwrites a modification it did
   not make; re-running the same command is the recovery and is idempotent.
10. This decision adds no Make target, no Layer-1 job, no required
    continuous-integration context, and no top-level shell gate. Its checks
    extend `test-drift`, which already exists for this class of staleness. No
    gate, Make target, workflow, or Bazel invocation on the gate path generates
    a tracked artifact. The only writers of
    `bazel/cargo/broker-workspace/**` are `cargo xtask gen-bazel` and, for that
    subtree alone, `cargo xtask bazel-repin --hub broker`; both are
    contributor-invoked and neither is reachable from Make or continuous
    integration. `cargo xtask gen-bazel --check` in `test-drift` stays
    read-only and remains the fail-closed gate for a stale tracked tree.
11. Let B be every target of a broker Cargo workspace member plus every
    `-broker` variant, and M every other first-party Rust target. Over Rust
    compile and link edges, the `deps` and `proc_macro_deps` closure, the
    first-party portion of `deps(B)` is a subset of B and the first-party
    portion of `deps(M)` is a subset of M. Runfiles, `data` and source-file
    edges are outside this invariant, since they carry no compilation across
    the boundary. Both sets are read from what the generator emitted, not
    matched from label text. The `@main//` and `@broker//` spoke assertion is
    supplemental and may not be the only check.

## Implementation checks

These are the mechanically evaluable conditions that make this decision
implementable without reopening it, so Spec 003 can be amended after this
record merges. Each is a command and a verdict.

1. `cargo xtask gen-bazel` on a clean tree, then `git status --short`, lists
   only `MODULE.bazel`, `bazel/cargo/broker-workspace/**`, `.bazelignore`, and
   the first-party `BUILD.bazel` files the generator already owns.
2. `cargo xtask gen-bazel --check` exits zero on the committed tree, and exits
   nonzero naming the file for each of these planted mutations, reverted after
   each: a version bump in a stub manifest, a removed `[features]` entry, a
   removed dev-dependency from the `d2b-priv-broker` stub, an added
   dev-dependency to a path-dependency stub, one byte changed in
   `bazel/cargo/broker-workspace/Cargo.lock`, a removed generated marker
   from the root manifest, and a stub path removed from the `broker` hub's
   `manifests` list in `MODULE.bazel`.
3. `cmp packages/d2b-priv-broker/Cargo.lock
   bazel/cargo/broker-workspace/Cargo.lock` exits zero, and
   `bazel/cargo/broker-workspace/Cargo.toml` declares the same resolver
   version the broker workspace resolves under.
4. `cargo metadata --manifest-path bazel/cargo/broker-workspace/Cargo.toml
   --locked --offline` exits zero, and exits nonzero with the constraint 4
   message for each of these planted mutations, reverted after each: a
   third-party dependency added to any stub, and an unactivated optional
   dependency of a closure package restored into its stub. Both must be
   demonstrated failing, not only asserted.
5. `cargo metadata --manifest-path packages/d2b-priv-broker/Cargo.toml
   --locked --format-version 1` exits zero and reports `workspace_root` at
   `packages/d2b-priv-broker` and exactly one entry in `workspace_members`,
   before and after the change, proving `packages/` was not touched.
6. `git diff --stat` over the implementing commit range shows no change under
   `packages/d2b-priv-broker/`.
7. `cargo xtask bazel-repin --hub broker` on the committed tree exits zero and
   changes nothing, because the generated subtree and the committed Bazel-side
   lock are both current. After a deliberate broker dependency change and with
   no other command run in between, the same invocation exits zero and reports
   exactly two kinds of changed path, `bazel/cargo/broker-workspace/**` and
   `bazel/cargo/broker.lock`, and no other tracked change. A subsequent
   `cargo xtask gen-bazel --check` exits zero over the subtree without further
   modification, which is the evidence that the repin's emission and the
   generator's emission are the same bytes. Running `cargo xtask gen-bazel`
   first and then the repin leaves the subtree byte-identical to the
   repin-first result.
8. The command refuses ambient work instead of overwriting it, and refuses
   before Bazel. For each of these planted pre-states, reverted after each,
   `cargo xtask bazel-repin --hub broker` exits nonzero, lists the offending
   path repository-relative, names both remedies, and leaves every path under
   `bazel/cargo/broker-workspace/` and `bazel/cargo/broker.lock`
   byte-unchanged: a hand edit to a stub manifest that a fresh generation would
   not produce; an untracked extra file under the subtree; a deleted stub
   `src/lib.rs`; and an uncommitted one-byte change to
   `bazel/cargo/broker.lock`. Each refusal is also produced identically with
   the Bazel binary absent from the command's `PATH`, which is the evidence
   that the refusal precedes the Bazel spawn rather than following it; the
   control is that the same run on a clean tree fails instead on the missing
   Bazel binary. No output base is created under the wrapper's derived output
   user root by any refused run.
9. The command refuses on the one input it may not write. With a planted
   first-party path dependency added to `packages/d2b-priv-broker/Cargo.toml`,
   so the closure membership changes and the hub's `manifests` list would move,
   `cargo xtask bazel-repin --hub broker` exits nonzero naming `MODULE.bazel`
   and `cargo xtask gen-bazel`, spawns no Bazel, and leaves the generated
   subtree and `bazel/cargo/broker.lock` byte-unchanged. After
   `cargo xtask gen-bazel` the same command proceeds. Reverted.
10. The round-1 ordering hazard is unreachable, and its residue is bounded.
    With a planted feature-only change to a broker manifest, one that moves no
    resolved version so nothing can be attributed to constraint 4, a single
    `cargo xtask bazel-repin --hub broker` exits zero and reports the subtree
    and `bazel/cargo/broker.lock` as the only changed paths, with no
    intervening `cargo xtask gen-bazel`. With the same planted change and the
    Bazel child forced to fail, for example through an injected failing wrapper
    on the command's `PATH`, the command exits nonzero, the subtree is
    byte-equal to a fresh `cargo xtask gen-bazel` emission,
    `bazel/cargo/broker.lock` is byte-unchanged, and the message names both
    paths and names re-running the same command as the recovery. Re-running it
    with the real binary then exits zero and reports `bazel/cargo/broker.lock`
    as the only further change.
11. The digest net fires. On a tree whose subtree and hub lock are both
    current, plant a one-byte change in one stub manifest and run an ordinary
    Bazel build of a `broker` hub target with no repin control anywhere in the
    environment: it fails closed with the substrate's repin-required message
    naming the `broker` hub, and `bazel/cargo/broker.lock` is byte-unchanged.
    Reverted. As a precondition of that check,
    `bazel query 'labels(srcs, //bazel/cargo/broker-workspace:all)'` resolves
    the seven manifest labels the `manifests` attribute names, all within the
    single generated `BUILD.bazel` package.
12. No `-broker` test target exists, and the check that says so refuses one.
    `bazel query 'kind(".*_test rule", <emitted broker target set>)'` names
    only targets of `//packages/d2b-priv-broker`, and the full first-party
    test target set, `bazel query 'kind(".*_test rule", //packages/...)'`, is
    equal before and after this change. No `-broker` label appears in either
    result. This must be demonstrated as a refusal, not only as an empty
    result: with a planted generator mutation that emits one `rust_test` for a
    `-broker` variant, regenerating and re-running the query fails and names
    that target. Reverted after. A check that has never been seen to fail is
    not evidence that the property holds.
13. The two set conditions of invariant 11 hold. The primary evaluation is
    over the generator's emitted compile-edge map, `deps` and
    `proc_macro_deps`, and reports empty for both directions: first-party
    edges out of B that land outside B, and first-party edges out of M that
    land in B. A `bazel query` over `kind("rust_library rule", ...)` and the
    emitted label sets confirms the same two results against the analysed
    graph. Each direction is demonstrated failing against its own planted
    mutation, reverted after each, and one mutation does not stand in for
    both: for B to M, rebind one `-broker` variant's dependency edge to the
    unsuffixed first-party variant, and require the check to fail naming that
    edge; for M to B, rebind one main-hub first-party target's dependency edge
    to a `-broker` variant, and require the check to fail naming that edge. The
    supplemental `@main//` and `@broker//` spoke assertion is present and is
    not the only check.
14. The Bazel-side hub lock's registry `(name, version)` key set equals the
    registry `(name, version)` key set of
    `packages/d2b-priv-broker/Cargo.lock`, checked offline, with 111 entries
    today.
15. `bazel query '@broker//:all'` names only crates present in
    `packages/d2b-priv-broker/Cargo.lock`, and `bazel build` of two of them
    succeeds. Two pieces of this are already measured. On the reproduction of
    this repository's workspace shape, the hub's rendered set matched the
    authoritative lock exactly, including the workspace member's own
    dev-dependency and excluding a path-dependency crate's dev-dependency,
    with both authoritative locks unchanged after the run and the lock mirror
    still byte-identical. On the real tree, the six-package stub tree built
    from the resolve satisfies `cargo metadata --locked --offline` against a
    byte-identical copy of the authoritative broker lock. What remains for the
    implementer is running the real hub through Bazel end to end.
16. A planted `build = "build.rs"` key on a closure package, a planted
    directory-name mismatch, and a planted first-party path dependency in
    `packages/d2b-guest-shell-runner/Cargo.toml` each make
    `cargo xtask gen-bazel` exit nonzero with its own named refusal and its own
    remedy; all three are reverted.
17. `tests/unit/meta/adr-index-coverage.sh` passes with this record indexed.

## References

- `bazelbuild/rules_rust` at `refs/tags/0.73.0`:
  `crate_universe/src/splicing/splicer.rs` (`parent_workspace`,
  `SplicerKind::new`, `splice_workspace`, `splice_multi_package`,
  `inject_workspace_members`, `symlink_roots`, `write_root_manifest`),
  `crate_universe/src/cli/splice.rs` (the
  `skip_cargo_lockfile_overwrite` branch and the two `--locked` invocations),
  `crate_universe/src/metadata.rs` (`LockGenerator::generate` and its
  `cargo fetch`), `crate_universe/src/metadata/cargo_tree_resolver.rs`
  (the copied project and its `--locked` metadata call),
  `crate_universe/src/lockfile.rs` (`Digest::new` and its inputs) and
  `crate_universe/src/splicing.rs` (`SplicingMetadata::try_from`, which reads
  and parses only the named manifests),
  `crate_universe/private/generate_utils.bzl` (`determine_repin` and the
  `cargo-bazel query` call behind it),
  `crate_universe/extensions.bzl` (`_from_cargo`, `_FROM_COMMON_ATTRS` and the
  `if repin:` guard around the splice)
- `MODULE.bazel` and `bazel/cargo/README.md` on `spec003-w0-planfix`
- Spec 003 W0 experiments `65fbe095` (`bazel: include broker path manifests`)
  and `e80ac1ef` (`bazel: restore standalone broker hub declaration`), and the
  parked checkpoint at tip `a3e7d68c`
- `packages/Cargo.toml`, its `exclude` list and its `[workspace.package]`,
  `[workspace.dependencies]` and `[workspace.lints]` tables
- `packages/d2b-priv-broker/Cargo.toml` and its 117-package lock;
  `packages/d2b-guest-shell-runner/Cargo.lock` and
  `tests/tools/no-bash-ast-walker/Cargo.lock`, each with one `source`-less
  entry
- `tests/test-rust.sh` lines 504 through 512, the three broker feature passes
  and the `--workspace --manifest-path` form they use
- [ADR 0052](0052-bazel-rust-build-and-test.md) sections 2, 3, 4 and 5 and
  invariants 2, 9 and 14
- [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md), the per-lock
  supply-chain policy this record preserves
- `specs/003-adr052-bazel-rust/contracts/workspace-and-tool-pinning.md`, the
  hub and repin contract this record leaves intact
