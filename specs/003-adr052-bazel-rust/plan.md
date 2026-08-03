# Implementation Plan: ADR 0052 Bazel Rust Gate

**Branch**: `adr052-bazel-rust-spec` | **Date**: 2026-08-03 |
**Spec**: [spec.md](./spec.md)

**Input**: The amended ADR 0052 and the feature specification in this
directory.

## Summary

ADR 0052 is accepted and amended as of 2026-08-03. The amendment corrected five
mechanics the first draft could not implement and two supporting statements
that were wrong about the build substrate; it is merged and is settled
authority for this plan. Add Bazel 8.6.0 beside the authoritative Cargo Rust
gate, preserve the exact eighteen-surface execution and isolation contract,
collect qualification evidence on protected `v3`, and promote only after every
mechanical gate passes. Cargo manifests, four workspace locks, policy files,
and the stable and nightly toolchain pins remain authoritative. Promotion
preserves the `test-rust` context and the Make interface. Alias removal and
Cargo implementation retirement are later independent changes, and neither
removes a public entry point name.

## Technical Context

**Language/Version**: Rust 1.97.0 for the gate and `nightly-2026-02-16` for the
API census. The nightly channel is reached by a repository-owned per-target
transition over the census subgraph, never by a global channel flag.

**Primary Dependencies**: Bazel 8.6.0 from pinned nixpkgs; Bzlmod with
`MODULE.bazel.lock` under `common --lockfile_mode=error` and
`common --check_direct_dependencies=error`; `rules_rust` 0.73.0
as the initial explicit pin, including `crate_universe`, subject to a W0
measurement scoped to this repository's graph rather than to basic version
compatibility, which upstream already publishes; the `cargo-bazel` generator
consumed from its registry-pinned URL and sha256 with the source-bootstrap
fallback refused; repository-owned `cargo xtask gen-bazel`, topology runner,
deadline wrapper, cleanup plumbing, dual-mode test locator, `rustdoc_json`
rule, channel transition rule, and vendor repository rule; and the existing
Cargo, nextest, cargo-deny, cargo-audit, RustSec, Make, and workflow
generators. The dev shell must expose `bazel_8` and `bazel-buildtools` before
any Bazel target lands; Bazelisk is not on the gate path and is not required.

**Storage**: Committed module lock, four per-hub `crate_universe` Bazel-side
locks, generated `BUILD.bazel` files, generated `.bazelignore`, governed-source
and coverage-map artifacts, derived censuses, the build-script and
action-environment inventory, and migration evidence summaries. Local output,
action cache, and repository cache live only below `.scratch/bazel/`, reached
through absolute startup options the wrapper supplies. Promoted action and
download caches are separate; the output base is never cached.

**Testing**: Existing Layer-1 Rust tests, `test-policy`, `test-drift`,
`test-fixture-contracts`, workflow policy tests, and the Cargo Rust gate. New
behavior uses the lowest existing surface. No new shell gate and no new Layer-1
job.

**Target Platform**: Linux `x86_64-linux`. Local reference: at least 12
physical cores, 32 GiB available RAM, and SSD/NVMe. CI reference:
`ubuntu-latest`, 4 vCPU and 16 GiB.

**Project Type**: Internal monorepo build, test, policy, and delivery
orchestration. No runtime daemon, broker, VM, package, image, fixture, or
release contract changes.

**Promotion Lineage**: Protected `v3`, settled by the merged ADR amendment. A
qualification record is a push event on `refs/heads/v3` produced by a merged
pull request, carrying the head commit, both workflow run identifiers, both
rollup verdicts, the same-commit fixture-contract companion verdict, and, for a
cold sample, the four slice durations. Pull-request runs are diagnostic and
stay path-filtered. W0 verifies the amended ADR commit is present in its base
and refuses to proceed otherwise; no task in this feature amends the ADR.

**Performance Goals**: Three warm local runs: median at most 10 minutes and
maximum at most 12. Three cold local runs: median at most 15 and maximum at
most 18. Five most recent qualifying cold qualification records: median at most
15 and none above 18. The cold continuous-integration ceiling does not become
binding until the W3 feasibility measurement records it as attainable on the
real runner class; the only pre-authorized answers to a shortfall are a larger
runner class or a further disjoint slice split. A promoted job has a 15-minute
total ceiling, 2-minute checkout allowance, 13-minute post-checkout in-band
window, and 17-minute outer backstop. The `api` slice's profiles carry the
second configuration the channel transition creates.

**Constraints**: Cargo and toolchain inputs stay authoritative; no network from
any Bazel action, no remote cache or execution, no fixture migration, no
Nix/package/release migration, no new linter, formatter, or hook, no new
Layer-1 job, and no new required context. Repository-rule fetch is permitted
and is always pinned by URL plus checksum or by git rev.
`CARGO_BAZEL_REPIN`, `REPIN`, and `CARGO_BAZEL_REPIN_ONLY` are never set in the
Make wrapper or in continuous integration. The single exception is the scoped
child environment `cargo xtask bazel-repin --hub <name>` constructs for the one
Bazel process it spawns; that command is not a Make target, no workflow may
reach it, and a structural guard allowlists exactly that one construction site.
`cargo xtask bazel-module-refresh`, `cargo xtask bazel-yanked-refresh`, and
`cargo xtask bazel-evidence prepare-cold-local` are likewise not Make targets
and unreachable from any workflow; `cargo xtask bazel-yanked-check` is offline,
runs inside the three supply-chain carriers as a declared-input action, and is
also not a Make target. The structural guard refuses all five by name in
`Makefile` and in `.github/workflows/`.
`bazel-yanked-refresh` is the one repository command that opens a socket, and
it does so only through the `IndexClient` implementation of the
`YankedIndex` boundary; no gate action reaches that boundary.
Every hub sets `lockfile`, `cargo_lockfile`, and
`skip_cargo_lockfile_overwrite = True`, because the last of those defaults to
false at `rules_rust` 0.73.0 and a repin would otherwise rewrite the
authoritative Cargo lock. No `.bazelrc` line and no wrapper
argument sets `@rules_rust//rust/toolchain/channel`. `.bazelrc` carries only
`common`, `build`, `test`, and `build:<config>` lines; every startup option is
supplied by the wrapper as an absolute path and is byte-identical across
`build`, `test`, `query`, `info`, `shutdown`, `clean`, and, from W2, the
repin and module-refresh children.
`D2B_RUST_BUDGET` remains the only resource control, and custom Bazel local
resources are inert because `--local_test_jobs` discards tag-derived resources.
`--test_output=streamed` is forbidden during any measured run. Any change to
the action-environment allowlist invalidates the entire action cache and must
be charged against the 4 GiB promoted budget in the same review. Any Bazel
version bump reopens the disk-cache garbage-collection design review rather
than being an ordinary version bump. Local action cache: 8 GiB/14 days; local
repository cache: 2 GiB; output-root soft/hard marks: 20/40 GiB. Promoted
action/download snapshots: 4/1 GiB, with repository use plus planned snapshot
at most 8 GiB.

**Scale/Scope**: Exactly eighteen execution-manifest IDs, four CI slices
(`main`, `api`, `broker`, `aux`), four `crate_universe` hubs over
`packages/Cargo.lock`, `packages/d2b-priv-broker/Cargo.lock`,
`packages/d2b-guest-shell-runner/Cargo.lock`, and
`tests/tools/no-bash-ast-walker/Cargo.lock`, plus `packages/Cargo.guest.lock`
as a generator and cache-key input that is not a hub. 56 main workspace
members, 205 integration-test files, and 912 tracked Rust files at the ADR
measurement. The governed no-bash input set, the executed harness-free census,
the doctest census, and the emitted schema census are all generator-derived and
drift-checked; no literal count in this plan is normative. The locator
migration covers 25 files that locate binaries through compile-time Cargo
environment expansion and 20 test files that resolve `CARGO_MANIFEST_DIR`, 11
of those through a `repo_root()` helper. Two fixture-backed surfaces remain on
Cargo/Nix and are carried as a required same-commit companion verdict.

