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
  this record refuses to dissolve;
  [ADR 0008](0008-supported-platforms-and-rejected-targets.md), whose kernel
  floor of 6.6 already covers the `openat2` and `renameat2` primitives section
  6's transaction uses, so this record raises no platform requirement.
- Scope: how the `broker` `crate_universe` hub declared in `MODULE.bazel`
  obtains a spliceable Cargo workspace, what the repository-owned generator
  must emit for it, and how the two contributor-invoked commands that write
  that generated tree do so safely. Build-graph shape and the commands that
  produce it.
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

### Measured constraints on the replacement primitive

Section 6 replaces a tracked directory that already exists with a directory of
the same shape, so the primitive that does it is part of the decision rather
than an implementation detail left to whoever writes the code. Measured on
2026-08-04 on Linux 7.0.10 with the worktree on ext4, by direct syscall from a
C probe rather than through a crate, so what follows is the kernel's answer and
not a library's documentation. Every primitive named here is already on this
repository's trusted path: `packages/d2b-host/src/hardlink_farm.rs` publishes a
staged tree with `renameat_with(.., RenameFlags::EXCHANGE)` today, and
`packages/d2b-host/src/bin/d2b-activation-helper.rs` resolves every untrusted
component with `openat2` and `ResolveFlags`. Both syscalls are far below the
ADR 0008 kernel floor of 6.6.

14. **`renameat2(RENAME_EXCHANGE)` swaps two non-empty directories in one
    step, and nothing else does.** Exchanging two non-empty directories
    returned 0, and afterwards each name resolved to the other's contents.
    `rename(2)` of a directory onto a non-empty directory returned
    `ENOTEMPTY`; `rmdir(2)` and `unlinkat(AT_REMOVEDIR)` on a non-empty
    directory returned `ENOTEMPTY`. So a path-based "remove the old tree, then
    move the new one in" cannot be written without a recursive delete, and it
    cannot avoid a window in which the tracked name is absent or half
    populated. The exchange has no such window: the name resolves to a
    complete tree before the call and to a complete tree after it. Measured on
    tmpfs as well, which also returned 0.
15. **The exchange is same-filesystem only, and cross-mount is `EXDEV`.**
    Exchanging a directory on the worktree's ext4 with one on `/dev/shm`
    returned `EXDEV` ("Invalid cross-device link"). A directory created by
    `mkdirat` under a descriptor is on that descriptor's filesystem by
    construction, which is why section 6 stages under `bazel/cargo/` and not
    under `.scratch/`: nothing in the repository constrains what `.scratch/`
    is mounted on, and a design whose atomicity depends on an unconstrained
    path is a design that fails on someone else's machine.
16. **The exchange refuses an absent destination, and `RENAME_NOREPLACE` is
    its exact complement.** `RENAME_EXCHANGE` with the destination absent
    returned `ENOENT`, and with the source absent returned `ENOENT`.
    `RENAME_NOREPLACE` returned 0 onto an absent destination and `EEXIST` onto
    an existing one. The two flags therefore cover the first-creation case and
    the replacement case with no overlap, and neither can clobber: whichever
    arm is wrong for the state on disk fails rather than removing something.
17. **Anchored resolution covers every operation this needs, and one detail
    bites.** `openat2` with
    `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS |
    RESOLVE_NO_XDEV` opened a multi-component relative path under a directory
    descriptor, refused a symlinked entry with `ELOOP`, and refused `../..`
    with `EXDEV`. `renameat2` accepted two distinct directory descriptors as
    its two anchors, and accepted `O_PATH` descriptors. But `fsync` on an
    `O_PATH` descriptor returned `EBADF`, while `fsync` on the same directory
    opened `O_RDONLY | O_DIRECTORY` returned 0, so the anchors section 6 makes
    durable are opened `O_RDONLY | O_DIRECTORY | O_CLOEXEC` and never
    `O_PATH`. `openat2` with `RESOLVE_NO_SYMLINKS` also refuses an absolute
    path whose ancestor is a symlink, with `ELOOP`, and a contributor's
    worktree legitimately sits under symlinked ancestors; so the worktree root
    is opened once by ordinary `open` and everything inside it is resolved
    beneath that descriptor.
18. **An open file description write lock excludes a second process and
    leaves nothing stale.** `F_OFD_SETLK` with `F_WRLCK` on a file opened
    through the anchored chain returned 0 for the first holder, `EAGAIN`
    ("Resource temporarily unavailable") for a second process while held, and
    0 again for that second process after the holder released it. The lock
    belongs to the open file description, so a killed writer leaves no lock
    for anyone to have to break. `xtask` already depends on `nix` for
    `F_OFD_SETLK` and on `rustix` for anchored opens, and
    `packages/xtask/Cargo.toml` records why, so neither is a new
    supply-chain edge.
19. **A descriptor that escapes into a child keeps that lock held after its
    holder is gone, and `O_CLOEXEC` is the only thing that prevents it.**
    Measured with the same probe, in two arms that differ by one open flag. A
    holder opened the lock file, took `F_OFD_SETLK` with `F_WRLCK`, forked a
    child that `exec`ed twice and outlived it, and then exited and was reaped.
    With the lock file opened `O_CLOEXEC`, the exec'd child's `/proc/<pid>/fd`
    named no descriptor for the lock file and a third process took the lock,
    returning 0. With `O_CLOEXEC` absent and nothing else changed, that child's
    `/proc/<pid>/fd` carried `3 -> <dir>/lock` and the third process was
    refused with `EAGAIN` even though no writer was running. An open file
    description lock is released only when the last descriptor referring to
    that description is closed, so a descriptor that survives `exec` into a
    process that outlives the command holds the lock for that process's
    lifetime, and no timeout, no `unlink` and no ownership record can take it
    back. Duplication defeats the flag: measured on the same descriptor,
    `dup`, `dup2` and `fcntl(F_DUPFD)` each returned a descriptor whose
    `F_GETFD` reported `FD_CLOEXEC` clear, while `fcntl(F_DUPFD_CLOEXEC)`
    preserved it.

Constraints 14 through 19 fix the transaction shape: exchange rather than
replace, stage as a sibling of the target rather than in `.scratch/`,
`RENAME_NOREPLACE` for the first creation, `O_RDONLY | O_DIRECTORY` anchors for
the descriptors that get fsynced, one open file description lock for
ownership, and `O_CLOEXEC` on that lock's descriptor so the ownership it
represents ends when the command does.

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

**One authorized sibling holds the transaction state.**
`bazel/cargo/.broker-workspace.txn/` is the only path outside
`bazel/cargo/broker-workspace/**` and `bazel/cargo/broker.lock` that this
decision authorizes either writer to create during a transaction; the four
quarantine slots the recovery mode below renames it onto are the only other
authorized paths, and no run that publishes creates one. It is a sibling of
the generated
subtree under the same anchored parent, which is what makes it the same
filesystem by construction rather than by hope (constraint 15), and it is
transaction state rather than general scratch: nothing generates into it,
nothing reads it as a build input, and between transactions it holds two files,
`lock` and `published`. During a transaction it also holds `staged/`, the copy
of the validated tree that will be exchanged in; `journal`, the durable record
recovery reads; `hub-lock.pre`, the exact prior bytes of
`bazel/cargo/broker.lock`; `journal.tmp` and `hub-lock.restore`, the
materialization names those two pass through before their rename; and
`probe.a` and `probe.b`, the two throwaway directories step 3 exchanges. Those
nine names are the whole of what either writer may create inside this
directory, and the set is closed on purpose: it is what lets step 2 remove the
residue a killed run left behind under a bound it can state, and it is what
makes a tenth name a refusal rather than a guess. The three names a run
rewrites in place, `journal.tmp`, `hub-lock.pre` and `hub-lock.restore`, are
created truncating rather than exclusively, so a stale one a crash left behind
is replaced rather than turned into an `EEXIST` during a recovery; `staged`
and every file under it are created exclusively, because there the
exclusivity is the check that step 2's sweep left nothing behind.
`published` is the receipt
of the last successful
publication, the same digest set the journal carried, installed by renaming
`journal` over it at teardown so one atomic operation both clears the recovery
trigger and records what was published. A committed `.gitignore` entry keeps
the directory out of
`git status --porcelain`, without which step 1's snapshot and step 14's
changed-path check would both trip over the command's own workspace, and a
generated `.bazelignore` entry keeps Bazel from defining a package for a staged
tree that carries its own `BUILD.bazel`. `cargo xtask gen-bazel --check`
refuses when either entry is missing.

Ordinary generation and validation still happen in the repository's ignored
`.scratch/` root, which is where an unvalidated tree belongs. Bytes cross from
there into the transaction directory by copy, once, after they are validated,
because nothing in the repository constrains what `.scratch/` is mounted on and
constraint 15 makes that the difference between an atomic publish and `EXDEV`.
The transaction directory never holds an unvalidated tree.

**One writer at a time.** Both writers of the generated subtree, this command
and `cargo xtask gen-bazel`, take an `F_OFD_SETLK` write lock on
`bazel/cargo/.broker-workspace.txn/lock` before touching it and hold it until
they exit, so a generator and a repin, or two repins, cannot race the exchange.
A second writer gets `EAGAIN` (constraint 18) and refuses, naming the lock path
and the fact that another generator or repin holds it. The lock belongs to the
open file description, so a killed writer leaves nothing stale to break,
provided the description dies with the writer: both writers open that file
`O_CLOEXEC` and never duplicate the descriptor, which is what keeps the
ownership token from being inherited by a child that outlives them
(constraint 19). It is a worktree file owned by the invoking user: this
decision creates no persistent root surface, no daemon, no unit, and no state
outside the worktree. `cargo xtask gen-bazel --check` is read-only and takes no
lock; because the subtree is published by exchange it can never observe a
partial tree, only the pre-write or the post-write one.

**The closed sequence.** `cargo xtask bazel-repin --hub broker` performs
exactly these steps, in this order, and fails closed at each. Steps 1 through 9
mutate no tracked path; they write only inside `.scratch/` and the transaction
directory. Step 10 is the only step that writes the generated subtree, and
`bazel/cargo/broker.lock` is written only by the child in step 12 and by step
13's restore of the snapshot taken before it.

1. **Snapshot.** Record the worktree state: `git status --porcelain` over the
   repository plus a content hash for every path it names. Step 14 retakes the
   same snapshot and subtracts, so the command's own change set is separable
   from whatever the contributor already had in flight.
2. **Anchor, lock, and clear residue no journal owns.** Open the worktree root
   once with an ordinary
   `open(O_RDONLY | O_DIRECTORY | O_CLOEXEC)`, because an absolute path whose
   ancestors are outside the repository may legitimately contain symlinks
   (constraint 17). Resolve `bazel/cargo` beneath it with `openat2` under
   `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS |
   RESOLVE_NO_XDEV`, opened `O_RDONLY | O_DIRECTORY | O_CLOEXEC` so the same
   descriptor can be a rename anchor and an fsync target. Every path this
   command reads or writes under `bazel/cargo/` is resolved relative to that
   descriptor under those flags, and no decision is ever taken from a `stat` on
   a string path and then acted on by a second open of the same string: the
   descriptor that answered the question is the descriptor that performs the
   operation. Then `mkdirat` the transaction directory, tolerating `EEXIST`,
   and take the write lock: resolve `lock` beneath the transaction directory's
   descriptor with `openat2` under the same four flags, opened
   `O_RDWR | O_CREAT | O_CLOEXEC` at mode 0600, and `fcntl(F_OFD_SETLK)` with
   `F_WRLCK` on it. Every descriptor this command opens against a worktree path
   carries `O_CLOEXEC`,
   including the two anchors, the transaction directory, every staged file of
   step 9, and the lock; the lock is the one where the flag is load bearing
   rather than hygienic, because step 12 spawns a Bazel client that leaves
   behind a server which outlives this command, and by constraint 19 a lock
   descriptor that survives `exec` into such a process holds the lock for that
   process's lifetime. The lock descriptor is opened in exactly one place and
   is never duplicated, since `dup`, `dup2` and `fcntl(F_DUPFD)` each hand back
   a descriptor with the flag cleared.

   Then, still holding the lock and before any other step runs, sweep the
   residue no journal accounts for. A run killed anywhere between this step
   and step 9's journal rename leaves entries behind that nothing owns:
   `staged` half populated by step 9, a `journal.tmp` that never reached its
   rename, `probe.a` and `probe.b` from step 3, a `hub-lock.pre` or
   `hub-lock.restore` orphaned by a teardown that did not finish. The next run
   would otherwise meet them as `EEXIST` from step 3's `mkdirat` or from step
   9's `O_EXCL`, or, worse, would stage into a directory its predecessor
   partly filled and exchange a tree it did not generate. The sweep is
   unconditional, runs before the probe and before any staging, and is bounded
   by the closed name set above rather than by anything it reads on disk.
   `lock` and `published` are never removed by it; they are the ownership
   token and the receipt, not transaction state. `journal` decides the rest.
   Absent, no recovery can be decided from anything, so nothing owns the
   residue and every entry named `staged`, `journal.tmp`, `hub-lock.pre`,
   `hub-lock.restore`, `probe.a` or `probe.b` is removed. Present and
   parseable at a format version this build knows, step 4 owns every one of
   them and this sweep removes nothing at all. Present and not parseable at a
   known version, the command exits nonzero naming `journal`, deletes nothing,
   and prescribes no remedy that deletes it, because a record this build
   cannot read is the one thing that could have bounded a removal. What it
   prescribes instead is this command's own recovery mode,
   `cargo xtask bazel-repin --hub broker --quarantine-transaction-state`, run
   once this command has exited, followed by re-running the ordinary repin. An
   entry
   under any name outside the closed nine is refused the same way: listed
   repository-relative, left exactly where it is, no other entry swept, and
   the command exits nonzero. Its remedy is that same quarantine whatever the
   listed entry turns out to be, because the quarantine's bound does not
   depend on what the entry is: it moves the directory the entry is in,
   without reading the entry, naming it in anything executable, or removing
   it. The listing itself is diagnostic only. It renders each unrecognized
   name with bytes outside printable ASCII escaped, and that rendering appears
   in no command line the contributor is told to run, so a name nobody chose
   never reaches a shell. The paragraphs on transaction-directory recovery
   below record what that mode does, what it deliberately does not do, and why
   a stash with per-shape removals cannot clear this refusal.

   Removing `staged` when there is no journal is safe by construction rather
   than by inspection. Step 9 makes the journal durable before step 10
   exchanges, so no journal means no exchange, so `staged` can only be an
   unpublished staging area that no tracked path has ever occupied. The
   removal is bounded the same way step 11's is, by descriptor rather than by
   trust: every entry is resolved from the transaction directory's descriptor
   with `openat2` under the four resolve flags of this step, `fstat`ed on the
   descriptor that will be unlinked, and required to be a directory for the
   three directory names or a regular file for the three file names, so a
   symlink, a device node, a fifo or a socket planted under one of those names
   refuses and is reported instead of being followed or unlinked blind;
   `staged` is walked bottom-up under the same per-entry rule with
   `unlinkat(AT_REMOVEDIR)` per directory, which by constraint 14 cannot
   outrun its own bound. `ENOENT` is success throughout, because the sweep
   names a goal state and not an action: a sweep killed halfway is finished by
   the next one, and a sweep with nothing to do is a no-op, which is what
   makes running the command twice in a row indistinguishable from running it
   once.
3. **Prove the primitive before doing any work.** `mkdirat` `probe.a` and
   `probe.b` inside the transaction directory, two fixed names rather than
   generated ones so that step 2's sweep has a closed set to bound itself by,
   and exchange them with
   `renameat2(RENAME_EXCHANGE)`. `EINVAL`, `ENOSYS`, `EOPNOTSUPP` or `EPERM`
   here means this worktree is on a filesystem that cannot publish the subtree
   safely; the command exits nonzero naming the filesystem and the remedy,
   having read no manifest and written nothing but two empty directories it
   then removes. There is no recursive fallback, here or anywhere: the
   alternative to the exchange is refusal.
4. **Finish or refuse an interrupted transaction.** A `journal` present in the
   transaction directory means a previous run did not reach its end. Recovery
   is decided by the live tree's content against the sets that journal already
   recorded, never by a phase flag, with exactly one recorded fact the tree
   cannot supply, and is stated in full below.
5. **Generate into scratch.** Emit the broker splice workspace into a scratch
   directory under the repository's ignored `.scratch/` root, through the
   generator entry point of section 4, from
   `cargo metadata --manifest-path packages/d2b-priv-broker/Cargo.toml
   --locked` plus the authoritative lock's bytes. The tracked subtree is not an
   input to this step; it is only a destination.
6. **Validate the scratch tree before its bytes may become the tracked tree.**
   Section 4's three named refusals run on it. Its `Cargo.lock` must be
   byte-identical to `packages/d2b-priv-broker/Cargo.lock`. And
   `cargo metadata --manifest-path <scratch>/Cargo.toml --locked --offline`
   must exit zero, which is section 5's second gate run pre-write: the
   generated tree's resolve is proved equal to the authoritative lock before
   anything is written and without a network or a Bazel process. Any failure
   names the reason and leaves the worktree byte-identical to how the command
   found it.
