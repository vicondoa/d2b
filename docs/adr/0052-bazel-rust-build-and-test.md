# ADR 0052: Bazel as the Rust build and test scheduler

- Status: Proposed
- Date: 2026-08-02
- Related: [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md) (Rust
  toolchain, MSRV, and supply-chain policy), which keeps its authority
  unchanged and is not superseded;
  [ADR 0000](0000-repository-layout-and-rust-bootstrap.md) (repository layout),
  which this decision extends with Bazel workspace files at the repository
  root; [ADR 0017](0017-no-bash-fallbacks-invariant.md) (no-bash fallbacks),
  whose AST scan is one of the surfaces migrated here;
  [ADR 0008](0008-supported-platforms-and-rejected-targets.md) (supported
  platforms), which bounds the host platform this graph builds for
- Scope: the eight Rust leaves behind the required `test-rust` context, the
  eighteen execution-manifest surfaces they publish, the new local Make entry
  points in section 8, and one new side-by-side GitHub Actions workflow. Rust
  only.
- Non-scope: Bazel-to-Nix packaging, `nix` package overrides, flake and
  `nix-unit` check migration, VM and image work, release-artifact migration,
  static guest binaries, and cross-compilation. Section 1 lists these as
  explicit non-decisions.

## Context

The Rust gate is the most expensive part of pull-request validation and the
part with the most duplicated work. `make test-rust` is a GNU Make DAG over
nine leaves; `tests/test-rust.sh` owns one leaf mode each; and
`tests/layer1-jobs.json` dispatches eight of them as independent GitHub jobs
behind the stable required `test-rust` rollup context. Each leaf receives the
full runner budget, and several of them compile overlapping dependency graphs
into separate Cargo target directories on purpose, because Cargo has no way to
share one build across concurrent invocations with different feature sets and
target directories.

That duplication is visible in the numbers. On run `30778148252` (a warm push
to `v3`), the eight Rust leaves took 4m24s (API census), 6m50s (main
workspace), 2m39s (schema), 7m46s (inventory), 1m21s (no-bash AST), 1m50s
(guest shell runner), 4m22s (broker) and 1m28s (supply chain): about 30m40s of
runner time to produce a 7m46s critical path across eight runners. The
workflow's own comment records the cold cost of a Rust-profile job as about 43
minutes. The gate is fast because it is wide, not because it is efficient.

Bazel is the standard answer to that shape: one analysis graph, one compile of
each unique `(crate, feature set, toolchain)` tuple, per-target failure
reporting, and a content-addressed action cache that is reused across leaves
instead of being partitioned by target directory.

### What the broader handoff proposes, and why this decision is narrower

A detailed migration handoff exists that covers Rust and Nix together: Bazel as
the outer graph, `rules_pkg` archives handed to a custom `nix_flake_build`
rule, NixOS package overrides consuming Bazel outputs, the dynamic flake matrix
collapsed into Bazel-scheduled Nix actions, and the release workflow rebuilt on
Bazel targets. That is a coherent end state and this decision does not argue
against it.

It is also four independent risk surfaces stapled together. The Rust surface
risks a coverage regression; the Nix surface risks a reproducibility regression
in derivations that build VM images; the packaging surface risks an ELF and
closure regression in privileged host binaries; the release surface risks
shipping a differently-linked artifact to consumers. Sequencing them as one
decision means the first serious problem in any of them blocks all of them, and
it means the acceptance evidence for "did we lose test coverage" gets mixed
with the acceptance evidence for "does the broker still have the right
`DT_NEEDED` entries".

This decision therefore takes the smallest slice that is independently
valuable and independently reversible: Bazel becomes the build and test
scheduler for the Rust gate, and nothing else changes. If the Rust slice
succeeds, the later decisions inherit a working Bazel workspace, a pinned tool
stack, and a demonstrated cache policy. If it fails, reverting is deleting
files that nothing else depends on.

### Measured constraints that shape the decision

These were measured on 2026-08-02 rather than taken from documentation,
because several of them contradict the guidance that would otherwise have been
followed.

1. **The repository Actions cache has almost no headroom.** The GitHub cache
   API reports 9,059,410,960 bytes (8.44 GiB) across 6 entries against a 10 GB
   repository limit: three `Swatinem/rust-cache` entries at about 2.17 GiB
   each, two Nix fixture entries at about 0.94 GiB each, and the 33 MiB
   realized-check entry. Free headroom is roughly 0.87 GiB. Any new
   multi-gigabyte cache entry evicts something the required path depends on.
2. **Pull requests currently do publish cache entries.** One of the three
   rust-cache entries sits on `refs/pull/368/merge`. GitHub scopes those to the
   pull-request ref, so they cannot overwrite a base-branch entry, but they do
   consume the shared budget. That is a large part of why the budget is nearly
   full.
3. **The pinned nixpkgs already provides Bazel.** `bazel` and `bazel_7` resolve
   to 7.6.0, `bazel_8` to 8.6.0, `bazel_9` to 9.1.0, `bazelisk` to 1.28.1, and
   `bazel-buildtools` to 8.5.1. No new fetch mechanism is needed to get a
   pinned Bazel.
4. **Custom local resources do not serialize tests.** With Bazel 8.6.0, two
   four-second `sh_test` targets tagged `resources:d2b_broker_slot:1`, run
   under `--local_resources=d2b_broker_slot=1 --local_test_jobs=4`, started
   within one millisecond of each other and overlapped completely. A target
   requesting four units of a resource with capacity one also ran and passed
   rather than failing analysis. The mechanism the handoff-style guidance
   implies for bounded serialization is not a mechanism.
5. **`tags = ["exclusive"]` does serialize.** The same two targets tagged
   `exclusive` ran strictly one at a time and strictly after the two untagged
   targets, which had overlapped. This is the mechanism that works.
6. **Bazel 8.6.0 has the flags this decision relies on.** `--lockfile_mode`
   exists with `error` among its values and defaults to `update`;
   `--disk_cache`, `--repository_cache`, `--local_test_jobs` and
   `--local_resources` exist; `--experimental_disk_cache_gc_max_size`,
   `--experimental_disk_cache_gc_max_age` and
   `--experimental_disk_cache_gc_idle_delay` exist, and the idle delay defaults
   to five minutes, which means garbage collection never runs during a short
   continuous-integration job unless it is set to zero.
7. **The workspace is pure first-party Rust with heavy repository coupling.**
   56 workspace members, 205 integration test files, 912 tracked `.rs` files
   under `packages/`, and locks resolving 544, 117 and 161 packages for the
   main, broker and guest-shell-runner workspaces. There are no first-party
   build scripts, no first-party proc-macro crates, and no `links =`
   declarations; there is exactly one git dependency (`wl-proxy`, pinned by
   rev) and six `harness = false` targets across two crates. But 20 test files
   resolve `CARGO_MANIFEST_DIR`, 11 of them define a `repo_root()` helper that
   walks out into the working tree, and 25 files locate binaries through
   `env!("CARGO_BIN_EXE_...")`.
8. **Every workflow must call an approved Make target.**
   `packages/xtask/tests/policy_ci.rs` holds `APPROVED_MAKE_TARGETS` as a
   closed list and `ALLOWLISTED_WORKFLOWS` as a three-entry allowlist asserted
   verbatim. A new workflow that runs `bazel` directly fails that policy test.
9. **The reference development host.** 12 physical cores (Intel i9-10920X),
   62 GiB RAM, local NVMe. Hosted runners for every Rust job in
   `pr-l1-static-fast.yml` are `ubuntu-latest`, 4 vCPU and 16 GiB.

Constraint 1 alone rules out the handoff's 4 GiB rolling cache as a starting
point. Constraints 4 and 5 rule out its implied scheduling primitive.
Constraint 8 rules out its workflow skeleton. Those are recorded here so a
future reader knows the divergences below are measurements, not preferences.

### Drift noted, not corrected

ADR 0009 section 5 names `tests/static.sh` as the script that runs the Rust
gate and section 6 names `checks.<system>.rust-{build,tests,clippy,deny,audit}`
as the only exposure of Rust packages. Committed code disagrees: the gate is
`tests/test-rust.sh` behind a Make DAG, and the flake exposes a different check
set. Committed code is canon; this ADR records the drift and does not re-align
either side. ADR 0009's live authority is the toolchain pin, the MSRV policy,
the `unsafe_code = "forbid"` workspace lint, and the requirement that
cargo-deny and cargo-audit results gate the Rust surface with no waiver. All of
that survives this decision unchanged. ADR 0009 also names 1.94.1 as the pinned
channel; `packages/rust-toolchain.toml` now says 1.97.0. Same treatment.

A third drift bears directly on this decision. The
`rust-schema-reproducibility` leaf in `tests/test-rust.sh` snapshots
`$ROOT/packages/xtask/out` before and after two `cargo xtask gen-schemas`
runs, but `gen_schemas` writes to `<repo root>/docs/reference/schemas/v2` and
nothing in the tree writes `packages/xtask/out`, which does not exist. Its
`snapshot_schema_out` helper returns empty when the directory is absent, so
the gate compares two empty strings and cannot fail. Committed code is canon
and this ADR does not correct it. It is recorded because the Bazel carrier for
that surface must not inherit the same shape, which is why section 6 requires
an exact, nonempty census before any digest comparison.

No entry in `docs/contributing/critical-subsystems.md` changes. That file lists
runtime subsystems; build orchestration is not among them.

## Decision

Bazel becomes the authoritative producer and scheduler for the Rust
compilation and Rust tests covered by the existing required `test-rust`
context, through a staged transition in which the current Cargo path stays
authoritative until named, mechanically checkable acceptance criteria hold.

### 1. Scope, and the decisions this ADR does not make

In scope: compiling every first-party Rust crate the Rust gate compiles today,
running every test the Rust gate runs today, and running the non-compilation
policy checks the Rust gate runs today, all as Bazel targets; plus the local
entry point and the shadow workflow that exercise them.

Explicitly not decided here, and not to be inferred from this ADR:

- Bazel-to-Nix packaging of any kind, including `rules_pkg` archives, a
  `nix_flake_build` rule, or importing Bazel outputs into the Nix store.
- Nix package overrides, `d2b.packageOverrides`, or any change to how NixOS
  modules select `d2b` packages.
- Migrating `nix flake check`, the dynamic x86 flake matrix, `test-nix-unit`,
  `test-flake-aarch64`, or any Nix evaluation into Bazel.
- VM images, `runNixOSTest`, fixture materialization, or the
  `test-fixture-contracts` lane.
- The release workflow, `release-host-binaries.yml`, or prebuilt artifacts.
- Static guest binaries, musl or `pkgsStatic` sysroots, and cross-compilation
  for `aarch64-linux`.
- Remote execution and any remote cache. See section 10.
- `test-proofs`, `test-policy`, `test-runtime-ledger`, `test-drift`,
  `test-lint`, and every other Layer-1 job outside the `test-rust` rollup.
  These continue to run Cargo directly and are untouched.

A later ADR may take any of these. This one must not be cited as having
settled them.

### 2. Cargo stays the authoritative dependency and toolchain input

`packages/Cargo.toml`, the three `Cargo.lock` files, `packages/deny.toml` and
the two `rust-toolchain.toml` files remain the single source of truth for
dependency resolution, feature selection and compiler version. Bazel consumes
them; it never becomes the place a dependency is declared.

Concretely:

- Third-party crates enter the graph through `crate_universe`
  (`crates_repository`) reading the committed `Cargo.lock` files, in
  non-vendored mode with a committed Bazel-side lock per Cargo workspace. A
  repin drift check (section 5) fails closed when the Bazel-side lock does not
  match what the Cargo lock resolves to, which is the Bazel equivalent of the
  `--locked` flag the gate passes today.
- The Rust toolchains registered in Bazel are exactly the channels named in
  `packages/rust-toolchain.toml` (1.97.0) and
  `packages/d2b-api-surface/rust-toolchain.toml` (nightly-2026-02-16). A guard
  asserts equality between the registered versions and the two committed pins,
  so a toolchain bump cannot land in one place only.
- Changing a dependency or a toolchain is still a Cargo-file edit followed by a
  regeneration, never a hand edit of Bazel files.

Moving dependency or toolchain authority into Bazel would require a follow-up
ADR justified by measured prototypes, and this ADR authorizes no such move.

### 3. Pinned tools, and no unpinned resolution

- The Bazel binary is `bazel_8` (8.6.0) from the repository's pinned nixpkgs,
  reached through the dev shell or `nix shell --inputs-from .`. `.bazelversion`
  records `8.6.0` for tooling that reads it, and the Make wrapper fails closed
  when `bazel --version` disagrees with `.bazelversion`.
- Bazelisk is not used to fetch a Bazel binary. ADR 0009 established that the
  gate's tools come from the pinned flake input rather than an ad hoc fetch,
  and a downloader that silently swaps the build system's own version is
  exactly the drift that policy exists to prevent. `bazelisk` remains available
  in nixpkgs for local experimentation; it is not on the gate path.
- `MODULE.bazel` and `MODULE.bazel.lock` are committed, and `.bazelrc` sets
  `common --lockfile_mode=error`. The default is `update`, which silently
  rewrites the lock on a resolution change; `error` makes an unpinned or
  drifted module resolution a failure with a named remediation instead.
- `rules_rust` is pinned to a single explicit version in `MODULE.bazel` (0.73.0
  is the newest release on the Bazel Central Registry as of this date; the
  implementer pins whatever is newest and compatible with Bazel 8.6.0 at
  implementation time and records the version in the wave notes). Version bumps
  are ordinary reviewed changes, not floating constraints.