## Constitution Check

### Pre-research gate

| Principle | Result | Basis |
| --- | --- | --- |
| I. Daemon-Only Control Plane | PASS | No service, unit, per-VM work, or runtime path is added. |
| II. Broker-Mediated Privilege | PASS | Validation is unprivileged. Cleanup acts only as the invoking user below `.scratch/`. |
| III. Reasonable Isolation | PASS | Broker suites retain exclusive per-binary topology; main and guest retain per-case processes with per-case temporary directories. |
| IV. Contract-Driven Compatibility | PASS | Execution-manifest v1 is reused unchanged. The amended ADR is a merged prerequisite that W0 verifies mechanically; generated, Make, and context contracts are guarded. Retirement removes implementations, never public names. |
| V. Test-Layer Discipline | PASS | Existing Rust, policy, drift, and workflow-policy surfaces carry coverage. The coverage guard is split so each half runs where it can actually execute. |
| VI. Panel-Gated Work | PASS | Every wave has plan and diff gates with all ten roles, with the `software` seat filled by the Bazel and `rules_rust` expert. Green tests waive nothing. This plan declines pipelining. |
| VII. Traceable Artifacts | PASS | Markers stay in planning, commits, and PRs. Code waves carry semantic fragments and ASCII hyphens. |

Broker topology, per-case evidence publication, binary location, execution
evidence, cleanup safety, cache permissions, and shell-free repository-owned
execution are load-bearing and receive positive and planted negative guards
before shadow use.

### Post-design gate

All seven principles still pass. Phase 1 adds only internal migration
contracts, defers to execution-manifest v1, uses existing Layer-1 carriers,
separates evidence collection from promotion, and blocks implementation until
the merged amended ADR is verified present. There is no violation.

## Project Structure

### Planning artifacts

```text
specs/003-adr052-bazel-rust/
|-- plan.md
|-- research.md
|-- data-model.md
|-- quickstart.md
|-- contracts/
|   |-- README.md
|   |-- make-target-compatibility.md
|   |-- coverage-map.md
|   |-- runner-environment.md
|   |-- workspace-and-tool-pinning.md
|   |-- execution-manifest-binding.md
|   |-- shadow-promotion-evidence.md
|   |-- cache-workflow-boundaries.md
|   `-- recovery-deadline.md
|-- evidence/                       # Added from W4; summaries only
|   |-- qualification.json          # W4 immutable qualification record set
|   |-- promotion-record.json       # W5 promotion outcome
|   `-- post-promotion.json         # Independent release/run clocks
`-- tasks.md                        # Phase 2 task list
```

### Expected implementation locations

```text
.bazelversion
.bazelrc                              # common/build/test/config lines only
.bazelignore                          # Generated; covers .scratch/ and all Cargo output dirs
MODULE.bazel
MODULE.bazel.lock
Makefile                              # Supplies all absolute startup options
flake.nix                             # Dev-shell tools only: bazel_8, bazel-buildtools
bazel/                                # Repository-owned Bazel rules/helpers
bazel/cargo/                          # Four crate_universe hubs and per-hub locks
bazel/rules/channel_transition.bzl    # Per-target nightly transition over the census
bazel/rules/rustdoc_json.bzl          # JSON render plus emitted toolchain version
bazel/vendor/                         # Vendor repository rule and lock classification
bazel/supply_chain/                   # Offline deny/audit carriers and the yanked snapshot
bazel/carriers/                       # Carrier fragments consumed by ci/rust
ci/rust/BUILD.bazel                   # ADR-fixed carriers, guard, and aggregate
packages/**/BUILD.bazel               # Generated first-party targets
tests/tools/no-bash-ast-walker/BUILD.bazel   # Generated; fourth hub consumer
packages/xtask/src/bazel.rs           # gen-bazel, gen-bazel --check, bazel-repin, bazel-module-refresh
packages/xtask/src/bazel_yanked.rs    # Yanked refresh and check; YankedIndex seam
packages/xtask/tests/bazel_module_refresh.rs # Real-Bazel module lock drift and remediation
packages/xtask/src/                   # Schema output and evidence helpers
packages/xtask/tests/policy_ci.rs
packages/xtask/tests/fixtures/ci/
packages/d2b-contract-tests/tests/policy_docs.rs # W0 wave-note lint; W2 source-shape tests
specs/003-adr052-bazel-rust/wave-notes/ # w0.md, w1.md, w2.md; one writer each
packages/d2b-bazel-support/src/fsops.rs    # Injectable FileSystem: writer, cleanup, providers
packages/d2b-bazel-support/src/runfiles.rs # Injectable RunfilesView for locator and runner
packages/d2b-bazel-support/src/startup.rs  # The one absolute startup-option construction
packages/d2b-bazel-runner/src/clock.rs # Injectable Clock and UptimeSource for deadlines
tests/unit/meta/w0-dep-direction.sh    # Extended with the build-tooling direction rule
packages/                             # Support, runner, locator crate paths fixed by W0 prep
tests/golden/bazel-rust-coverage.json
tests/golden/bazel-rust-query.json     # Committed drift-checked query result
tests/test-rust.sh                     # Cargo authority; fixture mode survives
tests/layer1-jobs.json                 # Promotion only
tests/ci/layer1-workflow.template.yml  # If generator input requires it
.github/workflows/pr-bazel-rust.yml    # Added W3, deleted W5
.github/workflows/pr-l1-static-fast.yml
changelog.d/                           # One unique semantic fragment per code scope
```

`gen-bazel` owns all generated BUILD files, `.bazelignore`, the governed-source
manifest, every derived census, and the build-script and action-environment
inventory. ADR 0052 fixes labels and ownership, not every helper filename. W0
prep selects the support crate, the runner crate, the locator crate, and exact
helper paths before parallel worktrees open.

### Dependency direction among the build-tooling crates

Three internal crates carry this migration's Rust code, and their edges are a
closed set rather than a convention:

```text
packages/d2b-bazel-support/   <- packages/d2b-bazel-runner/
                              <- packages/d2b-test-locator/
                              <- packages/xtask/            (from W2)