7. **Refuse ambient work rather than overwrite it.** The command may write only
   `bazel/cargo/broker-workspace/**` and `bazel/cargo/broker.lock`, so it
   inspects the subtree before writing anything. That inspection is one pass
   through the anchored descriptor chain: every directory and every file under
   the subtree is opened with `openat2` under the four resolve flags of step 2
   and hashed from the descriptor just opened, and that single pass produces
   both the admission decision and the old-tree inventory step 11's removal is
   bounded by, so the content the check approved is exactly the content the
   removal is authorized against. Every path under the generated subtree,
   tracked or untracked, must be a directory or a regular file whose bytes are
   byte-identical to its committed content at `HEAD`, or to what step 5
   produced, or to the digest the transaction directory's `published` receipt
   records for that path, and the subtree must hold no path outside the union
   of those three file sets. A symlink, a device node, a socket or a fifo is
   refused outright:
   the generator emits neither, so the command has no basis for deciding what
   one means. Anything else is a local modification this command did not make
   and would destroy: a hand edit of a generated file, a half-applied patch, a
   conflicted merge, a stray scratch file someone dropped in the subtree. The
   command exits nonzero, lists every offending path repository-relative,
   prescribes the reversible remedy that covers tracked and untracked entries
   alike, and spawns no Bazel. The three permitted subtree states are exactly
   the three in which no contributor work exists to lose: bytes that are
   already in git, bytes this run can reproduce, and bytes an earlier run of
   this command recorded writing. The third exists because without it a second
   dependency edit before the first is committed would be refused on a subtree
   the command itself published, which is the same defect, in the same command,
   that withdrawing the clean-at-`HEAD` rule on the hub lock removes. A missing
   or unreadable receipt is not fatal and is not repaired silently; it collapses
   the permitted set to the first two and the ordinary refusal names the paths.

   The authoritative broker manifest and lock are deliberately outside this
   check. They are the inputs the contributor is editing, and requiring them
   clean would refuse the case the command exists to serve.
   `bazel/cargo/broker.lock` is deliberately outside it too, for a different
   reason given below: it is the generated artifact this command exists to
   replace.
8. **Refuse on the one input it may not write.** If the fresh generation
   implies a `manifests` list for the `broker` hub other than the one committed
   in `MODULE.bazel`, the command exits nonzero naming `MODULE.bazel` and
   `cargo xtask gen-bazel` as the remedy, still having written nothing. That
   list changes only when the broker's path-dependency closure gains or loses a
   package, which a version change and a feature change never do, so this
   refusal is off the ordinary path. It exists because `MODULE.bazel` is
   outside the permitted final path set of step 14 and because rendering a hub
   from a declaration the repository has since replaced is the failure this
   record spent round 1 refusing.
9. **Stage the validated bytes beside the target.** `mkdirat` `staged` inside
   the transaction directory and reproduce the validated scratch tree into it
   descriptor-relatively: `mkdirat` per directory, `openat2` with
   `O_RDWR | O_CREAT | O_EXCL | O_CLOEXEC` under the same four resolve flags
   per file, write, `fsync` each file. The access mode is `O_RDWR` and not
   `O_WRONLY` for a reason this step depends on: the verification below reads
   each staged file back through the descriptor that wrote it, and a
   write-only description cannot be read from, so `O_WRONLY` would leave the
   readback with two options, both wrong. It could reopen the file by name,
   which is a second resolution of a name this command already resolved and
   exactly the defect the anchored descriptor chain of step 2 exists to
   remove; or it could be dropped, which would reduce the check to a
   restatement of the bytes the command already held. Read every staged file
   back through the descriptors just written, from offset zero, and require
   its digest to equal the validated
   scratch digest, so "the validated bytes" names the bytes that reached the
   device and not the bytes that were offered to it. `fsync` the staged
   directories bottom-up, then the transaction directory. Then write the
   journal once, before anything tracked moves: a format version, the
   repository-relative subtree path, the old-tree inventory step 7 measured,
   the staged tree's path and digest set, and no child record, because no
   child has run; materialized as `journal.tmp`,
   fsynced, `renameat`d over `journal`, with the transaction directory fsynced
   after.
10. **Exchange, once.** Re-read the live subtree through the same anchored
    descriptors and require it to still equal the inventory step 7 recorded.
    Then `renameat2(txn, "staged", cargo, "broker-workspace", RENAME_EXCHANGE)`
    when the subtree exists, or the same call with `RENAME_NOREPLACE` when it
    does not, and `fsync` the `bazel/cargo` descriptor. Constraint 16 makes the
    two arms exclusive: whichever is wrong for the state on disk returns
    `ENOENT` or `EEXIST` rather than removing anything. `EBUSY` means the
    subtree is a mount point and is refused by name. `EXDEV` cannot arise,
    because `staged` was created by `mkdirat` under a directory created by
    `mkdirat` under the anchored parent. `bazel/cargo/broker-workspace`
    resolves to a complete tree at every instant of this step: the old one
    before the call, the new one after it, never a partial one and never
    nothing. This is the first step that mutates a tracked path, and the only
    one that touches the generated subtree; the sole other tracked path this
    command can change is `bazel/cargo/broker.lock`, written by the child in
    step 12 and possibly restored in step 13.
11. **Snapshot the hub lock, then retire the old tree.** Read
    `bazel/cargo/broker.lock` through the anchor, write its exact bytes to
    `hub-lock.pre`, fsync it, and only then rewrite the journal to name it, so
    a journal that names a snapshot is a journal whose snapshot exists; a lock
    that is absent is recorded as absent and no snapshot file is written. Then
    remove the old tree, which the exchange left at `staged`: walk it
    descriptor-relatively and `unlinkat` bottom-up, requiring every entry to
    appear in the journal's old-tree inventory with the same type and, for a
    regular file, the same size and the same digest read back from the
    descriptor. One entry that does not match aborts the removal, leaves the
    tree in place, and reports that path: the only thing it can be is work that
    arrived after step 7 measured the tree, and this command does not delete
    what it cannot account for. `unlinkat(AT_REMOVEDIR)` refuses a non-empty
    directory (constraint 14), so the removal cannot outrun its own bound even
    if the bound is wrong.
12. **Spawn exactly one Bazel child.** ADR 0052 section 3's controls are
    unchanged: one child process, `CARGO_BAZEL_REPIN` and
    `CARGO_BAZEL_REPIN_ONLY=broker` set only in that child's environment and
    never process-globally, and the same absolute output user root, output base
    and symlink prefix the Make wrapper derives, so no second server starts.
    The child inherits no descriptor this command holds under `bazel/cargo/`,
    the transaction lock included: they are all `O_CLOEXEC` and none is
    duplicated, so the Bazel client, and the server it leaves running after
    this command exits, cannot hold the transaction lock open.
13. **Hold the child to one file, and settle the hub lock.** After the child
    exits, the only tracked file it changed, measured against the state step 11
    left behind, must be `bazel/cargo/broker.lock`. This is ADR 0052's rule
    unweakened, and it keeps its own job: it is what catches a
    `skip_cargo_lockfile_overwrite` regression rewriting an authoritative
    `Cargo.lock` (constraint 5) or a second hub's lock moving. On success the
    generated lock is accepted, both digests are reported, and the journal is
    rewritten once more, through the same `journal.tmp`, fsync, `renameat`,
    fsync-the-directory sequence step 9 uses, to record that the child
    completed successfully and to carry the accepted lock's digest. That
    record is written strictly after the
    child exited zero and after this step accepted what it wrote, and it is
    the one fact a recovering run cannot derive from the tree. `hub-lock.pre`
    itself is discarded by step 14, after the journal that names it has been
    retired, which is what keeps a journal and its snapshot from ever
    disagreeing. On failure the
    snapshot is restored exactly: materialize
    `hub-lock.restore` from `hub-lock.pre`, fsync it, `renameat` it over
    `bazel/cargo/broker.lock` through the anchor, fsync the anchor; or
    `unlinkat` the lock when the record says it was absent. The restore is
    reported with the digest the child left, so a child that half-wrote the
    lock is visible rather than merely undone. No child record is written on
    this path, and none is needed: a child that failed is settled here, and
    step 14 still runs.
14. **Hold the command to its permitted set, and tear the transaction down.**
    The command's own change set, step 1's snapshot subtracted from the same
    snapshot retaken, must be a subset of `bazel/cargo/broker-workspace/**`
    plus `bazel/cargo/broker.lock`. Any other changed path fails the command
    and is listed repository-relative. Steps 13 and 14 are different checks:
    13 bounds what Bazel wrote, 14 bounds what the whole command wrote, and
    only 14 sees a generator that scribbled outside its own subtree. Then tear
    down, in this order: `renameat` the
    journal over `published`, one atomic operation that both clears the
    recovery trigger and installs the receipt of what was published; and only
    after that unlink `hub-lock.pre` and any `.tmp` residue. That order is the
    opposite of the obvious one and it is the order that keeps recovery
    total. Unlinking first would leave a window in which a journal names a
    hub-lock snapshot that is already gone, which is the one hub-lock state a
    recovering run cannot interpret, since it cannot tell a finished teardown
    from a snapshot that was never taken. Renaming first means that for as
    long as a journal exists its snapshot exists, and that whatever survives
    the rename is residue no journal accounts for, which is precisely what
    step 2 sweeps. A crash
    before the rename leaves `journal` in place, so the next run re-enters
    recovery, which reads that journal's child record and finds either nothing
    left to do or a child still to spawn. `lock`, `published` and the
    transaction directory stay; they are the ownership token and the receipt,
    not transaction state.

    This step runs whether the child succeeded or failed, and the transaction
    is torn down either way. A failed child has already been settled by step
    13, the subtree it was handed is published and correct, and the receipt
    should record that; leaving the journal in place instead would make the
    contributor's next ordinary run a recovery of a transaction whose recorded
    sets could by then be older than the manifest they edited. So a failed
    child ends in an ordinary worktree with an exit status, and only a killed
    command leaves a journal.

**Recovery is decided by content, not by a marker.** There is no way to write
a marker atomically with `renameat2`, so a phase flag recovery could trust does
not exist and one it cannot trust would only mislead a reader into thinking it
does. Step 4 therefore reads the live subtree and `staged` through the anchor
and decides against the two sets the journal already records, not against a
fresh generation, so a manifest edited between the interrupted run and the
recovering one changes nothing about the decision:

- live subtree equal to the journal's old-tree inventory: the exchange had not
  happened. Any `staged` is an unpublished staging area that no tracked work
  has ever occupied, so it is removed bounded by the journal's staged path set,
  and the run continues at step 5 with a fresh generation of its own, which
  reaches step 12 and spawns the child like any other run.
- live subtree equal to the journal's staged digest set: the exchange had
  happened, and the exchange was never the end of the transaction. Any
  `staged` must equal the journal's old-tree inventory and is
  removed bounded by it; one that does not is left in place, named, and the
  command exits nonzero without deleting it. With the retired tree gone, the
  journal's child record and not any further reading of the tree decides what
  remains:

  - the journal records a successful child: the only thing the crash
    interrupted is the teardown. Apply step 14's command-scoped changed-path
    check against the snapshot this run took at step 1, tear the transaction
    down exactly as step 14 does, and exit zero. No child is
    spawned. A recorded success is never repinned a second time.
  - the journal records no successful child: the transaction is unfinished and
    the run rejoins the closed sequence at the step the record leaves safe,
    rather than tearing down a transaction whose whole point has not happened
    yet. The journal's hub-lock record decides where that step is. Carrying no
    hub-lock record at all, no snapshot was ever taken; step 11's snapshot
    precedes step 12's child, so no child can have run, so
    `bazel/cargo/broker.lock` still carries its pre-child bytes and step 11 is
    redone from the top, over any stale `hub-lock.pre` a crash left there.
    Recording the lock as absent, that record is itself the snapshot and
    nothing is reread. Recording a snapshot file that is present, that file is
    kept, because it is the pre-child bytes and this run cannot recover them
    from anywhere else. Recording a snapshot file that is not there, refuse
    and name both: the
    only writer that unlinks it is step 14's teardown, which retires the
    journal before it unlinks the snapshot precisely so this pair cannot
    arise, so meeting it means the transaction directory
    was changed by something other than this command, and spawning a child
    whose failure could then not be undone is not an acceptable way to find
    out. The remedy it prints is the transaction directory's remedy and only
    that,
    `cargo xtask bazel-repin --hub broker --quarantine-transaction-state`
    followed by a re-run; it names neither `bazel/cargo/broker-workspace/`
    nor `bazel/cargo/broker.lock`, for the reasons the paragraphs below
    record. Otherwise the run continues at step 12, spawns the child, settles
    the lock under step 13, records the child there, and tears down under
    step 14.
    Its exit status is that whole sequence's, not the recovery's alone.
- live subtree equal to neither: refuse. List every path that matched neither
  side, prescribe the reversible remedy, delete nothing, and exchange nothing.

The two branches can both hold only when the old tree and the staged tree are
byte-equal. A recorded successful child settles that case before the content
is consulted: the record exists only if the exchange preceded it, so the run
takes the settled branch and spawns nothing. Absent such a record either
continuation reaches the same end state, because both end in a child, a
settlement and a teardown. The decision is therefore total and unambiguous
rather than merely usually right.

**The one thing content cannot decide, and the only fact the journal records.**
Every branch above reads the tree, because for the tree the content is the
evidence. The hub lock is the exception and has to be: its bytes come out of a
network-touching resolve, so no offline predicate over them exists, which is
the same reason it sits outside step 7's ambient check. A recovering run
therefore cannot look at `bazel/cargo/broker.lock` and learn whether the child
that should have rendered it ever ran. So the journal records that one fact,
and it is not the phase flag this section rejects. A phase flag for the
exchange is untrustworthy because no write can be made atomic with
`renameat2`, so the flag and the fact disagree inside the crash window. The
child record has the opposite shape: it is installed by an atomic rename
strictly after the child exited zero and step 13 accepted its output, so it
can only ever understate what happened, never overstate it. Those two errors
are not symmetric. An understated record costs one further deterministic repin
of a hub whose inputs have not moved. An overstated one leaves
`bazel/cargo/broker.lock` rendered from a tree the repository has since
replaced, which is the artifact this record's own failure list names and which
three of the four gates are structurally blind to. The design takes the
direction whose error is redundant work.

Without that record the second branch would establish the published subtree,
retire the old tree, settle a lock no child had written, tear the transaction
down and exit zero. The contributor would then hold a green command, a current
generated tree, and a hub lock describing the previous one, with nothing in
the run's output saying so. A command that produces that state on its own
recovery path is not a command that fails closed, whatever the rest of the
section does; so the recovery path ends where an uninterrupted run ends, at a
settled child, and not at the first point where the tree looks finished.

**The window that cannot be closed, and what happens in it.** Step 10 re-reads
the live subtree immediately before the exchange, which bounds the interval
between measurement and publication to the microseconds between the last read
and the `renameat2`. It does not eliminate it: no filesystem offers a
compare-and-exchange over a directory tree, and claiming otherwise would be the
kind of plausible assertion this record exists to replace with measurements. An
edit that lands inside that window is not destroyed. The exchange still
publishes atomically, the edited tree survives under `staged` in the
transaction directory, step 11's inventory bound refuses to remove it, and the
command reports that path and exits nonzero. That is a relocation, which this
record otherwise argues against, and it is accepted here for one reason: the
alternative in that window is deletion, and a named path a contributor can look
at beats bytes that are gone.

**Why `bazel/cargo/broker.lock` is outside the ambient check.** Round 2 of this
record required it byte-identical to `HEAD` before the command would run, and
that rule is withdrawn. It is wrong on its own terms: a successful run leaves
that file modified against `HEAD`, so the very next invocation would refuse,
and the same section claims re-running is always safe and never the wrong thing
to do. Both cannot be true. It also refuses the two states a contributor most
needs the command in: a second dependency edit before the first is committed,
and a merge whose conflict landed in the generated hub lock, which is precisely
the artifact an explicit repin exists to regenerate.

The asymmetry with the subtree is principled rather than convenient. The
subtree's content is a pure function of inputs this command has in hand, so
"byte-equal to what I would produce" is a predicate the command can evaluate,
and a path failing it is work the command cannot reproduce. The hub lock's
content is produced by Bazel from a network-touching resolve; no offline
predicate over it exists, so there is no state of that file the command could
certify as safe to replace and no state it could certify as work to preserve.
What replaces the refusal is an exact record: the prior bytes are snapshotted
before the only writer that can touch them runs, restored exactly if that
writer fails, and reported as a digest transition when it succeeds. The
contributor who wants the prior bytes kept rather than reported takes the same
targeted stash the ambient refusal prescribes, before running the command, on
that one path. When the hub lock is the file a merge left conflicted, that
stash is unavailable, because `git stash push` refuses a pathspec naming an
unmerged path in every form, as the refusal paragraphs below record; and
nothing is lost, because the run replaces the file wholesale. Its index entry
stays unmerged until the
contributor stages it, measured at git 2.54.0, where overwriting an unmerged
path's worktree bytes left `git status --porcelain` reporting `UU`, so the
command says so and names `git add -- bazel/cargo/broker.lock` rather than
letting a green run leave a merge half finished.

**Nothing here generates on a gate or a build path.** `make`, every workflow,
and every Bazel invocation on the gate path remain incapable of generating a
tracked artifact, and `cargo xtask gen-bazel --check` in `test-drift` is still
the fail-closed gate for a stale tracked tree, still naming
`cargo xtask gen-bazel` as its remedy. Self-synchronization is a property of
one explicit contributor mutation command that is not a Make target, that no
workflow may invoke, and that is the only place the three repin environment
names may appear as a process-environment assignment.