### 4. First-party BUILD files are generated by a repository-owned generator

`cargo xtask gen-bazel` reads `cargo metadata` for the three Cargo workspaces
and emits the first-party `BUILD.bazel` files, plus the tracked
governed-source manifest that section 6's no-bash scan consumes.
`cargo xtask gen-bazel --check` regenerates into a scratch tree and fails on
any difference; it is wired into `test-drift`, which already exists to catch
exactly this class of staleness for the other `xtask gen-*` outputs.

`gazelle_rust` is rejected. It would add a Go toolchain and a third-party
generator to the trusted build path for a workspace whose interesting cases are
precisely the ones a generic generator handles worst: three feature variants of
one privileged workspace, two standalone workspaces with their own locks, six
`harness = false` targets, `compile_fail` doctests that are capability seals
and carry their own rustc flags, and a nightly-rendered API census. The
repository already has a codegen-plus-drift-gate idiom and a place to put it.
Using it costs one `xtask` subcommand and removes a dependency.

Hand-written Bazel fragments are permitted for the cases the generator does not
model, but each one must be listed in the coverage map (section 5), so an
unmodelled case is visible rather than merely absent.

### 5. The coverage map is a committed artifact with a fail-closed guard

`docs/reference/test-execution-manifest.md` pins the baseline set of Rust
sub-surface identifiers that a passing Rust aggregate must publish. Under
`D2B_SKIP_FIXTURE_BUILD=1`, which is what both the Layer-1 graph and
continuous integration use, that set is eighteen identifiers. Those eighteen
are the coverage contract this decision must preserve exactly.

`tests/golden/bazel-rust-coverage.json` maps each identifier to the Bazel
target or test suite that carries it, to the continuous-integration slice that
runs it, to the exact census that surface must observe, and, for a test suite,
to its declared process topology (section 7). Census is exact, not a floor:
every surface in this table has a derivable manifest, so a pinned minimum
count survives only as a fallback for a surface added later that provably has
none, and adding such a surface requires recording the absence of a manifest
in the map. The map is the normative one-to-one statement:

| Surface identifier | Today | Bazel carrier | Slice |
| --- | --- | --- | --- |
| `rust-api-surface` | `tests/tools/api-surface-json.sh`, nightly rustdoc JSON plus snapshot compare | `//ci/rust:api_census` | `api` |
| `rust-main-format` | `cargo fmt --all --check` | `//ci/rust:fmt` | `main` |
| `rust-main-clippy` | `cargo clippy --locked --workspace --all-targets -- -D warnings` | `//ci/rust:clippy` | `main` |
| `rust-main-workspace-tests` | `cargo nextest run --workspace --exclude d2b-contract-tests`, plus `cargo test --doc`, plus one `cargo test --test` per `harness = false` target | `//ci/rust:main_tests`, `//ci/rust:main_doctests`, `//ci/rust:main_harness_free` | `main` |
| `rust-no-bash-ast` | `no-bash-ast-walker` over `packages/` | `//ci/rust:no_bash_ast` | `main` |
| `rust-schema-reproducibility` | `cargo xtask gen-schemas` twice, digests compared | `//ci/rust:schema_reproducibility` | `main` |
| `rust-stub-no-socket` | `tests/tools/stub-no-socket.sh` | `//ci/rust:stub_no_socket` | `main` |
| `rust-assert-pinned` | `tests/tools/assert-pinned-tests.sh` | `//ci/rust:pinned_test_inventory` | `main` |
| `rust-broker-default` | broker workspace, default features | `//ci/rust:broker_default` | `broker` |
| `rust-broker-layer1` | broker workspace, `layer1-bootstrap` | `//ci/rust:broker_layer1` | `broker` |
| `rust-broker-fakebackends` | broker workspace, `fake-backends` | `//ci/rust:broker_fakebackends` | `broker` |
| `rust-guest-shell-runner` | standalone workspace, `real-libshpool`, fmt plus clippy plus tests plus companions | `//ci/rust:guest_shell_runner` | `aux` |
| `rust-deny-main` | `cargo deny check` (main) | `//ci/rust:deny_main` | `aux` |
| `rust-deny-broker` | `cargo deny check` (broker) | `//ci/rust:deny_broker` | `aux` |
| `rust-deny-guest` | `cargo deny check` (guest shell runner) | `//ci/rust:deny_guest` | `aux` |
| `rust-audit-main` | `cargo audit` (main lock, two ignores) | `//ci/rust:audit_main` | `aux` |
| `rust-audit-broker` | `cargo audit` (broker lock) | `//ci/rust:audit_broker` | `aux` |
| `rust-audit-guest` | `cargo audit` (guest lock, one ignore) | `//ci/rust:audit_guest` | `aux` |

`rust-contract-tests` and `rust-cli-contract-tests` are the two conditional
surfaces the aggregate publishes only when the fixture build is enabled. They
depend on evaluated Nix fixtures and therefore belong to the Nix bridge this
ADR defers. They stay on the current Cargo and Nix path, exactly as the
enforcing `test-fixture-contracts` lane already runs them. After promotion
(section 12), `make test-rust` is the Bazel path for the eighteen surfaces plus
the unchanged Cargo fixture leaf for those two, so the local target's current
behaviour of including fixture and CLI surfaces when Nix is available is
preserved.

`//ci/rust:coverage_map_guard` is a Bazel test that fails closed when any of
the following holds: an identifier in the baseline set has no mapping; a mapped
label does not exist in `bazel query //ci/rust/...`; a Rust test target exists
in the graph that no mapped suite transitively includes; a mapped test suite
declares no process topology or no exact census; or a hand-written Bazel
fragment exists that the map does not list. The guard's own baseline set is
read from the committed reference document, not duplicated, so the two cannot
drift apart.

### 6. Non-compilation policy checks stay real, named, enforcing targets

Nine of the eighteen surfaces are not `rules_rust` tests in any natural sense.
Pretending otherwise is how coverage disappears during a migration, so each one
gets a named representation and a named hazard.

**cargo-deny (three surfaces).** A Bazel test per workspace that runs the
nixpkgs-pinned `cargo-deny` against declared inputs: that workspace's
`Cargo.toml` set, its `Cargo.lock`, its `deny.toml`, and a materialized
vendored Cargo source tree.

Two measured facts rule out the obvious construction. First, `crate_universe`
produces Bazel repositories holding generated `BUILD` files; it is not a Cargo
registry index, and a `CARGO_HOME` populated from it does not let
`cargo metadata` resolve, so "point `CARGO_HOME` at the `crate_universe`
fetch" is not a hermeticity mechanism. Second, `cargo-deny`'s `[advisories]`
check reaches the network: on `cargo-deny 0.19.7`, the help text for
`--disable-fetch` states that when the `advisories` check runs, the configured
advisory database "will be fetched and opened". No Bazel action in this
decision is authorized to use the network, and this ADR authorizes none, so an
advisories check inside `cargo-deny` cannot run here at all.

The repository already solved this offline, in committed Nix. `flake.nix`'s
`rust-deny` check materializes a vendored source replacement per lock through
`rustPlatform.importCargoLock`, writes a `.cargo/config.toml` setting
`[source.crates-io] replace-with = "vendored-sources"` plus the pinned
`wl-proxy` git source replacement, overwrites every repository-local
`.cargo/config.toml` so the sccache wrapper cannot activate, and then runs
`cargo-deny ... check --config <deny.toml> bans licenses sources`. Its
advisories are carried separately by `rust-audit`, offline, against a pinned
RustSec snapshot with `--no-fetch`.

The Bazel targets take that same shape. A repository-owned rule materializes a
`cargo vendor`-shaped tree, extracted crate sources plus their
`.cargo-checksum.json`, for each of the three locks, from the crate archives
the pinned `crate_universe` fetch already brings into the repository cache.
The action declares that tree and a generated `.cargo/config.toml` carrying
the same source replacements, sets `CARGO_NET_OFFLINE=1`, and runs
`cargo-deny check --config <deny.toml> bans licenses sources`. `cargo-deny`
also accepts `--metadata-path`, measured to exist on 0.19.7, but supplying
metadata alone does not remove the need for the vendored tree, because the
`licenses` check harvests license text from crate sources. An implementer who
believes otherwise must record the measurement that shows it rather than
assume it.

This is an intentional decomposition of the executor, not a reduction of
policy. The aggregate supply-chain policy ADR 0009 requires is the union of
the two tools' enforcing outcomes: `bans`, `licenses` and `sources` from
`cargo-deny`, and `advisories` from `cargo-audit` against the pinned database.
The live ignore semantics travel with the advisories check and are already
expressed as `cargo-audit --ignore` flags in the current gate, matching the
`[advisories] ignore` entries in the committed `deny.toml` files:
`RUSTSEC-2026-0194` and `RUSTSEC-2026-0195` on the main workspace, none on the
broker, and `RUSTSEC-2024-0384` on the guest shell runner. Those `deny.toml`
entries become inert on the Bazel path, exactly as they already are on the
flake path, and stay committed because the Cargo path still reads them until
retirement. No waiver is created and no advisory becomes unenforced.

What could move silently is an enforcing outcome `cargo-deny`'s `advisories`
check produces that `cargo-audit` does not, yanked-crate detection being the
obvious candidate, because it needs a registry index that neither the vendored
tree nor the pinned advisory snapshot provides. Guard: before promotion the
implementer records in the wave notes the exit status and the finding set of
today's `cargo deny check` and of the decomposed pair, over all three locks.
Promotion is blocked when any enforcing outcome differs. If one does differ,
the remedy is to carry it explicitly, not to drop it.

The gate today falls back to `nix shell` when `cargo-deny` is absent and fails
closed when neither is available, citing ADR 0009's no-waiver rule; the Bazel
form has no fallback at all, because the tool is a declared input.

**cargo-audit (three surfaces).** Same shape, plus the advisory database
becomes a declared, pinned input rather than a network fetch, and, per the
decomposition above, `cargo-audit` becomes the sole carrier of the advisories
policy on the Bazel path. The flake already pins a RustSec advisory database
snapshot by rev and hash for its own `rust-audit` check; the Bazel targets
consume a pin of the same shape. This
removes the three-attempt retry loop that exists today only because the check
reaches the network, and it makes the result cacheable and reproducible. The
`--ignore` lists move with the checks verbatim, including the comments
explaining why each advisory is ignored and what unblocks removing it. Because
the database is pinned, staleness is now a visible property of a committed pin
rather than an invisible property of when the job ran; keeping the pin fresh is
an ordinary reviewed change and is not gated on a clock, because a
time-dependent gate is a nondeterministic gate.

**Schema reproducibility.** This surface cannot be migrated as it stands, for
a reason in committed code: `gen_schemas` computes its output directory as
`repo_root()/docs/reference/schemas/<SCHEMA_VERSION>`, where `repo_root()` is
derived from `env!("CARGO_MANIFEST_DIR")` at compile time. It accepts no
output argument, so under Bazel it would write into the source tree rather
than into a declared output tree.

Prerequisite, landed on the Cargo path before any Bazel work on this surface:
`cargo xtask gen-schemas --out-dir <path>` gains an explicit output directory
that defaults to today's path when the flag is absent, so `test-drift` and the
current gate are unchanged. The argument-validation idiom to copy is
`validate_output_path` and `output_repo_relative_path` in
`packages/xtask/src/inventory.rs`, minus its refusal to write under tracked
`docs/`, which does not apply to a generator whose default output is `docs/`.
The same change makes `gen-schemas` emit the manifest of paths it wrote;
`write_schemas` already returns them.

The Bazel carrier is **one** test action that performs two sequential
generations into two distinct directories inside its own test temporary
directory, then compares. Two separate Bazel actions are rejected: actions are
served from the action cache by action key, so a second action that is
identical in tool, inputs and command line is a cache replay of the first, and
the property under test, that two independent invocations agree, would never
be exercised. An implementer who still wants two actions must make them
provably distinct, meaning distinct declared outputs plus a recorded input
discriminator, and must record the discriminator in the coverage map. The
single-action form is the default and needs no such argument.

Census before comparison, because two empty trees have identical digests and
would otherwise pass. Before any digest comparison the test asserts that each
generation produced exactly the committed expected census: the twenty file
names `gen_schemas` declares today, under the `SCHEMA_VERSION` subdirectory,
each nonzero in length and each parsing as JSON. A set difference between a
generation and the census, or between the two generations, is reported as a
set difference and not as a digest mismatch, because the two failures have
different remedies. The census is committed once in the coverage map and is
compared against both the emitted manifest and the on-disk tree, so a schema
added without updating the census fails closed rather than silently widening
the surface.

**The no-bash AST scan.** A `rust_binary` for the walker and a test that runs
it over a declared source group covering `packages/`. The hazard is that a
sandboxed scan sees only declared inputs, so a crate missing from the source
group is silently unscanned, and a minimum file count is too weak a guard: it
proves the scan was not empty, not that it was complete. The walker also
`continue`s past any file it cannot read or cannot parse with `syn`, so a
declared-but-unparsed file is invisible to a count of declared inputs.

The contract is therefore an exact manifest, in four parts.

1. `cargo xtask gen-bazel` emits a tracked governed-source manifest as a
   generated Bazel file, derived from the repository inventory rather than
   from a glob. The governed set is the `git ls-files` inventory restricted to
   the scanner's own rules: paths under `packages/`, suffix `.rs`, excluding
   any path component named `target`, `tests`, `fixtures` or `.git`. Measured
   on this tree: 912 tracked `.rs` files under `packages/`, of which 641 are
   governed under those rules.