```

`d2b-bazel-support` is neutral: it declares no first-party dependency at all.
Every other edge among these three and `xtask` is refused, in particular
`xtask -> d2b-bazel-runner`, which an earlier draft would have created when the
wrapper's startup-option construction was going to live in the runner. That
edge is wrong in direction: `xtask` is the generator and the contributor
command surface, and the runner is a consumer of the graph `xtask` generates,
so a runner dependency makes the generator's own build depend on the thing it
generates targets for. Moving the shared construction into a neutral crate is
what removes the temptation rather than documenting it away.

The rule is enforced by the repository's existing crate-granular gate,
`tests/unit/meta/w0-dep-direction.sh`, which `tests/test-policy.sh` and
`tests/static.sh` already run. That gate resolves dependencies with
`cargo metadata --no-deps`, so it sees the real resolved crate name after a
`package =` rename, after workspace inheritance, and for target-specific
entries, and it already fails closed when the resolver cannot run. A
manifest-text scan in a Rust policy test would miss exactly those three forms,
which is why the guard extends the existing resolver-backed gate instead of
starting a parallel one. FR-053 forbids a new top-level shell gate; this adds
no gate, no Layer-1 job, and no required context.

W0 extends that gate with: `d2b-bazel-support` declares no workspace member and
no `d2b`-prefixed crate; `d2b-bazel-runner` and `d2b-test-locator` declare
`d2b-bazel-support` and nothing else first-party; `xtask` declares neither the
runner nor the locator in any dependency kind, so a dev-dependency cannot
smuggle the edge back through `packages/xtask/tests/`; the only members that
may declare `d2b-bazel-support` as a non-dev dependency are the runner, the
locator, and `xtask`, and no member at all may declare `d2b-bazel-runner`
outside the runner's own targets; and the gate carries a required-crate list
naming all three and refuses when any of them is absent from the resolver's
member set, rather than falling through its existing "not a workspace member
yet" skip, because a silent skip on a misspelled crate name is the one path by
which a direction gate passes while enforcing nothing. That required-crate
assertion is satisfiable only on the integrated tree, since the runner and the
locator become members at integration, so the planted-edge observations happen
there rather than inside a single scope worktree, and the list is never
weakened to make one worktree green.

`d2b-test-locator` is deliberately outside that reverse rule: the W1 migration
makes it a **dev-dependency** of every first-party crate whose tests locate a
binary or a fixture, which is the whole point of the crate, and the existing
gate already filters dev edges out of the direction check.

## Spec Corrections

| Drift | Canon retained | Treatment |
| --- | --- | --- |
| ADR 0009 names `tests/static.sh`, old flake checks, and Rust 1.94.1. | Current Make DAG, `tests/test-rust.sh`, current checks, Rust 1.97.0. | No code is realigned to ADR 0009. |
| Current schema leaf snapshots `packages/xtask/out`, which does not exist, so two empty snapshots compare equal. | Record current behavior; it is not valid reproducibility evidence. | W0 adds `--out-dir`, a generated emitted census, and exact nonempty checks. |
| Planning prose previously fixed the schema census at twenty. | The generator's returned manifest is the census. | The literal is removed as normative. The current `docs/reference/schemas/v2/` tree holds more entries than that literal, which is exactly why a literal cannot be the census. |
| Planning prose previously fixed six harness-free targets. | The set the gate executes under its current selector. | The census is generator-derived. Four `fuzz`-gated `[[test]]` entries in `packages/d2b-core` are out of census because `fuzz` is not a default feature, and the `packages/d2b-zone-routing` `[[bench]]` entry is out of census because discovery filters `bench` kinds. Both exclusions are recorded with reasons. |
| An earlier plan draft counted three Cargo workspaces and locks. | Four independently resolved workspaces feed the gate, plus `packages/Cargo.guest.lock`. | The walker gets its own hub and Bazel-side lock. The guest lock stays a generator and cache-key input, not a hub. |
| ADR prose once described reading crate archives out of the Bazel repository cache. | The cache has no enumeration interface; the amendment corrects this in place. | The vendor tree is produced by a repository rule that re-declares each download by URL and lock checksum. |
| ADR prose once generalized that custom local resources are not a serialization mechanism. | They are inert specifically because `--local_test_jobs` discards tag-derived resources, which this configuration always sets. | Recorded as the narrower, durable statement. |
| GitHub default branch is `main`, while protected `v3` never merges to `main`. | Repository branch policy and the `v3` promotion lineage. | Settled by the merged ADR amendment. W0 verifies the amendment is present rather than proposing it. |
| Workflow prose predates constitution pipelining. | Constitution 2.1.0 controls. | Use the stricter serialization. |
| An earlier plan draft validated a wave with `make test-rust-main` or `make test-policy` alone after editing a file the `d2b-contract-tests` crate reads. | `tests/test-rust.sh` sets `workspace_test_excludes=(--exclude d2b-contract-tests)`, and `policy_broker_schema.rs` walks every `.rs` file under `packages/`, so that crate executes only under `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts` and every wave that touches `packages/` is inside its input set. | Every code-changing wave now runs the fixture-contract target in its validation. The rule and the command that derives the input set are in the delivery rules above and in `tasks.md`. |
| An earlier plan draft treated the yanked-state carrier as conditional on the recorded comparison. | The amended ADR 0052 section 6 makes the snapshot and the three carriers unconditional. | The carrier lands in W1 either way; the comparison keeps its promotion-blocking force but no longer decides whether the capability exists. |
| An earlier plan draft forbade the repin controls without naming any supported regeneration path. | `rules_rust` 0.73.0 `determine_repin` treats `CARGO_BAZEL_REPIN_ONLY` as an exact-match comma-delimited hub allowlist, and `skip_cargo_lockfile_overwrite` defaults to false. | `cargo xtask bazel-repin --hub <name>` is the one supported path; the environment prohibition in Make and continuous integration is unchanged, and every hub sets `skip_cargo_lockfile_overwrite = True` so a repin cannot rewrite the authoritative Cargo lock. |
| An earlier plan draft left module lock drift with no named command, so the refusal could only point at Bazel's own diagnostic. | Measured at Bazel 8.6.0: `--lockfile_mode=error` exits 48 naming `bazel mod deps --lockfile_mode=update`, an invocation that carries no startup options and would run against the default output user root under the home directory. | `cargo xtask bazel-module-refresh` issues the measured invocation with the repository's absolute startup options, writes only `MODULE.bazel.lock`, and is the exact remediation the refusal names. ADR 0052 needs no amendment: it names `bazel-repin` as the supported path for a *hub* lock, and the module lock is the separately kept mechanism the ADR already distinguishes. |
| Planning prose treated `--lockfile_mode=error` as sufficient to fail closed on any module-input change. | Measured at Bazel 8.6.0: a direct `bazel_dep` version the graph absorbs produces a warning and exit zero; only a missing registry file checksum fails. | `.bazelrc` gains `common --check_direct_dependencies=error`. A check that warns is not a pin, and W0 proves the negative. |
| Planning prose required byte-identical options across every Bazel invocation without distinguishing startup options from command options. | Measured at Bazel 8.6.0: `bazel mod` rejects `--symlink_prefix` as unrecognized. | The identity requirement binds the startup-option set that selects the server and output base. `--symlink_prefix` is supplied to the commands that accept it, and the module-refresh child is held only to startup-option identity. |
| An earlier plan draft ended the yanked drift remedy at `bazel-yanked-refresh`, leaving verification to continuous integration. | The gate check must be offline and must not regenerate state, so it is a different operation from the networked refresh. | `cargo xtask bazel-yanked-check` is the offline validator, the three carriers run it as a declared-input action, and the recovery text ends on it. |
| An earlier plan draft put the shared absolute startup-option construction in `packages/d2b-bazel-runner/` and had `packages/xtask/Cargo.toml` take a path dependency on the runner to reach it. | Dependency direction: `xtask` generates the graph the runner's targets live in, so `xtask -> d2b-bazel-runner` inverts the relationship. | The construction moves to the neutral `packages/d2b-bazel-support/`, which declares no first-party dependency; `xtask`, the runner, and the locator all consume it, and `tests/unit/meta/w0-dep-direction.sh` refuses the runner edge in every dependency kind. |
| An earlier plan draft proved the locator's stale-provider negative by writing an out-of-date executable to the live Cargo path and removing a runfiles entry. | A guard whose setup writes an executable into `packages/target/` leaves that executable behind when the run is interrupted, in the one directory the shadow stage keeps full of real binaries. | The absent, non-executable, stale, and wrong-identity providers are all supplied states on the injected `FileSystem`, and the missing runfiles entry is a state on the injected `RunfilesView`. No provider test writes to or executes a live path. |
| An earlier plan draft had the wave-note refusal name the whole offending token and modeled absent note content as `Option<String>`. | A refusal is republished into CI output, panel comments, and PR bodies, so echoing the token spreads the leak; and a failed read is a diagnosable error, not a blank. | The refusal carries the note, the one-based line, and one remediation only, proven by running the rendered refusals back through the scanner's own rules; the entry API returns `std::io::Result` at both levels and keeps the real error. |

There is no eighteen-surface drift: committed code publishes eighteen with
`D2B_SKIP_FIXTURE_BUILD=1` and two fixture surfaces when enabled.

## Wave Graph and Delivery Rules

```text
Merged ADR 0052 amendment (prerequisite, not a task in this feature)
  -> W0 foundation -> W1 coverage -> W2 safety -> W3 shadow CI
  -> W4 evidence -> W5 promotion