**When the Bazel child fails after the subtree is published.** Steps 1 through
9 leave the worktree exactly as they found it, so every refusal before the
exchange is total. After step 10 there is one state this command can leave
behind that it did not find: `bazel/cargo/broker-workspace/**` current, and
`bazel/cargo/broker.lock` byte-identical to the snapshot step 11 took before
the child ran. A Bazel child that fails for its own reasons, a full disk, an
interrupt, produces it. The command exits nonzero and says exactly that, naming
both paths, their states, and the digest the child left on the lock before the
snapshot was restored over it.

That lock state is guaranteed rather than hoped for, and that is what step 13's
restore buys. Round 2 could only report that the hub lock was "still describing
the previous inputs", which is true of a child that wrote nothing and false of
a child that wrote a partial or wrong lock and then failed. A single regular
file whose exact prior bytes are on the device, whose only writer did not
complete, and whose interval is covered by the transaction lock, is restorable
exactly; so it is restored, and the partial state is one state rather than a
family of them.

The subtree is a different matter and is not rolled back. Rewriting the tree it
just proved correct would make a run that did real work indistinguishable from
one that never happened, would discard the validated generation for no gain,
and would put a second tracked mutation after the failure the report is about.
The command does not silently retry either. The recovery is one line in the
failure message: fix what the Bazel child reported and re-run
`cargo xtask bazel-repin --hub broker`. The second run regenerates the same
bytes, because step 5 is deterministic; finds the subtree already equal to
them, which is step 7's second permitted state, so step 7 passes and step 10
exchanges a tree for its own twin; and spawns the child again. Re-running is
therefore always safe and is never the wrong thing to do, which is the property
a partial-failure recovery has to have, and which round 2's clean-at-`HEAD`
rule on the hub lock silently contradicted.

A failed child and a killed command are not the same state, and it matters
which one the contributor is in. A failed child is settled here: step 13
restores the snapshot, step 14's check and teardown still run, the receipt is
installed, and no journal survives, so the next invocation is an ordinary run
that regenerates, exchanges a tree for its twin, and spawns the child. A
killed command leaves the journal, and then the child record inside it is what
decides whether the recovering run spawns a child at all. The one case where
the two look alike from outside, a run killed after the child succeeded but
before the record reached the device, resolves toward doing the work again: the
recovering run finds no record, spawns the child a second time, and renders
the same lock from the same inputs.

If instead the contributor wants the whole change gone, the recovery is one
reversible command that covers tracked, untracked and ignored entries alike,
`git stash push --all -- bazel/cargo/broker-workspace/
bazel/cargo/broker.lock`, and the command is what told them those are the only
two paths it touched. Measured at git 2.54.0: with a modified stub, a deleted
stub source file, an untracked extra file and an ignored
`bazel/cargo/broker-workspace/target/debug/out.bin` under the subtree, that
invocation stashed all four, left the subtree byte-identical to `HEAD`, left an
unrelated edit elsewhere in the worktree untouched, and `git stash pop`
restored all four including the ignored one. The flag is `--all` rather than
`--include-untracked` for one measured reason: on the same tree
`--include-untracked` took the first three and left the ignored file exactly
where it was. `git restore` on the subtree is not the remedy and is not offered
as one: measured on the same tree, it restored the modification and the
deletion and left the untracked and the ignored file exactly where they were.

That residue is not invisible, which is the property round 1 bought with a
refusal and this section has to buy some other way. Section 5's fourth gate
buys it: with the stub manifests named in the hub's `manifests` attribute, a
`bazel/cargo/broker.lock` rendered from a stub tree the repository has since
replaced fails `determine_repin` at the next analysis of the hub, on the pull
request, with the substrate's own repin-required message. Committing the
residue does not get it past review.

**What the command prescribes when it refuses, and what it never prescribes.**
Step 7's refusal lists every offending path repository-relative and prescribes
`git stash push --all -- bazel/cargo/broker-workspace/` followed
by re-running the command, because that one command is reversible, is bounded
to the pathspec, and covers the untracked and ignored cases that are the common
shape of this refusal: a stray file under a generated subtree is untracked far
more often than it is a tracked hand edit, and the most likely entry of all is
a `target/` directory left by the `cargo build` this section's discoverability
consequence predicts, which the repository's committed `.gitignore` rule
`target` makes ignored rather than untracked. `--include-untracked` is measured
to leave exactly that entry behind, so it is not what the command prints: a
remedy that leaves the refusal's most likely cause in place is a remedy that
refuses the contributor a second time. A contributor who would rather keep the
entries in place moves the named paths somewhere they choose; the command lists
them precisely so that is possible. Step 11's inventory refusal and step 4's
recovery refusal prescribe the same remedy against the paths they name.

**A conflicted index is a different refusal with a different remedy, and stash
is not it.** `git stash push` in every form refuses a pathspec that names an
unmerged path: measured at git 2.54.0 against a content conflict under the
subtree, `--all`, `--include-untracked` and the bare form each exited 1 with
`<path>: needs merge`, stashed nothing, and left the index and worktree
unchanged. Step 7 can meet that state, because a conflicted merge under the
generated subtree is one of the ambient shapes it exists to refuse, so the
command classifies the paths it names rather than printing one remedy for all
of them. For a path git reports as unmerged it prescribes resolving that path's
index entry, bounded to the same pathspec:
`git checkout HEAD -- <the unmerged paths>`, measured to exit zero and resolve
both shapes a generated subtree produces, a `UU` content conflict and an `AA`
add/add, or `git rm -- <path>` for the `DU` shape, where `HEAD` carries nothing
at that path and `git checkout HEAD --` exits 1 with `pathspec ... did not
match any file(s) known to git`. Both leave `MERGE_HEAD` in place, so the
incoming side stays addressable as `git checkout MERGE_HEAD -- <pathspec>` and
nothing is destroyed, and both put the subtree in step 7's first permitted
state, from which the re-run regenerates it anyway. Only after the index has no
unmerged entry under the pathspec does the stash remedy apply, and it then
succeeds on the same pathspec; measured in that order on the same fixture. The
command never claims otherwise, because a remedy that the tool refuses to
execute is worse than no remedy.

Four remedies are never prescribed. `rm -rf` is not, because an ADR that tells
a contributor to recursively delete a path under their worktree has externalized
exactly the risk the rest of this section is spent removing. A broad
`git clean` is not, because its blast radius is the worktree and the problem is
one subtree. `git restore` alone is not, because it does not remove
untracked or ignored files, which is measured above and is the specific way
round 2's wording was wrong: it named a remedy that leaves the refusal's most
likely cause in place, so the contributor re-runs, is refused identically, and
concludes the command is broken. And a list of per-entry removals over names
the command read off the disk is not, because its length, its depth and its
quoting are all decided by bytes nobody chose, and because a removal is the
wrong instrument for state a refusal exists to surface; the transaction-state
refusals below prescribe a rename this command performs itself instead.

**The three transaction-directory refusals prescribe one built-in recovery.**
The unparseable `journal`, the unrecognized entry and the recorded-but-missing
hub-lock snapshot are the three refusals whose offending paths lie inside
`bazel/cargo/.broker-workspace.txn/` rather than under the generated subtree.
Step 7's remedy names the wrong pathspec for all three: it would clear nothing
they name, so the contributor re-runs and is refused identically, which is the
same defect this section already corrected once for `git restore`. Their
remedy is one command this repository owns,
`cargo xtask bazel-repin --hub broker --quarantine-transaction-state`. It
renames the whole transaction directory to the first free one of four fixed
sibling names, `bazel/cargo/.broker-workspace.txn.quarantine.0` through
`bazel/cargo/.broker-workspace.txn.quarantine.3`, prints that path
repository-relative, tells the contributor to re-run
`cargo xtask bazel-repin --hub broker`, and does nothing else whatever.

**The hub guard is argument validation and runs before anything is opened.**
`--quarantine-transaction-state` is accepted only with `--hub broker`. Given
any other member of ADR 0052's closed four-hub set, `main`, `guest` or
`walker`, the command exits nonzero with the stable code `D2B-BZLTXN-HUB`,
states that the flag is broker-only because no other hub has a transaction
directory, and names
`cargo xtask bazel-repin --hub broker --quarantine-transaction-state` as the
only accepted form. That refusal is decided from the parsed arguments alone,
strictly before the worktree root is opened, before `bazel/cargo` is resolved,
before `.broker-workspace.txn` is resolved and before `lock` is opened. The
ordering is the guard, not tidiness around it. There is exactly one
transaction directory in this repository and it is the broker's, so a hub
argument read after the sequence has begun does not select a different
directory to quarantine, it quarantines the broker's; the lock is opened
`O_RDWR | O_CREAT`, so a guard placed after that open creates
`bazel/cargo/.broker-workspace.txn/lock` on a run that was never authorized to
write anything; and a guard placed after the transaction directory is resolved
is worse in the ordinary case than in the pathological one, because `ENOENT`
on that name exits zero, so
`cargo xtask bazel-repin --hub main --quarantine-transaction-state` would
report success for a hub the flag is meaningless for. The guard is therefore
the first thing this mode does, and the check that says so asserts against a
fixture where a late guard would leave a trace.

**Stated as the closed sequence it is.** Open the worktree root by ordinary
`open`, because a contributor's worktree legitimately sits under symlinked
ancestors (constraint 17). Resolve `bazel/cargo` beneath it with `openat2`
under `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS |
RESOLVE_NO_XDEV`, opened `O_RDONLY | O_DIRECTORY | O_CLOEXEC` so that one
descriptor is the `fstatat` anchor, both rename anchors and the fsync target;
using the one descriptor for all four is what keeps the parent from being
swapped between the observation and the act. Then, at most three times:

1. Resolve `.broker-workspace.txn` beneath that descriptor under the same four
   flags, opened `O_RDONLY | O_DIRECTORY | O_CLOEXEC`. `ENOENT` is success and
   the command exits zero having written nothing, because it names a goal
   state and not an action. `ELOOP` or `ENOTDIR` means the name does not
   denote a directory this command can bind; it refuses with
   `D2B-BZLTXN-CONCURRENT` having moved and created nothing.
2. Resolve `lock` beneath *that directory descriptor*, not by a two-component
   path from the parent, opened `O_RDWR | O_CREAT | O_CLOEXEC` at mode 0600,
   and take `F_OFD_SETLK` with `F_WRLCK`. `EAGAIN` means a generator or a
   repin is running, and the command names that lock path repository-relative
   and the fact that another generator or repin holds it, and exits nonzero
   having touched nothing.
3. `fstat` the directory descriptor and keep its `st_dev` and `st_ino` as the
   witness. That is the identity of the directory whose lock is now held.
4. `fstatat` the single component `.broker-workspace.txn` relative to the
   anchored `bazel/cargo` descriptor with `AT_SYMLINK_NOFOLLOW`, and require
   the same `st_dev`, the same `st_ino` and `S_ISDIR`. `ENOENT`, a different
   identity, or a type that is not a directory is a mismatch.
5. On a mismatch, close the lock descriptor, which releases the lock because
   it is the only descriptor on that open file description, close the
   directory descriptor, and start again at 1. What changed is which inode the
   name denotes, so re-examining descriptors already held would re-derive the
   same wrong answer; only a re-resolution converges.
6. On a match, with no syscall between that `fstatat` and the rename other
   than the rename itself, `renameat2(RENAME_NOREPLACE)` from that anchor to
   that same anchor, over the four slots in ascending order and stopping at
   the first that returns 0. `EEXIST` advances to the next slot and re-takes
   the identity check of 4 first, so the observation is always the syscall
   immediately preceding the rename it authorizes rather than merely the
   first of the four. All four occupied is a terminal refusal and not a retry
   trigger: the command lists the four slots repository-relative, leaves the
   transaction directory where it is, and stops. `ENOENT` from the rename can
   only be a source that vanished inside that window, since both anchors are
   descriptors this command holds open; it consumes an attempt and returns
   to 1.

Then the postcondition below, then one `fsync` of the anchor, which is one
call rather than two because the source and the destination sit in the same
directory, then the lock descriptor closes with the process. Three attempts is
the exact bound. One would turn a single benign interleaving into a refusal;
no bound at all would turn a wedged script that keeps recreating the name into
a hang, and a hang is the one failure a contributor cannot diagnose from a
message. Three absorbs two interleavings and terminates. On exhaustion the
command exits nonzero with `D2B-BZLTXN-CONCURRENT`, names
`bazel/cargo/.broker-workspace.txn` repository-relative, states that the entry
at that name changed under it, and has moved nothing and created nothing. It
never renames on a mismatch, and no flag makes it. Otherwise the
mode is what it was: it reads no
manifest, no lock, no tracked path and no entry inside the directory it moves;
it spawns no process at all and therefore no Bazel child; it sets none of the
three repin environment names; it is not a Make target and no workflow, gate
or build path may invoke it. It deletes nothing, ever, and there is no flag
that makes it.

**Why the lock was never the binding.** An open file description lock names an
inode, not a name, and the same measurement that makes the recovery work makes
an unchecked rename unsafe. Measured 2026-08-04 on Linux 7.0.10 with the
worktree on ext4, unprivileged, by direct syscall from the same C probe shape
constraints 14 through 19 use: a directory freshly created at
`.broker-workspace.txn` after the previous one had been renamed onto a
quarantine slot carried a different inode, and its own `lock` was a different
file on which `F_OFD_SETLK` with `F_WRLCK` returned 0 while the lock on the
moved directory's `lock` was still held by the same process; and
`F_OFD_SETLK` on that moved directory's `lock`, reached through its new name,
returned `EAGAIN`, which is the same lock reached by a different path. So a
sequence that locks the transaction directory it resolved and then renames by
name would, in the interleaving where its directory was quarantined by someone
else and a repin then created a fresh one, rename the live transaction
directory of a running repin that legitimately holds its own lock. That repin's
`staged` tree, its journal and its hub-lock snapshot all leave the name its
descriptors were opened against, its teardown installs the receipt inside a
quarantine nothing reads, and the next ordinary run finds neither journal nor
receipt. That is the failure this check exists for, and it is why the lock is
necessary and not sufficient.

The identity check is cheap and exact. Measured on the same probe: `fstat` on
the directory descriptor and `fstatat` on the name with `AT_SYMLINK_NOFOLLOW`
reported the same `st_dev` and `st_ino` while the name still denoted the
locked directory; the descriptor's `fstat` reported that identity unchanged
after the directory had been renamed, so the witness survives the very
operation it is taken for; and `fstatat` on the old name afterwards returned
`ENOENT`. `AT_SYMLINK_NOFOLLOW` is load bearing rather than hygienic: with a
symlink to a directory planted at the name, `fstatat` with the flag reported a
symlink and without it reported a directory, so the type half of the check
means nothing without it. A regular file planted at the name reported a
regular file. And the check has to exist at all because `renameat2` refuses
none of those shapes: with that symlink planted,
`renameat2(RENAME_NOREPLACE)` on the name returned 0 and moved the symlink
itself rather than following it, so an unchecked rename files a symlink into a
quarantine slot and prints success while the transaction directory is still
wherever it was.

This is not the stat-then-open pattern step 2 forbids. That rule forbids
taking a decision from a name and then acting through a second resolution of
the same name. Here the decision is taken from the descriptor that holds the
lock, the `fstatat` is the last observation before the act rather than the
thing the act trusts, and its only use is to refuse. It is also the smallest
observation available: the name is a single component and the anchor is a
descriptor, so there is no path walk for a symlinked ancestor to subvert, and
opening the name a second time would buy nothing, because `renameat2` takes a
directory descriptor and a name and has no form that takes the source as a
descriptor.

**After the identity match a cooperating writer cannot move it, and a
non-cooperating one is bounded after the fact.** Every writer this decision
authorizes takes `F_OFD_SETLK` with `F_WRLCK` on `lock` inside the transaction
directory before touching it, and the only one of them that ever renames that
directory is this quarantine mode. The ordinary repin and
`cargo xtask gen-bazel` create the directory with `mkdirat` tolerating
`EEXIST` and write inside it; neither moves it. So a second quarantine that
resolved the inode this one witnessed must take the same lock on the same file
and is refused `EAGAIN`, measured above through a second name for that same
inode, and a second quarantine that resolved a different inode is not talking
about this directory at all. That is the whole cooperating set, and after the
match it is closed.

A non-cooperating rename is a different matter and this record does not
pretend to prevent it. A contributor's own `mv`, a backup tool, an editor
performing an atomic save, or any process that honours no advisory lock, and
git was already measured to be one, can replace the entry between the
`fstatat` and the `renameat2`. No syscall closes that window. Measured on the
same probe, `renameat2`'s flag set is exactly `RENAME_NOREPLACE`,
`RENAME_EXCHANGE` and `RENAME_WHITEOUT`; an undefined flag bit returned
`EINVAL`, and `RENAME_NOREPLACE | RENAME_EXCHANGE` returned `EINVAL`, so there
is no form of the call that binds the source to an inode. A second lock one
level up, on `bazel/cargo` itself, is rejected for the same reason rather than
adopted: it would move the identical window one level up without removing it,
because the writer this window is about is precisely the one that takes no
lock, and it would add a persistent path outside the set invariant 12 closes.
The window is therefore bounded after the fact rather than claimed away.

