# Data Model: ADR 0052 Bazel Rust Gate

These are internal migration and evidence entities, not application data or a
new public API. The existing execution-manifest v1 reference and schema remain
authoritative.

## Modelling rules

Two rules apply to every entity below, because the round-one plan panel found
both classes of defect in an earlier draft:

1. **Variants, not a tag plus optional members.** Where a record has several
   shapes, the shape is the variant and the variant carries its own members. A
   `kind` field beside members that only some kinds may use admits states that
   cannot exist, and a validator then has to re-derive the invariant that the
   type should have made unrepresentable.
2. **No constant fields.** A field whose only legal value is a constant is not
   data; it is an invariant, and it belongs in prose where it cannot be set to
   the other value. Global invariants are listed once, below.

Global invariants, formerly modelled as always-true or always-false fields:

- No carrier action opens a network socket.
- Every carrier reports an independent verdict.
- No fixture-backed identifier is part of this model; the two fixture surfaces
  stay on the Cargo and Nix path.
- The two locator arms never chain, and no located path is an absolute
  execution-root path.
- Concurrency is always derived from `D2B_RUST_BUDGET`, which remains the only
  resource control.
- Under Bazel, every binary is resolved through declared runfiles, and only the
  declared test environment is forwarded to a child.
- Repository-owned execution paths are shell-free. The
  `rules_rust`-generated stable-channel doctest runner is not repository-owned
  and is a recorded deliberate difference.
- Result publication is enforcing for every test carrier.

## Rust Surface

Common to every variant:

| Field | Rule |
| --- | --- |
| `surface_id` | Unique member of the fixed eighteen-ID baseline. |
| `cargo_baseline` | Current Make leaf/mode and command family. |
| `carriers` | Nonempty set of Carrier Targets; exactly one owns the verdict. |
| `slice_id` | Exactly one of `main`, `api`, `broker`, `aux`. |

The variant is the shape, and each variant carries only what it can have:

| Variant | Members | Baseline identifiers |
| --- | --- | --- |
| `Compile` | none beyond the common set | `rust-main-format`, `rust-main-clippy` |
| `TestSuite` | every carrier is a Test Carrier and therefore has a topology | `rust-main-workspace-tests`, `rust-guest-shell-runner`, `rust-broker-default`, `rust-broker-layer1`, `rust-broker-fakebackends` |
| `Policy` | `check_inputs`: the committed policy files, pinned snapshots, and pinned artifacts the carriers declare, nonempty | `rust-deny-main`, `rust-deny-broker`, `rust-deny-guest`, `rust-audit-main`, `rust-audit-broker`, `rust-audit-guest`, `rust-stub-no-socket` |
| `Census` | `census_ref`: one generator-derived census artifact plus its derivation | `rust-api-surface`, `rust-assert-pinned` |
| `Scan` | `governed_source_ref`: the exact generated input manifest | `rust-no-bash-ast` |
| `Reproducibility` | `emitted_census_ref`: the census the generator returns, not a literal | `rust-schema-reproducibility` |

A `Compile` surface has no census member to leave empty and a `Policy` surface
has no topology member to leave absent, so neither state is expressible. The
identifier column is the current assignment and is itself checked against the
coverage map; moving an identifier between variants is a contract decision, not
a map edit.

The mapping is total and unambiguous: every `surface_id` has a nonempty
`carriers` set, and every carrier belongs to exactly one `surface_id`.
Cardinality one is not required and never was; `rust-main-workspace-tests`
already needs three carriers. Removing or adding a baseline ID requires a
separate contract decision, not a map edit.

## Carrier Target

Common to both variants:

| Field | Rule |
| --- | --- |
| `label` | Unique Bazel label; ADR-fixed labels live below `//ci/rust`. |
| `surface_id` | Exactly one Rust Surface. |
| `owns_verdict` | True for exactly one carrier per surface. |
| `declared_inputs` | Closed, nonempty input set. |
| `declared_outputs` | Exact outputs, if any; generated outputs must be nonempty. |
| `handwritten_fragments` | Every non-generated BUILD fragment used. |
| `runfiles_data` | Every binary and fixture this carrier's actions locate, as declared data. |
| `binary_identities` | The expected identity of every executable in `runfiles_data`, one per executable, no more and no fewer. |

