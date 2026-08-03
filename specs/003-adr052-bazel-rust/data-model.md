# Data Model: ADR 0052 Bazel Rust Gate

These are internal migration and evidence entities, not application data or a
new public API. The existing execution-manifest v1 reference and schema remain
authoritative.

## Rust Surface

| Field | Rule |
| --- | --- |
| `surface_id` | Unique member of the fixed eighteen-ID baseline. |
| `kind` | `compile`, `test`, `policy`, `scan`, or `reproducibility`. |
| `cargo_baseline` | Current Make leaf/mode and command family. |
| `fixture_backed` | Must be `false`; fixture IDs are outside this model. |
| `carrier_labels` | Nonempty set of Carrier Targets; one of them owns the verdict. |
| `slice_id` | Exactly one of `main`, `api`, `broker`, `aux`. |
| `census_ref` | Generator-derived census artifact plus its derivation. |
| `topology_ref` | Required for test suites; absent only when not applicable. |

The mapping is total and unambiguous: every `surface_id` has a nonempty
`carrier_labels` set, and every carrier belongs to exactly one `surface_id`.
Cardinality one is not required and never was; `rust-main-workspace-tests`
already needs three carriers. Removing or adding a baseline ID requires a
separate contract decision, not a map edit.

## Carrier Target

| Field | Rule |
| --- | --- |
| `label` | Unique Bazel label; ADR-fixed labels live below `//ci/rust`. |
| `surface_id` | Exactly one Rust Surface. |
| `owns_verdict` | Exactly one carrier per surface has `true`. |
| `declared_inputs` | Closed, nonempty input set. |
| `declared_outputs` | Exact outputs, if any; generated outputs must be nonempty. |
| `test_targets` | Transitive Rust test targets carried by the surface. |
| `handwritten_fragments` | Every non-generated BUILD fragment used. |
| `runfiles_data` | Every binary and fixture the carrier's tests locate, as declared data. |
| `binary_identities` | Expected identity for every located executable. |
| `result_document` | Required for test carriers; the per-case result contract this carrier satisfies. |
| `action_network_allowed` | Always `false`. |
| `independent_verdict` | Always `true`. |

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
| `handwritten_fragments` | Every non-generated fragment, including the channel transition rule, the `rustdoc_json` rule, and the vendor repository rule. |
| `query_result_ref` | The committed drift-checked graph query result the out-of-test completeness check consumes. |
| `locator_migration` | Reference to the Test Locator Migration record set. |
| `deliberate_differences` | ADR section 13 difference and rationale per affected surface. |

Validation is bidirectional: every baseline ID has one map row, every mapped
label exists at analysis time, every test target and fragment is claimed
exactly once, and every referenced census and topology exists. A minimum count
is invalid where an exact derivation exists, and a literal count committed by
hand is invalid where the generator can derive one.

## Test Topology

| Field | Rule |
| --- | --- |
| `topology_id` | Unique stable internal ID. |
| `mode` | `process-per-case`, `process-per-binary`, `doctest`, or `harness-free`. |
| `suite` | Main, guest, or one broker feature suite. |
| `case_census` | Exact nonempty libtest listing when supported. |
| `ignored_census` | Exact ignored names/count; ignored never means passed. |
| `internal_threads` | Positive bounded value for broker; one case per child otherwise. |
| `exclusive` | `true` for all broker feature carriers, otherwise `false`. |
| `budget_source` | Always derived from `D2B_RUST_BUDGET`. |
| `case_tmpdir` | Each case gets its own directory beneath the executor temporary root. |
| `binary_resolution` | Always through declared runfiles under Bazel. |
| `env_policy` | Only the declared test environment is forwarded. |
| `result_document` | Per-case structured result written to the executor-designated path. |
| `shell_free` | Always `true` for repository-owned execution paths. |

Main and guest must use `process-per-case`. Broker must use
`process-per-binary` and be exclusive. Exclusive carriers run one at a time and
strictly after the parallel phase, which is a property of the schedule rather
than of the carrier. Doctest and harness-free discovery is derived and refuses
empty discovery. The `rules_rust`-generated stable-channel doctest runner is a
shell script; it is a recorded deliberate difference and is not
repository-owned, so `shell_free` binds repository-owned paths only.

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
| `publication_is_enforcing` | A passing carrier fails when publication fails. |
| `failure_precedence` | An existing test failure remains primary; publication failure is reported additionally. |