Immediately after a `renameat2` that returns 0, the command `fstatat`s the
slot it just wrote, relative to that same anchored `bazel/cargo` descriptor,
with `AT_SYMLINK_NOFOLLOW`, and requires `st_dev`, `st_ino` and `S_ISDIR` to
equal the witness. Measured, after a legitimate quarantine that check reported
the witness identity exactly. When it does not, something other than the
directory this command locked has been moved into the slot: the command exits
nonzero with the stable code `D2B-BZLTXN-DISPLACED`, names the slot and
`bazel/cargo/.broker-workspace.txn` repository-relative, reports that what it
moved is not what it locked, and stops. It does not move it back. A restore
would be a second unsynchronized rename with the identical defect, against a
name whose content the command has just been shown it cannot predict; it would
either succeed, putting back a directory the command has no evidence belongs
there, or return `EEXIST` and leave two names and no report. Naming both paths
and refusing is the fail-closed answer, and this postcondition is the
difference between a corruption that is silent and one that is on the
operator's screen with both paths in it.

**Three stable codes, each naming only its own steps.** ADR 0052's rule that
one generic remedy is wrong for at least one member of a refusal set applies
here, so this mode refuses under exactly three static codes.
`D2B-BZLTXN-HUB`: the flag was given with a hub other than `broker`; nothing
was opened; re-run with `--hub broker`. `D2B-BZLTXN-CONCURRENT`: the entry at
`bazel/cargo/.broker-workspace.txn` could not be bound to the directory this
command locked within its three attempts, or does not denote a directory at
all; nothing was moved and nothing was created; inspect that entry and re-run
once no generator or repin is running. `D2B-BZLTXN-DISPLACED`: the rename
returned 0 and the slot does not hold the witnessed directory; both paths are
named and the operator inspects them before running anything else. The
`EAGAIN` contention refusal is unchanged and keeps naming the lock path and
the writer that holds it. None of the four prints an absolute path, a user
identifier, a process identifier, or any byte read from inside the directory
it refused over.

**Why a rename rather than a stash or a classified removal.** The state these
three refusals dispute is arbitrary: an unrecognized name can be any entry a
filesystem can hold, at any depth. A rename is the only remedy whose bound
does not depend on that. It moves one directory entry in one parent, so it
preserves every nested byte, every special file and every name whose bytes are
not valid UTF-8, and it never parses, renders or interpolates a name a
contributor did not write. Measured 2026-08-04 on Linux 7.0.10 with the
worktree on ext4, unprivileged, by direct syscall from the same C probe shape
constraints 14 through 19 use: `renameat2` with `RENAME_NOREPLACE` moved a
transaction directory holding `lock`, `published` and an unrecognized
directory carrying a regular file, a fifo, a unix socket, a symlink, an empty
directory, a name whose bytes are not valid UTF-8, and a second level holding
another fifo, and returned 0. Afterwards `openat2` on the transaction name
returned `ENOENT`, every one of those entries was present under the quarantine
name with its type intact, the unrecognized directory's inode number was
unchanged, and the nested regular file's bytes read back identical. A
character device is the one shape the probe could not plant, since `mknodat`
for one returned `EPERM` unprivileged; this record does not claim to have
measured it, and it is the shape the argument least needs, because a rename
enumerates nothing and so cannot distinguish it from the fifo and the socket
it did move.

**The stash cannot clear these three, which is why it is not prescribed for
them.** An earlier round of this record prescribed
`git stash push --all -- bazel/cargo/.broker-workspace.txn/` here, with a
per-shape single-path removal for each listed entry git cannot represent, and
that is withdrawn because it is measured not to work. Git's object store has
no representation for an empty directory or for a fifo, a socket or a device
node, and the failing case is not the top-level shape classification covers:
it is an unrecognized directory git can represent, holding entries it cannot.
Measured at git 2.54.0 on a fixture carrying the committed `.broker-*/` rule,
with an unrecognized directory holding a regular file, a fifo and a
subdirectory holding another fifo, the stash exited 0, took the regular file
along with `journal`, `lock` and `published`, and left the unrecognized
directory, its subdirectory and both fifos exactly where they were. The
refusal therefore reproduces on the next run, and the classified arms cannot
finish the job: against that same surviving directory `rmdir --` exited 1 with
`Directory not empty` and `rm --` exited 1 with `Is a directory`, both of them
correctly, because both are built to refuse the moment the path holds
anything. So the printed remedy exits 0, the state it was supposed to clear is
still on disk, part of the evidence has been filed into a stash and the rest
has not, and the next run refuses identically. Making classification work
would mean walking to the leaves and printing a removal per arbitrary nested
name for the contributor to paste into a shell, a remedy whose depth, whose
bound and whose quoting all depend on bytes nobody chose. The rename has no
depth, no per-entry decision and no interpolated argument to get wrong.

**Taking the lock is the second reason the recovery belongs in the tool.**
Git honours no advisory lock: measured, with an `F_OFD_SETLK` write lock held
on `bazel/cargo/.broker-workspace.txn/lock`, `git stash push --all` on that
pathspec exited 0 and unlinked the lock file out from under its live holder,
which against a running repin takes the ownership token away from a writer
mid-transaction. The built-in takes that lock before it moves anything and is
refused with `EAGAIN` while another writer holds it, measured on the same
probe, so a contributor who runs the recovery at the wrong moment gets a
refusal rather than a corrupted transaction. That is also why this recovery
could not be a line of shell in the refusal text however carefully quoted: the
safety here is the lock, and no shell command can take it.

**Collision is bounded and exhaustion is a refusal.** The four slots are tried
in ascending order and the first `RENAME_NOREPLACE` that returns 0 wins;
`EEXIST` advances to the next. Measured, `RENAME_NOREPLACE` onto an occupied
slot returned `EEXIST` and left the transaction directory exactly where it
was, so an occupied slot costs nothing and clobbers nothing, which is
constraint 16's complement doing the work it was measured for. With all four
occupied the command exits nonzero, lists the four quarantine paths
repository-relative, moves nothing, and tells the contributor to read what
they have collected and clear what they no longer want. It prints no command
for that, because by then those directories are inert evidence that no command
reads and no gate sees. Four is chosen so that the second, third and fourth
occurrence of the same corruption still recover with no manual step, and so
that a fifth
is a signal that something is producing corrupt transaction state faster than
anyone is reading it. One fixed name would refuse the second occurrence with
`EEXIST` and leave a manual recursive delete as the only way forward, which is
where this started.

**Why the quarantine touches nothing else.** Neither
`bazel/cargo/broker-workspace/` nor `bazel/cargo/broker.lock` is read or
written by it, and naming either would be a defect rather than caution. The
missing-snapshot refusal is reached only on the branch where the live subtree
equals the journal's staged digest set, which is the tree step 5
deterministically reproduces, so the re-run meets it in step 7's second
permitted state and passes; setting that subtree aside would file a validated
tree away in order to regenerate it byte for byte. The hub lock sits outside
step 7's ambient check by the decision recorded above, so no state of that
file can refuse the re-run, and the re-run snapshots whatever bytes it finds
and lets the child replace them wholesale. The other two refusals fire before
any of that is read at all. No Cargo manifest and no Cargo lock in either
workspace is opened by this mode. And the move is invisible to git: measured
at git 2.54.0, `git check-ignore -v` matched the committed `.broker-*/` rule
against all four quarantine names, so they need no `.gitignore` entry of their
own, and `git status --porcelain` was byte-identical before and after a
quarantine on a fixture carrying a modified tracked file, an untracked file,
an ignored `target/` output and a staged modification, all four of which were
byte-unchanged afterwards. That invisibility is what keeps the recovery out of
both changed-path checks. What the generator does emit is one `.bazelignore`
entry per quarantine slot beside the transaction directory's own, because a
quarantined directory can carry a `staged/` tree with its own `BUILD.bazel`.
Fixed slot names are what make that possible: an ignore entry can be emitted
and drift-checked for a name known in advance and not for a timestamp, and
this record does not assume a pattern syntax in that file it has not measured.
`cargo xtask gen-bazel --check` refuses when any of those entries is missing.

One cost is real and is named rather than hidden: the quarantine takes
`published` with it, because the directory whose integrity these three
refusals dispute is not a directory whose receipt should still be trusted.
That collapses step 7's permitted set from three states to two on the next
run, exactly as a missing receipt does anywhere else, so a contributor whose
subtree carries bytes only the receipt would have admitted meets step 7's
ordinary refusal and its ordinary remedy. That is one further bounded refusal
with a working remedy, not a loop.

**The evidence stays readable, and it is not put back automatically.** A
quarantined directory is an ordinary directory in the worktree: the journal
this build could not parse, the unrecognized entry and the recorded snapshot
are all still there under their own names and their own bytes, and reading
them needs no git command, no stash listing and no restore step. Nothing
offers to move a quarantine back into place. The state was refused because
this build could not account for it, and a mode that reinstated it would only
reproduce the refusal it had just cleared. Re-running
`cargo xtask bazel-repin --hub broker` after a quarantine meets an absent
transaction directory, creates a fresh one and proceeds as an ordinary run:
measured on the probe, a `lock` created under a freshly created transaction
directory took `F_OFD_SETLK` and returned 0 while a descriptor on the
quarantined one was still held open, so even a stale holder of the moved lock
cannot block the next run.

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

**Census before predicate.** Both sets are asserted against an exact expected
census before either condition below is evaluated, for the reason ADR 0052
section 6 already states about digests: a subset predicate over an empty set is
vacuously true, so a generator that emitted nothing, a query whose pattern
stopped matching, or a rename that moved the whole package would pass both
conditions while proving nothing. The census is mechanically derived from the
same two `cargo metadata` facts the rest of this decision rests on, not written
down by hand:

- The closure is the set of `source`-less entries in
  `packages/d2b-priv-broker/Cargo.lock`, six today:
  `d2b-contracts`, `d2b-core`, `d2b-host`, `d2b-priv-broker`, `d2b-realm-core`
  and `d2b-realm-provider`.
- The members are `workspace_members` for that manifest, one today,
  `d2b-priv-broker`.
- The `-broker` variants are therefore exactly closure minus members, five
  today, one library target each: `d2b-contracts-broker`, `d2b-core-broker`,
  `d2b-host-broker`, `d2b-realm-core-broker` and `d2b-realm-provider-broker`.
- B is exactly those five plus the targets the generator emitted for
  `//packages/d2b-priv-broker`, which must cover that package's Cargo targets,
  today one `lib`, one `bin` and thirteen `test`, under whatever mapping ADR
  0052 section 4's generator applies.
- M must be nonempty, must contain the five unsuffixed first-party variants of
  the closure crates, and must contain no `-broker` variant and no
  `//packages/d2b-priv-broker` target.

An empty set, a missing member, or an extra member fails the check before any
predicate runs, and is reported as a set difference rather than as an edge
violation, because the two failures have different remedies: a census
difference means the generator or the query moved, an edge violation means the
build graph is wrong. The counts are consequences of the derivation and are
recorded here so a reader can see what the check should be looking at; the
check computes them rather than asserting them, so a legitimate sixth closure
package changes the census in one place.

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
drift-checked, none authored. Six tracked one-line additions carry the
transaction directory and its four quarantine slots: a `.gitignore` entry for
the transaction directory, which is committed and hand maintained, and five
`.bazelignore` entries, one for the transaction directory and one for each
quarantine slot, all of which the generator emits.
`cargo xtask gen-bazel --check` refuses when any is missing, because the
first is what keeps `bazel/cargo/.broker-workspace.txn/` out of
`git status --porcelain` and therefore out of both changed-path checks, and the
rest are what keep Bazel from defining a package for a staged tree carrying
its own `BUILD.bazel`, whether that tree is live or quarantined. The
quarantine slots need no `.gitignore` entry of their own: the committed
`.broker-*/` rule is measured to match all four.

**Untracked surface, and it is one directory, plus up to four a refusal
creates.**
`bazel/cargo/.broker-workspace.txn/` exists in a contributor's worktree after
the first generator or repin run and persists, holding one zero-byte `lock`
file between transactions. It is authorized transaction state and nothing else:
no gate reads it, no build input names it, it is owned by the invoking user,
and this decision creates no root-owned path, no unit, and no state outside the
worktree. Removing it by hand between runs is harmless; removing it during a
run breaks that run's ownership lock, which is why it is named for what it is
rather than looking like a cache. The lock it holds is never held by anything
but a running writer: the descriptor carries `O_CLOEXEC`, so no Bazel client,
no Bazel server and no `cargo metadata` child inherits the open file
description, and there is therefore no state in which a contributor has to
break a lock, no stale-lock timeout to tune, and no `--force` to add later.
`bazel/cargo/.broker-workspace.txn.quarantine.0` through
`bazel/cargo/.broker-workspace.txn.quarantine.3` are the only other paths
either writer creates, they are created only by the quarantine mode and only
by renaming the transaction directory onto one of them, they are never read by
anything afterwards, and a worktree that has never met a transaction-state
refusal never holds one.

**Repin surface.** `cargo xtask bazel-repin --hub broker` keeps every property
ADR 0052 section 3 gave it: an explicit hub from the closed four-hub set,
exactly one Bazel child, `CARGO_BAZEL_REPIN` and `CARGO_BAZEL_REPIN_ONLY`
scoped to that child's environment and forbidden everywhere else, the wrapper's
derived output user root and output base so no second server starts, no Make
target and no workflow reachability, and exit zero with nothing changed when
the lock is already current. Four things change, all narrow.

It generates and validates the broker splice inputs itself instead of reading
them, so the mutation is one command rather than an ordered pair, and the
generated tree is not an input the contributor has to have gotten right first.

It publishes the subtree by exchanging a staged sibling with the live name
under `renameat2(RENAME_EXCHANGE)`, holding an open file description lock that
excludes the other writer on a descriptor opened `O_CLOEXEC` and never
duplicated, so the lock dies with the command rather than with the last process
that happened to inherit it, and it takes an exact snapshot of the hub lock
before the Bazel child runs so that a failing child leaves that file
byte-identical to what the child was handed. The whole transaction lives in one
ignored sibling directory under `bazel/cargo/`.

And the post-run rule becomes two rules. The Bazel child is still held to
exactly one changed tracked file, the hub's Bazel-side lock; that rule is not
weakened, and it is the one that catches an authoritative `Cargo.lock` being
rewritten. The command as a whole is held to that file plus the one subtree it
synchronized, `bazel/cargo/broker-workspace/**`, and it fails listing any other
changed path repository-relative.

And it gains one flag that performs no repin,
`--quarantine-transaction-state`, which is the printed remedy for the three
refusals whose offending paths lie inside the transaction directory. It takes
the same lock, renames that directory onto the first free one of four fixed
ignored sibling names, fsyncs the shared parent and stops: no generation, no
validation, no Bazel child, no deletion, and nothing read out of the directory
it moves. It is a contributor command like the repin itself, unreachable from
Make, from any workflow and from any gate. Nothing else in ADR 0052 section 3
moves.

**A partial state this command can leave behind.** If the Bazel child fails
after step 10, the generated subtree is current and `bazel/cargo/broker.lock`
is byte-identical to the snapshot taken before the child ran. The command
reports both, restores that snapshot rather than leaving whatever the child
left, does not roll the subtree back, and does not overwrite work it did not
make; recovery is to fix the reported Bazel failure and re-run the same
command, which is idempotent, or
`git stash push --all -- bazel/cargo/broker-workspace/
bazel/cargo/broker.lock` to drop the change entirely. The residue cannot merge
quietly, because section 5's fourth gate fails the hub's next analysis with the
substrate's repin-required message. This is a deliberate trade: round 1 had no
such residue and paid for it with a refusal that made the ordinary mutation a
two-command ritual.

A killed command is a different state from a failed child and is handled
differently. A failed child is settled and torn down, so the next invocation is
an ordinary run. A killed one leaves its journal, and the journal records
whether the Bazel child ever completed successfully, because that is the one
fact the published tree cannot tell a recovering run. Without a recorded
success the recovery continues to the hub-lock snapshot, the child and the
settlement rather than tearing down a transaction that had only published; with
one it tears down and spawns nothing, so a recorded success is never repinned
twice. The transaction directory is also swept of the residue a run killed
before its journal existed left behind, under a closed set of names, so the
next run is not refused by its predecessor's half-written staging area.

**Idempotence is now unconditional, and it was not.** Round 2 required
`bazel/cargo/broker.lock` clean at `HEAD` before the command would run, which
made a successful run poison the next one: the file it had just legitimately
written was the state its own precondition refused. That rule is withdrawn, and
the same defect in the subtree check is closed the other way, by permitting the
bytes an earlier run recorded publishing alongside the committed bytes and the
reproducible ones. Re-running after a success, after a partial failure, after a
second dependency edit with the first still uncommitted, or on a merge whose
conflict landed in the generated hub lock all reach the same end state, and the
only file whose prior bytes matter is snapshotted and reported rather than
defended by a refusal.

The cost is one small receipt file in the transaction directory, and the
failure it makes possible is worth naming: a receipt that disagrees with what
is actually in the subtree would widen what the command may overwrite. It
cannot, because the receipt is a digest set rather than a permission and every
permitted path is still compared byte for byte; a stale receipt matches nothing
and simply drops back to the two-state rule.

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
broker workspace, and beside it a dot-directory that is neither. Someone will
run `cargo build` in the first and it will succeed, because empty stubs
compile. It also leaves `bazel/cargo/broker-workspace/target/`, which the
repository's committed `.gitignore` rule `target` makes an ignored path rather
than an untracked one, and which step 7 then refuses because it is under the
subtree and in none of the three permitted file sets. That is the concrete
reason the prescribed remedy is `git stash push --all` on the pathspec:
`--include-untracked` is measured to leave that exact directory in place.
Mitigations: the generated marker on line one of the root manifest, a
generated `.bazelignore` entry for `bazel/cargo/broker-workspace/target/`, for
`bazel/cargo/.broker-workspace.txn/` and for each of the four quarantine
slots, a committed `.gitignore` entry for
the transaction directory, and `gen-bazel --check` refusing when the marker or
any ignore entry is absent.

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