2. The Bazel source group the scan consumes is that generated list,
   byte-for-byte. It is never a `glob()`, because a glob resolves against
   whatever happens to be on disk and cannot disagree with itself.
3. Drift guard: `gen-bazel --check` in `test-drift` fails when the manifest
   does not match the inventory, which is the existing idiom for every other
   `xtask gen-*` output.
4. Meta guard: a Bazel test compares the manifest against the inputs actually
   present in the scan's runfiles tree and fails closed on any tracked
   governed file absent from the declared inputs, and on any declared input
   absent from the manifest. Set equality in both directions, with the
   offending paths named.

The walker additionally reports the number of files it parsed, and the test
requires that number to equal the manifest size. That closes the silent
read-or-parse skip, which neither the drift guard nor the meta guard can see.

The planted control is kept and stays useful: one fixture input carrying a
`Command::new("bash")` site that the scan must detect, run as its own negative
target so a control input can never enter the real scan's declared inputs or
its verdict.

**The stub-no-socket check.** A test that executes the built CLI and daemon
binaries as declared runfiles and asserts they exit cleanly and leave no
runtime state.

**The pinned test inventory.** Each `rust_test` binary in the main and broker
suites is run with the libtest `--list` interface as a Bazel action, the same
listing the section 7 runner uses to build its per-binary census; the union of
those listings is the census the committed pins under `tests/golden/pinned/`
are compared against. `harness = false` targets have no listing interface and
are covered by target-level census instead, which is the
same distinction the current gate makes when it derives the harness-free set
from `nextest list`. The test fails closed when any pinned name is absent and
when any suite's listing is empty.

**The API census.** The nightly rustdoc JSON render becomes a Bazel action
using the registered nightly toolchain, with the rendered JSON as declared
outputs and a diff test against `tests/golden/api-surface`. A guard asserts the
toolchain the action used equals the pin in
`packages/d2b-api-surface/rust-toolchain.toml`, which is the property the
current script asserts by reading the pin and refusing to proceed on drift.

### 7. Failure reporting, test process topology, and the broker

Every logical check is its own Bazel target. No aggregate shell script wraps
several checks into one pass or fail, because per-surface attribution is the
property that makes the current gate diagnosable and it is the property a
naive migration destroys first.

The Make wrapper maps Bazel's build event protocol test results back onto the
eighteen surface identifiers, so `D2B_EXECUTION_MANIFEST` keeps publishing the
same versioned evidence with the same identifiers, the same `completed_leaves`
and `failed_surfaces` semantics, and the same partial evidence on failure and
interruption. The execution-manifest contract in
`docs/reference/test-execution-manifest.md` is unchanged by this decision; only
the executor beneath it changes.

The three privileged-broker suites carry `tags = ["exclusive"]`. Committed code
records that broker tests manipulate process-global signal and reap state, that
they are not process-per-test safe, and that the three feature passes are kept
serial deliberately. Bazel's default is to run test targets concurrently, which
would be a new and unreviewed exposure of that state. The measurement in the
context section is why the tag is `exclusive` and not a custom local resource:
custom resources did not serialize anything. The cost is bounded, because the
three passes run 528 tests in about 1.4 seconds each, so the serial barrier
costs seconds. Removing `exclusive` later requires the same dedicated isolation
review the existing comment demands.

**The baseline process topology.** It is tempting to describe the current gate
as "one test binary at a time with intra-binary threads". That is wrong, and
building on it would silently weaken two suites. Committed code runs three
topologies and the Bazel path must preserve each of them:

| Suite | Today | Topology |
| --- | --- | --- |
| Main workspace | `cargo nextest run --workspace` | one fresh process per test case |
| Guest shell runner | `cargo nextest run --features real-libshpool` | one fresh process per test case |
| Broker, three feature passes | `cargo test -- --test-threads N` | one process per binary, bounded threads inside |

The main and standalone workspaces are process-per-test under cargo-nextest,
and `tests/test-rust.sh` says so directly. The broker deliberately is not:
committed code records that under nextest `runtime::tests::usbip_bind_*` fails
with `LiveHandler("USB device 1-2.3 is missing required sysfs attr devpath")`
because whatever keeps handler selection off live sysfs does not survive being
run in its own process, and that the same case passes under `cargo test`. That
is a recorded harness-environment dependency in a critical, privileged
subsystem, not a defect to be fixed in passing.

Plain `rules_rust` `rust_test` execution, which runs a libtest binary once
with intra-process threads, is therefore **not** acceptable for the main
workspace or the guest shell runner. It would silently convert two
process-per-test suites into one shared address space per binary, which is a
weaker isolation guarantee than the one committed code buys today, and no
guard in this ADR would notice.

**The runner.** Bazel-built test binaries are executed by a repository-owned
Rust test runner, a `rust_binary` in this repository. It is not a shell script
and invokes no shell, which ADR 0017 requires anyway. It:

- enumerates cases through the libtest `--list` interface and fails closed on
  an empty listing or on any difference from the exact per-binary census
  pinned in the coverage map, so a case that disappears is a failure rather
  than a silent shrink;
- honours the per-suite topology the coverage map declares: one fresh process
  per case, invoked with the exact case name and `--exact`, for the main
  workspace and the guest shell runner; one process per binary with a bounded
  `--test-threads` for the three broker feature passes;
- carries `#[ignore]` semantics faithfully: ignored cases are enumerated,
  reported as ignored, and never counted as passed, so the ignored count is
  itself part of the census;
- bounds concurrency from the same `D2B_RUST_BUDGET`-derived value the rest of
  the gate uses (section 8), and never lets per-target concurrency multiply
  `--local_test_jobs` into an unbounded process count;
- reports per-case results against the surface identifier the coverage map
  names, so a single failing case names itself rather than naming a suite.

Doctests and `harness = false` companions keep their own targets and are not
routed through the case runner: doctests expose no such listing interface, and
`harness = false` binaries expose none at all. Their discovery stays derived
from workspace metadata rather than hardcoded, and the main workspace's
refusal to report a passing companion surface on an empty `harness = false`
discovery is preserved as a Bazel-side assertion.

An equivalent mechanism may replace the repository-owned runner only if it is
measured to deliver the same four properties: exact census, per-case process
freshness where declared, faithful ignored-case reporting, and bounded
concurrency with no shell in the execution path.

**Promotion is blocked** until a prototype, recorded in the wave notes, proves
this topology for the main workspace, all three broker feature passes, and the
guest shell runner: census equal to the current `cargo nextest list` and
`cargo test -- --list` output for each suite, equal ignored-case counts, no
shell in the execution path, and the measured wall clock per suite under the
runner against the current gate. Converting the broker to process-per-test is
not part of this decision and stays gated on the isolation review the
committed comment demands.

### 8. `make test-bazel-rust` lands beside `make test-rust`

`make test-rust` is unchanged and stays authoritative. `make test-bazel-rust`
is added as a peer, plus four slice targets `make test-bazel-rust-main`,
`-api`, `-broker` and `-aux`, plus `make bazel-shutdown`, the dedicated
server-shutdown target that section 11's stuck-server remedy names. All six
are added to `APPROVED_MAKE_TARGETS` in `packages/xtask/tests/policy_ci.rs` in
the same change, because that list is a closed set and a workflow calling an
unlisted target fails the policy test. `make bazel-shutdown` issues
`bazel shutdown` with the same startup options every other target uses and
does nothing else; it deletes nothing, which is what makes it safe to run
while a cleanup refusal or a stuck server is still unresolved.

Locally, `make test-bazel-rust` is one Bazel invocation over the whole Rust
suite. There is one machine and one cache; splitting it would only defeat the
scheduler.

Concurrency is derived from the existing `D2B_RUST_BUDGET` computation rather
than a new control. That computation already takes the smaller of logical CPUs
and a memory-derived cap, reads `MemAvailable` and the effective cgroup v2
allowance, reserves 2 GiB for the host, and fails closed to a budget of one
when cgroup state is unreadable. Its output sets `--jobs` and
`--local_test_jobs`. Introducing a second, differently-shaped local budget
control for the same machine would be a regression in a surface the
contributor documentation already teaches.

**Local persistent output is bounded, and reclaiming it is proved safe.**
Three trees persist between local runs, all inside the worktree's gitignored
`.scratch/`: the Bazel output user root, the disk (action) cache, and the
repository/download cache. Each is bounded and each is reclaimable.

- Budgets. The disk cache runs with
  `--experimental_disk_cache_gc_max_size=8G`,
  `--experimental_disk_cache_gc_max_age=14d` and
  `--experimental_disk_cache_gc_idle_delay=0s`, the last because the flag
  defaults to five minutes and collection would otherwise never run. Bazel
  ships no collector for the repository cache, so the Make wrapper bounds it
  at 2 GiB itself; over budget, the wrapper removes it as a whole through the
  anchored deletion contract below and prints the refetch cost, because it
  holds only re-fetchable downloads.
- High-water checks. Before each invocation the wrapper measures the output
  user root with `du -s -B1`. Below the soft mark it is silent. At or above
  the soft mark it prints the measured size and the exact reclaim command. At
  or above the hard mark it refuses to start a build and names the reclaim
  command, because the failure this prevents is filling the disk mid-link on a
  machine that is also running a heavy lane. Both marks are documented Make
  variables with defaults of 20 GiB and 40 GiB.
- Cleanup safety. Before any targeted deletion of an output base or output
  user root, the wrapper first runs `bazel shutdown` **with the same startup
  options**, because startup options select the server: a `shutdown` issued
  with different startup options starts a second server against a different
  output base and leaves the live one running, which is precisely how a
  cleanup ends up deleting a tree a running server still owns. The deletion
  that follows is **not** a `realpath` proof followed by a recursive remove
  through the same string. Resolving a path and then deleting through that
  path is a time-of-check-to-time-of-use window, and a check that a later
  resolution can invalidate is not a check. Deletion is instead performed by
  repository-owned cleanup plumbing that resolves once and never returns to
  string path resolution afterwards. It opens the worktree scratch anchor
  `<worktree>/.scratch/` exactly once, with `O_PATH|O_DIRECTORY|O_CLOEXEC`,
  and holds that descriptor for the whole operation; it resolves the Bazel
  subtree `.scratch/bazel/` beneath that anchor descriptor-relative, refusing
  any symlink or magic link at any component and refusing any escape above the
  anchor (`openat2` with
  `RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS` and
  `O_PATH|O_DIRECTORY|O_CLOEXEC` in the `open_how` flags, or the equivalent
  component-by-component `openat` fd-walk with
  `O_PATH|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC` and a per-descriptor type check
  where that syscall is unavailable); and it removes entries with `unlinkat`
  relative to the directory descriptor that enumerated them, never by
  reconstructing a path string. `O_CLOEXEC` is explicit on every descriptor
  the cleanup path opens, on the `openat2` route exactly as on the fallback:
  the anchor, every traversal descriptor, and the
  `O_DIRECTORY|O_RDONLY|O_CLOEXEC` descriptor that each directory is reopened
  as for enumeration, which an `O_PATH` descriptor cannot serve. No cleanup
  descriptor is inherited across an exec, so nothing this path spawns - the
  `git ls-files` probe behind the tracked-file refusal, or anything else -
  ever holds the anchor or a directory beneath it open. This is the idiom
  already committed in `tests/tools/execution-manifest.pl`, whose policy test
  in `packages/d2b-contract-tests/tests/policy_docs.rs` requires the `openat`
  and `unlinkat` markers and forbids path-based recursive cleanup outright, so
  this is reuse of a proved mechanism rather than a new one. Two refusals
  from the earlier contract are preserved: the subtree must hold no
  git-tracked file (the `assert_removable` guard in
  `tests/tools/clean-worktree.sh`), and only the Bazel subtree beneath the
  anchor is ever a target, never the anchor itself and never anything above
  it. A refused resolution deletes nothing and never widens, and it reports
  without echoing anything that identifies the machine or the operator. The
  message is a stable static error code plus a fixed sentence naming the class
  of refusal - `D2B-BZLCLEAN-TRACKED` for a git-tracked file inside the
  subtree, `D2B-BZLCLEAN-SYMLINK` for a symlinked or magic-link component,
  `D2B-BZLCLEAN-ESCAPE` for a resolution that leaves the anchor,
  `D2B-BZLCLEAN-LIVE` for a tree a server still owns - and nothing else. It
  never prints the rejected absolute path, the worktree location, the output
  base or its hash, a user identifier, a process identifier, or any opaque
  handle, because a refusal gets pasted into issues and chat and those values
  describe a person's machine rather than the defect. The remediation is
  exact, repository-relative, and **specific to the code**, because one
  generic remedy is wrong for at least one of the four: a dry run tells an
  operator nothing about a live server, and "correct the tracked or symlinked
  entry" is not an action a `D2B-BZLCLEAN-LIVE` refusal can take. Each code
  names its own steps and only those:

  - `D2B-BZLCLEAN-TRACKED`: run `D2B_CLEAN_DRY_RUN=1 make clean` to see the
    sweep without removing anything, remove or relocate the unexpected
    tracked entry from `.scratch/bazel/` as appropriate, then rerun
    `make clean`.
  - `D2B-BZLCLEAN-SYMLINK` and `D2B-BZLCLEAN-ESCAPE`: run
    `D2B_CLEAN_DRY_RUN=1 make clean` to see the sweep without removing
    anything, remove the offending symlink or magic link, or the escaping
    layout, from under `.scratch/bazel/`, then rerun `make clean`. Nothing
    has to be put in its place: `make clean` reclaims the subtree as a whole,
    so replacing the entry with a directory is not part of the remedy and is
    not required for the rerun to succeed. Whatever the link resolved to is
    outside `.scratch/bazel/`, is outside managed cleanup, and is not
    traversed, not removed and not modified by any step here; it stays
    exactly as it was. An operator who intends to reclaim that target must
    inspect it separately and remove it only after independently verifying
    they own it. This remedy authorizes no recursive removal, names no path,
    and gives no command that reaches outside the anchor.
  - `D2B-BZLCLEAN-LIVE`: close any Bazel clients running against this
    worktree, run `make bazel-shutdown`, then rerun `make clean`. Neither a
    dry run nor an edit under `.scratch/bazel/` is part of this remedy: the
    tree belongs to a live server until the shutdown completes, which is
    exactly why nothing there may be corrected first.

  Every step above is repository-relative and names nothing outside
  `.scratch/bazel/` and the two Make targets, and the one statement that
  refers to something outside the anchor does so without naming it, so no
  remedy can leak a path, a
  process identifier or an opaque handle by being followed. The dry run is the
  sanctioned place for local paths to appear, because it runs on the
  operator's own machine at their request and writes to their terminal rather
  than into a shared log. Nothing is deleted against a live server tree.
  Section 14 carries the tests that make each of these refusals and the
  descriptor discipline mechanical. This
  is local developer disk reclamation, running as the invoking user inside a
  gitignored directory it already owns: it is deliberately not a privileged
  operation, not a broker op, and not a new host mutation surface.
