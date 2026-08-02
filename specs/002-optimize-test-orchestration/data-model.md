# Data Model: Optimize Test Orchestration

This feature does not add persistent application data. The model below defines
the test-infrastructure records and invariants used by implementation,
benchmarking, and acceptance.

## Test Target

Represents one stable contributor-facing validation command.

| Field | Meaning |
|---|---|
| `name` | `test-rust`, `test-nix-unit`, or `test-flake` |
| `make_target` | Stable public Make target |
| `local_plan` | Ordered and parallel execution graph used locally |
| `ci_plan` | Existing CI shard or matrix mapping |
| `coverage_inventory` | Required tests, cases, checks, and companion surfaces |
| `cache_conditions` | Warm and cold measurement definitions |
| `success_threshold` | Warm median no greater than 50% of baseline |

### Validation rules

- The Make target name is unchanged.
- Local and CI plans may differ, but their enforcing coverage inventories are
  equal.
- A target fails if any required leaf fails.
- A target may not report success with a missing inventory item.

## Execution Leaf

Represents the smallest independently schedulable unit.

| Field | Meaning |
|---|---|
| `id` | Stable internal leaf name |
| `target` | Owning Test Target |
| `command_kind` | Cargo, nextest, Nix evaluation, Nix build, or repository leaf |
| `inputs` | Manifest, flake attr, feature set, fixtures, and toolchain |
| `target_directory` | Cargo output directory, if applicable |
| `cpu_weight` | Share of the aggregate CPU budget |
| `memory_class` | Ordinary or memory-sensitive |
| `dependencies` | Leaves that must complete first |
| `can_overlap` | Leaves proven safe to run concurrently |
| `coverage_ids` | Inventory entries discharged by the leaf |

### Validation rules

- Leaves sharing a mutable Cargo target directory do not overlap.
- Broker feature leaves remain in one serial chain.
- Every required coverage id is owned by exactly one leaf in a local run.
- A leaf cannot allocate more than the aggregate host budget.

## Nix Installable Set

Represents derivations addressed in one native Nix invocation.

| Field | Meaning |
|---|---|
| `system` | Native Nix system |
| `flake_ref` | `git+file://` repository reference |
| `installables` | Ordered flake attribute references |
| `mode` | Instantiate-only or realized |
| `keep_going` | Whether independent failures continue |
| `max_jobs` | Native Nix build concurrency |

### Validation rules

- Nix unit installables contain every discovered `nix-unit*` check.
- The realized flake set contains only checks in the committed realized class.
- No bare-path flake reference is accepted.

## Coverage Inventory

Represents the exact pre-change contract that must survive optimization.

| Field | Meaning |
|---|---|
| `target` | Owning Test Target |
| `items` | Canonical test/check identifiers |
| `source` | nextest listing, flake attrs, pin files, or policy manifest |
| `digest` | Stable comparison digest |
| `captured_at` | Commit used for baseline |

### Validation rules

- Every baseline inventory item remains present after optimization.
- New tests added for the orchestration change are permitted and are recorded
  separately from the preserved baseline set.
- Deliberately reused prerequisites are recorded separately from test items.
- Removed duplicate operations do not remove inventory items.

## Benchmark Run

Represents one timed command execution.

| Field | Meaning |
|---|---|
| `target` | Test Target measured |
| `commit` | Exact Git commit |
| `cache_condition` | Warm or cold |
| `sample` | Sample ordinal |
| `elapsed_seconds` | Wall-clock duration |
| `exit_status` | Command result |
| `host_fingerprint` | CPU count, memory, system, and tool versions |
| `inventory_digest` | Coverage inventory used |
| `external_contention` | Recorded invalidating contention, if any |

### State transitions

```text
prepared -> running -> passed
                    -> failed
                    -> invalidated
```

- `passed` samples contribute to the median.
- `failed` samples prove failure behavior but do not contribute to performance.
- `invalidated` samples are repeated and retain the invalidation reason.

## Performance Baseline

Aggregates accepted samples for one target and cache condition.

| Field | Meaning |
|---|---|
| `target` | Test Target |
| `cache_condition` | Warm or cold |
| `samples` | Three valid Benchmark Runs |
| `median_seconds` | Median elapsed time |
| `slowest_seconds` | Slowest valid sample |
| `inventory_digest` | Coverage contract measured |

### Acceptance transition

```text
baseline-recorded -> optimized-measured -> accepted
                                       -> rejected
```

- Warm is accepted only when optimized median is at most half the baseline.
- Cold is always reported and reviewed but does not block acceptance.