Variants:

| Variant | Members |
| --- | --- |
| `TestCarrier` | `topology`: exactly one Test Topology. `test_targets`: the Rust test targets carried, nonempty. `result_document`: the Per-Case Result Document this carrier publishes. |
| `CheckCarrier` | `check_inputs`: the committed configuration, snapshot, manifest, or pinned artifact the check consumes, nonempty. |

Topology and the per-case result document belong to the carrier, not to the
surface. `rust-main-workspace-tests` carries a process-per-case suite, a
doctest carrier, and a harness-free carrier, and those are three different
topologies under one identifier; a surface-level topology field could not
represent that without lying about two of them.

Label existence is proved at analysis time by a real dependency edge from the
coverage guard, not by a query issued from inside a test. Every Rust test
target and hand-written fragment is claimed exactly once across carriers.

## Coverage Map

| Field | Rule |
| --- | --- |
| `baseline_source` | Existing execution-manifest reference. |
| `surfaces` | Exactly eighteen Rust Surface records, sorted by ID. |
| `carriers` | Referenced Carrier Targets, with no orphan and no carrier claimed twice. |
| `slices` | Fixed four-slice assignment. |
| `generated_build_digest` | Digest of generated first-party BUILD tree. |
| `governed_source_manifest` | Exact no-bash input manifest. |
| `derived_censuses` | Generator-derived executed harness-free, doctest, and emitted-schema censuses. |
| `out_of_census_entries` | Every manifest entry the executed selector excludes, each with its reason. |
| `handwritten_fragments` | Every non-generated fragment, including the channel transition rule, the `rustdoc_json` rule, the vendor repository rule, and the yanked-state carrier fragment. |
| `query_result_ref` | The committed drift-checked graph query result the out-of-test completeness check consumes. |
| `locator_migration` | Reference to the Test Locator Migration record set. |
| `deliberate_differences` | ADR section 13 difference and rationale per affected surface. |

Validation is bidirectional: every baseline ID has one map row, every mapped
label exists at analysis time, every test target and fragment is claimed
exactly once, and every referenced census and topology exists. A minimum count
is invalid where an exact derivation exists, and a literal count committed by
hand is invalid where the generator can derive one.

## Test Topology

Common to every variant:

| Field | Rule |
| --- | --- |
| `topology_id` | Unique stable internal ID. |
| `carrier_label` | The one Test Carrier this topology describes. |
| `case_tmpdir` | Each unit of execution gets its own directory beneath the executor temporary root. |

Variants:

| Variant | Members |
| --- | --- |
| `ProcessPerCase` | `suite`: main workspace or guest shell runner. `case_census`: exact nonempty libtest listing. `ignored_census`: exact ignored names and count. One fresh process per case. |
| `ProcessPerBinary` | `suite`: one broker feature suite. `binary_census`: exact nonempty binary listing. `case_census` and `ignored_census` as above. `internal_threads`: positive bounded value. Exclusive by construction. |
| `Doctest` | `discovered_census`: derived, nonempty. |
| `HarnessFree` | `discovered_census`: derived, nonempty, matching the selector the Cargo gate uses. |

`internal_threads` exists only where a binary runs several cases in one
process, and exclusivity is a property of the `ProcessPerBinary` variant rather
than a boolean any topology could set. Exclusive carriers run one at a time and
strictly after the parallel phase, which is a property of the schedule rather
than of the carrier. `Doctest` and `HarnessFree` discovery is derived and
refuses an empty result; those two variants carry a census rather than a
process contract, so the qualification evidence records exactly five topology
proofs, two `ProcessPerCase` and three `ProcessPerBinary`.

## Per-Case Result Document