- `make clean` learns the scratch Bazel tree and reclaims it as one unit under
  the existing `D2B_CLEAN_KEEP_SCRATCH` control, so a contributor preserving
  scratch keeps the Bazel caches with it. That target already reclaimed 68 GB
  on a working worktree before Bazel existed; a second uncollected cache tree
  is not acceptable.
- Interaction with warm measurements. The warm profile in section 11 requires
  a live server and a populated action cache. A reclaim, a hard high-water
  refusal, or a `make clean` between the two invocations voids the warm
  measurement, and the procedure records the output-user-root size before and
  after so that a voided measurement is visible rather than merely low. The
  soft high-water print is safe during a warm measurement because it deletes
  nothing.

### 9. One side-by-side shadow workflow, never required

A new workflow `.github/workflows/pr-bazel-rust.yml` runs the Bazel Rust path
beside the existing one. It is hand-written rather than generated from
`tests/layer1-jobs.json`, because it must not perturb the required graph and
because its measurement and cache steps have no template support. It is not
added to `V3_PR_GATE_WORKFLOWS`; it is not a gate.

Topology: four parallel jobs, one per slice, plus a rollup job that requires
all four. Each job runs its `make test-bazel-rust-<slice>` target, satisfying
the approved-target policy and keeping the local and continuous-integration
entry points identical.

Four jobs rather than the single job the handoff recommends, for a reason
grounded in this repository's committed comments. The handoff's rule is to
avoid runner fan-out that hides duplicated compilation. These four slices
duplicate nothing: the API census renders through a separately pinned nightly
toolchain into its own tree and, as the gate documentation already states,
"shares nothing with the workspace build"; the broker and guest-shell-runner
are separate Cargo workspaces with separate locks; and the supply-chain targets
compile no first-party code. The one slice that does contain a shared
dependency graph, `main`, is not split, and that is where Bazel's deduplication
is actually collected. Splitting along boundaries that already share nothing
converts runner minutes into wall clock, which is what the budget in section 11
measures.

Triggers: pushes to `main` and `v3`; `workflow_dispatch`; a weekly schedule for
the cold measurement; and pull requests filtered to paths that own the Bazel
surface (`MODULE.bazel*`, `.bazelrc`, `.bazelversion`, `bazel/**`, generated
`BUILD.bazel` files, the generator, the workflow, and the Makefile). The shadow
path does not run on every pull request during the shadow stage, because with
no cache published (section 10) every run is a cold run, and paying a cold Rust
build on every pull request to learn nothing new is not a good trade.

The workflow satisfies the existing structural policies without exception:
`defaults.run.shell: sh tests/tools/ci-shell {0}`, and every
`actions/checkout` step immediately followed by `with:` and
`persist-credentials: false`.

### 10. Cache design, and the trust boundary

**Nothing is published during the shadow stage.** Measured headroom in the
repository Actions cache is about 0.87 GiB against a hard 10 GB limit, and
GitHub evicts least-recently-used entries when the limit is exceeded. Adding
even a 2 GiB Bazel snapshot while the old path is still authoritative would
evict the rust-cache entries that keep the required Rust leaves warm, taking
the required path from a 7m46s critical path toward the documented 43-minute
cold cost and into its 60-minute job timeouts. The shadow workflow therefore
restores nothing and saves nothing. Its measurement target is the cold
continuous-integration number, which is the budgeted one.

**Caches arrive at promotion, after the space is actually freed.** The
promotion change deletes the `Swatinem/rust-cache` configuration, which stops
further writes, and the cache maintenance job below deletes the retired
entries themselves, freeing about 6.5 GiB. Only then are introduced:

- one Bazel **disk cache** (the action cache), budgeted at 4 GiB;
- one Bazel **repository/download cache** (`--repository_cache`), budgeted at
  1 GiB, as a separate entry with a separate key.

These are distinct caches with distinct invalidation and must never be merged
into one entry. The Bazel **output base is never cached**: it holds absolute
paths, symlinks into the source tree and server state, it is machine-specific,
and it is the single largest thing a naive implementation would try to carry.

**Keys.** The primary key is unique per successful default-branch run; the
restore prefix omits the run identifier and never contains a commit SHA. Both
key and prefix bind: `.bazelversion`, `MODULE.bazel`, `MODULE.bazel.lock`,
`.bazelrc`, both `rust-toolchain.toml` files, the three `Cargo.lock` files and
`packages/Cargo.guest.lock`, the three `deny.toml` files, the advisory-database
pin, and a digest of the generated BUILD tree. A change to any of those must
produce a different key rather than a subtly stale cache.

**Writer policy.** Pull requests restore read-only and never save. Default
branch pushes restore, run, trim, and publish exactly one refreshed snapshot,
from exactly one job. This is stronger than the current path, which does
publish pull-request-ref entries; that is measured, is a contributing cause of
the full cache, and is not carried forward. A structural assertion added to
`packages/xtask/tests/policy_ci.rs` by this implementation - that file owns
the approved-target list and the workflow allowlist today, and carries no
cache policy yet - fails closed when any workflow reaches a saving cache
action on a `pull_request` event, so the policy is enforced by a test rather
than by review attention.

That assertion is worthless without proof that it can fail, because a checker
that always returns clean passes against a compliant repository. It therefore
ships with committed fixtures under `packages/xtask/tests/fixtures/ci/`, kept
outside `.github/workflows/` so they are not real workflows and cannot perturb
the workflow allowlist or the shell and checkout structural gates. Required
negative fixtures, each of which the test asserts is **rejected**:

- `actions/cache` reachable from a `pull_request`-triggered job without
  `lookup-only: true`, which saves in its post step;
- `actions/cache/save` reachable from a `pull_request`-triggered job;
- `Swatinem/rust-cache` reachable from a `pull_request`-triggered job without
  `save-if: false`, which likewise saves in its post step;
- a cache action the checker does not recognize, reachable from a
  `pull_request`-triggered job, because an unknown writer is denied rather
  than assumed harmless.

Required positive fixture, which the test asserts is accepted: a
default-branch-only saving job alongside a restore-only pull-request job. A
change that weakens the checker into always returning clean fails on the
negative fixtures; a change that over-tightens it fails on the positive one.

**Size enforcement.** `.bazelrc`'s continuous-integration configuration sets
`--experimental_disk_cache_gc_max_size=4G` and
`--experimental_disk_cache_gc_idle_delay=0s`, the latter because the flag
defaults to five minutes and a short job never goes idle that long.
Independently of whether that experimental collector behaves as documented, the
save step measures the directory with `du` and refuses to publish an
over-budget snapshot. A refused publish is a failed step, not a silent skip.

**A commit cannot delete cache entries, so promotion needs a deletion job.**
Cache entries are runtime objects that exist only through the Actions cache
API; merging the promotion commit stops new rust-cache entries from being
written but frees nothing that already exists. Waiting for GitHub's
least-recently-used eviction is worse than useless here, because LRU chooses
its victim by age of use, not by whether the entry is retired: it may evict a
Nix fixture entry, or the Bazel entry just written, while the retired
rust-cache entries survive. The result is a deadlock in which the save step
correctly refuses to publish because the repository is over budget, and
nothing ever deletes the entries that made it over budget.

The promotion sequence is therefore explicit and ordered:

1. The promotion change removes the `Swatinem/rust-cache` step, so no new
   rust-cache entry is written from the moment it merges.
2. A **default-branch-only cache maintenance job** with
   `permissions: actions: write` enumerates cache entries through the GitHub
   API, paginating to completion, and deletes only entries whose key matches
   an authorized prefix: the retired `Swatinem/rust-cache` prefixes committed
   in the workflow, and superseded Bazel entries under this decision's own
   prefixes beyond the newest generation kept.
3. It re-queries repository cache usage and confirms that the post-delete
   total plus the planned snapshot stays at or under 8 GiB of the 10 GB limit.
4. Only then does the saving job publish, and it re-queries usage immediately
   before publishing and refuses when headroom has regressed since step 3, so
   a concurrent run cannot slip the repository over budget between the two
   jobs.

The job fails closed. An entry matching neither an authorized retired prefix
nor an authorized Bazel prefix is not deleted, and the job fails naming the
entry key, the measured usage, the headroom shortfall, and the two authorized
remedies: identify the entry's owner and delete it deliberately, or shrink the
Bazel cache budget. Incomplete pagination, a failed usage query, and an
ambiguous prefix match are the same failure with the same shape. Guessing is
never the fallback.

Pull-request jobs carry `permissions: contents: read` and never
`actions: write`, and the structural policy test asserts that no
`pull_request`-reachable job requests `actions: write`. Cache deletion is a
default-branch capability by construction, not by convention.

**Cache maintenance and the Rust verdict are separate.** Cache maintenance is
its own job. It is not part of the `test-rust` rollup, its failure never marks
a Rust surface failed, never appears in `failed_surfaces`, and never changes
the required context's verdict; and its success never contributes to that
verdict either. A cache problem must be diagnosable as a cache problem, and a
red Rust context must mean a Rust test failed.

**Credentials.** Cache input and output happen only inside the cache action's
own process. No `ACTIONS_RUNTIME_TOKEN`, `ACTIONS_CACHE_URL` or equivalent
credential is exported into a `run:` step. The existing workflow already
documents this property for `Swatinem/rust-cache` and it is preserved verbatim,
for a sharper reason under Bazel: the Bazel process executes build scripts and
proc macros from 544, 117 and 161 locked third-party packages, so any
credential visible to Bazel is a credential visible to third-party code
executing at build time.

**No remote cache and no remote execution.** Neither `--remote_cache` nor
`--remote_executor` nor a build event service upload is configured, and
configuring one is out of scope for this ADR. Backing a Bazel remote cache with
the GitHub Actions cache service would require exactly the credential exposure
the previous paragraph prohibits. A future decision may revisit this with a
credential model that keeps the token out of the build-script environment.

### 11. Performance budgets

Three ceilings bind, all wall clock:

| Profile | Ceiling |
| --- | --- |
| Warm local | 10 minutes |
| Cold local | 15 minutes |
| Cold continuous integration | 15 minutes |

**Reference local host.** `x86_64-linux`, at least 12 physical cores, at least
32 GiB of RAM available to the build, worktree and Bazel output base on local
NVMe or SSD, the pinned dev shell entered, and no heavy lane holding a
heavy-gate slot. The host of record for this ADR is the 12-core i9-10920X with
62 GiB measured above.

**Reference runner.** `ubuntu-latest`, GitHub-hosted, 4 vCPU and 16 GiB, which
is what every Rust job in `pr-l1-static-fast.yml` uses today. A different
runner class is a change to the budget's basis and must be recorded with the
measurement.

**Cold local** means a fresh Bazel output user root, an empty disk cache, and a
populated repository/download cache with the pinned Bazel already in the Nix
store. The download cache is deliberately warm: the ceiling must measure the
build, not the network. Fetch time is measured and reported separately and
carries no ceiling here.

**Warm local** means an immediately preceding successful `make test-bazel-rust`
on the same commit, then exactly one edit, appending a comment line to
`packages/d2b-core/src/lib.rs`, then a second `make test-bazel-rust` with the
Bazel server still live. `d2b-core` is the widest-fanout first-party crate, so
this is the worst realistic incremental case and it is exactly reproducible.

**Cold continuous integration** means a shadow-workflow run that restores no
Bazel cache of any kind. Its measured value is the GitHub job wall clock of the
slowest slice job, from job start to job completion, including checkout, Nix
installation, Bazel acquisition, fetch, analysis, build and test, and excluding
queue time. No step is carved out of the measurement, because a budget with
carve-outs is a budget nobody can audit.

**Start and stop.** Locally, the number is the wall clock of the
`make test-bazel-rust` process from start to exit, printed by the target as its
final line. In continuous integration, the number is the job duration reported
by the Actions API, which no step can influence.