The section 3 repin command is refined in exactly four places, all narrow and
all stated in full in the repin-surface consequence above: it synchronizes the
broker hub's generated splice inputs before it spawns Bazel; it publishes that
subtree as a transaction, exchanging a validated staged sibling with the live
name under an ownership lock and snapshotting the hub lock before the child
runs so a failing child leaves that file exactly as the child found it; its
changed-path rule splits into a child-scoped rule, unchanged from ADR 0052, and
a command-scoped rule permitting that one file plus the one subtree the command
synchronized; and it gains one flag,
`--quarantine-transaction-state`, which performs no repin at all and exists
only to move this command's own transaction state aside when this command has
refused over it. That flag spawns no Bazel child, so ADR 0052's rules about
the child, its environment and its output base have nothing to bind on it.
Its required hub argument, its scoped child environment, its
single output base, its single child process, its exit-zero-when-current
behaviour and its absence from Make and continuous integration are unchanged,
and the flag is subject to every one of those that still applies.
Nothing in ADR 0052 required the hub's Bazel-side lock to be clean at `HEAD`
before a repin; round 2 of this record added that and round 3 withdraws it, so
this is a correction inside this record and not a change to ADR 0052.

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
or a conflicted merge in that subtree, and an untracked file someone dropped
under a generated directory. The guard is section 6 step 7, which reads before
it writes, through the anchored descriptor chain, and permits exactly three
states per path, the committed content, the content this run would produce, and
the content an earlier run of this command recorded publishing, so the only
bytes the command can replace are bytes already in git, bytes it can reproduce,
or bytes it wrote itself. Everything else is listed repository-relative and
refused, with the reversible targeted stash as the prescribed remedy because
the untracked and ignored cases are the common ones and are exactly the cases
`git restore` does not cover, and with the bounded index resolution prescribed
instead for any path git reports unmerged, because stash refuses a pathspec
that names one. The reason it is a refusal rather than an automatic stash or a
backup copy is that a command with one output set and no hidden state is
reviewable; a command that relocates a contributor's work on its own initiative
is one more place to look when something goes missing.

**A replacement that leaves the tracked subtree absent, mixed, or refused mid
way.** This is what a path-based "check the tree, then swap the directory in
from `.scratch/`" produces, and every part of it is measured rather than
theoretical. The move fails with `EXDEV` when `.scratch/` is on a different
filesystem, which nothing in the repository forbids (constraint 15). Moving
onto the existing directory fails with `ENOTEMPTY` (constraint 14), so the
implementation grows a recursive delete of a tracked tree, and between that
delete and the move `bazel/cargo/broker-workspace/` does not exist: a
concurrent `gen-bazel --check`, a concurrent Bazel analysis, or an interrupt
lands in that window and sees an absent or half-populated tracked directory.
And a check that resolves a string path and then a write that resolves the same
string path again is two resolutions of one name, so a symlink swapped in
between them redirects the write out of the subtree entirely. The guards are
structural: descriptor-anchored resolution under
`RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS |
RESOLVE_NO_XDEV` so the descriptor that answered the question performs the
operation; a staged sibling created by `mkdirat` under the anchored parent so
the filesystem is the same by construction; `RENAME_EXCHANGE` so there is no
window of absence and no recursive delete on the publish path; the support
probe of step 3 so an unsupported filesystem refuses before any tracked
mutation; and the inventory bound of step 11 so the only recursive removal in
the design is confined to entries the command measured itself.

**A recovery that calls the job done because the tree looks done.** This is
the failure the transaction itself makes possible, and it is the one that
would ship a wrong artifact through a green command. Kill the command anywhere
after step 10's exchange and before step 13 has settled a child: the generated
subtree on disk is complete, current, and equal to the journal's staged digest
set. A recovering run that read only the tree would conclude that publication
was the transaction, retire the old copy, settle a hub lock no child had
rendered, tear the transaction down and exit zero, leaving
`bazel/cargo/broker.lock` describing the tree the repository just replaced.
That is the same wrong artifact this section already names under the ordering
hazard, arrived at from the other direction, and it is invisible to the same
three gates: `gen-bazel --check` compares the tree to the manifests and they
agree, the offline `cargo metadata --locked` compares the tree to the lock and
they agree, and constraint 4 never fires because the resolve did not move. The
guard is the journal's child record and the branch that reads it: a recovering
run whose journal carries no successful child rejoins the closed sequence at
the hub-lock snapshot, spawns the child, settles it, and only then tears down,
so the recovery path ends where an uninterrupted run ends. The record is
written after the fact it records and by an atomic rename, so its only failure
direction is one redundant repin, and a recorded success is what makes the
recovering run skip the child rather than repin twice.

**Residue from a run that died before its journal existed.** Kill the command
between step 2 and step 9's journal rename and the transaction directory keeps
whatever that run had reached: a half-populated `staged`, a `journal.tmp` that
never renamed, `probe.a` and `probe.b` from the support probe. No journal
names any of it, so no recovery branch owns it and no inventory bounds it. The
next run then meets it as `EEXIST` from step 3's `mkdirat` or from step 9's
`O_EXCL` staging, which is a confusing refusal on a healthy worktree; and a
staging step written to tolerate `EEXIST` instead would be far worse, because
it would exchange in a directory partly filled by a run whose generation it
never validated. The guard is step 2's sweep, which runs before the probe and
before any staging, bounds itself by the closed nine-name set rather than by
what it finds, verifies each entry's type on the descriptor it is about to
unlink so a planted symlink or device node refuses rather than being followed,
refuses any name outside the set without touching the others, and treats
`ENOENT` as success so it is a goal state rather than an action. `lock` and
`published` are outside its reach by name, which is what keeps a sweep from
eating the ownership token or the receipt that step 7's third permitted state
depends on.

**A refusal whose printed remedy cannot clear it.** The sweep's two refusals,
the unparseable `journal` and the unrecognized entry, and step 4's
missing-snapshot refusal, all name paths inside
`bazel/cargo/.broker-workspace.txn/`, and that directory is ignored by a
committed rule. A remedy carried over from step 7 would name the generated
subtree, clear none of what was listed, and refuse the contributor
identically on the next run, which is the `git restore` defect repeated
against a different pathspec. A stash against the transaction directory
itself, with a per-shape removal for the entries git cannot represent, has the
same defect one level down and is the shape this record carried until round 3
found it: an unrecognized directory git can represent, holding a fifo it
cannot, is measured to survive that stash intact, after which `rmdir --` on it
exits 1 with `Directory not empty` and `rm --` exits 1 with `Is a directory`,
both correctly, and the refusal repeats forever with the evidence split
between a stash and the disk. The guard is that all three refusals print
`cargo xtask bazel-repin --hub broker --quarantine-transaction-state`, whose
bound is a single `renameat2(RENAME_NOREPLACE)` of the containing directory
under the transaction lock: it does not enumerate what it moves, so no nested
shape can outlast it, and it removes nothing, so the state stays readable
where the contributor can see it.

**A Bazel child that half-writes the hub lock and then fails.** `cargo-bazel`
renders `bazel/cargo/broker.lock` from a resolve that touches the network; a
child killed or failed partway can leave that file truncated, or complete but
rendered from an incomplete fetch. Round 2's report told the contributor the
lock was "still describing the previous inputs", which is a claim about a file
the command did not look at after the child touched it. The guard is section 6
step 11's snapshot and step 13's restore: the exact prior bytes are on the
device before the child starts, the restore is a single-file `renameat` through
the same anchor, and the digest the child left is reported rather than
discarded, so the failure is legible and the end state is one state.

**Two writers racing the exchange.** `cargo xtask gen-bazel` and
`cargo xtask bazel-repin --hub broker` both write the generated subtree, and
nothing stops a contributor from running them in two terminals, or a repin
twice. Without exclusion, two exchanges interleave and the surviving live tree
is one run's while the retired tree the other run is about to delete is the
first run's staged copy. The guard is the open file description write lock both
writers take on `bazel/cargo/.broker-workspace.txn/lock`, measured to return
`EAGAIN` to the second holder and to release with the process (constraint 18),
plus step 11's inventory bound, which refuses to remove a tree it did not
measure even if the lock were somehow bypassed.

**A lock the Bazel server holds after the command that took it is gone.** This
is the failure the exclusion mechanism itself makes possible, and it is worse
than the race it prevents because it is permanent. The lock lives on the open
file description, not on the process and not on the file name, so it is
released only when the last descriptor referring to that description closes. A
lock descriptor that reaches step 12 without `O_CLOEXEC` is inherited by the
Bazel client and by the server that client leaves running, and that server is a
daemon that outlives the command by design. Measured (constraint 19), a child
that survives its parent this way keeps the lock: the third process asking for
it got `EAGAIN` although the holder had exited and been reaped, and the
descriptor was visible in the child's `/proc/<pid>/fd`. The contributor's next
`cargo xtask bazel-repin --hub broker` and next `cargo xtask gen-bazel` would
then both refuse, naming a writer that does not exist, and the only remedy
would be to stop a Bazel server nothing in the message mentions. The guard is
that the lock is opened `O_CLOEXEC` through the anchored chain in exactly one
place and never duplicated, since `dup`, `dup2` and `fcntl(F_DUPFD)` each clear
the flag, and the check for it is a descriptor inventory of the child and the
server rather than an assertion about the source, with the non-`O_CLOEXEC`
build as the control that proves the inventory can fail. The alternative guards
are all worse: a stale-lock timeout guesses, a `--force` flag reopens the race,
and a lock file carrying a pid is a lie as soon as the pid is reused.

**A recovery that quarantines a live transaction.** This is the failure the
quarantine mode itself makes possible, and it is the one the lock does not
catch, because an open file description lock names an inode and the rename
that files the directory away names a name. Run the quarantine, and in the
window between it resolving `.broker-workspace.txn` and renaming it, let a
second quarantine complete over the same directory and an ordinary repin
create a fresh transaction directory at the vacated name. Measured, that fresh
directory carries a different inode and a different `lock` file on which
`F_OFD_SETLK` returned 0 while the first run's lock on the moved directory was
still held, so the first run still believes it owns the name, and the rename
it then issues moves the running repin's live transaction: its `staged` tree,
its journal and its hub-lock snapshot all leave the name its descriptors were
opened against, its teardown installs the receipt inside a quarantine nothing
reads, and the contributor's next run finds neither journal nor receipt on a
subtree that may already have been exchanged. Nothing else in this record
would notice. The guard is the identity binding: `fstat` on the locked
directory's descriptor for a witness, `fstatat` on the single-component name
through the anchored parent with `AT_SYMLINK_NOFOLLOW` immediately before the
rename, the same `st_dev`, the same `st_ino` and a directory type required, a
mismatch releasing the lock and re-resolving within a bound of three attempts,
and `D2B-BZLTXN-CONCURRENT` rather than a rename when the bound is spent. In
the interleaving above the re-resolution lands on the live directory and is
then correctly refused `EAGAIN` by the repin that holds its lock. What the
binding cannot prevent is a rename by something that takes no lock, because
measured on this kernel `renameat2`'s whole flag set is `RENAME_NOREPLACE`,
`RENAME_EXCHANGE` and `RENAME_WHITEOUT` and no form of the call binds the
source to an inode; that residue is caught after the fact by the postcondition
`fstatat` on the slot, which requires the witness identity and refuses with
`D2B-BZLTXN-DISPLACED` naming both paths rather than attempting a restore that
would repeat the defect.

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

**An isolation check that passes because it is looking at nothing.** The
condition section 7 states is a subset predicate, and a subset predicate over
an empty set is true. So the failure mode of the guard above is not that it
reports the wrong edge, it is that it reports nothing at all: a generator that
emitted no `-broker` variant, a query pattern that stopped matching after a
package rename, or a set the generator failed to write leave both directions
empty and both conditions green, and the check then certifies a build graph it
never examined. This is the same shape as the drift that ADR 0052 records in
`rust-schema-reproducibility`, where a helper that returns empty on a missing
directory makes the gate compare two empty strings. The guard is section 7's
census: the exact expected first-party census, derived from the broker lock's
six `source`-less entries and the one `workspace_members` entry, is asserted
first, and an empty, short or over-long set fails as a set difference before
either predicate is evaluated.

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

**Roll back the published subtree when the Bazel child fails.** Rejected.
It would rewrite a tree the command just proved correct against the
authoritative lock, in order to restore a tree it proved stale. It makes a run
that did real work indistinguishable from one that never started, which is the
opposite of what a failure report should leave behind. It cannot be made safe
against a concurrent edit arriving between the publication and the
rollback, and the rollback itself is a write the changed-path check would then
have to be taught to permit. The recorded consequence plus an idempotent
re-run is smaller and honest.

The single-file restore of `bazel/cargo/broker.lock` is not the same decision
and is adopted. That file is one regular file, its exact prior bytes are on the
device before the child starts, its only writer did not complete, and the
transaction lock covers the whole interval, so the restore is exact rather than
a reconstruction. The subtree has none of those properties: it is fifteen files
whose prior state the command deliberately proved stale, and restoring it would
mean performing a second tracked mutation to undo a first one that succeeded.

**Replace the subtree by checking the path, then moving the generated tree in
from `.scratch/`.** Rejected on measurement, not on taste. `rename` of a
directory onto a non-empty directory returns `ENOTEMPTY` and `rmdir` on a
non-empty directory returns `ENOTEMPTY` (constraint 14), so this shape requires
a recursive delete of a tracked tree followed by a move, which leaves a window
in which `bazel/cargo/broker-workspace/` is absent or half populated. The move
itself returns `EXDEV` whenever `.scratch/` is on a different filesystem
(constraint 15), which nothing in the repository forbids and which a contributor
can create by pointing `.scratch/` at tmpfs for speed. And resolving the path
once to check it and again to write it is two resolutions of one string, so a
symlink swapped in between them redirects the write. `RENAME_EXCHANGE` from a
staged sibling has none of the four problems and needs no fallback, so no
fallback is authorized: an unsupported filesystem refuses.

**Require `bazel/cargo/broker.lock` byte-identical to `HEAD` before the command
will run.** This was round 2 of this record and it is withdrawn. It is
self-contradictory: the same section calls re-running always safe, while a
successful run leaves that file modified against `HEAD` and so makes the next
run refuse. It also refuses the two states the command is most needed in, a
second dependency edit before the first is committed and a merge conflict in
the generated hub lock, and the second of those is exactly the artifact an
explicit repin exists to regenerate. What round 2 was right about is that a
writer must not silently replace bytes nobody has looked at; what it got wrong
is the instrument. A refusal is the right instrument for the generated subtree,
whose content the command can reproduce and therefore certify; for a file
produced by a network-touching Bazel resolve, no offline predicate exists, and
the honest instrument is an exact snapshot taken before the writer runs,
restored exactly if it fails, and reported as a digest transition if it
succeeds.

**Recover a conflicted subtree by deleting it, and say so in the error.**
Rejected in all three of its usual spellings. `rm -rf` under a contributor's
worktree, printed by a tool, is the tool externalizing the risk the rest of
this design spends its complexity removing. `git clean -fd` has the worktree as
its blast radius when the problem is one subtree. And `git restore` on the
subtree, which round 2 named, does not remove untracked or ignored files at
all: measured at git 2.54.0 on a subtree carrying a modified stub, a deleted
stub source file, an untracked extra and an ignored
`target/debug/out.bin`, it restored the first two and left the other two
exactly where they were, so a contributor following that remedy is refused a
second time by the same paths and reasonably concludes the command is broken.
What is prescribed instead is
`git stash push --all -- bazel/cargo/broker-workspace/`, measured
on the same tree to take all four, to leave the subtree byte-identical to
`HEAD`, to leave an unrelated edit elsewhere untouched, and to be reversed
exactly by `git stash pop`, which brought back the ignored file too. Reversible,
bounded to a pathspec, and it is a command a contributor can read before
running.

`--include-untracked` is rejected as the flag for the same command, which is a
narrower correction than the one above and matters for the same reason. On that
tree it took the modification, the deletion and the untracked extra and left
the ignored `target/debug/out.bin` in place, and an ignored `target/` under this
particular subtree is not a hypothetical: the consequences record that a
contributor who runs `cargo build` in a directory that looks like a Cargo
workspace will create one, and the repository's committed `.gitignore` rule
`target` is what makes it ignored rather than untracked. A remedy that clears
three of the four classes step 7 refuses is a remedy that sends the contributor
back into the same refusal.

Deleting is not the only thing that is wrong for a conflicted subtree; so is
stashing it, and this is the case neither paragraph above covers. `git
stash push` refuses a pathspec naming an unmerged path outright: measured at
git 2.54.0, `--all`, `--include-untracked` and the bare form each exited 1 with
`<path>: needs merge` and stashed nothing. So the remedy for a genuinely
conflicted index is not a stash and is never printed as one. It is the bounded
index resolution of section 6, `git checkout HEAD -- <the unmerged paths>` for
the `UU` and `AA` shapes and `git rm -- <path>` for the `DU` shape where `HEAD`
carries nothing at that path, both measured on the same fixture, both leaving
`MERGE_HEAD` in place so the incoming side is still readable at
`git checkout MERGE_HEAD -- <pathspec>`, and both landing the subtree in a
state the next run regenerates from anyway. The rejected alternative here is
printing one remedy for every refusal shape, which is how round 2's single
`git restore` line became wrong; the command classifies the paths it names.