| Field | Rule |
| --- | --- |
| `path_source` | The path the executor supplies through `XML_OUTPUT_FILE`. |
| `entries` | One per enumerated case, with `passed`, `failed`, or `ignored`. |
| `permitted_content` | Stable case name, outcome, bounded duration, bounded sanitized failure text. |
| `forbidden_content` | Environment values, arguments, absolute paths, store paths, socket paths, runfiles or worktree locations, unit names, PIDs, UIDs, opaque handles, terminal bytes, shell names, raw child output. |
| `raw_output_location` | The ordinary per-target `test.log` artifact only. |
| `write_semantics` | Anchored close-on-exec parent descriptor, link and magic-link refusal, close-on-exec same-directory temporary, sync, descriptor-relative rename. |
| `ownership` | Only a runner-created temporary is ever unlinked; a failed creation unlinks nothing. |
| `ordering` | No output descriptor is opened before every child is reaped. |
| `failure_precedence` | An existing test failure remains primary; publication failure is reported additionally. |

Publication is enforcing, per the global invariants: a passing carrier fails
when publication fails. Every property in this table has a planted mutation the
test must reject, and every one of those mutations is produced through the
injected boundaries below rather than by arranging host state.

## Injected Boundaries

Four boundaries exist so that failure states are supplied rather than
provoked. All are W0-frozen module paths, so later scopes open against a stable
surface. The first two live in `packages/d2b-bazel-support/`, the neutral
internal crate that declares no first-party dependency, because the runner, the
locator, and `xtask` all read them; the crate exists so that no consumer has to
depend on another consumer to reach a boundary.

| Boundary | Path | Serves | Supplied states |
| --- | --- | --- | --- |
| `FileSystem` | `packages/d2b-bazel-support/src/fsops.rs` | Per-case result publication, scratch cleanup, and every provider existence, mode, freshness, and identity check | `openat2` and forced component-walk routes, symlink and magic-link parents, anchored `..` escape, `EEXIST` collision, short write, `EINTR`, `EAGAIN`, `ENOSPC`, replacement race, tracked entry, foreign decoy, and an absent, non-executable, out-of-date, or wrong-digest provider |
| `RunfilesView` | `packages/d2b-bazel-support/src/runfiles.rs` | The locator's Bazel arm and the runner's child-binary resolution | A declared entry present, a declared entry missing, and a runfiles environment that indicates no Bazel test at all |
| `Clock` and `UptimeSource` | `packages/d2b-bazel-runner/src/clock.rs` | Deadline parsing, remaining-budget arithmetic, child duration, expiry escalation | Every accepted and rejected uptime field, truncate on capture and round up on read, exactly-zero remaining budget, overflow, expiry reached without sleeping |
| `YankedIndex` | `packages/xtask/src/bazel_yanked.rs` | The reviewed networked yanked-snapshot refresh | All-clear index, a yanked version, a key the locks declare and the index omits, a key no lock declares, a missing index revision, a transport failure, a malformed payload |

`Clock` and `UptimeSource` stay in the runner rather than moving to the support
crate, because only the runner's deadline and process paths read them. The
locator needs no clock: provider freshness compares two timestamps the
`FileSystem` boundary already returns, and provider identity is a byte digest
read through the same boundary rather than an execution.

No test of cleanup, result publication, deadline handling, or provider
resolution may depend on live host filesystem state, a full disk, a privileged
mount, or the host clock. A property that can only be exercised by arranging
host state is a property that will be marked ignored, which is the same as not
testing it. That applies with particular force to the stale-provider case: a
test that writes an out-of-date executable into `packages/target/` has planted
the exact hazard the locator exists to refuse, on the host every other suite
shares, and leaves it there if the run is interrupted.

The same rule holds for the network. `IndexClient` is the single networked
implementation of `YankedIndex` and the only site permitted to open a socket
for the refresh; every unit test of the refresh injects a fake instead, so no
test resolves a name or reaches the live index. `bazel-yanked-check` names
neither the trait nor its networked implementation, which is what makes the
offline validator offline by construction. Real-index behavior is measured
separately, by the reviewed contributor-run refresh whose diff and observed
index revision the committing wave records.

## Test Locator Migration

Every record identifies one affected first-party file and one of two
dispositions. The disposition is the variant, so a record cannot claim to need
no migration while also carrying a runfiles path.

Common:

| Field | Rule |
| --- | --- |
| `file` | Repository-relative path of one affected first-party file. |
| `site` | `binary-location`, `manifest-path`, or `repo-root-walk` with the helper named. |

Variants:

| Variant | Members |
| --- | --- |
| `Migrated` | `bazel_runfiles_path` and the `data` label providing it; `cargo_call_site_crate`, the test crate the Cargo arm expands in; for a `binary-location` site, the identity the located executable must report before use. |
| `NoMigrationNeeded` | `reason`: the recorded reason this file needs no change. |

The affected set is enumerated, not sampled: 25 files locating binaries through
compile-time Cargo environment expansion and 20 test files resolving
`CARGO_MANIFEST_DIR`, 11 of those through a `repo_root()` helper. A file that
is in neither variant is a gap the coverage map makes visible. Both arms stay
green on the Cargo path for the whole shadow stage.

Provider negatives are supplied, never arranged. The absent, non-executable,
out-of-date, and wrong-identity providers, and the missing runfiles entry that
turns a Bazel-mode lookup into a refusal, are all states of the `FileSystem`
and `RunfilesView` fakes in `packages/d2b-bazel-support/`. No record in this
set is proven by writing an executable to `packages/target/` or to any other
live path, and no provider check executes the located file: identity is the
digest of its bytes read through the same boundary.

## Hermeticity Inventory

| Field | Rule |
| --- | --- |
| `hub` | One of the four `crate_universe` hubs: `main`, `broker`, `guest`, `walker`. |
| `hub_lock_attrs` | `lockfile`, `cargo_lockfile`, and `skip_cargo_lockfile_overwrite = True` are all present. |
| `build_script_crates` | Every third-party crate for which a build-script target is generated. |
| `required_annotations` | Per crate: build-script environment, data, and toolchain requirements. |
| `action_env_allowlist` | The explicit minimal set of host environment values any action may observe. |
| `bazelignore_entries` | `.scratch/` plus every Cargo output directory any workspace or tool creates. |
| `symlink_prefix` | Absolute path beneath `.scratch/`. |
| `startup_options` | Absolute values supplied by the wrapper from the one construction in `packages/d2b-bazel-support/src/startup.rs`, byte-identical across build, test, query, info, shutdown, clean, and, from W2, the repin and module-refresh children. |
| `generator_pin` | `cargo-bazel` URL plus sha256; source bootstrap refused. |
| `module_lock_modes` | `.bazelrc` carries `common --lockfile_mode=error` and `common --check_direct_dependencies=error`; neither may be relaxed by a wrapper argument. |

Repin controls are absent from the wrapper and from every
continuous-integration environment. The single scoped exception is the child
environment `cargo xtask bazel-repin --hub <name>` constructs, which sets
`CARGO_BAZEL_REPIN` and `CARGO_BAZEL_REPIN_ONLY=<hub>` for that one process,
writes only that hub's Bazel-side lock, and fails when any other tracked
derived artifact changed. `cargo xtask bazel-module-refresh` sets no repin
control at all, refuses to run when one is ambient, writes only
`MODULE.bazel.lock`, fails when any other tracked derived artifact changed, and
changes nothing on an already-current tree. Neither command is a Make target or
reachable from a workflow.

Every field here is a cache-key input. A change to `action_env_allowlist`
invalidates the entire action cache and is reviewed against the promoted size
budget in the same change.

## Execution Manifest Binding

| Field | Rule |
| --- | --- |
| `authority` | `docs/reference/test-execution-manifest.md` and its v1 schema. |
| `executor` | `cargo` during shadow; `bazel` after promotion. Not a schema field. |
| `surface_mapping` | Carrier result to existing surface ID. |
| `completed_mapping` | Success only after all commands for that surface complete. |
| `failure_mapping` | Observed carrier failures to existing `failed_surfaces`. |
| `interruption_mapping` | Handled interruption publishes available partial evidence. |

This binding cannot add, rename, reinterpret, or version manifest fields.
Prior evidence is invalidated before dispatch. Passing promotion evidence
requires a v1 `passed` manifest with all eighteen IDs; partial evidence is
diagnostic only.

## Qualification Record