**Repetition and percentile.** One rule for all three profiles: take the
measurement set, and the budget holds when the median is at or under the
ceiling **and** no single measurement exceeds 1.2 times the ceiling. The
measurement set is three consecutive runs locally on the reference host, and
the last five scheduled cold runs on the default branch in continuous
integration. All measurements are reported, not just the median.

**Fail-closed response when a budget is missed.**

During the shadow stage, the budget-evaluation step of the shadow workflow
exits nonzero, so the miss is red and visible, and promotion is blocked while
the evidence does not satisfy the rule. Because the shadow workflow is not a
required context, a miss does not block unrelated work. The shadow stage does
not bound the run in band, because a truncated run produces no measurement;
its job timeout stays at 30 minutes.

**How the ceiling is enforced at promotion.** A bare `timeout-minutes` is a
ceiling nobody can act on: the job is killed, the log ends mid-target, and the
contributor is left to guess. The enforcement is in band and actionable.

- *Anchor.* The job records a monotonic anchor from `/proc/uptime`, which is
  boot-relative and immune to wall-clock steps, and records the epoch value
  beside it only for the human-readable report. It exports through
  `$GITHUB_ENV` exactly one control, and that control is an **absolute
  deadline in integer milliseconds** in the same boot-relative `/proc/uptime`
  domain, computed as `anchor + ceiling - checkout allowance`, where the
  anchor is necessarily read after checkout has completed. It is a point in
  time, not a duration, and no consumer may treat it as one. The next bullet
  writes the arithmetic out, because that subtraction is correct only on the
  post-checkout reading of the anchor and wrong on any other.
- *The type, because the file is not an integer.* Measured on the reference
  host on 2026-08-02, `/proc/uptime` is two whitespace-separated fixed-point
  decimal fields and the first is uptime in seconds with exactly two
  fractional digits: `86423.03`. Handing that field to a validator that
  requires a positive integer rejects every well-formed reading, so the raw
  field is never the control and is never exported. Capture and read both
  convert through **one repository-owned parser**, so the two ends of the
  handoff share one implementation and one refusal set rather than two
  spellings of a decimal parse. That parser takes the first field, delimited
  by whitespace, accepts `<digits>` optionally followed by `.` and one or
  more digits, ASCII digits only, and rejects a sign, an exponent, a second
  separator, a missing field, and any trailing content; it converts seconds
  and fraction to milliseconds with checked arithmetic; and it refuses a
  malformed field or a conversion that overflows with an actionable message
  that does not echo the value, which is the redaction rule
  `D2B_RUST_BUDGET` already follows. Every conversion rounds so that the
  error moves the bound earlier: the capture truncates toward zero, so the
  minted deadline is never later than the true anchor plus the window.
- *Where the anchor can sit, measured.* The anchor cannot precede checkout.
  `tests/unit/meta/ci-coverage.sh` requires every `shell:` declaration in
  every workflow to be exactly `shell: sh tests/tools/ci-shell {0}`, and
  `tests/tools/ci-shell` is a repository file, so no `run:` step can execute
  before `actions/checkout` has placed it. The anchor is therefore the first
  step after checkout, forced there by the mandated workflow shell living in
  the repository, and the checkout step carries its own `timeout-minutes: 2`.
  The deadline consequently bounds the post-checkout window only. Written out
  once for the 15-minute continuous-integration ceiling, so no reader can take
  the anchor for a pre-checkout one:

  ```text
  uptime_field    = first field of /proc/uptime, fixed-point decimal seconds
                    ("86423.03" on the reference host), read as the first
                    action of the first step after checkout
  anchor_ms       = checked conversion of uptime_field, truncated toward zero
  deadline_ms     = anchor_ms + (900000 - 120000) = anchor_ms + 780000
  checkout_actual <= 2 minutes    bounded by the checkout step timeout
  post_checkout   <= 13 minutes   bounded by the deadline above
  total job time  = checkout_actual + post_checkout <= 2 + 13 = 15 minutes
  ```

  The read is the anchor step's first action, before that step does anything
  else, including anything it does to reach the parser. Work performed after
  the read falls inside the 780000 ms the deadline bounds, so no unbounded
  segment can open between the end of checkout and the anchor.

  Checkout is bounded once, by its own step timeout, and subtracted once, from
  the window the anchor opens. It is not subtracted twice: the anchor never
  covers checkout, so there is no second segment for the allowance to come out
  of, and `checkout_actual` enters the total at its real value rather than at
  a reduced one. The alternative reading, `deadline_ms = anchor_ms + ceiling`,
  bounds the job at `checkout_actual + 15`, which is up to 17 minutes and
  exceeds the ceiling; it is rejected. Nothing is carved out - the ceiling
  still covers checkout, Nix and Bazel setup, fetch, analysis, build and tests
  - and the only change is that one bounded segment is bounded by its own step
  timeout.
- *Handoff.* The approved Make target reads that **absolute deadline**, not a
  remaining duration. It validates the value the way `D2B_RUST_BUDGET` is
  validated: a positive integer count of milliseconds, an actionable message
  on a bad value, and no value echoed back. It then reads `/proc/uptime`
  itself, in the same boot-relative domain the deadline was minted in, and
  converts the first field through the same repository-owned parser the
  capture used, rounding this conversion **up** rather than truncating, the
  mirror of the capture's rounding, so both conversion errors move the bound
  earlier. It computes `remaining_ms = deadline_ms.checked_sub(now_ms)` in
  unsigned integer milliseconds. **A `None` here is an expired budget, not a
  bad value.**
  Unsigned `checked_sub` returns `None` under exactly one condition,
  `now_ms > deadline_ms`, which is the ordinary outcome of a job that used
  its whole window, and reporting it as malformed input would send a
  contributor whose job merely ran long to inspect an input that was
  perfectly well formed. `None` and `Some(0)` therefore take the same normal
  expired-budget path: the target fails immediately, before starting any
  work, with the same actionable budget report the expiry path prints - the
  elapsed duration against the ceiling, the target that was about to run, and
  the two remedies pre-authorized for a missed ceiling below. Bad-value
  refusal is reserved for the cases that really are malformed input or a
  broken domain: a `/proc/uptime` field the shared parser rejects, a
  conversion that overflows, and a deadline control that is present but
  non-numeric, signed, or not representable as an unsigned millisecond
  count. Those refuse with the parse or domain message and, as everywhere
  else, without echoing the value. An **absent** control is neither of those
  and is not a refusal at all; it is the unbounded local default named at the
  end of this bullet. Only the relative remaining duration of a surviving
  `Some(n)` with `n > 0` is passed
  to the child-bounding mechanism, and it is rounded **down** to whatever
  granularity that mechanism accepts, so a rounding step can only shorten the
  bound and can never lift the total above the ceiling. The absolute deadline
  is never passed to anything that interprets its argument as a duration - a
  boot-relative timestamp read as a timeout is a bound of many hours, which
  is a silent no-op rather than a visible failure, and that is the specific
  mistake this paragraph exists to make impossible. Absent the control the
  target runs unbounded, which is the local default; a structural assertion
  requires every Bazel Rust job in the promoted workflow to set it.
- *Bounding.* The target bounds the Bazel invocation with that computed
  remaining duration. The Bazel client is spawned into a **new dedicated
  process group**, created between fork and exec by repository-owned Rust
  plumbing (`Command::process_group(0)`, the idiom already committed in
  `packages/d2b-exec-runner/src/service_mode.rs`) and never by a shell, a
  `setsid` helper or any other external gate, which ADR 0017 forbids anyway.
  The group identifier is then the child's own process identifier and the
  wrapper is not a member of that group. Creating the group before exec is
  precisely what makes group signalling safe: without it the child inherits
  the wrapper's group, and signalling that group reaches the wrapper itself,
  the Make process that invoked it, and whatever else shares the invoking
  shell's job. On expiry the wrapper signals only that dedicated group:
  SIGTERM, then SIGKILL after a fixed grace. **That grace is a fixed period
  and is waited in full, unconditionally.** Nothing the wrapper observes
  during it shortens it, and the final group SIGKILL is sent when it expires
  whether or not the leader is still alive. **It reaps the direct child at
  no point before that final group SIGKILL.** A process group outlives its
  leader, so a leader that exits during the grace period can still have
  descendants running - descendants that are still inside their own SIGTERM
  handling, which is precisely what the remaining grace is for - and the
  escalation must therefore run its full course whenever the leader exits.
  Holding the leader unreaped until after the
  final SIGKILL is a deliberately conservative requirement, and the reason is
  not the one an earlier draft of this ADR gave: reaping the leader does
  **not** by itself cost the escalation its group. Measured below, the group
  stayed addressable and a group SIGKILL still reached a surviving descendant
  after the leader had been reaped. The requirement stands because it removes
  the identifier-reuse question rather than answering it per kernel. While
  the leader is held, its identifier - which is also the group identifier -
  cannot be recycled under any scheduling, so the wrapper never has to reason
  about which descendant happens to be keeping the group alive at the moment
  it signals. The cost is one delayed `wait`.

  The wrapper therefore leaves an exited leader unreaped and observes the
  exit **without consuming it and without blocking** - `waitid` with
  `WEXITED|WNOWAIT|WNOHANG`, which is `rustix::process::waitid` with
  `WaitidOptions::EXITED | WaitidOptions::NOWAIT | WaitidOptions::NOHANG`
  and needs no first-party `unsafe`, or an equivalent non-consuming,
  non-blocking wait. All three flags are load-bearing. `EXITED` selects the
  transition being polled. `NOWAIT` leaves the leader in a waitable state, so
  the observation does not consume the status and the escalation keeps its
  handle. `NOHANG` is what makes the observation a poll rather than a wait:
  without it the call parks in the kernel until the leader exits, so a leader
  that has not exited when the grace timer should fire holds the wrapper
  inside `waitid` and the grace period stops being a bound the wrapper
  controls. With it, a poll against a leader that has not exited returns
  immediately reporting no state change - `Ok(None)` from that rustix
  signature, which is a result and not an error - so the wrapper polls,
  waits against its own grace deadline, and escalates on schedule whether or
  not the leader ever exits. The observation is **informational only**: it
  feeds the report and keeps the escalation's handle on the leader, and it
  never shortens the grace and never ends the escalation early. A leader that
  exits in the first millisecond of the grace buys nothing, because its exit
  says nothing about the descendants it left behind. After the final group
  SIGKILL, and only then, it reaps the direct child.

  Measured on the reference host on 2026-08-02, Linux 7.0.10, because this is
  the step the correction turns on. Against a still-running leader,
  `waitid(WEXITED|WNOWAIT|WNOHANG)` returned in under a millisecond reporting
  no state change (`si_pid == 0`), while the same call without `WNOHANG`
  blocked for the leader's entire remaining lifetime, 1.400 seconds in the
  probe; that is the grace-timer overrun `NOHANG` exists to prevent. With the
  leader held as an unreaped zombie, `kill(-pgid, 0)` returns 0 whether or not
  a descendant survives; a repeated `waitid(WEXITED|WNOWAIT|WNOHANG)` returns
  the same exit status twice, so the observation does not consume it; the
  group SIGKILL reaches a descendant the exited leader left behind; and a
  group SIGKILL whose only remaining member is the held zombie returns 0 and
  does nothing, leaving that zombie's recorded exit status unchanged. The
  measurement that corrects the earlier draft: after the leader was reaped
  while a descendant was still running, `kill(-pgid, 0)` still returned 0 and
  a group SIGKILL still killed that descendant, and ESRCH appeared only once
  the group held no member at all. Two consequences are binding. The final
  SIGKILL is **unconditional**, because a liveness probe
  cannot report an empty group while the wrapper is holding the leader, so
  there is nothing for a conditional to read; and it is free, because
  signalling a group down to its held zombie is a no-op. The single skip this
  ADR authorizes is a complete enumeration of the group's membership that
  observes no member other than the held leader zombie. Leader exit alone is
  not that proof, a `kill(-pgid, 0)` probe alone is not that proof, and
  neither ever authorizes skipping the escalation.

  It never signals its own process group, and never signals group zero or -1.
  It owns the child identity by the parent-child relationship and holds that
  identity unreaped for the whole escalation window, so it cannot be aiming
  at a recycled process identifier. That is the reuse argument the committed
  `signal_process_group` comment in
  `packages/d2b-exec-runner/src/service_mode.rs` records, applied to the
  entire escalation rather than only to its first signal.

  It never reads a server PID file and never signals a server process
  identifier, because that process is detached, is
  not this wrapper's child, is not reaped by it, and its identifier may
  already have been reused between the file being written and the signal being
  sent; signalling it is a signal aimed at whatever now holds that number.
  Server termination is requested only after the escalation has completed and
  the client is reaped, and only through `bazel shutdown` with the same
  startup options, which is the interface that owns server
  identity. That shutdown carries its own short bound; if it does not complete
  within that bound the wrapper fails with the stable static code
  `D2B-BZLSERVER-STUCK` and a fixed message, and does not escalate to a raw
  signal against a detached identifier. That message carries no output base,
  no output-base hash, no path, no user identifier and no process identifier.
  It states that a Bazel server did not shut down within its bound and gives
  two exact steps: close any other Bazel client running against this worktree,
  then run `make bazel-shutdown`, which reissues the shutdown with the same
  startup options and reports either success or the same code. While the
  condition is unresolved the message forbids two things outright: deleting
  `.scratch/bazel/`, because a live server still owns that tree, and
  signalling any process identifier by hand, because the server is detached
  and its identifier may already belong to something else. If
  `make bazel-shutdown` does not clear it, the escalation is a bug report
  carrying that code, not a manual kill. The job-level `timeout-minutes` below
  stays the backstop for a runner still stuck after all of that. **On a
  missed ceiling, and only there,** the target prints the measured duration
  against the ceiling and the target that was executing - never the raw
  deadline value, an output base, a path or a process identifier - and exits
  nonzero naming only the two remedies pre-authorized for a ceiling miss
  below. It never prints "relax the ceiling", because that is not an
  available remedy. That restriction is scoped to the performance ceiling and
  to nothing else. It does not suppress, replace or shorten the recovery
  steps any other failure names: a `D2B-BZLSERVER-STUCK` failure still names
  its two steps, close other Bazel clients and run `make bazel-shutdown`, and
  a cleanup refusal still names the per-code remedy section 8 gives it. Those
  are static recovery actions for an operation that refused or did not
  complete, not remediations for a budget that was exceeded, and the two
  lists never merge in either direction. Section 14 carries the tests that
  make the group discipline, the deadline conversion and the message
  redaction mechanical.