**Let the command stash or move the conflicting entries itself.** Rejected for
the generated subtree, and the boundary is worth stating exactly. Prescribing
a reversible command the contributor runs keeps the command's output set at
two paths and keeps its behaviour reviewable; performing the stash makes the
command a thing that relocates a contributor's work, which is one more place
to look when something goes missing and one more state a failed run can leave.
Two relocations are accepted against that rule, and neither one moves
contributor work: the interrupt window of step 10, where the alternative is
deletion rather than a prompt, and the quarantine mode, which moves only the
transaction state this command itself owns, only when separately and
explicitly invoked, and never as a side effect of a run that publishes
anything.

**Prescribe a stash against the transaction directory for the three
transaction-state refusals.** Carried by an earlier round of this record,
withdrawn here on measurement, and kept as a negative the checks still run.
`git stash push --all -- bazel/cargo/.broker-workspace.txn/` does clear the
representable state: measured at git 2.54.0 on a fixture carrying the
committed `.broker-*/` rule, it exited 0, captured an unparseable `journal`, a
`lock`, a `published`, a `hub-lock.pre` and a populated `staged/`, removed the
directory, and left an unrelated tracked edit, an untracked file, an ignored
`target/` output and a staged modification to `bazel/cargo/broker.lock`
byte-identical, staging included; on the same state `--include-untracked`
exited 0 reporting `No local changes to save` and captured nothing whatever,
because a committed rule makes every entry there ignored rather than merely
untracked, and `git restore` exited 1 with `pathspec ... did not match any
file(s) known to git`. Three things defeat it anyway. It has no representation
for an empty directory, a fifo, a socket or a device node, and the per-shape
`rmdir --` and `rm --` arms added to cover those do not reach the case that
matters: an unrecognized directory git can represent, holding a fifo it
cannot, survives the stash whole and then refuses both arms, which is the
permanent loop this round removes. It honours no advisory lock: measured, it
unlinked the transaction lock out from under a live `F_OFD_SETLK` holder. And
its record is wider than its pathspec:
`git stash show --include-untracked --name-only stash@{0}` listed an unrelated
staged path alongside the captured entries, because `git stash push` snapshots
the whole index whatever pathspec it is handed, so the refusal had to send
contributors to `git show --name-only --format= stash@{0}^3` instead, and to
follow a targeted `git checkout stash@{0}^3 -- <pathspec>` with
`git reset -- <pathspec>` to unstage what the checkout staged. A remedy that
needs three paragraphs of qualification to be safe is a remedy in the wrong
place.

**Classify the unrecognized entry and prescribe bounded removals per shape.**
Rejected, and it is the fix the finding against the previous round proposed.
Classifying a directory correctly means recursing to its leaves, and the
remedy that comes out is a list of removals over names nobody chose, each
quoted into a shell the contributor pastes, at a depth the message cannot
bound, over bytes that need not be valid UTF-8. It also has to delete in order
to succeed, which destroys the evidence the refusal exists to surface. One
`renameat2` subsumes every one of those cases without reading a single name.

**Quarantine into `.scratch/` rather than into a sibling under
`bazel/cargo/`.** Rejected by constraint 15. Nothing in the repository
constrains what `.scratch/` is mounted on, and a cross-mount rename returned
`EXDEV`, so the one operation that must not fail would fail exactly on the
machines whose `.scratch/` is a tmpfs. The quarantine is a sibling under the
anchored parent for the same reason `staged` is.

**One fixed quarantine name, or a timestamped unique one.** Rejected at both
ends. A single name refuses the second occurrence with `EEXIST` and leaves a
manual recursive delete as the only way forward, which is the remedy this
section exists not to print. A timestamp or a random suffix cannot be named in
a `.bazelignore` entry ahead of time, and a quarantined `staged/` carries its
own `BUILD.bazel`; nor can a check assert a path it cannot predict. Four fixed
slots are ignorable in advance, assertable exactly, and bounded.

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
   synchronizes its own generated inputs. In order, and mutating no tracked
   path until the exchange: it anchors on the worktree root and resolves
   `bazel/cargo` beneath it with `openat2` under
   `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS |
   RESOLVE_NO_XDEV`, and every path it reads or writes under that directory is
   resolved relative to that descriptor under those flags, with no decision
   taken from a `stat` on a string path and acted on by a second open of the
   same string; it takes an `F_OFD_SETLK` write lock on
   `bazel/cargo/.broker-workspace.txn/lock`, refusing when another writer holds
   it, on a descriptor resolved through that same anchored chain and opened
   `O_RDWR | O_CREAT | O_CLOEXEC`, opened in exactly one place and never
   duplicated by `dup`, `dup2` or `fcntl(F_DUPFD)`, so no child it spawns and
   no daemon such a child leaves running can inherit the open file description
   and hold the lock past this command's exit; it clears, under that lock and
   before anything else, the pre-journal residue no journal accounts for,
   bounded by the closed name set of the transaction directory and refusing
   any other name; it proves `RENAME_EXCHANGE`
   works on that filesystem before doing any
   other work and refuses with the filesystem and the remedy named when it does
   not; it finishes or refuses an interrupted transaction; it regenerates the
   broker splice workspace into an ignored scratch root through the same
   generator entry point `cargo xtask gen-bazel` calls, never reading the
   tracked subtree as an input; it validates that
   scratch tree, requiring section 4's refusals to pass, its lock mirror to be
   byte-identical to `packages/d2b-priv-broker/Cargo.lock`, and
   `cargo metadata --locked --offline` on it to exit zero; it refuses when any
   path under `bazel/cargo/broker-workspace/`, tracked or untracked, is none of
   its committed `HEAD` content, the freshly generated content, and the content
   the transaction directory's publication receipt records, or when the subtree
   holds a path outside the union of those three file sets, or when it
   holds any entry that is not a directory or a regular file; it refuses when
   the committed `manifests` list for the hub is not what the fresh generation
   implies, naming `MODULE.bazel` and `cargo xtask gen-bazel`; and it then
   publishes only `bazel/cargo/broker-workspace/**` from the validated result.
   It writes nothing under `packages/`, no other hub's inputs, and not
   `MODULE.bazel`. `bazel/cargo/broker.lock` is not required clean at `HEAD`
   and never has been by ADR 0052; requiring it would make a successful run
   refuse the next one.
   Every refusal above names every offending path repository-relative,
   prescribes a remedy that is bounded to what the refusal names, that
   destroys nothing and that can be undone, and
   spawns no Bazel process, creates no output base, and places
   `CARGO_BAZEL_REPIN` and `CARGO_BAZEL_REPIN_ONLY` in no child environment.
   The remedy is classified by path state rather than printed uniformly:
   `git stash push --all` on the named pathspec for tracked, untracked and
   ignored entries under the generated subtree, never `--include-untracked`,
   which is measured to leave an
   ignored `target/` under the subtree in place; for any path git reports
   unmerged, the bounded index resolution instead, since `git stash push`
   refuses a pathspec that names an unmerged path in every form; and for any
   refusal whose offending paths lie inside the transaction directory, the
   built-in quarantine of invariant 9 rather than any git command at all.
   `rm -rf`, a
   broad `git clean`, and a bare `git restore` offered as a way to remove an
   untracked or ignored entry are never printed, and no remedy this command
   prints interpolates a path it read from disk into a command line.
9. The publication is a transaction with no window of absence and no
   unbounded deletion. The validated bytes are copied into
   `bazel/cargo/.broker-workspace.txn/staged`, a directory created by `mkdirat`
   under the anchored `bazel/cargo` descriptor and therefore on the same
   filesystem by construction; every staged file is created with
   `O_RDWR | O_CREAT | O_EXCL | O_CLOEXEC`, fsynced, and read back
   through the descriptor that wrote it, the access mode being what makes that
   readback possible without resolving the name a second time; a journal
   recording the measured
   old-tree inventory and the staged digest set is made durable first; and the
   subtree is then published by one
   `renameat2(.., RENAME_EXCHANGE)` when it exists or one
   `renameat2(.., RENAME_NOREPLACE)` when it does not, followed by an `fsync`
   of the anchor. There is no recursive-delete fallback and no path-based
   replacement; an unsupported primitive refuses before any tracked mutation.
   After the exchange the old tree sits at the transaction name and is removed
   descriptor-relatively only if every entry matches the journal's old-tree
   inventory by path, type, size and digest; one entry that does not aborts the
   removal, leaves the tree in place, and reports that path. Recovery from an
   interruption is decided by comparing the live subtree's content to the two
   sets the journal already recorded, its old-tree inventory and its staged
   digest set, never by a phase marker and never against a fresh generation
   that a later manifest edit could have moved, and refuses without deleting or
   exchanging anything when the live subtree matches neither. Before the
   support probe and before any staging, and unconditionally under the same
   lock, the command sweeps the residue no journal accounts for: when no
   `journal` is present it removes exactly the entries named `staged`,
   `journal.tmp`, `hub-lock.pre`, `hub-lock.restore`, `probe.a` and `probe.b`,
   each resolved and type-checked on the descriptor it is about to unlink so
   that anything other than a directory under the three directory names or a
   regular file under the three file names refuses instead of being followed;
   when a parseable journal is present it removes nothing and recovery owns
   them all; when a `journal` is present that this build cannot parse it
   refuses naming that file and deletes nothing. Any entry outside those nine
   names refuses, is left in place, and is reported. `lock` and `published`
   are never removed by the sweep, `ENOENT` counts as success throughout, and
   a sweep interrupted partway is completed by the next run. Every refusal
   whose offending paths lie inside the transaction directory, these two and
   step 4's missing recorded hub-lock snapshot, prescribes
   `cargo xtask bazel-repin --hub broker --quarantine-transaction-state` and
   that command alone, never the generated subtree, never
   `bazel/cargo/broker.lock`, and never a git command. That mode is the only
   recovery for transaction state and is bounded by what it does not do. It
   is accepted only with `--hub broker`, and that guard is decided from the
   parsed arguments alone before the worktree root is opened, before
   `bazel/cargo` or the transaction directory is resolved and before the lock
   is opened, so an invocation naming `main`, `guest` or `walker` exits
   nonzero with `D2B-BZLTXN-HUB` having created nothing, moved nothing and
   opened nothing under the worktree. It resolves `bazel/cargo` and everything
   under it descriptor-relatively under the four resolve flags through one
   descriptor that is the `fstatat` anchor, both rename anchors and the fsync
   target. It then binds the entry it is about to move to the directory it
   holds the lock on, and never renames without that binding: resolve
   `.broker-workspace.txn` beneath that descriptor
   `O_RDONLY | O_DIRECTORY | O_CLOEXEC`; open the transaction lock
   `O_RDWR | O_CREAT | O_CLOEXEC` beneath *that directory descriptor* and
   refuse with the lock path named when `F_OFD_SETLK` returns `EAGAIN`;
   `fstat` the directory descriptor for a witness `st_dev` and `st_ino`;
   `fstatat` the single component `.broker-workspace.txn` relative to the
   anchored parent with `AT_SYMLINK_NOFOLLOW` and require the same `st_dev`,
   the same `st_ino` and a directory type immediately before the rename. An
   absent name, a differing identity or a non-directory type releases the lock
   by closing its only descriptor and retries from resolution, at most three
   resolution attempts in total, after which the command exits nonzero with
   `D2B-BZLTXN-CONCURRENT` having moved and created nothing; a name that does
   not resolve to a directory at all refuses under the same code. Only on a
   match does it rename the whole transaction
   directory onto the first free one of the four fixed sibling names
   `bazel/cargo/.broker-workspace.txn.quarantine.0` through
   `bazel/cargo/.broker-workspace.txn.quarantine.3` with
   `renameat2(RENAME_NOREPLACE)`, re-taking that identity check before each
   slot attempt so the observation is always the syscall immediately preceding
   the rename it authorizes, treating all four slots occupied as a terminal
   refusal rather than a retry trigger, and treating `ENOENT` from that call
   as an attempt consumed and a return to resolution. After a rename returns 0
   it `fstatat`s the
   slot through the same anchor with `AT_SYMLINK_NOFOLLOW` and requires the
   witness identity and a directory type; on any difference it exits nonzero
   with `D2B-BZLTXN-DISPLACED`, names the slot and the transaction path
   repository-relative, and neither moves anything further nor attempts a
   restore, because a restore is a second unsynchronized rename with the same
   defect. Otherwise it fsyncs the shared parent, releases the lock,
   prints the quarantine path repository-relative and names re-running the
   ordinary repin. It reads no entry inside what it moves, removes nothing,
   spawns no process, writes nothing under `bazel/cargo/broker-workspace/`,
   `bazel/cargo/broker.lock` or any Cargo manifest or lock, exits zero without
   writing when the transaction directory is absent, and refuses without
   moving anything when all four slots are occupied. Its refusals carry
   exactly the codes `D2B-BZLTXN-HUB`, `D2B-BZLTXN-CONCURRENT` and
   `D2B-BZLTXN-DISPLACED` beside the unchanged `EAGAIN` contention refusal,
   each naming only its own remedy, and none prints an absolute path, a user
   identifier, a process identifier, or any byte read from inside the
   directory it refused over.
10. That command then spawns exactly one Bazel child under ADR 0052 section 3's
    scoped controls and derived output root. Two changed-path rules bound the
    result. The child may change only `bazel/cargo/broker.lock`, which is ADR
    0052's rule unweakened. The command's own change set, computed by
    subtracting a pre-run worktree snapshot from the same snapshot retaken, must
    be a subset of `bazel/cargo/broker-workspace/**` plus
    `bazel/cargo/broker.lock`; any other changed path fails the command and is
    listed repository-relative. The exact bytes of `bazel/cargo/broker.lock`,
    or the fact of its absence, are captured before the child starts; on child
    failure they are restored exactly by a single-file `renameat` through the
    anchor, or by `unlinkat` when the record says absent, and the digest the
    child left is reported; on child success the rendered lock is accepted, the
    digest transition is reported, the journal is rewritten by atomic rename to
    record that the child completed successfully, and the snapshot is discarded
    at teardown, after the journal naming it has been retired, so a journal and
    its snapshot never disagree. Publishing the subtree is never the end of the
    transaction. A
    recovering run that finds the live subtree equal to the journal's staged
    digest set and no recorded successful child continues to the hub-lock
    snapshot, the Bazel child and the settlement before it applies the
    command-scoped changed-path check and tears the transaction down; it may
    skip the child only when the journal records one that succeeded, and it
    never spawns a second child after such a record. When the child
    fails after publication the command reports the resulting state, subtree
    current and hub lock byte-identical to its pre-child bytes, never rolls the
    subtree back and never overwrites a modification it did not make, and tears
    the transaction down so that no journal survives a settled failure;
    re-running the same command is the recovery and is idempotent from every
    state this command can leave, including one in which
    `bazel/cargo/broker.lock` differs from `HEAD`.
11. This decision adds no Make target, no Layer-1 job, no required
    continuous-integration context, and no top-level shell gate. Its drift
    checks extend `test-drift`, which already exists for this class of
    staleness, and its transaction, recovery, residue, quarantine,
    quarantine-identity, hub-guard,
    changed-path and lock-descriptor negatives
    are `#[test]`s in `packages/xtask`, running under the existing
    `rust-main-workspace-tests` surface against throwaway fixture repositories
    rather than the contributor's worktree; no new shell gate carries any of
    them. The observation points those tests drive the quarantine's identity
    binding through are a `#[cfg(test)]`-gated parameter of the library entry
    point, so the shipped `xtask` binary carries no fault-injection surface
    and the release CLI path constructs none. No
    gate, Make target, workflow, or Bazel invocation on the gate path generates
    a tracked artifact. The only writers of
    `bazel/cargo/broker-workspace/**` are `cargo xtask gen-bazel` and, for that
    subtree alone, `cargo xtask bazel-repin --hub broker`; both are
    contributor-invoked, both take the same write lock before writing, on a
    descriptor opened `O_CLOEXEC` so the lock cannot outlive the writer that
    took it, and neither is reachable from Make or continuous
    integration. The quarantine mode is the same command under a flag and
    inherits all of that: it is contributor-invoked, takes that same lock,
    writes neither the subtree nor anything else, and is reachable from no
    Make target, no workflow and no gate.
    `cargo xtask gen-bazel --check` in `test-drift` stays
    read-only, takes no lock, and remains the fail-closed gate for a stale
    tracked tree.
12. `bazel/cargo/.broker-workspace.txn/` is the only path outside
    `bazel/cargo/broker-workspace/**` and `bazel/cargo/broker.lock` that either
    writer may create during a transaction, and
    `bazel/cargo/.broker-workspace.txn.quarantine.0` through
    `bazel/cargo/.broker-workspace.txn.quarantine.3` are the only other paths
    this decision authorizes at all. Those four come into existence only by the
    quarantine mode renaming the transaction directory onto one of them, never
    by a run that publishes, and nothing reads one afterwards.
    It is transaction state, not general scratch: no gate,
    build, or hub declaration reads it, it holds only the ownership lock and
    the publication receipt between transactions, it is owned by the invoking
    user, and this decision creates no
    root-owned path, no systemd unit, and no state outside the worktree. The
    ownership lock is held only by a running writer, because the descriptor
    carrying it is `O_CLOEXEC` and never duplicated; there is no stale-lock
    timeout, no lock-breaking flag, and no pid file, and none may be added. It
    is named in a committed `.gitignore` entry and in a generated `.bazelignore`
    entry, each quarantine slot is named in a generated `.bazelignore` entry of
    its own and is covered by the same committed `.broker-*/` rule without one
    of its own, and `cargo xtask gen-bazel --check` refuses when any of them is
    missing.
    Unvalidated generation stays in `.scratch/`, whose filesystem this decision
    does not constrain and does not depend on. The receipt widens no
    permission: every path it names is still compared byte for byte, so a
    stale or absent receipt narrows the permitted set rather than opening one.