| Field | Rule |
| --- | --- |
| `head_sha` | The commit both workflows tested; identical for both run IDs. |
| `source_event` | `push` on `refs/heads/v3` produced by a merged pull request. |
| `bazel_run_id` | Unique immutable shadow workflow run ID. |
| `cargo_run_id` | Unique immutable required workflow run ID at the same `head_sha`. |
| `cargo_verdict` | `passed` or `failed` from `D2B_SKIP_FIXTURE_BUILD=1 make test-rust`. |
| `bazel_verdict` | `passed` or `failed` from the Bazel rollup. |
| `fixture_verdict` | `passed` required, from same-commit `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`. |
| `slice_verdicts` | Exactly four attributed results. |
| `slice_seconds` | Four complete job durations; required for a cold-sample record. |
| `manifest_ref` | Immutable evidence reference. |
| `cache_restored` | Must equal zero for a qualifying cold sample. |
| `cache_writes` | Must equal zero during the shadow stage. |
| `permissions` | PR-reachable jobs: only `contents: read`; no `actions: write`. |

Records are ordered by `v3` push completion. The promotion streak is ten
consecutive records whose two compared verdicts match with a passing
`fixture_verdict`. Streak arithmetic is fail-closed:

- differing verdicts reset the streak to zero;
- a Bazel run that reaches no verdict while its paired Cargo run reaches one
  counts as a mismatch and resets the streak;
- a push where neither side reaches a verdict is not a record and neither
  extends nor resets.

Pull-request, `main`-push, scheduled, and dispatched runs are diagnostic. They
never enter a streak or a measurement set, because `refs/pull/N/merge` is
recomputed against a moving base and a Bazel-path-filtered pull-request sample
cannot contain the divergence class the streak exists to detect.

## Seeded Failure Record

| Field | Rule |
| --- | --- |
| `surface_id` | Unique across the evidence set; all eighteen required. |
| `seed_commit` | Immutable disposable commit or patch digest. |
| `seed_description` | The single protected invariant intentionally broken. |
| `invoked_make_target` | Owning approved slice or aggregate target. |
| `expected_carrier` | Carrier for `surface_id`. |
| `observed_failed_surfaces` | Exactly `[surface_id]`. |
| `unrelated_failures` | Empty. |
| `partial_manifest_ref` | Failed v1 manifest reference. |

A record is invalid if the seed changes more than one protected condition or
if an unrelated surface fails.

## Performance Measurement Set

| Field | Rule |
| --- | --- |
| `profile` | `warm-local`, `cold-local`, or `cold-ci`. |
| `environment` | ADR reference local host or runner facts and tool pins. |
| `sample_commits` | One SHA per sample. Local samples use one candidate SHA; cold-CI samples use each record's `head_sha`. |
| `sample_refs` | Run IDs for every sample; cold-CI refs also carry the `push`-on-`v3` source event. |
| `cache_state` | Exact ADR profile; warm records the edit and live server, cold local retains only the repository cache, cold CI restores nothing. |
| `invocation_flags` | The exact flags each sample ran under; `--test_output=streamed` invalidates the sample. |
| `samples_seconds` | Three local samples, or the five most recent qualifying cold qualification records. |
| `qualifying_rule` | A cold-CI sample qualifies only when no Bazel cache was restored and all four slice jobs completed with a recorded duration. |
| `ceiling_seconds` | 600 warm; 900 cold local and cold CI. |
| `feasibility_ref` | Required for `cold-ci`: the W3 feasibility measurement that made the ceiling binding, or the pre-authorized remedy taken instead. |
| `median_seconds` | Computed over all required valid samples. |
| `maximum_seconds` | Maximum sample. |
| `output_root_sizes` | Before/after for local samples. |
| `valid` | True only if median is at/below ceiling and max is at/below 1.2 times ceiling. |

A cleanup, hard refusal, server restart, wrong edit, cache-state change, heavy
lane overlap, streamed test output, or mismatched environment invalidates a
sample. Invalid samples are retained with their reason and replaced; they do
not enter the median. The `api` slice's samples include the second
configuration the channel transition creates; that cost is inside the ceiling,
not carved out of it.

## Cache Generation