- *Backstop.* The job keeps a `timeout-minutes` slightly above the ceiling,
  17 against a 15-minute ceiling with a 2-minute checkout allowance, purely so
  a dead runner is still reaped. The in-band deadline is the failure a
  contributor should ever see; the job timeout firing means the runner itself
  stopped responding.

This does not contradict `tests/layer1-jobs.json` classifying
`test-performance-budgets` as advisory. That classification governs a
*measurement* surface, which a hosted runner cannot support as an enforcing
assertion. A deadline on the executor is the same class of object as
`timeout-minutes`, a structural ceiling with no percentile machinery, only
one that reports what it bounded.

When a ceiling cannot be met, exactly two remediations are pre-authorized:
raise the runner class for the Bazel Rust jobs, or split a slice further while
keeping slices disjoint and keeping the surface-to-target map one-to-one.
Reducing coverage, reclassifying an enforcing surface as advisory, moving a
surface out of the Rust gate, and relaxing a ceiling are **not** authorized and
require a superseding ADR. This pair is the closed remedy list for a *ceiling
miss* and for nothing else. The recovery steps a cleanup refusal or a
`D2B-BZLSERVER-STUCK` failure names are not remediations under this rule, are
not constrained by it, and are never replaced by it.

**Honest expectation.** The cold ceilings are ambitious against the measured
baseline. The workflow's own comment puts a cold Rust-profile job at about 43
minutes on the reference runner, and while Bazel removes the duplicated
compilation across leaves, it does not add cores. The `main` slice is where
this will be decided. This ADR sets the ceiling as a promotion gate precisely
so the answer arrives from the first shadow run rather than at cutover, and so
the response to a miss is a named remediation rather than an improvised one.

### 12. Promotion, and only then retirement

`make test-rust` and the required `test-rust` continuous-integration context
switch to Bazel in a single later change, and only when all of the following
hold. Each is mechanically checkable.

1. **Coverage.** `//ci/rust:coverage_map_guard` passes: all eighteen baseline
   identifiers map to targets that exist, no Rust test target is unmapped, and
   no hand-written fragment is unlisted.
2. **Equivalence, positive.** Ten consecutive default-branch shadow runs in
   which the Bazel rollup and the Cargo `test-rust` rollup reach the same
   verdict.
3. **Equivalence, negative.** A recorded seeded-failure matrix: for each of the
   eighteen surfaces, a deliberately broken tree makes exactly that Bazel
   target fail and does not make an unrelated surface fail. This is recorded
   once, in the wave notes, not on every run. Without it, the positive evidence
   only proves the targets are green, not that they are enforcing.
4. **Census.** `//ci/rust:pinned_test_inventory` passes against the Bazel
   census, every suite's listing is nonempty, every scan surface matches its
   exact manifest rather than a floor, and the schema surface produces the
   committed twenty-file census in both generations before any digest
   comparison.
5. **Topology and repetition.** The section 7 prototype is recorded for the
   main workspace, the three broker feature passes and the guest shell runner,
   with matching census and ignored-case counts. The three broker suites pass
   twenty consecutive executions (`--runs_per_test=20`) with `exclusive` in
   force, evidencing that Bazel's scheduling has not reintroduced the
   process-global-state failure the current gate avoids by construction.
6. **Budgets.** Section 11's rule is satisfied for all three profiles, with
   every measurement recorded, and the in-band deadline handoff is wired.
7. **Supply chain.** The section 6 comparison is recorded: today's
   `cargo deny check` against the decomposed `cargo-deny` plus `cargo-audit`
   pair, over all three locks, with no differing enforcing outcome.
8. **Cache.** The shadow stage published no Actions cache entry; the cache
   maintenance job exists, is default-branch-only, and has deleted the retired
   `Swatinem/rust-cache` entries; measured headroom satisfies the section 10
   rule before the first Bazel snapshot is saved; and the writer-policy
   fixtures, negative and positive, are committed and enforcing.

The promotion change then:

- replaces the eight `ciKind: rust` leaves in `tests/layer1-jobs.json` with the
  four Bazel slices while keeping `ciJobId: test-rust` on the rollup, so the
  required context name is unchanged and branch protection needs no edit;
- regenerates `pr-l1-static-fast.yml` through `make layer1-workflow` and
  deletes `.github/workflows/pr-bazel-rust.yml`, whose continued existence is
  itself the mechanical signal that promotion is incomplete;
- points `make test-rust` at Bazel for the eighteen surfaces while leaving the
  Cargo fixture leaf for `rust-contract-tests` and `rust-cli-contract-tests`
  untouched;
- keeps the eight `make test-rust-<leaf>` names as thin aliases onto the Bazel
  targets for their surfaces, so contributor muscle memory and the
  contributor documentation stay correct;
- keeps `make test-bazel-rust` and its four slice targets as **compatibility
  aliases** rather than deleting them. Each forwards to `make test-rust` or
  the corresponding leaf target, prints a one-line deprecation notice on
  standard error naming the replacement target, and exits with the forwarded
  target's status. They stay in `APPROVED_MAKE_TARGETS` while they exist.

Deleting those names at promotion would break every contributor script,
worktree note and open branch that already runs them, on the same day the
underlying executor changes, which makes one failure look like the other. The
drift this repository actually guards against is two names running *different
code*; a forwarding alias runs the same code and says so.

The window is bounded and the condition is mechanical: the aliases are removed
in a separate change, which may not land before at least one release tag
contains the promotion commit (`git tag --contains <promotion commit>` is
nonempty) and a changelog fragment announcing the deprecation shipped with the
promotion. Removal lands its own fragment. Until removal, a structural
assertion forbids any workflow from calling the aliases, so they remain a
human convenience and can never quietly become the gate path.

Retirement of the Cargo implementation, meaning deletion of the eighteen
surfaces' leaf modes from `tests/test-rust.sh` and their Make leaves, happens
in a further change after promotion has been green on the default branch for
ten consecutive runs. The `fixture-contracts` mode stays. Until that deletion,
the old path is recoverable by reverting one commit, which is the point of
sequencing it separately.

### 13. Deliberate differences, recorded rather than discovered

These are places where the Bazel path is knowingly not byte-identical to the
Cargo path. Each is listed in the coverage map with this rationale, so it is a
recorded decision rather than a silent regression.

- **Warnings-as-errors scope.** The gate exports `RUSTFLAGS=-D warnings` and
  `RUSTDOCFLAGS=-D warnings` globally, which today also applies to
  third-party crates. The Bazel path applies warnings-as-errors to first-party
  compilation, including doctests and any first-party build script, and not to
  third-party crates. Third-party crates are pinned by `Cargo.lock`; their
  warning output is not a d2b coverage surface, and the surface that is one,
  Clippy with `-D warnings` over the `--all-targets` equivalent, is preserved
  exactly. Compile-fail fixtures continue to carry their own explicit mutation
  flags so expected diagnostics stay attributable to the capability seal they
  exercise.
- **`--locked` becomes a repin drift check.** The property is preserved: a
  stale committed `Cargo.lock` fails the gate. The mechanism moves from a Cargo
  flag to the Bazel-side lock comparison in section 2.
- **cargo-audit stops reaching the network, and carries advisories alone.**
  The advisory database becomes a pinned input. This removes the retry loop and
  makes the surface deterministic; it also means database freshness is now a
  property of a committed pin. Per section 6, `cargo-deny` runs only
  `bans licenses sources` on the Bazel path and the `[advisories]` sections of
  the three `deny.toml` files become inert there, with the live ignore
  semantics carried by `cargo-audit --ignore`. The aggregate policy is
  preserved as the union of the two tools; the recorded outcome comparison in
  section 6 is what proves it.
- **Test process topology.** The current gate is not uniformly intra-binary:
  the main workspace and the guest shell runner are process-per-test under
  cargo-nextest, while
  the three broker feature passes are deliberately one process per binary with
  bounded threads. Section 7 preserves each of those and requires a
  repository-owned runner over the Bazel-built test binaries rather than plain
  `rust_test` execution. What genuinely changes is that Bazel schedules test
  *targets* concurrently, which is why the broker suites are `exclusive` and
  why section 12 requires the repetition evidence.

### 14. The guards that make sections 8 and 11 mechanical

Sections 8 and 11 assert behaviour, not prose: close-on-exec descriptors,
descriptor-relative deletion, refusals that delete nothing, a dedicated
process group whose leader is held unreaped through escalation, a deadline
parsed as checked integer milliseconds, and messages that carry a static code
and no local identifier. Every one of those can regress silently into a
working build that is quietly unsafe. They therefore ship with enforcing tests
in the wave that implements the plumbing, not at promotion, and each guard
ships with a **planted negative fixture or mutation that proves the guard
fails when the invariant is removed**, because a checker that has never been
observed failing is an assertion about the author's intent rather than about
the code.

The tests are wired into surfaces that already exist. Behavioural tests are
Layer-1 Rust tests in the crate that owns the plumbing, carried by the
`rust-main-workspace-tests` surface and therefore by `//ci/rust:main_tests`
after migration; source-shape assertions extend
`packages/d2b-contract-tests/tests/policy_docs.rs`, carried by `test-policy`;
workflow-shape assertions extend `packages/xtask/tests/policy_ci.rs`. That
file today owns `APPROVED_MAKE_TARGETS` and the `ALLOWLISTED_WORKFLOWS`
allowlist and nothing else relevant here; the cache writer-policy assertion of
section 10, the two workflow-structure assertions below, and their committed
fixtures under
`packages/xtask/tests/fixtures/ci/` do **not** exist there today and are added
to it by this implementation, in the same change as the six new approved
targets.
**No new top-level shell gate is added**: a new gate would mean a new Layer-1
job, a new required-context question, and a new bash surface ADR 0017
forbids, to carry assertions three existing surfaces already carry. Nothing
new is added to `tests/layer1-jobs.json` and no new Make target is created for
them. The behavioural process tests signal only their own dedicated child
groups and never touch the test process's global signal or reap state, so
they stay safe under the main workspace's process-per-test topology and need
no `exclusive` tag; a test that cannot meet that condition belongs in a
different suite and its placement is reviewed, not assumed.

**Cleanup helper.** Positive: a populated planted tree beneath the anchor is
removed, and every removal is descriptor-relative. Descriptor discipline: the
helper execs a planted child that enumerates its own `/proc/self/fd` and
reports what it inherited; the test fails when any cleanup descriptor appears
there, which is what proves `O_CLOEXEC` rather than asserting it was written.
**The `O_CLOEXEC` mutation belongs to that behavioural test**, not to the
source scan: run against a planted cleanup variant that opens one descriptor
without `O_CLOEXEC` - the anchor, a traversal descriptor, or the
`O_DIRECTORY|O_RDONLY` reopen used for enumeration, one case per position -
the planted child must observe the leaked descriptor in its own
`/proc/self/fd` and the exec-inheritance case must fail. That is the only
placement where the mutation proves anything, because the property under test
is what a child actually inherits across `exec`, and only a child that
enumerates its own descriptors can observe it.
The `openat2` route and the component-by-component `openat` fallback are
exercised as separate cases, with the fallback forced even where `openat2` is
available, so the fallback's flags are covered on every kernel the gate runs
on. Refusals: a symlinked component, a magic-link component, a resolution
that escapes the anchor, and a git-tracked file inside the subtree. Each
asserts the exact static code, asserts that the planted tree is unchanged
afterwards, and asserts the refusal never widened to the anchor or above it.
Race: the subtree name is replaced by a symlink to a decoy tree after the
anchor descriptor is opened; the decoy must be untouched, which is the
property string re-resolution cannot deliver. Negative, source shape: the
existing `policy_docs.rs` marker set that requires `openat` and `unlinkat`
and bans path-based recursive cleanup is extended to cover the new cleanup
source, and is proved failing against a planted mutation of that source that
substitutes a path-based recursive removal, in the same in-test
mutation-fixture idiom that file already uses. The source policy is kept for
the deletion idiom because a path-based recursive remove is a shape a scan
can see. It is deliberately **not** kept as the proof of `O_CLOEXEC`: a
marker scan cannot distinguish a flag that is written from a flag that
reaches the descriptor that matters, so it would pass a variant that sets
`O_CLOEXEC` on three descriptors and leaks the fourth.