W5 -> W6 alias removal              # release-containment gate only
W5 -> W7 Cargo retirement           # ten-green-run gate only
```

W0 begins with a mechanical verification that the amended ADR commit is an
ancestor of the W0 base. No implementation branch, generator change, or Bazel
workspace file may predate that commit.

Every scope uses its own worktree and branch, commits before validation, and
owns a unique semantic changelog fragment when code changes. The integrator
merges scope commits into one wave branch and opens one wave PR. Shared
contracts land in an integrator prep commit before parallel scopes open. Each
boundary has a plan panel and an integrated-diff panel with `software`, `test`,
`nixos`, `networking`, `security`, `rust`, `product`, `docs`, `observability`,
and `kernel`. For this delivery run the `software` seat is filled by the Bazel
and `rules_rust` expert, because every finding that forced the ADR amendment
was substrate-level. All ten must sign off with no recommendations. Reviewers
inspect supplied evidence and do not rerun validation. Content changes
invalidate prior signoffs.

### Wave notes record command shapes, not local values

Several waves record a measured invocation in their wave notes: W0 records the
one that repins exactly one hub and the one that updates the module lock, W1
records the reviewed networked yanked-snapshot refresh together with the index
revision it observed, and
W2 records the consolidated startup-option construction. Every such note
records the command as a **shape** with placeholders, for example
`<worktree>/.scratch/bazel` for the output user root and
`<worktree>/.scratch/bazel/<base>` for the output base. A real absolute path
from the machine the measurement ran on is never written into a note, a PR
body, a panel comment, or an evidence file.

This is the same rule that keeps absolute paths and environment values out of
refusal text, applied to the artifacts that describe how those refusals were
measured. A wave note is quoted forward into later waves and into review, so a
home directory path in one has escaped just as surely as an echoed variable
would have. Panels check notes for this, and the redaction fixtures that cover
message text list the note paths in the same forbidden set.

The notes have one fixed home, because a rule about a file no command can name
is not enforceable. Each wave's notes are one committed file under
`specs/003-adr052-bazel-rust/wave-notes/`, named `w0.md`, `w1.md`, and `w2.md`,
and each is owned by the single scope that made the measurement: `generator` in
W0, `aux` in W1, and `local-wrapper` in W2. One writer per file keeps the notes
disjoint in the ownership map without a prep commit.

The rule is carried by a test, not by a ritual. W0 lands a type-5 policy lint
in `packages/d2b-contract-tests/tests/policy_docs.rs` that enumerates every
entry of that directory and refuses four things: an empty corpus, any entry
that is not a readable regular file named `w<digits>.md`, any line that still
holds a `/`-rooted path token once every `<worktree>`-rooted path and every
`http` or `https` scheme-and-authority prefix has been removed, and any line
carrying the worktree's own absolute path, or that path without its leading
slash, as a bare substring. A `<worktree>`-rooted path is the exact literal
`<worktree>` followed by `/`-separated segments, each an ordinary segment or a
further angle-bracket placeholder, so
`<worktree>/.scratch/bazel/<base>/execroot`
is consumed whole rather than leaving `/execroot` behind. The scheme allowlist
is exactly `http` and `https`, so `file:` is refused rather than parsed: a
`file:` URI is an absolute path wearing a scheme, and parsing it would be the
one hole an enumerated denylist of root directories always leaves. Nothing is
allowlisted as a permitted real absolute path, `/dev/null` included, because a
note records a shape and not a transcript. The substring rule is what catches a
leak that arrives with no leading slash at all.

**The refusal itself is redacted.** A violation names the note file, the
one-based line number, and one remediation sentence, and stops there. It never
prints the offending token, not truncated, not summarized, not "first segment
only". This is not a new rule: FR-029 already forbids a refusal message from
carrying an absolute path, and a refusal that echoes the leaked token is
exactly such a message. It is also the worse case, because a lint refusal
travels further than the file it refused: it lands in the
`test-fixture-contracts` output, is pasted into panel comments, and is quoted
into PR bodies and wave notes. A refusal that echoes the leaked absolute path
has published it into three more artifacts than the note did, which is the
exact escape this lint exists to close, and it would do so at the moment
everyone is looking. `w1.md:37` plus "rewrite the path as a `<worktree>`-rooted
shape or drop it" is enough to fix it, because the contributor already has the
file open. The scanner therefore refuses its own rendered refusals: one test
runs every violation's rendered text back through the same path-token and
worktree-substring rules and requires no violation, so a future change that
adds the token to the message fails the lint that added it.

**Absent content is an error, not a blank.** The enumerator returns
`std::io::Result` at both levels: one for reading the directory, and one per
entry holding exactly what `read_to_string` returned for it. It does not
collapse a failed read into `None` or into an empty string. The difference
matters twice. A directory the lint cannot read is a fail-closed refusal rather
than a corpus that happens to look empty, and an entry that is a directory, a
dangling symlink, a permission denial, or non-UTF-8 is refused with its real
errno rather than with one indistinguishable "absent" state, so a reviewer can
tell "someone committed a subdirectory here" from "the file is unreadable in
CI". Preserving the real `std::io::Error` is also what stops a later refactor
from quietly mapping every failure onto the same benign-looking value; an
`Option` has exactly one way to be empty and a `Result` has to say why.

The lint runs in the one lane that reaches that crate,
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`. That is measured, not
assumed: `tests/test-rust.sh` sets `workspace_test_excludes=(--exclude
d2b-contract-tests)`, and `tests/test-policy.sh` runs seven fixture-independent
contract-test binaries, none of which is `policy_docs`. No new gate, shell
script, or `tests/test-policy.sh` entry is added, per FR-053.

The lint carries its own proof that it can fail, and it carries it in-test
rather than on disk. One case plants the generic absolute path
`/home/planted-d2b-note-leak/adr052/.scratch/bazel`, which belongs to no
machine in this run; one passes an empty corpus; one plants the worktree path
with its leading slash removed; one plants a non-note entry name and an entry
whose content is a real `std::io::Error`; and one requires `<worktree>` in
exactly that spelling by refusing `<WORKTREE>`, `<worktree-root>`, and a
`file:` URI while accepting a placeholder-rooted path that carries a nested
`<base>` segment. All five are observed failing against an inert scanner before
the real one lands. A sixth case, the self-application above, requires every
rendered refusal from those same planted inputs to name the entry, the line
where a line exists, and the remediation, and to carry no path token and no
worktree substring of its own.

No wave plants a note fixture in the real directory any more. An untracked leak
file there is either named like a note, in which case the lint reads it, or
named anything else, in which case the lint refuses the entry. That is strictly
stronger than the tracked-versus-untracked scan it replaces, and unlike a scan
a validation task performs by hand, it cannot be skipped by a reviewer who is
in a hurry. Adding `specs/003-adr052-bazel-rust/wave-notes` to this crate's
input set is also what makes the fixture-dependent validation rule below bind
for W0, W1, and W2 by derivation rather than by the incidental `.rs`
intersection.

### Fixture-dependent validation rule

`tests/test-rust.sh` sets `workspace_test_excludes=(--exclude
d2b-contract-tests)`, so the `d2b-contract-tests` crate never runs under
`make test-rust-main` or any other workspace leaf. It runs only under
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`.
`tests/test-policy.sh`
does not close that gap: it runs seven named fixture-independent contract-test
binaries, and `policy_docs` is not one of them. A wave that edits a
file that crate reads therefore has no coverage at all unless its validation
task runs that target.

The crate's committed input set is derived mechanically, not by memory:

```bash
grep -rhoE 'read_repo_file(_opt)?\("[^"]+"' packages/d2b-contract-tests/ \
  | sed -E 's/.*\("//; s/"$//' | sort -u
grep -rhoE 'repo_root\(\)\.join\("[^"]+"' packages/d2b-contract-tests/ \
  | sed -E 's/.*\("//; s/"$//' | sort -u