| Field | Rule |
| --- | --- |
| `generation_id` | Unique successful protected-`v3` run identifier. |
| `kind` | `action` or `repository`; never `output-base`. |
| `key_input_digest` | Digest over `.bazelversion`, `MODULE.bazel`, `MODULE.bazel.lock`, `.bazelrc`, both `rust-toolchain.toml` files, all four hub Cargo locks, `packages/Cargo.guest.lock`, all four per-hub `crate_universe` Bazel-side locks, the `cargo-bazel` URL and sha256, all deny configurations, the advisory-database pin, the committed yanked snapshot, `.bazelignore`, the symlink-prefix and startup-option configuration, the build-script annotation and action-environment digest, and the generated BUILD tree digest. |
| `restore_prefix` | Omits run ID and commit SHA. |
| `trim_evidence` | Reference proving the explicit synchronous collector completed before measurement. |
| `size_bytes` | At most 4 GiB action or 1 GiB repository, measured after the trim. |
| `writer_job` | Same single protected-`v3` writer for both coordinated saves. |
| `source_event` | Protected-`v3` push only. |
| `state` | `planned`, `restored-read-only`, `trimmed`, `published`, `superseded`, `deleted`. |

PR jobs can only reach `restored-read-only`. Publication requires complete
maintenance pagination, unambiguous authorized prefixes, an observed
synchronous trim, and two checks that repository usage plus planned snapshot is
at most 8 GiB. Credentials cannot enter a run step or a Bazel environment. Any
key input changing without changing the key is a defect, not a tuning choice.

## Recovery Condition

Common:

| Field | Rule |
| --- | --- |
| `code` | Unique stable static code. |
| `trigger` | One exact refusal or expiry class. |
| `message_template` | Fixed and actionable. |
| `required_steps` | Exact repository-relative remedy for this code. |
| `forbidden_values` | Absolute path, output hash, user/PID, raw deadline, opaque handle. |
| `forbidden_actions` | Code-specific unsafe actions. |

Variants:

| Variant | Members | Codes |
| --- | --- | --- |
| `CleanupRefusal` | Deletes nothing, by construction of the variant. Exercised through the injected `FileSystem`. | `D2B-BZLCLEAN-TRACKED`, `D2B-BZLCLEAN-SYMLINK`, `D2B-BZLCLEAN-ESCAPE`, `D2B-BZLCLEAN-LIVE` |
| `ServerRefusal` | Bounded shutdown attempt, no manual signal instruction. | `D2B-BZLSERVER-STUCK` |
| `DeadlineOutcome` | `measured_duration` and `target`. Exercised through the injected `Clock`. | Expired budget and ceiling miss |

`deletes_nothing` is not a field, because only `CleanupRefusal` can carry it
and it is always true there. Expired budget and ceiling miss are normal
deadline outcomes rather than refusals. Remedies cannot be
borrowed across codes. A ceiling miss names only a larger runner or further
disjoint split.

## Qualification Evidence Record

This is the concrete immutable record for the feature specification's
Promotion Evidence Set before executor authority changes.

| Field | Validity rule |
| --- | --- |
| `candidate_commit` | One immutable integrated commit. |
| `coverage_map_digest` | Both guard halves pass for all eighteen. |
| `qualification_records` | Ten consecutive matching push-to-`v3` records, each with one shared `head_sha`, both run IDs, and a passing fixture-contract verdict. |
| `seeded_failures` | Exact eighteen-record set. |
| `topology_proofs` | Main, guest, and three broker suites; exact generator-derived censuses and ignored counts, plus per-case result publication. |
| `locator_migration_proof` | Every enumerated file migrated or recorded as needing none, plus the passing injected stale-provider negative in which the `FileSystem` fake reports an out-of-date, wrong-digest executable at the Cargo path while the `RunfilesView` fake reports the entry missing. |
| `broker_repetitions` | Twenty consecutive passes per broker suite with exclusivity. |
| `performance_sets` | Three valid profiles. Local sets bind the candidate; cold-CI samples carry their own `head_sha` values and reference the W3 feasibility measurement. |
| `supply_chain_comparison` | Three locks, no differing enforcing outcome, with the yanked carrier landed and `cargo xtask bazel-yanked-check` passing offline against all three. |
| `cache_shadow_proof` | Zero shadow publications. |
| `workflow_policy_proof` | Positive and every required negative fixture pass. |
| `status` | `collecting`, `qualified`, or `invalidated`. |

Before W4 merge, any candidate-content change invalidates evidence tied to
affected content and returns the draft to `collecting`. `qualified` is
required before promotion. Once committed as `qualified`, the record is
immutable. Promotion references its digest and does not mutate it.