**Timeout wrapper.** The tests spawn planted children, never a real Bazel
client, so they cost milliseconds, cannot be perturbed by a live server, and
stay correct once these very tests are themselves scheduled by Bazel. The
Bazel client's process group identifier differs from
the wrapper's own and equals the child's process identifier. A sibling
process placed in the wrapper's own group is still alive after expiry, which
is what proves the wrapper did not signal its own group.

*Escalation order is tested as order, not as a race.* The wrapper's
signalling and waiting sit behind a small process backend, and the order test
drives it with a recording fake that logs every call in sequence. It asserts
the exact order `group SIGTERM` -> non-consuming, non-blocking observation
across the grace -> `group SIGKILL` -> reap of the direct child; that no reap
call appears anywhere before the group SIGKILL; that the observation calls
carry `EXITED|NOWAIT|NOHANG` and never consume; that the full fixed grace
elapses on the backend's clock before the group SIGKILL **even when the first
observation already reports the leader exited**, which is what proves the
observation is informational and cannot shorten the grace; and that every
signal call names the dedicated group and never the wrapper's own group,
group zero or group -1. A state-machine test over the same backend is an
acceptable equivalent provided it asserts the same ordering, the same
absence, and the same full grace.

*The real-process cases stay, and prove what only real processes can.* A
descendant that ignores SIGTERM and outlives the leader is dead once
escalation completes, which proves the final group SIGKILL actually reaches
descendants; a leader that exits during the grace period is observed
non-destructively and is still waitable at the moment that SIGKILL is sent.
That surviving-descendant case is a positive for the unconditional final
SIGKILL and is deliberately **not** the early-reap negative, because on the
measured kernel an early reap does not by itself make the group
unaddressable, so the case can stay green against an early-reap mutation.

A decoy process whose identifier is planted
in a server PID file inside a scratch output base is alive and unsignalled
after expiry, and the policy surface asserts the wrapper source carries no
server PID path literal and no signal aimed at a value read from a file. The
`bazel shutdown` that follows is issued with the same startup options under
its own bound, with a planted stub that never returns producing
`D2B-BZLSERVER-STUCK` within that bound rather than hanging or escalating to
a raw signal.

Negative: five planted mutations, each of which a named case above must fail
against, and a guard that stays green against any of them is not enforcing.

- A variant that reaps the leader as soon as it exits: the **order test**
  must fail, on the reap call preceding the group SIGKILL. This is the
  early-reap negative, and it is deliberately not carried by the
  surviving-descendant case.
- A variant that sends the group SIGKILL as soon as the observation reports
  the leader exited, shortening the grace: the **order test** must fail, on
  the grace elapsed on the backend's clock. This is the informational-only
  negative, and it is separate from the early-reap mutation because a variant
  can hold the leader correctly and still cut the grace.
- A variant that omits `process_group(0)` before exec, so the child inherits
  the wrapper's group: the sibling-survival case must fail, because the
  sibling planted in the wrapper's own group is killed by the expiry signal.
- A variant that signals the wrapper's own process group identifier directly
  while still spawning the child into its dedicated group: the
  sibling-survival case must fail here too. This mutation exists separately
  because it is the failure that survives a correct spawn, and a test that
  only checks group creation would miss it.
- A variant that signals the planted server identifier read from the PID
  file: the decoy-alive case must fail.

**Deadline conversion.** Table-driven over the parser both ends share, and
the table tests the grammar the parser actually accepts rather than a
stricter one. Accepted: a well-formed reading with the measured two
fractional digits converts to the expected millisecond count, **and a field
with no fractional part at all converts to an exact multiple of one thousand
milliseconds**, because the grammar is `<digits>` optionally followed by `.`
and one or more digits, so the fractional part is optional and an integer
field is well formed. Refused: a missing field, an empty field, a separator
with no digits after it, a sign, an exponent, a second separator, non-ASCII
digits, and any trailing content; and a value that overflows the checked
conversion. Every refusal asserts the message does not contain the
rejected input. Rounding direction is asserted, not assumed: capture
truncates, read rounds up, the child timeout rounds down, and a case built
from the 15-minute ceiling asserts the resulting bound never exceeds it. A
separate case asserts that a `deadline_ms.checked_sub(now_ms)` of `None` and
a result of `Some(0)` both take the expired-budget path with the duration
report and the ceiling-miss remedies, and that neither is reported as a bad
value. Negative: a planted variant that parses with a floating-point
conversion, one that rounds the child timeout up, one that refuses the
integer field, and one that reports an expired budget as malformed input are
all rejected.

**Recovery messages.** Table-driven over every refusal path in sections 8 and
11: each is fed an absolute worktree path, an output-base hash, a process
identifier, a raw deadline value, and an opaque handle, and the emitted
message is asserted to contain none of those planted values as a substring,
to contain the exact static code, and to contain the repository-relative
remedy that **the section owning that code** maps to it, and not the remedy
of another code. The mapping is read per code from its own source section,
never from one section for all of them: section 8 owns
`D2B-BZLCLEAN-TRACKED`, `D2B-BZLCLEAN-SYMLINK`, `D2B-BZLCLEAN-ESCAPE` and
`D2B-BZLCLEAN-LIVE`; section 11 owns `D2B-BZLSERVER-STUCK` and the
expired-budget and ceiling-miss reports. Each row asserts the exact steps:

- `D2B-BZLCLEAN-TRACKED` (section 8): the dry run, removing or relocating the
  unexpected tracked entry from `.scratch/bazel/`, then `make clean`.
- `D2B-BZLCLEAN-SYMLINK` and `D2B-BZLCLEAN-ESCAPE` (section 8): the dry run,
  removing the offending symlink, magic link or escaping layout from under
  `.scratch/bazel/`, then `make clean`; plus the statement that any external
  target is outside managed cleanup and stays untouched and that reclaiming
  it requires separate inspection and independent verification of ownership.
  The message is asserted to carry no instruction to replace the entry with a
  directory, no recursive removal command, and no path.
- `D2B-BZLCLEAN-LIVE` (section 8): closing other Bazel clients,
  `make bazel-shutdown`, then `make clean`, and neither a dry run nor a
  correction under `.scratch/bazel/`.
- `D2B-BZLSERVER-STUCK` (section 11): closing other Bazel clients then
  `make bazel-shutdown`, plus the refusals to delete `.scratch/bazel/` or to
  signal any process identifier by hand.
- Expired budget and ceiling miss (section 11): the measured duration against
  the ceiling and only the two remedies authorized for a ceiling miss, never
  a section 8 cleanup remedy and never "relax the ceiling".

Negative: a planted message variant that interpolates
the rejected path, one that omits the remedy, one that answers
`D2B-BZLCLEAN-LIVE` with the tracked-entry remedy, one that answers
`D2B-BZLCLEAN-SYMLINK` or `D2B-BZLCLEAN-ESCAPE` with a step that removes or
replaces the link's external target, one that tells the operator to replace
the refused entry with a directory, one that carries a recursive removal
command, one that answers `D2B-BZLSERVER-STUCK` with a section 8 cleanup
remedy, one that answers a ceiling miss with a section 8 cleanup remedy, and
one that suggests relaxing the ceiling, must all be rejected. Those planted
variants prove the redaction, forbidden-instruction, and per-code assertions
are not vacuous.

**Workflow structure.** Two assertions land in
`packages/xtask/tests/policy_ci.rs` beside the cache writer-policy assertion
of section 10, over the same committed fixture directory
`packages/xtask/tests/fixtures/ci/`, which stays outside `.github/workflows/`
so the fixtures are not real workflows:

- every promoted Bazel Rust job sets the absolute deadline control section 11
  defines, because an absent control is the unbounded local default and a
  promoted job that omits it runs the required gate with no in-band ceiling
  and no visible failure;
- no `pull_request`-reachable job requests `actions: write`, which is the
  permission the section 10 cache maintenance job holds and the one a pull
  request must never inherit.

Required negative fixtures, each asserted **rejected**, in addition to the
four cache-writer negative fixtures of section 10, which are kept unchanged:

- a Bazel Rust job invoking an approved `make test-bazel-rust-<slice>` target
  without setting the deadline control;
- a `pull_request`-triggered job granting `actions: write`, as two cases, one
  granting it at job level and one at workflow level, because a
  workflow-level grant reaches every job beneath it and a checker that only
  reads job blocks would pass it.

Required positive fixture, asserted **accepted**: a workflow whose Bazel Rust
jobs all set the deadline control and whose `pull_request`-reachable jobs
carry `contents: read` and nothing more. The negatives prove the checker can
fail; the positive proves it has not been tightened into rejecting the shape
the promoted workflow actually has.

## Consequences

1. Positive: the shared dependency graph compiles once instead of once per
   Cargo target directory. That is the entire performance thesis and it is the
   only structural reason to expect the budgets in section 11 to be reachable.
2. Positive: failure attribution improves. A Bazel target failure names the
   target; today a leaf failure names a surface identifier and the contributor
   reads a log to find which of several commands inside it failed.
3. Positive: the supply-chain checks become hermetic and cacheable, and the
   only network-dependent surface in the Rust gate goes away.
4. Positive: the local and continuous-integration entry points converge on one
   Make target running one build system, instead of a Make DAG locally and an
   eight-job matrix remotely with different profile logic for each.
5. Negative: two build systems live in the tree for the duration of the shadow
   stage, and every Rust change must keep both green. This is the direct cost
   of not doing a big-bang cutover and it is accepted deliberately.
6. Negative: the repository takes on a `BUILD.bazel` generator it must
   maintain, and that generator must track `rules_rust` API changes across
   version bumps.
7. Negative: 544, 117 and 161 locked third-party packages begin executing their
   build scripts under Bazel's sandbox rather than under Cargo. Sandbox
   behaviour differences in third-party build scripts are the most likely
   source of unexpected work during implementation.
8. Negative: local disk grows before it shrinks. The Bazel output base and disk
   cache coexist with the Cargo target directories and sccache until
   retirement.
9. Negative: contributors must learn a second tool to debug a gate failure
   during the shadow stage. The mitigation is that `make test-bazel-rust` and
   `make test-rust` both exist and either can reproduce a failure.
10. Neutral: the eighteen execution-manifest surface identifiers, the
    fixture-lane split, the advisory classification of
    `test-performance-budgets`, and the heavy-lane semaphore are all
    unchanged. This ADR changes the executor beneath the Rust gate and nothing
    above it.

### The specific failures this design makes possible

Generic risk sections are noise. These three are the ones this design actually
creates, each with the guard that catches it.

**A repository-scanning test that passes because it scanned nothing, or
compared nothing.** Twenty test files resolve `CARGO_MANIFEST_DIR` and eleven
define a `repo_root()` helper that walks out into the working tree. Under
Bazel those tests see a runfiles tree containing only declared inputs. A policy
scan that finds no files reports no violations and exits zero, which is
indistinguishable from a clean repository; a reproducibility check that
compares two empty output trees finds them identical, which is
indistinguishable from a reproducible generator. Both are already real here:
the committed schema leaf snapshots a directory nothing writes and therefore
compares two empty strings today. The gate stays green while the coverage
evaporates. Guard: every scanning and comparing surface asserts an **exact**
census, not a floor, derived from a committed manifest that a drift check ties
to the repository inventory; the no-bash scan additionally requires the number
of files the walker parsed to equal the manifest size, closing the walker's
silent skip on an unreadable or unparsable file; the schema surface asserts
its twenty-file census in both generations before comparing digests; and each
scan carries a planted control input, in its own negative target, that the
scan must detect. The repository already uses these idioms, in the
`harness = false` discovery that refuses to infer an empty set and in the
nonzero-coverage assertions added for ADR 0051.

**Eviction of the required path's cache, or a deletion deadlock.** The Actions
cache is measured at 8.44 GiB of 10 GB across six entries. Publishing a Bazel
snapshot while the Cargo path is still required evicts rust-cache entries, and
the required Rust leaves fall from a 7m46s critical path toward the documented
43-minute cold cost, into 60-minute job timeouts, on unrelated pull requests.
The failure is worst because it appears on other people's branches with no
connection to the Bazel work. The second form is quieter: the save step
correctly refuses to publish over budget, nothing deletes the retired entries
because a merged commit cannot delete a cache entry, and the promoted path
never gets a warm cache at all. Guard: the shadow stage publishes no cache
entry; the promotion change stops rust-cache writes and a default-branch-only
maintenance job with `actions: write` deletes the retired entries through the
API, confirms headroom, and only then permits the save; unauthorized or
ambiguous deletion fails closed with the entry key and the shortfall; and the
maintenance job's verdict is kept out of the Rust rollup so a cache failure
never reads as a test failure.

**A binary-locating test that exercises nothing, or the wrong thing.**
Twenty-five files locate binaries through `env!("CARGO_BIN_EXE_...")`, which
`rules_rust` does not define. Supplying it as a compile-time string that
resolves at run time to an absent path makes the test fail loudly, which is
fine; supplying one that resolves to a stale artifact makes the test pass
against the wrong binary, which is not. Guard: every migrated binary-locating
test asserts that the located path exists, is executable, and reports the
expected identity before use, and the coverage map records which Bazel target
provides each binary.

## Alternatives considered

- **Stay on Make and Cargo, and keep optimizing.** The current path has been
  optimized hard and recently: nextest adoption, per-leaf sharding, split API
  census targets, removal of redundant `cargo check` passes, a bounded local
  budget with memory awareness. It works. It is rejected because the remaining
  cost is structural: Cargo cannot share one compilation across the concurrent
  invocations that the feature variants and separate workspaces require, so the
  leaves will keep paying for overlapping graphs no matter how well they are
  scheduled. The measured 30m40s of runner time behind a 7m46s critical path is
  the shape of that limit.