```

A wave whose owned path set intersects that union MUST run
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts` in its validation
task. Measured at the current base, the union already contains
`repo_root().join("packages")`, which
`packages/d2b-contract-tests/tests/policy_broker_schema.rs` walks for every
`.rs` file, plus `Makefile`, `tests/test-rust.sh`, `flake.nix`,
`packages/Cargo.toml`, `packages/xtask/src/main.rs`, `AGENTS.md`,
`docs/contributing`, and `nixos-modules`. Every code-changing wave in this plan
adds or edits at least one `.rs` file under `packages/`, so W0, W1, W2, W3, W5,
W6, and W7 all carry the target. W0 adds
`repo_root().join("specs/003-adr052-bazel-rust/wave-notes")` to the union when
it lands the wave-note lint, so from W0 onward the wave that writes a note is
bound to the target by derivation rather than by the incidental `.rs`
intersection. W4 owns no implementation file and runs it
anyway, because promotion evidence requires the same-commit companion verdict.
The practical consequence is that the target is not optional for a wave whose
diff looks unrelated to fixtures: a wave that adds a new runner test file has
already changed an input this crate scans. Each wave recomputes the
intersection against its final owned path set rather than inheriting this list.

### W0 - Reversible foundation

**Deliverable**: Verified amendment ancestry, pinned Bazel and Bzlmod state,
four `crate_universe` hubs with committed per-hub locks, the scoped
single-hub repin command, the no-argument module-lock refresh command, the
reviewed networked yanked-snapshot refresh
command, the pinned
`cargo-bazel` acquisition, the generated workspace boundary, the Cargo-derived
generator, the schema output prerequisite, the generated first-party graph, the
coverage-map structure, the third-party build-script and action-environment
inventory, the wave-note policy lint that carries the command-shape rule, and
the frozen support, runner, and locator crate decisions, including the shared
`FileSystem` and `RunfilesView` boundaries, the frozen `startup` seam, the
runner-local `clock` boundary, and the dependency-direction guard that pins the
edges between the three. Cargo remains authoritative.

**Ownership**:

- `foundation-tools`: `.bazelversion`, `.bazelrc`, `MODULE.bazel`,
  `MODULE.bazel.lock`, `flake.nix`, hand-written `bazel/`, `bazel/cargo/` hub
  declarations and per-hub locks, and `packages/xtask/tests/policy_ci.rs` for
  the pinning and repin-control guards. The one exception is the main-hub lock
  after workspace membership changes, which only the integrator regenerates and
  commits; the other three must be byte-identical across integration.
- `generator`: `packages/xtask/src/bazel.rs`, including `gen-bazel`,
  `gen-bazel --check`, `bazel-repin`, and `bazel-module-refresh`;
  `packages/xtask/src/bazel_yanked.rs`, including `bazel-yanked-refresh`
  written against the prep-frozen `YankedIndex` boundary, its `IndexClient`
  implementation, and its fake;
  `packages/xtask/tests/bazel_module_refresh.rs`; their tests;
  the wave-note policy lint in
  `packages/d2b-contract-tests/tests/policy_docs.rs` and the W0 note
  `specs/003-adr052-bazel-rust/wave-notes/w0.md`, which the scope that made the
  measurement writes together with the lint that polices it;
  and its generated outputs including `.bazelignore` and the derived censuses.
- `schema`: schema generator, its tests, and the current schema leaf only.
- `runner`: `packages/d2b-bazel-support/` and `packages/d2b-bazel-runner/`, plus
  `tests/unit/meta/w0-dep-direction.sh`. It lands the support crate complete:
  the `FileSystem` trait with the operation set the runner-environment and
  recovery contracts fix, its host-backed implementation, its in-memory fake,
  the `RunfilesView` trait with its runfiles-backed implementation and its
  in-memory fake, and the empty W0-frozen `startup` module W2 fills. It lands
  the runner crate as a skeleton with the runner-local `clock` boundary W2
  implements against. It also extends the dependency-direction gate, because
  FR-051 puts an enforcing guard in the same wave as the plumbing it
  constrains. One scope authoring both crates does not make the support crate
  less neutral: neutrality here is a dependency property that the gate decides
  mechanically, not an authorship property, and no other W0 scope owns a file
  under either directory.
- `locator`: the frozen locator crate skeleton and its own tests. It consumes
  the support boundaries and declares no other first-party dependency.
- Integrator prep: `Makefile`, Cargo workspace membership,
  `packages/xtask/src/main.rs` seams, the `YankedIndex` trait declaration in
  `packages/xtask/src/bazel_yanked.rs`, coverage-map format, the
  `packages/d2b-bazel-support/` directory and its module files so runner and
  locator open against one resolvable crate, support, runner, and locator crate
  selection, generated output reconciliation, and shared changelog folding.

**Validation**: `make check-tier0`, `make test-lint`, `make test-rust-schema`,
`make test-rust-inventory`, `make test-drift`, `make test-policy`,
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts` per the
fixture-dependent validation rule, and a Cargo-authoritative
`D2B_SKIP_FIXTURE_BUILD=1 make test-rust`.

**Done when**: the amended ADR commit is an ancestor of the W0 base; Bazel is
8.6.0; module lock mode is `error` and direct-dependency checking is `error`;
all four hubs declare `lockfile`,
`cargo_lockfile`, and `skip_cargo_lockfile_overwrite = True`, and a stale-lock
mutation fails closed; no repin environment control exists in the
wrapper or CI; `cargo xtask bazel-repin --hub <name>` refuses an unknown hub,
refuses an ambient repin control, changes only the named hub's lock, and fails
when any other generated artifact changed; `cargo xtask bazel-module-refresh`
takes no arguments, refuses an ambient repin control, changes only
`MODULE.bazel.lock`, fails when any other tracked derived file changed, and
exits zero having changed nothing on an already-current tree; planted module
drift makes the pinned Bazel fail under `--lockfile_mode=error` and the
repository surfaces the exact `cargo xtask bazel-module-refresh` remediation
beside that failure; neither command is named in `Makefile` or in any workflow;
every row of the actionable failure contract is triggered and its exact string
asserted by a test in the module that emits it; the `cargo-bazel` URL and
sha256 are pinned and the source
bootstrap is refused; `.bazelrc` contains no startup line and no channel flag;
generated `.bazelignore` covers `.scratch/` and every Cargo output directory,
proven by a drift mutation; `gen-bazel --check` is clean; `bazel-yanked-refresh`
reaches the index only through the `YankedIndex` boundary, every one of its unit
tests supplies a fake response and none opens a socket, and `IndexClient` is the
one site that may; both schema generations report the exact generated nonempty
valid census; the build-script
and action-environment inventory is committed and drift-checked; the W0 wave
notes record both measured invocations as command shapes with `<worktree>`
placeholders and the wave-note policy lint in
`packages/d2b-contract-tests/tests/policy_docs.rs` passes over every entry of
that directory under `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`,
with its planted absolute-path, empty-corpus, non-note-entry,
worktree-substring, and placeholder-spelling cases all present and each
observed failing against the inert scanner before the real one landed, its
entry API returning `std::io::Result` at both levels rather than an `Option`,
and its rendered refusals carrying the note, the line, and the remediation and
no offending token, proven by running those refusals back through the
scanner's own rules; `packages/d2b-bazel-support/` declares no first-party
dependency, the runner and the locator declare it and nothing else first-party,
and `tests/unit/meta/w0-dep-direction.sh` refuses a planted
`xtask -> d2b-bazel-runner` edge and a planted first-party edge out of the
support crate, both observed failing and reverted;
Cargo remains authoritative and green; the ten-role panel and wave PR are
sealed and merged.

### W1 - Coverage carriers

**Deliverable**: Eighteen attributed carriers with a total and unambiguous
carrier map, four slices, the shell-free topology runner with its full
environment and per-case evidence contract, the dual-mode locator migration,
the nightly channel transition and `rustdoc_json` rule, the vendor repository
rule and offline supply-chain carriers, derived censuses, the split coverage
guard, the manifest adapter, and the six ADR Make targets.

**Ownership after prep**:

- `main`: main format, clippy, tests, doctests, harness-free companions,
  no-bash, schema, stub, and pinned-label carriers.
- `api`: the channel transition rule, the `rustdoc_json` rule, the census
  snapshots, the emitted-toolchain guard, and `packages/xtask/tests/policy_ci.rs`
  for the global-channel-flag refusal.
- `broker`: three feature carriers and their exclusivity.
- `aux`: the guest runner carrier and the offline supply-chain carriers,
  including the vendor repository rule, the unconditional lock-bounded
  yanked-state snapshot and its three carriers,
  `packages/xtask/src/bazel_yanked.rs` for the offline `bazel-yanked-check`
  validator those carriers run and its message tests, and the W1 note
  `specs/003-adr052-bazel-rust/wave-notes/w1.md`.
- `runner`: only the frozen runner crate, its environment and per-case evidence
  implementation, and its tests. It consumes the W0 support boundaries and adds
  no operation to them.
- `locator`: the locator crate and the enumerated first-party migration.
- `coverage`: `ci/rust/BUILD.bazel`, the coverage JSON, the committed query
  result, and the split guard.
- `generator`: `packages/xtask/src/bazel.rs` only, for the W1 emission
  extension. It does not touch `packages/xtask/src/bazel_yanked.rs`, which
  `aux` owns for W1, so the two `xtask` scopes stay file-disjoint.
- Integrator prep: `Makefile`, approved targets, the manifest adapter boundary,
  the `bazel-yanked-check` CLI seam in `packages/xtask/src/main.rs`, and the
  locator's public macro surface, which the migration scope consumes.

No W1 scope owns a file under `packages/d2b-bazel-support/`. W0 landed that
crate complete, which is what lets the `runner` and `locator` scopes, whose
provider and result-file tests both drive the same fake, open in parallel
without one waiting on the other's worktree. If W1 discovers the boundary is
short an operation, it lands in the W1 prep commit and nowhere else; a scope
that quietly extends a shared trait has reintroduced the coupling the prep rule
exists to remove.

Generated BUILD outputs remain generator-owned and are regenerated once after
scope merges.

**Validation**: `make test-bazel-rust`, all four slice targets, an aggregate
with `D2B_EXECUTION_MANIFEST=.scratch/adr052-w1-manifest.json`, `make
test-rust`, `make test-policy`,
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`, and `make
test-drift`.

