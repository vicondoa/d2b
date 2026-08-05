# ADR 0054: A generated splice workspace for the privileged broker's Bazel dependency hub

- Status: Proposed
- Date: 2026-08-04
- Related: [ADR 0052](0052-bazel-rust-build-and-test.md) (Bazel as the Rust
  build and test scheduler), whose sections 2, 3 and 4 and invariant 2 this
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

Constraints 1 and 2 are the two committed experiments. Constraints 3, 4 and 6
together rule out every arrangement that splices the real first-party
manifests. Constraint 8 rules out serving the broker from the main hub or
mixing hubs. Constraint 10 rules out waiting. Constraint 11 fixes what the
generator must read.

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
    manifests = ["//bazel/cargo/broker-workspace:Cargo.toml"],
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
- `BUILD.bazel`, exporting `Cargo.toml` and `Cargo.lock`.

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
plus the authoritative lock file's bytes. `cargo xtask gen-bazel --check`
regenerates into a scratch tree and fails on any difference, exactly as it
already does for the first-party `BUILD.bazel` files and the governed-source
manifest under ADR 0052 section 4. It is wired into `test-drift`, which is
where every other `xtask gen-*` staleness is already caught.

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

### 5. Three independent gates, layered on purpose

- **Byte equality.** `cargo xtask gen-bazel --check` in `test-drift` proves the
  generated tree is what the authoritative manifests currently imply. This is
  the only gate that catches a change which moves no resolved version: a new
  optional dependency, a changed `[features]` table, a changed feature default.
- **Lock equivalence, offline, on every pull request.** `test-drift` also runs
  `cargo metadata --manifest-path bazel/cargo/broker-workspace/Cargo.toml
  --locked --offline` and requires exit zero. This proves the generated tree's
  resolution is exactly the authoritative broker lock, needs no network and no
  Bazel, and runs on every pull request rather than only when someone repins.
- **The substrate's own refusal.** `cargo xtask bazel-repin --hub broker`
  cannot produce a hub from a tree whose resolve differs from the lock,
  because constraint 4's `--locked` metadata pass refuses first. This is the
  backstop, not the primary gate, because repins are rare.

The first two are stated as a pair deliberately. Version drift and feature
drift are different failures and neither gate catches both.

### 6. Ordering, and the one place the order is load-bearing

Changing a dependency of the broker or of any crate in its closure is:

1. edit the Cargo manifest;
2. `cargo xtask gen-bazel`;
3. `cargo xtask bazel-repin --hub broker`.

Step 2 before step 3 is binding. A repin run against a stale generated tree
either splices the old dependency set into the hub lock or refuses under
constraint 4; the first outcome is silent and the second is confusing. The
repin command's existing rule, that it fails when any tracked file other than
the named hub's Bazel-side lock changed, now also protects the generated tree:
a contributor who skips step 2 gets a refusal naming a changed file rather
than a wrong hub.

### 7. Two first-party target sets, named

The five crates in the broker's closure are compiled twice, once against
`@main//` for the main workspace and once against `@broker//` for the broker's
three feature suites. This is forced by constraint 8, not chosen: the two locks
resolve different versions of crates those five crates depend on, so a single
compilation would be wrong for one of the two consumers.

The generator emits the broker-hub variant beside the main-hub one in the same
Bazel package, suffixed `-broker`: `//packages/d2b-contracts:d2b-contracts` and
`//packages/d2b-contracts:d2b-contracts-broker`. The suffix is greppable, the
`crate_name` is unchanged in both, and both appear in the ADR 0052 section 5
coverage map so an unmapped variant fails analysis rather than being merely
absent.

Binding a broker target to a main-hub first-party library is forbidden. A
graph check derived from the coverage map asserts that no target reachable
from a `-broker` variant depends on a `@main//` spoke, and no target reachable
from a main-hub variant depends on a `@broker//` spoke.

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

**Repin surface.** Unchanged in shape. `cargo xtask bazel-repin --hub broker`
still touches exactly one Bazel-side lock, still refuses ambient repin
controls, and still scopes `CARGO_BAZEL_REPIN_ONLY` to one child process. Its
input set gains the generated tree.

