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
| `carrier_label` | Exactly one Carrier Target. |
| `slice_id` | Exactly one of `main`, `api`, `broker`, `aux`. |
| `census_ref` | Exact committed census or a recorded derivation. |
| `topology_ref` | Required for test suites; absent only when not applicable. |

The pair `(surface_id, carrier_label)` is one-to-one. Removing or adding a
baseline ID requires a separate contract decision, not a map edit.

## Carrier Target

| Field | Rule |
| --- | --- |
| `label` | Unique Bazel label; ADR-fixed labels live below `//ci/rust`. |
| `surface_id` | Exactly one Rust Surface. A surface may name companion labels, but one carrier owns its verdict. |
| `declared_inputs` | Closed, nonempty input set. |
| `declared_outputs` | Exact outputs, if any; generated outputs must be nonempty. |
| `test_targets` | Transitive Rust test targets carried by the surface. |
| `handwritten_fragments` | Every non-generated BUILD fragment used. |
| `binary_identities` | Expected identity for every located executable. |
| `network_allowed` | Always `false`. |
| `independent_verdict` | Always `true`. |

Labels must exist in query results. Every Rust test target and hand-written
fragment is claimed exactly once across carriers.

## Coverage Map

| Field | Rule |
| --- | --- |
| `baseline_source` | Existing execution-manifest reference. |
| `surfaces` | Exactly eighteen Rust Surface records, sorted by ID. |
| `carriers` | Referenced Carrier Targets, with no orphan. |
| `slices` | Fixed four-slice assignment. |
| `generated_build_digest` | Digest of generated first-party BUILD tree. |
| `governed_source_manifest` | Exact no-bash input manifest. |
| `deliberate_differences` | ADR section 13 difference and rationale per affected surface. |

Validation is bidirectional: every baseline ID has one map row, every mapped
label exists, every test/fragment is claimed, and every referenced census and
topology exists. A minimum count is invalid where an exact derivation exists.

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
| `shell_free` | Always `true`. |

Main and guest must use `process-per-case`. Broker must use
`process-per-binary` and be exclusive. Doctest/harness-free discovery is
derived and refuses empty discovery.

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

## Shadow Run

| Field | Rule |
| --- | --- |
| `run_id` | Unique immutable workflow run ID. |
| `tested_commit` | Commit tested by both paths. |
| `merge_commit` | Resulting `v3` merge commit for a qualifying merged PR. |
| `pull_request` | PR number targeting protected `v3`. |
| `branch` | Always `v3` for qualification and promotion evidence. |
| `cargo_verdict` | `passed` or `failed`. |
| `bazel_verdict` | `passed` or `failed`. |
| `slice_verdicts` | Exactly four attributed results. |
| `manifest_ref` | Immutable evidence reference. |
| `cache_writes` | Must equal zero in shadow. |
| `permissions` | PR jobs: only `contents: read`; no `actions: write`. |
| `cold_ci_seconds` | Complete slowest-slice job duration for a qualifying merged-PR run. |

Runs are ordered by protected-`v3` completion. The promotion streak is ten
consecutive matching verdicts; skipped, canceled, or incomparable runs break
the streak. Cold-CI measurements use the five most recent qualifying runs
whose PRs merged into `v3`.

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
| `sample_commits` | One SHA per sample. Local samples use one candidate SHA; cold-CI samples retain tested and `v3` merge SHAs. |
| `sample_refs` | Run IDs for every sample; cold-CI refs also carry PR number and merged status. |
| `cache_state` | Exact amended-ADR profile; warm records edit and live server, cold local retains only repository cache, cold CI restores nothing. |
| `samples_seconds` | Three local samples or five most recent qualifying merged-PR CI samples. |
| `ceiling_seconds` | 600 warm; 900 cold local/CI. |
| `median_seconds` | Computed over all required valid samples. |
| `maximum_seconds` | Maximum sample. |
| `output_root_sizes` | Before/after for local samples. |
| `valid` | True only if median is at/below ceiling and max is at/below 1.2 times ceiling. |

A cleanup, hard refusal, server restart, wrong edit, cache-state change, heavy
lane overlap, or mismatched environment invalidates a local sample. Invalid
samples are retained with reason and replaced; they do not enter the median.

## Cache Generation

| Field | Rule |
| --- | --- |
| `generation_id` | Unique successful protected-`v3` run identifier. |
| `kind` | `action` or `repository`; never `output-base`. |
| `key_input_digest` | Digest over every ADR-named key input. |
| `restore_prefix` | Omits run ID and commit SHA. |
| `size_bytes` | At most 4 GiB action or 1 GiB repository. |
| `writer_job` | Same single protected-`v3` writer for both coordinated saves. |
| `source_event` | Protected-`v3` push only. |
| `state` | `planned`, `restored-read-only`, `trimmed`, `published`, `superseded`, `deleted`. |

PR jobs can only reach `restored-read-only`. Publication requires complete
maintenance pagination, unambiguous authorized prefixes, and two checks that
repository usage plus planned snapshot is at most 8 GiB. Credentials cannot
enter a run step or Bazel environment.

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
| `coverage_map_digest` | Guard passes for all eighteen. |
| `shadow_runs` | Ten consecutive matching protected-`v3` runs, each retaining tested and merge SHAs. |
| `seeded_failures` | Exact eighteen-record set. |
| `topology_proofs` | Main, guest, and three broker suites; exact censuses/ignored counts. |
| `broker_repetitions` | Twenty consecutive passes per broker suite with exclusivity. |
| `performance_sets` | Three valid profiles. Local sets bind the candidate; merged-PR cold-CI samples retain their own SHAs. |
| `supply_chain_comparison` | Three locks, no differing enforcing outcome. |
| `cache_shadow_proof` | Zero shadow publications. |
| `workflow_policy_proof` | Positive and every required negative fixture pass. |
| `status` | `collecting`, `qualified`, or `invalidated`. |

Before W4 merge, any candidate-content change invalidates evidence tied to
affected content and returns the draft to `collecting`. `qualified` is
required before promotion. Once committed as `qualified`, the record is
immutable. Promotion references its digest and does not mutate it.

Historical shadow and merged-PR cold-CI records are sequences, not
candidate-owned samples. Each retains its own SHA and run ID. Candidate-bound
coverage, seeded-failure, topology, local-performance, and supply-chain
evidence must match `candidate_commit`.

## Promotion Record

| Field | Rule |
| --- | --- |
| `promotion_commit` | Immutable SHA that changes executor authority. |
| `qualification_digest` | Digest of the immutable qualified W4 record. |
| `maintenance_run_id` | Default-branch cache maintenance run. |
| `deleted_generations` | Only authorized retired/superseded keys. |
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
  implementations and must preserve fixture mode.
- A failure before promotion remains in `shadowing`; it never weakens a gate.
- A promoted correctness failure reverts promotion and returns to
  `safety-complete` or `shadowing`, retaining evidence only as historical.

## Relationships

```text
Coverage Map 1 -- 18 Rust Surfaces
Rust Surface 1 -- 1 Carrier Target
Rust Surface 0..1 -- 1 Test Topology
Carrier Target many -- 1 CI Slice
Shadow Run many -- many protected-v3 commits
Seeded Failure Record 18 -- 1 Qualification Evidence Record
Performance Measurement Set 3 -- 1 Qualification Evidence Record
Cache Generation many -- 1 authorized writer policy
Recovery Condition many -- 1 owning safety subsystem
Qualification Evidence Record 1 -- 1 Promotion Record
Promotion Record 1 -- 1 Post-Promotion Observation
```