**Done when**: every baseline ID has a nonempty carrier set and every carrier
belongs to exactly one ID; carrier existence fails at analysis when a mapped
label does not exist; graph completeness and query drift are proven outside the
Bazel test and no Bazel test invokes `bazel query`; censuses and ignored counts
are exact and generator-derived; the census emits the toolchain version it
actually used and the guard compares it to the pin; a global channel flag is
rejected by a guard; the vendor rule refuses an unclassifiable lock entry and
asserts materialized package count equals the lock's; the committed
lock-bounded yanked snapshot exists, `cargo xtask bazel-yanked-check` passes
offline for all three locks from both the carriers and a contributor shell,
that validator names neither `YankedIndex` nor `IndexClient` so it cannot reach
the index at all, the wave notes record the reviewed networked refresh that
produced the snapshot as a command shape plus the index revision it observed
and the W0 wave-note lint passes over the added `w1.md` in the same
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts` run, with no shell
scan and no planted note added here,
its drift refusal names refresh, review and commit, then the check, and it
reports under the existing `rust-deny-*` identifiers without adding a
nineteenth surface; every migrated test resolves its binary
and fixtures through the locator, the four provider negatives and the
stale-provider case are all supplied by the injected `FileSystem` and
`RunfilesView` fakes with no executable written into `packages/target/` or any
other live path, and both arms stay green under Cargo;
per-case JUnit results are published with the canonical redaction set absent
and raw output only in `test.log`; failed and interrupted runs publish partial
evidence; repository-owned runner execution uses no shell, with the
`rules_rust`-generated stable-channel doctest runner recorded as the deliberate
difference it is; Cargo is still authoritative; panel and PR are sealed and
merged.

### W2 - Operational safety

**Deliverable**: Budget propagation, bounded scratch state, absolute
wrapper-supplied startup options with byte-identical reuse, synchronous
on-demand disk-cache trimming, safe cleanup, the deadline and process-group
wrapper, per-code recovery, and all guards. An evidence-only
`cargo xtask bazel-evidence prepare-cold-local` helper prepares the ADR
cold-local state without adding a Make target or a persistent contributor
control. W5 removes that helper after W4 qualification is complete.

**Ownership**:

- `process-control`: deadline, process group, wait ordering, shutdown tests.
- `cleanup`: cleanup modules and tests, plus its `policy_docs.rs` marker block.
- `local-wrapper`: `Makefile`, `.bazelrc`, the one startup-option construction
  in `packages/d2b-bazel-support/src/startup.rs` and its identity tests in
  `packages/d2b-bazel-support/tests/startup_options.rs`, the
  synchronous trim step, scratch budgets, `packages/xtask/Cargo.toml`,
  `packages/xtask/src/bazel.rs` solely to replace the minimal W0 startup-option
  derivation inside `bazel-repin` and `bazel-module-refresh` with the one shared
  construction, and `packages/xtask/tests/bazel_module_refresh.rs` solely to add
  the case proving every command the wrapper issues surfaces the same
  module-lock remediation, plus the W2 note
  `specs/003-adr052-bazel-rust/wave-notes/w2.md`. That
  replacement adds a `d2b-bazel-support` path dependency, which changes the
  generated graph, so W2 gains an integrator regeneration step and
  `make test-drift` in validation. It is the support edge and not a runner
  edge: `xtask -> d2b-bazel-runner` stays refused by the W0
  dependency-direction guard, which is why the construction lives in the
  neutral crate rather than in the runner it was first drafted into.
- `recovery`: the recovery table and its tests only.
- `boundaries`: `packages/d2b-bazel-support/src/fsops.rs` and
  `packages/d2b-bazel-runner/src/clock.rs`. Both are W0-frozen module paths
  whose operation set W0 fixed and W1 consumed, so this scope owns only the W2
  changes to `fsops.rs`, which should be none, and `clock.rs` with its
  in-memory fake; the `fsops` and `runfiles` fakes already exist from W0.
  Cleanup, result publication, and
  deadline handling consume those traits rather than calling the standard
  library directly and rather than adding operations to these two files. No
  cleanup or deadline test may depend on live host filesystem state or on the
  host clock. This scope and `local-wrapper` both open files under
  `packages/d2b-bazel-support/`, and they stay file-disjoint the same way the
  two W1 `xtask` scopes do: ownership in this plan is per file, never per crate.

Prep first splits stable interfaces if process-control and cleanup would share
a file. `clock` lands in the prep commit, and `fsops` and `runfiles` are
already whole from W0, so cleanup, deadline, and result-publication scopes open
against a stable trait surface.

**Validation**: `make test-rust-main`, `make test-policy`, `make check-tier0`,
`make test-drift`, `make test-bazel-rust`, `D2B_CLEAN_DRY_RUN=1 make clean`,
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts` per the
fixture-dependent validation rule, and planted
mutations through their existing Rust and policy carriers.

**Done when**: cleanup, descriptor inheritance and race, signal order,
deadline, redaction, and wrong-remedy mutations fail; no descendant survives;
unrelated processes survive; the 20/40 GiB marks work; startup options are
proven byte-identical across `build`, `test`, `query`, `info`, `shutdown`, and
`clean`, with a mutation that perturbs one of them failing closed; the trim
step is synchronous and its completion is observed before any size measurement;
every cleanup, result-file, and deadline property is proven through the
injected `fsops` and `clock` boundaries with no live-host dependency; the one
startup-option construction lives in `packages/d2b-bazel-support/src/startup.rs`
and both `xtask` commands call it rather than deriving their own, with
`xtask -> d2b-bazel-runner` still refused by the W0 dependency-direction guard;
the W2 wave notes record the consolidated startup-option construction as a
command shape with `<worktree>` placeholders and the W0 wave-note lint passes
over the added `w2.md` in the same `D2B_ENABLE_FIXTURE_BUILD=1 make
test-fixture-contracts` run, with the W2 source-shape work in the same policy
file leaving that lint and its cases byte-identical;
panel and PR are sealed and merged.

### W3 - Shadow CI

**Deliverable**: A non-required four-slice workflow with no restore or save,
workflow and cache permission guards with fixtures, qualification-record
capture, and the cold-CI feasibility measurement that must precede the binding
ceiling. The required Cargo CI is unchanged.