Historical qualification records are a sequence, not candidate-owned samples.
Each retains its own `head_sha` and run IDs. Candidate-bound coverage,
seeded-failure, topology, locator, local-performance, and supply-chain evidence
must match `candidate_commit`.

## Promotion Record

| Field | Rule |
| --- | --- |
| `promotion_commit` | Immutable SHA that changes executor authority. |
| `qualification_digest` | Digest of the immutable qualified W4 record. |
| `maintenance_run_id` | Protected-`v3` cache maintenance run. |
| `deleted_generations` | Only authorized retired/superseded keys. |
| `trim_evidence` | Reference proving the synchronous collector completed before both headroom checks. |
| `headroom_checks` | Both pre-save checks at or below 8 GiB. |
| `writer_run_id` | The one authorized publishing job. |
| `first_promoted_verdict` | Required `test-rust` verdict after promotion. |
| `rollback_rehearsal` | Reference proving one revert restores Cargo authority. |

The record is written after the ordered protected-`v3` promotion run and is
immutable once reviewed.

## Post-Promotion Observation

| Field | Rule |
| --- | --- |
| `promotion_commit` | Must equal Promotion Record SHA. |
| `release_tags` | Tags that contain promotion, recorded independently. |
| `green_run_ids` | Ordered promoted `v3` `test-rust` run IDs. |
| `consecutive_green_count` | Derived from uninterrupted green run sequence. |
| `alias_removal_eligible` | True when at least one containing release exists. |
| `cargo_retirement_eligible` | True when consecutive green count is at least ten. |

The two eligibility values are independent. Either may become true first and
neither depends on the corresponding change having landed.

## Migration Lifecycle

```text
planned
  -> foundation-ready
  -> coverage-complete
  -> safety-complete
  -> shadowing
  -> evidence-qualified
  -> promoted
  -> release-qualified -> aliases-removed
  -> green-run-qualified -> cargo-retired
```

Transition rules:

- Each transition before promotion requires the prior wave merged and sealed
  by the unanimous ten-role panel. After promotion, each independent child
  transition requires W5 plus its own evidence gate and panel.
- `shadowing -> evidence-qualified` requires a valid Qualification Evidence
  Record.
- `evidence-qualified -> promoted` is the only executor-authority change.
- Before `cargo-retired`, rollback is one promotion revert because Cargo
  implementation still exists.
- `promoted -> release-qualified` requires a release containing promotion and
  is independent of the green-run clock and Cargo retirement.
- `promoted -> green-run-qualified` requires ten consecutive green promoted
  `v3` runs and is independent of release containment and alias
  removal.
- `release-qualified -> aliases-removed` removes only compatibility aliases.
- `green-run-qualified -> cargo-retired` removes only the eighteen Cargo
  implementations and unreachable Cargo-only plumbing. It must preserve the
  fixture mode and every public Make name, which continue to invoke the
  authoritative Bazel carriers.
- A failure before promotion remains in `shadowing`; it never weakens a gate.
- A promoted correctness failure reverts promotion and returns to
  `safety-complete` or `shadowing`, retaining evidence only as historical.

## Relationships

```text
Coverage Map 1 -- 18 Rust Surfaces
Rust Surface 1 -- 1..n Carrier Targets
Carrier Target 1 -- 1 Rust Surface
Carrier Target (TestCarrier) 1 -- 1 Test Topology
Carrier Target (TestCarrier) 1 -- 1 Per-Case Result Document
Carrier Target many -- 1 CI Slice
Injected Boundaries 2 -- many Carrier Targets and cleanup paths
Coverage Map 1 -- many Test Locator Migration records
Hermeticity Inventory 4 -- 1 Coverage Map
Qualification Record many -- many protected-v3 push events
Seeded Failure Record 18 -- 1 Qualification Evidence Record
Performance Measurement Set 3 -- 1 Qualification Evidence Record
Cache Generation many -- 1 authorized writer policy
Recovery Condition many -- 1 owning safety subsystem
Qualification Evidence Record 1 -- 1 Promotion Record
Promotion Record 1 -- 1 Post-Promotion Observation
```