13. Let B be every target of a broker Cargo workspace member plus every
    `-broker` variant, and M every other first-party Rust target. Over Rust
    compile and link edges, the `deps` and `proc_macro_deps` closure, the
    first-party portion of `deps(B)` is a subset of B and the first-party
    portion of `deps(M)` is a subset of M. Runfiles, `data` and source-file
    edges are outside this invariant, since they carry no compilation across
    the boundary. Both sets are read from what the generator emitted, not
    matched from label text, and both are asserted against an exact expected
    census before either subset predicate is evaluated: the `-broker` variants
    are exactly the `source`-less entries of
    `packages/d2b-priv-broker/Cargo.lock` minus that manifest's
    `workspace_members`, five today, and B additionally covers every Cargo
    target of each member package, while M is nonempty, carries the five
    unsuffixed variants, and carries no member target and no `-broker` variant.
    An empty, short or over-long set is a failure reported as a set difference,
    because a subset predicate over an empty set proves nothing. The `@main//`
    and `@broker//` spoke assertion is supplemental and may not be the only
    check.

## Implementation checks

These are the mechanically evaluable conditions that make this decision
implementable without reopening it, so Spec 003 can be amended after this
record merges. Each is a command and a verdict.

1. `cargo xtask gen-bazel` on a clean tree, then `git status --short`, lists
   only `MODULE.bazel`, `bazel/cargo/broker-workspace/**`, `.bazelignore`, and
   the first-party `BUILD.bazel` files the generator already owns.
   `bazel/cargo/.broker-workspace.txn/` exists on disk afterwards and appears
   in neither `git status --short` nor `git status --porcelain`, which is the
   evidence that the committed `.gitignore` entry is present and that neither
   changed-path check can trip over the command's own workspace. Removing
   the `.gitignore` entry, the transaction directory's `.bazelignore` entry,
   or any of the four quarantine slots' `.bazelignore` entries makes
   `cargo xtask gen-bazel --check` exit nonzero naming the missing entry;
   reverted. `git check-ignore -v` on each of
   `bazel/cargo/.broker-workspace.txn.quarantine.0` through
   `bazel/cargo/.broker-workspace.txn.quarantine.3` reports the committed
   `.broker-*/` rule, which is why those four need no `.gitignore` entry of
   their own.
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
   repin-first result. Re-running the repin immediately after that success,
   with `bazel/cargo/broker.lock` now differing from `HEAD`, exits zero rather
   than refusing, which is the check round 2's clean-at-`HEAD` rule would have
   failed; so does a run started from a worktree in which
   `bazel/cargo/broker.lock` carries conflict markers from a merge, which ends
   with that file replaced by a freshly rendered lock and no marker left, and
   with that path still reported unmerged by `git status --porcelain` until it
   is staged, which the run's report names alongside `git add --` on that one
   path. And
   so does a second deliberate broker dependency change made with the first
   still uncommitted: the run finds the subtree matching neither `HEAD` nor its
   own fresh generation, accepts it against the publication receipt, and exits
   zero. Deleting the receipt first makes that same run refuse and name the
   subtree paths, which is the evidence that the receipt is what admitted them
   and that its absence fails closed. Reverted.
8. The command refuses ambient work instead of overwriting it, refuses before
   Bazel, and prescribes a remedy that actually clears the refusal. For each of
   these planted pre-states, reverted after each,
   `cargo xtask bazel-repin --hub broker` exits nonzero, lists the offending
   path repository-relative, and leaves every path under
   `bazel/cargo/broker-workspace/` byte-unchanged with no path added or
   removed: a hand edit to a stub manifest that a fresh generation would not
   produce; an untracked extra file under the subtree; an ignored
   `bazel/cargo/broker-workspace/target/debug/out.bin`, which the committed
   `.gitignore` rule `target` makes ignored rather than untracked; a deleted
   stub `src/lib.rs`; and a symlink placed under the subtree. For each, running
   the prescribed
   `git stash push --all -- bazel/cargo/broker-workspace/` and
   re-running the command then exits zero, and `git stash pop` restores every
   planted entry, tracked, untracked and ignored alike, which is what makes the
   remedy reversible rather than merely effective. The ignored case is also run
   against `--include-untracked`, which must leave that entry in place and make
   the re-run refuse identically; that is the control proving why the printed
   flag is `--all`. The message contains no `rm -rf`, no `git clean`, and no
   bare `git restore` offered as a way to remove an untracked or ignored entry,
   and it prints `--all` rather than `--include-untracked`. With a stub
   manifest left unmerged in the index by an interrupted merge, the same
   refusal occurs and the message names the bounded index resolution instead of
   a stash for that path; running the stash it does not print is separately
   demonstrated to exit 1 with `needs merge` and stash nothing, and running
   what it does print clears the refusal. An
   uncommitted one-byte change to `bazel/cargo/broker.lock` is explicitly not
   in this set: that pre-state must exit zero and end with the file replaced by
   the rendered lock. Each refusal is also produced identically with
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
    `bazel/cargo/broker.lock` is byte-identical to its pre-child bytes, and the
    message names both paths and names re-running the same command as the
    recovery. Re-running it with the real binary then exits zero and reports
    `bazel/cargo/broker.lock` as the only further change.
11. The hub-lock snapshot is restored exactly, and the restore is demonstrated
    against a child that wrote. With an injected wrapper that truncates
    `bazel/cargo/broker.lock` to zero bytes and then exits nonzero, the command
    exits nonzero, `bazel/cargo/broker.lock` is byte-identical to its pre-run
    bytes under `cmp`, and the report names the zero-length digest the child
    left. With an injected wrapper that appends a byte and exits zero, the
    command holds the child to that one file and accepts the result, reporting
    both digests. With `bazel/cargo/broker.lock` deleted before the run and an
    injected failing wrapper, the file is absent afterwards rather than
    resurrected. All three reverted.
12. Both changed-path rules are demonstrated failing, and the command-scoped
    one is demonstrated failing on its own. These are `#[test]`s in
    `packages/xtask`, running under the existing `rust-main-workspace-tests`
    surface, not a new shell gate and not a mutation of the contributor's
    worktree: each drives the transaction as a library entry point against a
    throwaway fixture repository created under the test's temporary directory,
    which constraint 14 measured `RENAME_EXCHANGE` to support on tmpfs as well
    as on ext4, with the writer and the child both supplied by the test, so no
    fault-injection hook exists in any shipped code path. Two arms.

    Child-scoped, step 13. An injected wrapper standing in for the Bazel child
    modifies one forbidden tracked path outside the generated subtree and
    outside `bazel/cargo/broker.lock`, `packages/d2b-priv-broker/Cargo.lock`,
    which is constraint 5's regression shape. The command exits nonzero, names
    that path repository-relative, attributes the refusal to the child-scoped
    rule, and does not revert or delete the path it names.

    Command-scoped, step 14. The writer the test supplies emits the correct
    subtree and additionally writes one forbidden tracked path,
    `packages/Cargo.toml`, while the injected child touches only
    `bazel/cargo/broker.lock`. Step 13 therefore passes and step 14 is what
    fails; the command exits nonzero, names `packages/Cargo.toml`
    repository-relative, and attributes the refusal to the command-scoped rule.
    This arm is the one that proves step 14 is not dead code masked by step 13,
    and it must fail for that reason and not because the child was also caught.

    In both arms the fixture's `git status --porcelain` after the failed run
    contains exactly the forbidden path plus the permitted set, so the report
    is complete rather than merely nonempty; `bazel/cargo/broker.lock` is
    settled by step 13's rule rather than left as the child wrote it; the
    transaction directory is left in a state from which re-running the same
    command reaches the same end state as an uninterrupted run, with the
    retired tree either removed under the journal's inventory or left in place
    and named; nothing outside the subtree is deleted; and no real Bazel
    process is spawned and no output base is created.
13. The publication is atomic and the primitive is proved before any tracked
    mutation. During a run, a concurrent reader looping on
    `test -f bazel/cargo/broker-workspace/Cargo.toml` never observes the path
    absent, and no run leaves a `.tmp` or partially populated directory at
    `bazel/cargo/broker-workspace/`. The subtree's directory inode number
    changes across a run that publishes new content and is unchanged across a
    run that publishes nothing, which is the observable signature of an
    exchange rather than an in-place rewrite. With `renameat2(RENAME_EXCHANGE)`
    forced to fail, for example by running the command against a worktree on a
    filesystem that does not implement it, the command exits nonzero naming the
    filesystem and the remedy, `git status --porcelain` is identical before and
    after, and no Bazel process is spawned. No code path in the implementation
    calls a recursive remove against `bazel/cargo/broker-workspace/`; the only
    recursive remove is the inventory-bounded one against the transaction
    directory, and a grep for an unbounded recursive remove over the subtree
    path returns nothing.

    The staged bytes are verified against the device rather than against
    memory. Every staged file is opened
    `O_RDWR | O_CREAT | O_EXCL | O_CLOEXEC` and its digest is read back from
    the same descriptor; the control is a build with that access mode changed
    to `O_WRONLY`, under which the readback must fail with `EBADF` rather than
    be skipped, silently pass, or reopen the file by name. A grep of the
    implementation finds exactly one open of a staged file and no second
    resolution of a staged path by name after it. Reverted.
14. Recovery from an interruption is exact and bounded, and it never ends
    before the child does. For each of these
    injected abort points, with the process killed rather than allowed to
    unwind, re-running `cargo xtask bazel-repin --hub broker` reaches the same
    end state as an uninterrupted run and reports what it recovered: after the
    staged tree is written but before the journal is durable; after the journal
    is durable but before the exchange; after the exchange but before the
    hub-lock snapshot; after the snapshot but before the old tree is
    retired; after retirement but before the Bazel child is spawned; during
    the child; after the child succeeded but before the journal records it;
    and after that record but before the journal is renamed over the
    receipt. In the third through eighth cases the recovering run must not
    re-exchange, which is checked by the subtree's inode number being unchanged
    across the recovery. Editing a broker manifest between the interrupted run
    and the recovering one changes none of those outcomes, which is the
    evidence that recovery decides against the journal's recorded sets and not
    against a fresh generation. With the retired tree seeded with one extra
    file that the journal's inventory does not name, the recovering run leaves
    that directory in place, reports its path, and exits nonzero rather than
    removing it. With the live subtree replaced by content matching neither the
    journal's inventory nor the journal's staged digest set, the recovering run
    refuses, names the paths, deletes nothing, and exchanges nothing.

    The child half of that list is checked by what the recovering run spawns,
    not only by where it ends, and each expectation is exact. For the abort
    after the exchange, after the hub-lock snapshot, after retirement, and
    during the child, the recovering run spawns exactly one Bazel child,
    ends with `bazel/cargo/broker.lock` rendered from the published subtree
    and reported as a digest transition, and exits zero; a recovering run in
    any of those cases that exits zero having spawned no child fails this
    check, which is the arm that would have passed before the journal carried
    a child record. For the abort after the child succeeded but before the
    record is durable, the recovering run spawns exactly one child again and
    ends in the same state, which is the accepted cost of a record that can
    only understate. For the abort after that record, the recovering run
    spawns no child at all, performs the command-scoped changed-path check,
    tears the transaction down, and exits zero, with
    `bazel/cargo/broker.lock` byte-identical under `cmp` to what the recorded
    child left, so a recorded success is never repinned twice. In every one of
    these the child count is measured by an injected wrapper that counts its
    own invocations, not inferred from timing or from the report.

    Pre-journal residue is cleared unconditionally, and the sweep is
    idempotent. These are `#[test]`s in `packages/xtask` on the same throwaway
    fixture repositories as check 12. For each of these abort points, all
    before any journal exists, the process is killed and the command is then
    run twice: after step 3's probe directories are created but before they
    are removed; after `staged` is created but before any file is written into
    it; after `staged` is partly populated; and after `journal.tmp` is written
    but before it is renamed. One further abort point is on the other side of
    the transaction and is swept by the same rule: after step 14's journal
    rename but before `hub-lock.pre` is unlinked, which leaves a snapshot with
    no journal to own it. In each case the first re-run exits zero, the
    transaction directory afterwards holds only `lock` and `published`, and
    the second re-run exits zero and changes nothing further, which is what
    makes the sweep a goal state rather than an action. `published` is
    byte-identical across all of it and `lock` is never unlinked, which is
    checked by its inode number being unchanged. With a symlink, a fifo, a
    socket, a character device and a directory each planted in turn at
    `hub-lock.pre`, and with a regular file planted at `staged`, the run exits
    nonzero, names that path, unlinks nothing, and spawns no Bazel. With an
    unrecognized name planted in the transaction directory, the run exits
    nonzero, names it, and leaves every other residue entry in place, which is
    the evidence that an unknown entry refuses rather than being swept
    alongside the known ones. With a `journal` present and parseable, none of
    the residue names is swept and recovery owns them, checked by the staged
    tree surviving a run that then recovers through it; with a `journal`
    present whose format version this build does not know, the run exits
    nonzero naming `journal` and deletes nothing, `journal` included. All
    reverted.

    The remedy each transaction-directory refusal prints is executed, not
    merely string-matched, and it is one command rather than a shell line.
    These are `#[test]`s in `packages/xtask` on the
    same throwaway fixture repositories, each carrying the committed
    `.broker-*/` ignore rule and four unrelated states that must survive
    every case: a modified tracked file, an untracked file, and an ignored
    `target/` output, all outside `bazel/cargo/`, plus a staged modification
    so the index is covered too. For each of the three refusals, an
    unparseable `journal`, an unrecognized entry, and a `journal` recording a
    `hub-lock.pre` that is not on disk, the test plants that state, runs the
    command, asserts that the refusal names
    `cargo xtask bazel-repin --hub broker --quarantine-transaction-state` and
    contains no `git` command, no `rm`, no `rmdir` and no interpolated path
    from the directory it refused over, then runs that mode as a library entry
    point and asserts four things. It exits zero.
    `bazel/cargo/.broker-workspace.txn/` is gone and
    `bazel/cargo/.broker-workspace.txn.quarantine.0` holds exactly what the
    transaction directory held, compared entry by entry on name bytes, type
    and content. A second `cargo xtask bazel-repin --hub broker` does not
    reproduce the same refusal, asserted against the refusal identity and not
    against exit status alone, so a re-run stopped for some unrelated reason
    cannot pass it silently. And the four unrelated states are byte-identical
    to their pre-remedy values with `git status --porcelain` unchanged, which
    here is a stricter assertion than it is for the subtree remedies: the
    quarantine names are covered by the committed `.broker-*/` rule, measured
    with `git check-ignore -v`, so that output must be byte-identical rather
    than merely free of unrelated paths.

    The quarantine preserves what nothing else can, and that is the arm the
    round-3 finding forces. On the unrecognized-entry fixture the planted
    entry is a directory holding, one of each: a regular file with known
    bytes; a file whose name a `.gitignore` rule matches and one whose name it
    does not, both planted to prove the distinction means nothing inside a
    directory a committed rule already ignores; a fifo; a unix socket; a
    symlink; an empty directory; a name whose bytes are not valid UTF-8; and a
    second level holding another fifo. After the quarantine every one of them
    is present under the quarantine path with its type intact, the nested
    regular file's bytes compare equal, and the containing directory's inode
    number is unchanged, which is the evidence that the entries were moved
    rather than copied and reconstructed. The character-device arm is
    conditional and says so: `mknodat` for one returns `EPERM` unprivileged,
    so the test skips that shape with a recorded reason rather than asserting
    something it did not run. No assertion in any of these arms renders a
    planted name into a command line.

    Lock contention, collision, repetition and absence are each their own arm.
    With an injected pause holding the transaction lock, the quarantine mode
    exits nonzero naming `bazel/cargo/.broker-workspace.txn/lock`, moves
    nothing, and the transaction directory is byte-unchanged; killing the
    holder lets the next invocation proceed. With `.quarantine.0` already
    present, the next quarantine lands in `.quarantine.1` and `.quarantine.0`
    is byte-unchanged, which is the collision arm and is asserted against that
    exact slot rather than against any free one. With all four slots occupied
    the mode exits nonzero, lists all four repository-relative, leaves the
    transaction directory in place, prints no removal command of any kind, and
    leaves `git status --porcelain` unchanged. Run twice in a row it exits
    zero both times, the second run having found no transaction directory and
    written nothing. After any successful quarantine an ordinary
    `cargo xtask bazel-repin --hub broker` creates a fresh transaction
    directory, takes the lock, and reaches the same end state as a run on a
    clean tree, which is the arm that proves the recovery recovers rather than
    merely stopping the command from working. Throughout, no arm spawns a
    Bazel process or creates an output base, and
    `bazel/cargo/broker-workspace/**`, `bazel/cargo/broker.lock`,
    `packages/d2b-priv-broker/Cargo.toml` and
    `packages/d2b-priv-broker/Cargo.lock` are byte-identical before and after
    every one of them.

    The identity binding is exercised at each fault point it exists for, and
    the injection surface does not ship. These are in-crate `#[test]`s in
    `packages/xtask` on the same throwaway fixture repositories, driving the
    quarantine as a library entry point whose named observation points are a
    `#[cfg(test)]`-gated parameter, so the release build of `xtask` carries no
    injection surface at all; a grep of the non-`cfg(test)` build for that
    parameter type returns nothing, and the release CLI path constructs none.
    Six arms, each planting at a different point in the sequence and each
    asserting a different outcome, with every identity assertion made on
    `st_dev` and `st_ino` rather than on path strings.

    Before the transaction directory is opened, the entry at
    `bazel/cargo/.broker-workspace.txn` is replaced. With a symlink to a
    directory, and with a regular file, the run exits nonzero with
    `D2B-BZLTXN-CONCURRENT`, creates no quarantine slot, and leaves the
    planted entry exactly where it is; the control is the third shape, an
    unrelated directory, which is legitimately what the name denotes, so the
    run quarantines it, the postcondition passes against that directory's own
    inode, and the run exits zero.

    Between the transaction directory being opened and the lock being taken,
    and again after the lock is taken but before the identity check, the name
    is repointed at a second directory. In both arms the lock is held on the
    witnessed inode, because it was opened beneath that directory's
    descriptor; the identity check reports the mismatch, the run releases the
    lock and re-resolves rather than renaming, and no quarantine slot exists
    at any point during the first attempt, which is asserted by an observer
    the test drives inside the retry rather than only at the end. These two
    arms are kept separate because the second is the state in which an
    implementation that trusted the lock would consider itself safe.

    Identity mismatch renewed on every attempt exhausts the bound. With the
    test replacing the name with a fresh directory at each retry, the run
    exits nonzero with `D2B-BZLTXN-CONCURRENT` after exactly three resolution
    attempts, counted by the observation point rather than inferred, moves
    nothing, creates no slot, and leaves `git status --porcelain` unchanged.
    A bound that is not three fails this check in either direction.

    A replacement landing after the identity check and before `renameat2` is
    the only arm the postcondition exists for and it must fail loudly rather
    than silently. With a second directory swapped in at that point, the
    rename returns 0, the postcondition finds an inode that is not the
    witness, and the run exits nonzero with `D2B-BZLTXN-DISPLACED`, names both
    `bazel/cargo/.broker-workspace.txn` and the slot repository-relative,
    leaves the moved entry in the slot rather than attempting a restore, and
    does not touch the witnessed directory. A build with the postcondition
    removed must make this arm pass silently, which is how the check proves
    the postcondition is not dead code; reverted.

    A fresh live transaction is never moved, which is the failure the whole
    binding exists to prevent. The fixture is driven to the exact interleaving:
    the run resolves and locks transaction directory T, the test then moves T
    onto `.quarantine.0` and creates a fresh T2 at the name with its own
    `lock`, a second holder takes `F_OFD_SETLK` on `T2/lock` as a running
    repin would, and the run resumes. It must find the mismatch, release, and
    re-resolve to T2, whereupon the lock is refused `EAGAIN` and it exits
    nonzero naming `bazel/cargo/.broker-workspace.txn/lock`. Afterwards
    `bazel/cargo/.broker-workspace.txn` still resolves to T2's inode,
    `.quarantine.1` does not exist, and `.quarantine.0` still holds T's inode
    with its entries intact. Running the same interleaving against a build
    with the identity check removed must instead move T2 onto `.quarantine.1`,
    which is the measured proof that the round-3 sequence had this defect and
    that the check is what removes it; reverted.

    The flag is broker-only and the guard runs before any worktree state is
    touched. This is a later `#[test]` in `packages/xtask` on the same fixture
    shape and it is a negative in all three of its arms. For each of `main`,
    `guest` and `walker`, the closed four-hub set minus `broker`,
    `cargo xtask bazel-repin --hub <name> --quarantine-transaction-state`
    exits nonzero with `D2B-BZLTXN-HUB`, its message states that the flag is
    broker-only and names
    `cargo xtask bazel-repin --hub broker --quarantine-transaction-state` as
    the only accepted form, and no quarantine slot exists afterwards. Two of
    the three fixture states are what make the ordering mechanically
    checkable rather than asserted. In the first, the transaction directory
    exists and carries `published` but no `lock`: after the refused run,
    `fstatat` on `bazel/cargo/.broker-workspace.txn/lock` must still return
    `ENOENT`, which a guard placed after the `O_RDWR | O_CREAT` open could not
    satisfy, and the directory's inode is unchanged. In the second, the
    transaction directory is absent entirely: the run must still exit nonzero,
    because a guard placed after the transaction directory is resolved would
    take the absent-directory path and exit zero, reporting success for a hub
    the flag is meaningless for. In the third, an ordinary transaction
    directory with `lock` and `published` present is byte-identical
    afterwards, entry by entry on name bytes, type and content, with its inode
    unchanged. Across all three, `git status --porcelain` is byte-identical,
    no Bazel process is spawned, no output base is created, and
    `bazel/cargo/broker-workspace/**`, `bazel/cargo/broker.lock`,
    `packages/d2b-priv-broker/Cargo.toml` and
    `packages/d2b-priv-broker/Cargo.lock` are byte-unchanged. The control is
    the same third fixture under `--hub broker`, which must exit zero and
    quarantine it, so the arms prove the guard refuses rather than the fixture
    being inert.

    The negatives are the remedies this record rejects, run against the same
    planted states, each asserted to leave the state uncleared, and each
    measured at git 2.54.0 before being written down. Against a planted
    `journal`, step 7's pathspec
    `git stash push --all -- bazel/cargo/broker-workspace/` exits 0 reporting
    `No local changes to save` and `journal` is still on disk. Against the
    same state `git stash push --include-untracked` on the transaction
    pathspec exits 0 and captures none of the entries, and `git restore` on
    it exits 1 with `pathspec ... did not match any file(s) known to git` and
    captures none either. The remedy this round withdrew is a negative in its
    own right: against an unrecognized directory holding a regular file, a
    fifo and a subdirectory holding another fifo,
    `git stash push --all -- bazel/cargo/.broker-workspace.txn/` exits 0,
    takes the regular file, and leaves the directory, the subdirectory and
    both fifos on disk, after which `rmdir --` on that directory exits 1 with
    `Directory not empty` and `rm --` on it exits 1 with `Is a directory`, so
    the sequence the earlier round prescribed clears nothing and the refusal
    reproduces; the test asserts that reproduction explicitly. A negative that
    starts passing means either the printed message has drifted onto a remedy
    that works by accident or the substrate's behaviour has changed, and this
    record wants both surfaced rather than absorbed. That no stash is printed
    by any of these three refusals is asserted too, which is what keeps a
    later edit from reintroducing one. All reverted.