**Ownership**:

- `shadow-workflow`: `.github/workflows/pr-bazel-rust.yml`.
- `workflow-policy`: `policy_ci.rs` and CI fixtures.
- `target-policy`: the approved-target list if it did not land in W1.
- Integrator: triggers, path filters, and workflow allowlist reconciliation.

**Validation**: `make test-rust-main`, `make test-policy`, `make test-lint`,
`make check-tier0`, `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`
per the fixture-dependent validation rule, four Bazel slices, and inspection of
the shadow workflow's
`pull_request` run from a draft W3 PR for permissions, zero writes, slice
verdicts, and rollup attribution.

**Done when**: the workflow is non-required and outside `V3_PR_GATE_WORKFLOWS`;
jobs call approved Make targets only; PR reachability has only
`contents: read`, no writer, and no `actions: write`; shadow publishes nothing;
policy fixtures pass; push events on protected `v3` produce complete
qualification records carrying both run identifiers, the shared head commit,
both verdicts, the same-commit fixture-contract verdict, and the four slice
durations, while pull-request runs produce none; the cold-CI feasibility
measurement is recorded with its four slice durations and either clears the
ceiling or names which pre-authorized remedy is being taken; panel and PR are
sealed and merged.

### W4 - Evidence qualification

**Deliverable**: A reviewed promotion evidence summary, with no executor flip
and no cache publication.

**Ownership**: One curator owns the immutable
`specs/003-adr052-bazel-rust/evidence/qualification.json`. No implementation
source.

**Validation**: Audit ten consecutive matching qualification records, each at a
shared head commit with a passing same-commit fixture-contract verdict;
eighteen isolated seeded failures; exact generator-derived census, topology,
and ignored counts; twenty exclusive broker repetitions; three warm, three
cold-local, and the five most recent qualifying cold qualification-record
measurements; supply-chain equivalence together with the landed yanked carrier
and its passing offline key-set drift check; and zero shadow cache publication.
Run both Rust aggregates, policy, and fixture contracts on the evidence commit.

**Done when**: the Qualification Evidence Record validates every threshold and
reference, no item is pending, and the load-bearing documentation wave gets
unanimous panel signoff and merges. The committed record is immutable.
Qualification evidence and promotion never combine.

### W5 - Promotion

**Entry**: W4 merged; maintenance code and fixtures green; a pre-merge cache
API audit with complete pagination, no ambiguous prefix, no retired writer run
after the audit, and enough projected headroom after a synchronous trim.

**Deliverable**: Keep `test-rust`, switch the eighteen surfaces to Bazel,
retain the Cargo fixture mode, replace eight leaves with four slices, delete
the shadow workflow, stop old writes, delete authorized retired caches, verify
at most 8 GiB after an explicit synchronous trim, and publish through one
protected-`v3` writer. Add the structural assertions the amended ADR requires
as future guards: no `pull_request`-reachable job requests `actions: write`,
and every promoted Bazel Rust job sets the in-band deadline control. Remove the
W2 evidence-only cold-local preparation helper after the qualified W4
measurements have been used.

**Ownership**:

- `promotion-make`: `Makefile`, `tests/test-rust.sh`.
- `promotion-manifest`: `tests/layer1-jobs.json` and generator inputs.
- `cache`: maintenance implementation and fixtures.
- Integrator: regenerate the PR workflow, delete the shadow workflow, order
  shared jobs.

**Validation**: `make layer1-workflow`, `make test-drift`, `make check`,
fixture contracts, alias status tests, deadline policy, a maintenance dry run,
and the first ordered protected-`v3` maintenance and save run.

**Done when**: the context remains `test-rust`; the eighteen surfaces use
Bazel; two fixture surfaces use Cargo/Nix; the shadow workflow is absent; old
and Bazel names forward with equal status; workflows call no deprecated alias;
retired keys are absent; the trim completes synchronously and both headroom
checks pass; exactly one writer publishes; the two future guards are committed
and enforcing; panel and PR are sealed and merged. After the first ordered
protected-`v3` run, a W5 follow-up records `promotion-record.json` with the
promotion commit, cache maintenance result, and first promoted verdict.
Rollback is reverting W5.

### W6 - Compatibility alias removal

**Deliverable**: Remove only Bazel-specific aliases after release. Keep
authoritative Rust leaf names and the Cargo fallback implementation.

**Ownership**: Make and approved-target policy, related contributor docs, and a
unique semantic fragment. No Cargo leaf deletion.

**Entry**: `promotion-record.json` exists and `git tag --contains` confirms at
least one release contains the promotion commit. The ten-green-run clock and
Cargo retirement state are irrelevant to this entry.

**Validation**: Recheck release containment, then run `make test-rust`, all
authoritative leaf targets, `make test-rust-main`, `make test-policy`,
`make check-tier0`, `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`
per the fixture-dependent validation rule, and workflow
absence checks for the removed names.

**Done when**: release containment is rechecked; aliases are absent;
authoritative names retain status; release notes, panel, and PR are complete.

### W7 - Cargo implementation retirement

**Deliverable**: Remove Cargo implementations only for the eighteen surfaces
after ten promoted green runs. Preserve every public target name, the fixture
mode, and both fixture surfaces.

**Ownership**: `tests/test-rust.sh`, obsolete Cargo leaf internals, unreachable
Cargo-only plumbing, related docs, and a unique semantic fragment. Fixture
files are read-only.

**Entry**: `promotion-record.json` exists and ten consecutive promoted `v3` run
IDs are recorded in `post-promotion.json`. Release containment and
compatibility-alias removal are irrelevant to this entry.