- **Bazel as a continuous-integration wrapper only, with local development
  unchanged.** Rejected because it produces the worst of both: contributors
  cannot reproduce a gate failure locally, and the cache that makes Bazel worth
  having is the one a developer's inner loop benefits from most. It also
  violates the repository's existing rule that continuous integration runs
  approved Make targets, which exists so that local and remote are the same
  command.
- **Big-bang replacement of the Rust gate in one change.** Rejected because the
  acceptance evidence this decision requires, ten consecutive equivalence runs
  and an eighteen-surface seeded-failure matrix, cannot be collected without a
  period in which both paths run. A cutover without that evidence would be a
  claim that coverage was preserved, not a demonstration.
- **Migrate Rust and Nix together, as the handoff describes.** Rejected for
  this decision, not on the merits of the end state, but because it couples
  four independent risk surfaces into one gate and mixes the evidence for "did
  we lose test coverage" with the evidence for "does the privileged broker
  still link the same libraries". The narrow slice leaves the broader migration
  strictly better positioned: a working Bazel workspace, a pinned stack, and a
  demonstrated cache policy.
- **`gazelle_rust` for BUILD generation.** Rejected in section 4: it adds a Go
  toolchain and a third-party generator to the trusted build path, and this
  workspace's interesting cases are exactly the ones a generic generator
  handles worst.
- **`crate_universe` in vendored mode.** Rejected because it commits a large
  generated third-party BUILD tree to the repository for a determinism benefit
  the committed Bazel-side lock plus a repin drift check already provides, at a
  much smaller diff. That rejection is about committing a generated tree; it
  does not bear on the build-time vendored source replacement section 6
  materializes for `cargo-deny`, which an action produces and nothing commits.
- **Plain `rules_rust` `rust_test` execution for the migrated suites.**
  Rejected on the corrected baseline in section 7: the main workspace and the
  guest shell runner run one fresh process per test case under cargo-nextest
  today, and a `rust_test` runs its libtest binary once with intra-process
  threads. Accepting it would quietly trade a stronger isolation guarantee for
  a weaker one across two suites, with no guard able to see the trade.
- **A shared sccache or a Cargo-level remote cache instead of Bazel.**
  Rejected because it addresses only compilation reuse and not scheduling,
  test-level caching, or per-target attribution; and because committed code
  already records that sccache is incompatible with this workspace's
  integration tests in continuous integration, since its environment whitelist
  drops `CARGO_BIN_EXE_<name>`.
- **Buck2 or another build system.** Rejected without deep evaluation. Bazel
  has a maintained first-party Rust rule set, a Cargo lock importer, and a
  presence in the pinned nixpkgs. The cost of being wrong about the specific
  build system is bounded by the narrow scope; the cost of a long comparative
  evaluation is not.
- **A custom local resource to serialize the broker suites.** Rejected on
  measurement, not on design: it does not serialize anything in Bazel 8.6.0.
- **Reaping the timed-out Bazel client as soon as it exits, and ending the
  escalation there.** Rejected: a process group outlives its leader, so a
  leader that exits during the SIGTERM grace can still have descendants
  running, and an escalation that stops at leader exit leaves them with no
  further signal aimed at them. The measured correction to an earlier draft
  of this ADR is that the reap itself is not what would lose them: after the
  leader was reaped with a descendant still running, the group stayed
  addressable and a group SIGKILL still killed the descendant. What binds is
  therefore the unconditional final group SIGKILL. Holding the leader
  unreaped until after that SIGKILL is kept as well, at a cost of one delayed
  `wait`, because it closes the identifier-reuse window by construction
  rather than by an argument that has to be re-made per kernel.
- **Cutting the SIGTERM grace short once the leader is observed exited.**
  Rejected for the same reason as the bullet above, and separately, because a
  variant can hold the leader correctly and still shorten the grace: the
  descendants still inside their own SIGTERM handling are exactly who the
  remaining grace is for, and the leader's exit carries no information about
  them. The observation is informational only; the grace is a fixed period
  waited in full.
- **Carrying the deadline as a floating-point value, or as the raw
  `/proc/uptime` field.** Rejected because the raw field is fixed-point
  decimal and fails an integer validator outright, and because a float
  carries an unspecified rounding direction across the export boundary: a
  deadline that rounds up is a ceiling that is quietly exceeded. Integer
  milliseconds with a stated rounding direction at every step is checkable by
  reading the code.
- **A new top-level shell gate for the section 14 guards.** Rejected: it
  would add a Layer-1 job, a required-context question and a bash surface
  ADR 0017 forbids, to carry assertions that three existing Rust and policy
  surfaces already carry.

## Invariants this decision creates

1. Bazel is the Rust build and test scheduler for the surfaces in the coverage
   map, and for nothing else. It does not build Nix outputs, package artifacts,
   images, or release binaries under this decision.
2. `Cargo.toml`, `Cargo.lock` and the two `rust-toolchain.toml` files remain
   the authoritative dependency and toolchain inputs. A dependency or toolchain
   change is a Cargo-file change followed by regeneration.
3. Every one of the eighteen baseline execution-manifest surfaces has exactly
   one Bazel carrier, and a guard fails closed on an unmapped identifier, a
   missing target, an unmapped test target, or an unlisted hand-written
   fragment.
4. Every logical check is an independently reported Bazel target. No aggregate
   shell wrapper may collapse several surfaces into one result.
5. The versioned execution-manifest contract is unchanged: the same surface
   identifiers, the same partial evidence on failure and interruption.
6. Pull requests never publish a shared cache entry and never hold
   `actions: write`, and a structural policy test with committed negative and
   positive fixtures enforces both. Only one default-branch job writes.
7. Cache credentials never enter a `run:` step or the Bazel process
   environment. No remote cache and no remote execution are configured.
8. The Bazel output base is never cached as a blob. The action cache and the
   repository/download cache are separate entries with separate keys.
9. Cache keys bind the Bazel version, module lock, `.bazelrc`, both toolchain
   pins, all Cargo locks, all deny configurations, the advisory-database pin,
   and the generated BUILD tree digest.
10. Retired and superseded cache entries are deleted through the Actions API
    by a default-branch-only maintenance job before a new snapshot is saved,
    never by merging a commit and never by waiting for eviction. That job's
    verdict is separate from the Rust test verdict in both directions.
11. The three privileged-broker feature suites never run concurrently with each
    other or with other tests, and relaxing that requires a dedicated isolation
    review.
12. The main workspace and the guest shell runner run one fresh process per
    test case; the three broker feature passes run one process per binary with
    bounded threads. A repository-owned runner over Bazel-built test binaries
    enforces the declared topology, an exact per-binary census, faithful
    ignored-case reporting, and bounded concurrency, with no shell in the
    execution path.
13. Every scanning and comparing surface asserts an exact census derived from a
    committed manifest that a drift check ties to the repository inventory. A
    minimum count is a fallback only for a surface with no derivable manifest,
    and its absence must be recorded in the coverage map.
14. `cargo-deny` runs only `bans licenses sources` against a materialized
    offline vendored source replacement, `cargo-audit` carries advisories
    against the pinned database, and no Bazel action is authorized to use the
    network. The union of the two is the ADR 0009 supply-chain policy, and a
    recorded outcome comparison proves the union is unchanged.
15. A missed performance ceiling blocks promotion or fails a job in band with
    the measured duration and the two remedies authorized for a ceiling miss.
    It never licenses reducing coverage, reclassifying an enforcing surface as
    advisory, or
    relaxing the ceiling. That two-remedy restriction is scoped to a ceiling
    miss and to nothing else: it never suppresses or replaces the static
    recovery actions invariant 19 requires. The exported control is the
    absolute deadline `anchor + ceiling - checkout allowance` in integer
    milliseconds, with the
    anchor taken after checkout, so `checkout_actual + post_checkout` stays at
    or under the ceiling. Both ends convert the fixed-point `/proc/uptime`
    field through one repository-owned checked parser that accepts an
    optional fractional part and refuses malformed
    and overflowing values without echoing them, and every rounding step is
    conservative: capture truncates, read rounds up, the child timeout rounds
    down. A deadline that has already passed at read time is an expired
    budget reported on the ceiling-miss path, not a malformed-input refusal.
    The in-band bound is always a relative duration derived from that
    deadline at read time, never the deadline itself. A structural policy
    test, with a committed negative fixture that omits the control, asserts
    every promoted Bazel Rust job sets it. The bounding wrapper
    spawns its Bazel client into a dedicated process group created before
    exec, signals only that group, observes leader exit with a non-consuming
    and non-blocking wait (`EXITED|NOWAIT|NOHANG`) so a leader that has not
    exited cannot park the wrapper inside the wait and overrun the grace,
    treats that observation as **informational only** so it can never shorten
    the fixed SIGTERM grace, waits that grace in full before the final group
    SIGKILL whether or not the leader has exited,
    holds an exited leader unreaped through the
    final group SIGKILL so its identifier cannot be reused, reaps the direct
    child only after that SIGKILL, and never signals its own process group or
    a server process identifier read from a file. Skipping the escalation
    requires a complete enumeration proving the group empty; leader exit and a
    liveness probe are not that proof.
16. Local Bazel state lives inside the worktree scratch tree, is size and age
    bounded, and is never deleted without first shutting the server down with
    the same startup options. Deletion is anchored on a descriptor for the
    worktree scratch tree, resolves the Bazel subtree beneath that descriptor
    with no symlink or magic-link traversal, unlinks descriptor-relative
    without returning to string path resolution, refuses any subtree holding a
    git-tracked file, and reaches nothing but the Bazel subtree. Every
    descriptor the cleanup path opens is opened `O_CLOEXEC`, on the `openat2`
    route as well as the fallback, so no cleanup descriptor is inherited
    across an exec.
17. The required continuous-integration context name `test-rust` does not
    change across promotion.
18. `.github/workflows/pr-bazel-rust.yml` existing means promotion is
    incomplete.
19. A cleanup refusal and a stuck Bazel server each fail with a stable static
    error code and a fixed message that carries no absolute path, output base
    or output-base hash, user identifier, process identifier, raw deadline
    value, or opaque handle, and each names the exact repository-relative
    recovery **for its own code**, never one generic recovery for all of
    them. `D2B-BZLCLEAN-TRACKED` directs the operator to
    `D2B_CLEAN_DRY_RUN=1 make clean`, to removing or relocating the
    unexpected tracked entry from `.scratch/bazel/`, then to `make clean`.
    `D2B-BZLCLEAN-SYMLINK` and `D2B-BZLCLEAN-ESCAPE` direct the operator to
    that same dry run, to removing the offending symlink, magic link or
    escaping layout from under `.scratch/bazel/`, then to `make clean`, and
    state that any external target is outside managed cleanup, stays
    untouched, and may be reclaimed only after separate inspection and
    independent verification of ownership; they never direct a replacement
    directory, never carry a recursive removal command, and never name a
    path. `D2B-BZLCLEAN-LIVE` directs the operator
    to close other Bazel clients, run `make bazel-shutdown`, then rerun
    `make clean`, and never to inspect or correct the tree first.
    `D2B-BZLSERVER-STUCK` directs the operator to close other Bazel clients
    and run `make bazel-shutdown`. Deleting `.scratch/bazel/` or signalling
    any process identifier by hand while a shutdown is unresolved is never an
    authorized remedy, and the ceiling-miss remedy pair in invariant 15 never
    replaces any of these.
20. Invariants 15, 16 and 19 are carried by enforcing tests that land with the
    plumbing they constrain, not at promotion: Layer-1 Rust tests in the crate
    that owns the plumbing, source-shape assertions in
    `packages/d2b-contract-tests/tests/policy_docs.rs` extending the existing
    ban on path-based recursive cleanup, and workflow-shape assertions in
    `packages/xtask/tests/policy_ci.rs`. Each guard ships with a planted
    negative fixture or mutation proving it fails when the invariant is
    removed, placed where the mutation is observable: the `O_CLOEXEC`
    mutation is carried by the behavioural exec-inheritance test rather than
    by a source marker, and the early-reap mutation is carried by an
    escalation-order test rather than by the surviving-descendant case. No
    new top-level shell gate, Layer-1 job, or Make target carries
    them.

## References

- `Makefile`, the `test-rust` DAG and its leaf targets
- `tests/test-rust.sh`, the nine leaf modes and their surface identifiers
- `tests/layer1-jobs.json`, the authoritative job list and enforcement
  classification
- `.github/workflows/pr-l1-static-fast.yml` and
  `tests/ci/layer1-workflow.template.yml`
- `docs/reference/test-execution-manifest.md`, the baseline surface set
- `docs/contributing/gates-and-lints.md`, the Rust budget and execution
  manifest section
- `packages/xtask/tests/policy_ci.rs`, the approved Make target list and
  workflow allowlist
- `packages/d2b-contract-tests/tests/policy_docs.rs`, the secure-cleanup
  marker set and the ban on path-based recursive cleanup
- `packages/d2b-exec-runner/src/service_mode.rs`, the committed
  `process_group(0)` spawn and `signal_process_group` reuse argument
- `tests/unit/meta/ci-coverage.sh`, the workflow shell and checkout-credential
  structural gates
- [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md), toolchain, MSRV and
  supply-chain policy
- [ADR 0000](0000-repository-layout-and-rust-bootstrap.md), repository layout