15. Two writers cannot race the publication. With one
    `cargo xtask bazel-repin --hub broker` held at an injected pause inside its
    transaction, a second invocation of the repin and an invocation of
    `cargo xtask gen-bazel` each exit nonzero naming
    `bazel/cargo/.broker-workspace.txn/lock` and the writer that holds it,
    within the pause window, and leave the subtree byte-unchanged. Killing the
    first process releases the lock with no manual cleanup, and the next
    invocation proceeds. `cargo xtask gen-bazel --check` run inside the same
    window exits with a verdict about a complete tree, never about a partial
    one.
16. The transaction lock descriptor does not escape the command, and the check
    for it is a descriptor inventory rather than a source assertion. With a run
    held at an injected pause after the Bazel child is spawned,
    `/proc/<child>/fd` and `/proc/<bazel server>/fd` contain no entry whose
    `readlink` target ends in `bazel/cargo/.broker-workspace.txn/lock`, and
    neither does any other descendant of the command. After an ordinary
    successful run, with the Bazel server left running as it always is, a
    second `cargo xtask bazel-repin --hub broker` acquires the lock and exits
    zero rather than reporting a holder. The control is a build with
    `O_CLOEXEC` removed from that one open: the same inventory then names the
    descriptor in the child, and the second run is refused with the lock path
    and `EAGAIN` although no writer is running, which is the evidence that the
    inventory can fail and that the flag is what prevents it. Reverted. The
    lock is opened in exactly one place in the implementation, that call site
    passes `O_CLOEXEC`, and a grep for `dup`, `dup2` and `F_DUPFD` applied to
    that descriptor returns nothing.
17. The digest net fires. On a tree whose subtree and hub lock are both
    current, plant a one-byte change in one stub manifest and run an ordinary
    Bazel build of a `broker` hub target with no repin control anywhere in the
    environment: it fails closed with the substrate's repin-required message
    naming the `broker` hub, and `bazel/cargo/broker.lock` is byte-unchanged.
    Reverted. As a precondition of that check,
    `bazel query 'labels(srcs, //bazel/cargo/broker-workspace:all)'` resolves
    the seven manifest labels the `manifests` attribute names, all within the
    single generated `BUILD.bazel` package.
18. The first-party census is exact, and it is asserted before checks 19 and
    20 evaluate anything. Derived, not hard-coded: the `source`-less entries of
    `packages/d2b-priv-broker/Cargo.lock` are exactly `d2b-contracts`,
    `d2b-core`, `d2b-host`, `d2b-priv-broker`, `d2b-realm-core` and
    `d2b-realm-provider`, six;
    `cargo metadata --manifest-path packages/d2b-priv-broker/Cargo.toml
    --no-deps --offline` reports exactly one `workspace_members` entry,
    `d2b-priv-broker`, and reports that package as carrying one `lib`, one
    `bin` and thirteen `test` Cargo targets. The generator's emitted broker
    target set therefore contains exactly five `-broker` library targets,
    `d2b-contracts-broker`, `d2b-core-broker`, `d2b-host-broker`,
    `d2b-realm-core-broker` and `d2b-realm-provider-broker`, plus the targets
    emitted for `//packages/d2b-priv-broker`, which cover each of that
    package's Cargo targets; and the emitted main set is nonempty, contains the
    five unsuffixed variants, and contains no `-broker` target and no
    `//packages/d2b-priv-broker` target. The check fails on an empty set, on a
    missing member, and on an extra member, reports the difference as a set
    difference rather than as an edge violation, and exits before any subset
    predicate runs. Demonstrated as a refusal, not only as a pass, against
    three planted generator mutations, reverted after each: emitting no
    `-broker` variant at all, which must fail as an empty set rather than
    passing checks 19 and 20 vacuously; dropping
    `d2b-realm-provider-broker`; and emitting a sixth `-broker` variant for a
    package outside the closure. The same three are also confirmed against the
    `bazel query` form of the sets, so a census that holds over the emitted map
    but not over the analysed graph fails too.
19. No `-broker` test target exists, and the check that says so refuses one.
    With check 18 passing over the same emitted set,
    `bazel query 'kind(".*_test rule", <emitted broker target set>)'` names
    only targets of `//packages/d2b-priv-broker`, and the full first-party
    test target set, `bazel query 'kind(".*_test rule", //packages/...)'`, is
    equal before and after this change. No `-broker` label appears in either
    result. This must be demonstrated as a refusal, not only as an empty
    result: with a planted generator mutation that emits one `rust_test` for a
    `-broker` variant, regenerating and re-running the query fails and names
    that target. Reverted after. A check that has never been seen to fail is
    not evidence that the property holds.
20. The two set conditions of invariant 13 hold, over the sets check 18 has
    already censused. The primary evaluation is
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
21. The Bazel-side hub lock's registry `(name, version)` key set equals the
    registry `(name, version)` key set of
    `packages/d2b-priv-broker/Cargo.lock`, checked offline, with 111 entries
    today.
22. `bazel query '@broker//:all'` names only crates present in
    `packages/d2b-priv-broker/Cargo.lock`, is nonempty, and `bazel build` of
    two of them succeeds. Two pieces of this are already measured. On the
    reproduction of
    this repository's workspace shape, the hub's rendered set matched the
    authoritative lock exactly, including the workspace member's own
    dev-dependency and excluding a path-dependency crate's dev-dependency,
    with both authoritative locks unchanged after the run and the lock mirror
    still byte-identical. On the real tree, the six-package stub tree built
    from the resolve satisfies `cargo metadata --locked --offline` against a
    byte-identical copy of the authoritative broker lock. What remains for the
    implementer is running the real hub through Bazel end to end.
23. A planted `build = "build.rs"` key on a closure package, a planted
    directory-name mismatch, and a planted first-party path dependency in
    `packages/d2b-guest-shell-runner/Cargo.toml` each make
    `cargo xtask gen-bazel` exit nonzero with its own named refusal and its own
    remedy; all three are reverted.
24. `tests/unit/meta/adr-index-coverage.sh` passes with this record indexed.

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
- [ADR 0008](0008-supported-platforms-and-rejected-targets.md), whose kernel
  floor of 6.6 already exceeds what `openat2` and `renameat2` require, and
  under which `O_CLOEXEC` and open file description locks have been available
  far longer, so constraints 14 through 19 raise no platform requirement
- `packages/d2b-host/src/hardlink_farm.rs`, which publishes a staged tree with
  `renameat_with(CWD, .., RenameFlags::EXCHANGE)` after fsyncing the tree
  bottom-up and the directory, then removes the retired copy: the same shape
  section 6 uses, already on this repository's trusted path
- `packages/d2b-host/src/bin/d2b-activation-helper.rs` and
  `packages/d2b-host/src/cgroup.rs`, the existing `openat2` plus `ResolveFlags`
  anchored-resolution surfaces, and `packages/xtask/Cargo.toml`, whose
  `rustix` and `nix` entries already carry the anchored-open and `F_OFD_SETLK`
  rationale in comments
- Constraints 14 through 19: a C probe issuing `renameat2`, `openat2`,
  `fsync`, `unlinkat`, `fcntl(F_OFD_SETLK)`, `fcntl(F_GETFD)` and the `dup`
  family directly, plus a `fork`/`exec` arm that inventories a surviving
  child's `/proc/<pid>/fd`, run 2026-08-04 on
  Linux 7.0.10 with the worktree on ext4 and a cross-mount arm on tmpfs; and
  the `git stash push --all -- <pathspec>`,
  `git stash push --include-untracked -- <pathspec>` and `git restore`
  behaviour quoted in section 6 and in the alternatives, measured at git
  2.54.0 on throwaway repositories carrying a modified stub, a deleted stub
  source file, an untracked extra file, an ignored `target/debug/out.bin`
  under the subtree and one unrelated edit outside it, plus `UU`, `AA` and
  `DU` conflict fixtures for the unmerged-path arms
- The quarantine measurements of section 6: a second C probe run 2026-08-04 on
  Linux 7.0.10, unprivileged, with the worktree on ext4, issuing
  `renameat2(RENAME_NOREPLACE)` over a transaction directory holding `lock`,
  `published` and an unrecognized directory carrying a regular file, a fifo, a
  unix socket, a symlink, an empty directory, a name whose bytes are not valid
  UTF-8 and a second level holding another fifo, with `mknodat` for a
  character device returning `EPERM` and that shape therefore left unmeasured;
  plus its `EEXIST` collision arm, its `F_OFD_SETLK` contention arm, an arm
  taking the lock on a freshly created transaction directory while a
  descriptor on the quarantined one is still held, and a git 2.54.0 fixture
  carrying the committed `.broker-*/` rule on which
  `git stash push --all -- bazel/cargo/.broker-workspace.txn/` left an
  unrecognized directory, its subdirectory and both nested fifos on disk,
  `rmdir --` and `rm --` then refused it, `git check-ignore -v` matched all
  four quarantine names, and `git status --porcelain` was byte-identical
  across the quarantine with a modified tracked file, an untracked file, an
  ignored `target/` output and a staged modification present
- The identity-binding measurements of section 6: a third C probe run
  2026-08-04 on Linux 7.0.10, unprivileged, with the worktree on ext4,
  issuing `openat2` under the four resolve flags, `fstat`, `fstatat` with and
  without `AT_SYMLINK_NOFOLLOW`, `fcntl(F_OFD_SETLK)` and `renameat2`
  directly. Its arms: `fstat` on the transaction directory descriptor and
  `fstatat` on the name agreeing on `st_dev`, `st_ino` and directory type
  while the name still denotes the locked directory; that descriptor's
  `fstat` reporting the same identity after the directory has been renamed
  onto a quarantine slot, with `fstatat` on the old name then returning
  `ENOENT` and `fstatat` on the slot returning the witness identity; a
  directory freshly created at the vacated name carrying a different inode
  and a different `lock` file on which `F_OFD_SETLK` with `F_WRLCK` returned
  0 while the moved directory's lock was still held by the same process,
  while `F_OFD_SETLK` on that moved directory's `lock` reached through its
  new name returned `EAGAIN`; a symlink to a directory planted at the name
  reported as a symlink by `fstatat` with `AT_SYMLINK_NOFOLLOW` and as a
  directory without it, refused `ELOOP` by `openat2` under
  `RESOLVE_NO_SYMLINKS`, and moved rather than followed by
  `renameat2(RENAME_NOREPLACE)`, which returned 0; a regular file planted at
  the name reported as a regular file; and the flag surface, where
  `RENAME_NOREPLACE`, `RENAME_EXCHANGE` and `RENAME_WHITEOUT` are the whole
  defined set, an undefined flag bit returned `EINVAL` and
  `RENAME_NOREPLACE | RENAME_EXCHANGE` returned `EINVAL`, so no form of the
  call binds the source to an inode
- `specs/003-adr052-bazel-rust/contracts/workspace-and-tool-pinning.md`, the
  hub and repin contract this record leaves intact