**Validation**: `make check`, `make test-rust`, four slices,
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`, `make test-policy`,
`make test-drift`, and an inventory proving that only the eighteen Cargo
implementations disappeared and that `make test-rust` plus all eight
`make test-rust-<leaf>` names still exist and still invoke Bazel carriers.

**Done when**: ten runs are rechecked; no migrated Cargo implementation
remains; every public name still resolves to an authoritative carrier; fixture
mode passes; coverage stays exact; panel and PR merge.

W6 and W7 are independent children of W5. Either may land first once its own
entry condition holds; neither consumes or weakens the other's condition.

Host, VM, live-host, hardware, and manual deployment tiers do not cover this
internal build feature and are not claimed.

## Risks and Rollback

Generic risk rows are noise. These are the specific failures this design makes
possible, each with the guard that catches it.

| Failure | Guard | Rollback |
| --- | --- | --- |
| A sandboxed scan or generator passes because it scanned nothing, or scanned a tree it could not see. | Generator-derived exact census, declared-input equality in both directions, parsed count equal to declared count, `test-drift`, planted violation. | Revert the carrier; Cargo remains the authority. |
| The locator misses under Bazel and silently finds a stale binary in `packages/target/`, which holds real executables for the whole shadow stage. | Mode is selected once and the arms never chain; a Bazel-mode miss fails naming the expected runfiles path; identity is asserted before use; and the planted case supplies a stale, wrong-identity provider at the Cargo path through the injected `FileSystem` while the injected `RunfilesView` reports the entry missing, so the refusal is proven without any test writing an executable into a live path. | Revert the locator migration scope; the Cargo arm is unchanged. |
| A provider negative is arranged on disk instead of injected, so the shadow stage's own `packages/target/` becomes test scaffolding and one abandoned run leaves a stale executable behind that a later Cargo test finds. | All four provider negatives are supplied states on the shared `FileSystem` and `RunfilesView` fakes; the locator and topology never call the standard library directly, and no provider test executes the planted file, because identity is a digest read through the same boundary. | Revert the locator or topology scope; nothing was written outside the fake. |
| The channel transition silently stops applying in a `rules_rust` bump, so the census renders on stable and still looks correct. | The census emits the toolchain version the action actually used as a declared output, and a test compares it to the committed pin. | Revert the `api` scope; the census stays on the Cargo path. |
| The channel flag is set globally instead, so every first-party crate compiles on nightly while the gate stays green. | A guard fails closed on any `.bazelrc` line or wrapper argument that sets the channel flag. | Remove the flag; the guard blocks the merge before it ships. |
| The vendor tree is quietly short a crate, so `licenses` harvests fewer files, reports fewer findings, and exits zero. | Total classification with named refusals for mirrors and checksum-less non-git entries, plus an assertion that the materialized package count equals the lock's before any tool runs. | Revert the vendor rule; the Cargo supply-chain leaf remains authoritative. |
| The decomposed supply-chain pair loses yanked-state detection and nothing notices, because the migration comparison happened to run on a lock set with no yanked crate. | The lock-bounded snapshot and its three carriers land in W1 unconditionally, so the capability exists before the first finding; the comparison still blocks promotion on any differing enforcing outcome, and the offline key-set drift check ties the snapshot to the three locks. | Fix the snapshot or the drift check; omitting the carrier is not authorized in either direction. |
| A contributor hits the stale-lock refusal, finds no supported regeneration path, and runs `CARGO_BAZEL_REPIN=1 make ...`, rewriting every hub lock and, at the 0.73.0 default, the authoritative `Cargo.lock` as well. | `cargo xtask bazel-repin --hub <name>` is the one supported path: closed hub set, `CARGO_BAZEL_REPIN_ONLY` scoped to that hub, controls set only on the spawned child, `skip_cargo_lockfile_overwrite = True` on every hub, and a post-check that fails unless the named hub lock is the only changed tracked file. The Make and workflow prohibition is unchanged and the guard allowlists exactly one construction site. | Discard the worktree change; the refusal is fail-closed and the lock is committed. |
| A contributor hits the module lock refusal and copies the invocation Bazel prints, which carries no startup options, so a second server starts against the worktree and a second output base lands outside `.scratch/` under the home directory. | The repository's own remediation names `cargo xtask bazel-module-refresh` and nothing else. That command issues the measured invocation with the repository's absolute startup options, writes only `MODULE.bazel.lock`, and refuses when any other tracked derived file changed. The module-lock integration test plants real drift, runs the pinned Bazel, and asserts the repository line is present beside the upstream one. | Delete the stray output base and rerun the repository command; the lock was never rewritten by the failing run. |
| A `bazel_dep` version disagreement is absorbed by the resolved graph, so `--lockfile_mode=error` warns and exits zero and the module lock records a version nobody declared. | `.bazelrc` carries `common --check_direct_dependencies=error`, and W0 proves the negative by planting a direct-dependency version the graph would otherwise absorb. | Correct the declared version or the pin; the build refuses until they agree. |
| A repin or module refresh quietly carries an unrelated tracked change into the same commit, so a review reads one lock update and merges two. | Both commands digest every committed derived artifact before the child runs and fail afterwards on any other tracked change, listing the affected paths repository-relative. The recovery is to commit or restore those paths and rerun the same scoped command. | Nothing was committed; the command refused before writing a result. |
| A yanked snapshot is refreshed over the network, reviewed, committed, and never validated, so a key set that does not match the locks reaches continuous integration. | The drift recovery ends on `cargo xtask bazel-yanked-check`, the same offline validator the three carriers run, so the contributor sees the same message in their own shell. | Rerun the refresh and the check; the snapshot is committed and revertible. |
| A cleanup or deadline guard is written against the live host filesystem or clock, so its planted negative silently never runs and the guard is decorative. | `fsops` in `packages/d2b-bazel-support/` and `clock` in `packages/d2b-bazel-runner/` are injected boundaries; every negative is produced by the fake, and the mutation tests assert errno mapping, ownership state, call ordering, and rounding without a full disk, a privileged mount, or a manipulated host clock. | Revert the affected W2 scope; the boundary is a W0-frozen module path, so reverting an implementation does not move the seam. |
| The shared startup-option construction is put in the runner because that is where the wrapper lives, so `xtask` takes a dependency on the crate whose build targets `xtask` itself generates, and the next shared helper follows it there. | The construction lives in the neutral `packages/d2b-bazel-support/`, and `tests/unit/meta/w0-dep-direction.sh` refuses `xtask -> d2b-bazel-runner` in every dependency kind and refuses any first-party edge out of the support crate, resolving names with `cargo metadata` so a rename or a target-specific entry cannot evade it. | Move the helper into the support crate; the guard blocks the merge before the edge ships. |
| The wave-note lint refuses a leaked absolute path and prints the token, so the leak is copied into CI output, a panel comment, and a PR body by the guard that caught it. | The refusal carries the note name, the one-based line, and one remediation and nothing else, and one test runs every rendered refusal back through the scanner's own path-token and worktree-substring rules. | Fix the message; the note itself never merged, because the lint refusal is merge-blocking. |
| The coverage guard is green while half its invariants never executed, because a Bazel test cannot query the graph. | Analysis-time dependency edges prove label existence; completeness and query drift run in the wrapper and `test-drift` over a committed drift-checked query result; no Bazel test invokes `bazel query`. | Revert the guard split; do not weaken either half. |
| The disk cache is measured before asynchronous trimming finishes, so a compliant run refuses to publish forever. | An explicit synchronous on-demand collector runs as a named step before measurement and save; the size refusal stays a backstop. | Revert to the previous snapshot; the maintenance verdict is outside the Rust verdict. |
| A cache-key input changes without changing the key, restoring a subtly stale cache. | The key binds all four hub locks, all four per-hub Bazel-side locks, the guest lock, the generator sha256, `.bazelignore`, the startup and symlink configuration, the build-script and action-environment digest, and the generated BUILD digest. | Rotate the restore prefix; a stale generation is superseded and deleted by the maintenance job. |
| Graph discovery traverses `.scratch/` or a Cargo output directory and either fails or absorbs generated files. | Generated drift-checked `.bazelignore` plus an absolute `--symlink_prefix` beneath `.scratch/`, with a mutation that removes a directory from the list failing closed. | Regenerate `.bazelignore`; the wrapper refuses to run without it. |
| A third-party build script probes the host, diverges from Cargo behavior, or fails under sandboxing. | W0 enumerates every build-script-producing crate per hub, records required annotations, and pins a minimal action-environment allowlist that is itself a cache-key input. | Fix declared inputs and annotations, or stop the migration at W0. |
| A `--test_output=streamed` run silently serializes every test and produces a measurement that means nothing. | The profile invalidation list forbids it, and every recorded sample carries the flags it ran under. | Invalidate and replace the sample. |
| The equivalence streak is laundered by pairing runs that tested different trees, or by cancelling a run about to go red. | Records are push events on `v3` with a shared head commit; a Bazel run reaching no verdict while its paired Cargo run does is a mismatch that resets the streak. | Reset the streak; a double cancellation produces no record and buys nothing. |
| Cleanup follows a replacement or leaks descriptors. | Descriptor-relative removal, forced fallback route, exec-leak and decoy race tests. | Revert W2; never use raw recursive removal. |
| A timeout kills the caller or leaves descendants alive. | Dedicated-group escalation order with real sibling and descendant tests. | Revert W5 to Cargo. |
| Retirement deletes a public entry point along with its Cargo implementation. | The retirement inventory proves only the eighteen implementations disappeared and that `make test-rust` plus all eight leaf names still resolve to Bazel carriers. | Revert W7; W6 and W5 are unaffected. |

After each merge run `nix-collect-garbage`, prune old system generations per
operator policy, and remove finished worktree targets. Never share
`packages/target` or `.scratch/bazel`. The wrapper reports sizes, enforces age
and size, and refuses unsafe cleanup.

## Delivery Memory

### Deferred findings

| Severity | Subject | Wave | Round | Tracking item |
| --- | --- | --- | --- | --- |
| None | None | None | None | None |

Only LOW and MEDIUM findings may be deferred from round nine. CRITICAL and HIGH
block.

### Friction log

| Wave | Category | Impact | Follow-up |
| --- | --- | --- | --- |
| None | None | None | None |

These tables store classification metadata only, never transcripts, validation
output, credentials, or attestations. A category recurring across three waves
becomes a separately filed task.