Filesystem operations sit behind an injectable trait so errno mapping,
ownership state, and call ordering are hermetically testable. Every property in
this table has a planted mutation the test must reject.

## Test Locator Migration

| Field | Rule |
| --- | --- |
| `file` | Repository-relative path of one affected first-party file. |
| `kind` | `binary-location`, `manifest-path`, or `repo-root-walk`. |
| `disposition` | `migrated` or `no-migration-needed` with a recorded reason. |
| `bazel_arm` | Declared runfiles path plus the `data` label providing it. |
| `cargo_arm` | Call-site macro expansion in the calling test crate. |
| `chaining_allowed` | Always `false`. |
| `absolute_execroot_path` | Always absent. |
| `identity_assertion` | Located binary is checked to exist, to be executable, and to report the expected identity. |

The affected set is enumerated, not sampled: 25 files locating binaries through
compile-time Cargo environment expansion and 20 test files resolving
`CARGO_MANIFEST_DIR`, 11 of those through a `repo_root()` helper. A file that
is neither migrated nor recorded as needing no migration is a gap the coverage
map makes visible. Both arms stay green on the Cargo path for the whole shadow
stage.

## Hermeticity Inventory

| Field | Rule |
| --- | --- |
| `hub` | One of the four `crate_universe` hubs. |
| `build_script_crates` | Every third-party crate for which a build-script target is generated. |
| `required_annotations` | Per crate: build-script environment, data, and toolchain requirements. |
| `action_env_allowlist` | The explicit minimal set of host environment values any action may observe. |
| `bazelignore_entries` | `.scratch/` plus every Cargo output directory any workspace or tool creates. |
| `symlink_prefix` | Absolute path beneath `.scratch/`. |
| `startup_options` | Absolute values supplied by the wrapper, byte-identical across build, test, query, info, shutdown, and clean. |
| `repin_controls` | Must be absent from the wrapper and CI environments. |
| `generator_pin` | `cargo-bazel` URL plus sha256; source bootstrap refused. |

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
| `key_input_digest` | Digest over `.bazelversion`, `MODULE.bazel`, `MODULE.bazel.lock`, `.bazelrc`, both `rust-toolchain.toml` files, all four hub Cargo locks, `packages/Cargo.guest.lock`, all four per-hub `crate_universe` Bazel-side locks, the `cargo-bazel` URL and sha256, all deny configurations, the advisory-database pin, the yanked snapshot when present, `.bazelignore`, the symlink-prefix and startup-option configuration, the build-script annotation and action-environment digest, and the generated BUILD tree digest. |
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

| Field | Rule |
| --- | --- |
| `code` | Unique stable static code. |
| `owner` | Cleanup or deadline/server subsystem. |
| `trigger` | One exact refusal/expiry class. |
| `message_template` | Fixed and actionable. |
| `required_steps` | Exact repository-relative remedy for this code. |
| `forbidden_values` | Absolute path, output hash, user/PID, raw deadline, opaque handle. |
| `forbidden_actions` | Code-specific unsafe actions. |
| `deletes_nothing` | Required for cleanup refusal. |

Required cleanup codes are `D2B-BZLCLEAN-TRACKED`,
`D2B-BZLCLEAN-SYMLINK`, `D2B-BZLCLEAN-ESCAPE`, and
`D2B-BZLCLEAN-LIVE`; server shutdown uses `D2B-BZLSERVER-STUCK`. Expired
budget and ceiling miss are normal deadline outcomes. Remedies cannot be
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
| `locator_migration_proof` | Every enumerated file migrated or recorded as needing none, plus the passing stale-binary negative fixture. |
| `broker_repetitions` | Twenty consecutive passes per broker suite with exclusivity. |
| `performance_sets` | Three valid profiles. Local sets bind the candidate; cold-CI samples carry their own `head_sha` values and reference the W3 feasibility measurement. |
| `supply_chain_comparison` | Three locks, no differing enforcing outcome, with the yanked carrier landed if the comparison required it. |
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
Rust Surface 0..1 -- 1 Test Topology
Test Topology 1 -- 1 Per-Case Result Document
Carrier Target many -- 1 CI Slice
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