**Build cost.** Five first-party libraries and their test targets compile twice.
The duplicate third-party compilation, up to eighty-four crates shared between
the main and broker locks at identical versions, is pre-existing under the
four-hub design and is neither created nor removed here.

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
gains one output class. The four-hub set, the three authoritative locks, the
repin command's contract, and the ban on the source bootstrap and the repin
environment controls are all unchanged.

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
dev-dependencies, and the offline `--locked` check proves it.

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

## Invariants this decision creates

1. `packages/d2b-priv-broker/Cargo.toml` and
   `packages/d2b-priv-broker/Cargo.lock` are authoritative and are never
   written by a generator, a Bazel command, or a repin. The broker remains a
   Cargo workspace distinct from `packages/Cargo.toml`.
2. Every hub keeps `skip_cargo_lockfile_overwrite = True`,
   `cargo_lockfile` pointing at an authoritative Cargo lock, and `lockfile`
   pointing at a committed Bazel-side lock. The `broker` hub's `manifests`
   attribute is the only one that names a generated manifest, and it names
   exactly one.
3. `bazel/cargo/broker-workspace/**` is generated in full by
   `cargo xtask gen-bazel` and is never hand-edited. Its `Cargo.lock` is
   byte-identical to `packages/d2b-priv-broker/Cargo.lock`.
4. The generated tree's resolution equals the authoritative broker lock. This
   is proved twice and both proofs are enforcing: byte equality through
   `cargo xtask gen-bazel --check`, and
   `cargo metadata --manifest-path bazel/cargo/broker-workspace/Cargo.toml
   --locked --offline` exiting zero. Removing either because the other exists
   is not authorized; they catch different drift.
5. A stub carries dev-dependencies if and only if its package appears in the
   broker workspace's `workspace_members`, and carries an optional dependency
   if and only if the broker workspace's resolve activates it. The generator
   derives both from `cargo metadata` and hard-codes neither.
6. The generator refuses, by name and with a remedy, when a package in the
   closure declares `build` or `links`, when a package's directory name and
   package name disagree, or when any Cargo workspace resolves a lock with
   more than one `source`-less entry and has no generated splice workspace.
7. First-party crates in the broker's closure carry two Bazel library
   variants. No target reachable from a `-broker` variant depends on a
   `@main//` spoke and no target reachable from a main-hub variant depends on
   a `@broker//` spoke. Both variants appear in the ADR 0052 section 5 coverage
   map.
8. Regeneration order is `gen-bazel` then `bazel-repin --hub broker`. The
   repin command's existing rule, that it fails when any tracked file other
   than the named hub's Bazel-side lock changed, covers the generated tree.
9. This decision adds no Make target, no Layer-1 job, no required
   continuous-integration context, and no top-level shell gate. Its checks
   extend `test-drift`, which already exists for this class of staleness.

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
   `bazel/cargo/broker-workspace/Cargo.lock`, and a removed generated marker
   from the root manifest.
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
   changes nothing, because the committed Bazel-side lock is current; and
   after a deliberate broker dependency change followed by
   `cargo xtask gen-bazel`, it reports a changed `bazel/cargo/broker.lock` and
   no other tracked change.
8. `cargo xtask bazel-repin --hub broker` run without the preceding
   `gen-bazel` on a tree whose broker manifest changed exits nonzero naming the
   stale generated file.
9. The Bazel-side hub lock's registry `(name, version)` key set equals the
   registry `(name, version)` key set of
   `packages/d2b-priv-broker/Cargo.lock`, checked offline, with 111 entries
   today.
10. `bazel query '@broker//:all'` names only crates present in
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
11. A planted `build = "build.rs"` key on a closure package, a planted
    directory-name mismatch, and a planted first-party path dependency in
    `packages/d2b-guest-shell-runner/Cargo.toml` each make
    `cargo xtask gen-bazel` exit nonzero with its own named refusal and its own
    remedy; all three are reverted.
12. `tests/unit/meta/adr-index-coverage.sh` passes with this record indexed.

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
  `crate_universe/extensions.bzl` (`_from_cargo` and `_FROM_COMMON_ATTRS`)
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
- [ADR 0052](0052-bazel-rust-build-and-test.md) sections 2, 3, 4 and 5 and
  invariants 2, 9 and 14
- [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md), the per-lock
  supply-chain policy this record preserves
- `specs/003-adr052-bazel-rust/contracts/workspace-and-tool-pinning.md`, the
  hub and repin contract this record leaves intact
