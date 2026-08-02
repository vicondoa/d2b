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
| `cpu_weight` | Relative share used to derive a runtime quota |
| `memory_class` | Ordinary or memory-sensitive |
| `dependencies` | Leaves that must complete first |
| `can_overlap` | Leaves proven safe to run concurrently |
| `coverage_ids` | Inventory entries discharged by the leaf |

### Validation rules

- Leaves sharing a mutable Cargo target directory do not overlap.
- Broker feature leaves remain in one serial chain.
- Every required coverage id is owned by exactly one leaf in a local run.
- The heavy-lane concurrency limit is no greater than the runtime aggregate
  budget.
- Runtime Cargo and nextest quotas are derived from relative weights and the
  active-lane limit; every possible active set sums to no more than the runtime
  aggregate budget, including constrained overrides below the lane count.

## Execution Manifest

Represents what the aggregate target actually completed, rather than what the
repository merely made available for discovery.

| Field | Meaning |
|---|---|
| `version` | Execution-manifest schema version, initially `1` |
| `target` | Owning Test Target |
| `commit` | Exact Git commit |
| `run_status` | `passed`, `failed`, or `interrupted` |
| `completed_leaves` | Stable leaf identifiers written only after required commands succeed |
| `failed_surfaces` | Stable identifiers for failures observed before finalization |
| `installables` | Nix attrs actually submitted by the target |
| `realized_checks` | Flake checks actually realized |
| `source_inventory_digest` | Digest of the matching source census |
| `external_contention` | `not-measured`, `none`, `nix-daemon-shared`, or `host-busy` |

### Validation rules

- The manifest is deterministic after sorting.
- A target acquires exclusive ownership of the requested manifest path before
  invalidating prior evidence. Fragment storage is created with `mktemp -d`,
  mode `0700`, and the current effective uid; the finalizer rejects symlinks,
  owner mismatches, or broader permissions.
- The top-level target removes the requested prior manifest and only its own
  prior temporary fragment directory before any evaluation or dispatch.
  Concurrent leaves write one uniquely named temporary fragment each and
  rename it atomically to the final fragment name. The finalizer reads only
  renamed fragments.
- The scheduler runs in a dedicated process group. Handled `INT` or `TERM`
  forwards the signal to that group, waits at most 10 seconds, sends `SIGKILL`
  to survivors, reaps the group, then invokes the idempotent finalizer. The
  finalizer also runs after normal scheduler return, sorts available fragments,
  records `run_status`, atomically replaces the requested manifest, removes
  run-specific temporary state, and preserves the original exit status.
- An uncatchable termination may leave no manifest, but the prior success
  manifest has already been removed. A later invocation removes only stale
  temporary state that passes the same type, ownership, and permission checks.
- A required leaf is absent when its command did not complete successfully.
- A failed or interrupted manifest is diagnostic partial evidence and cannot
  satisfy coverage acceptance; acceptance requires `run_status = "passed"`.
- Every baseline execution leaf remains represented after optimization.
- A source inventory comparison without an execution-manifest comparison is
  insufficient acceptance evidence.

## Nix Candidate Run

Represents one committed Nix-unit runner or evaluator experiment.

| Field | Meaning |
|---|---|
| `candidate` | Tuned pool, pure aggregate, lix-unit, nix-eval-jobs, consolidated flake check, or conditional nix-fast-build |
| `system` | Native Nix system |
| `flake_ref` | `git+file://` repository reference |
| `command` | Exact candidate command |
| `selected_attrs` | Tests, checks, or derivations submitted |
| `workers` | Evaluator or runner concurrency, when applicable |
| `memory_limit` | Per-worker or aggregate bound, when applicable |
| `failure_set` | Every failing case or shard reported by the probe |
| `realized_outputs` | Outputs realized by the candidate |

### Validation rules

- Every candidate covers the complete baseline Nix-unit inventory.
- A candidate reports simultaneous failures without a discovery rerun.
- Candidate branches start from the same committed base.
- Tool acquisition time and steady warm execution are reported separately.
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
| `effective_cpu_budget` | CPU concurrency available to the target after host and cgroup limits |
| `heavy_interval_usec` | Microseconds from first CPU-heavy leaf start through last CPU-heavy leaf completion |
| `measurement_scope` | Rust target accounting, combined Nix client plus daemon-cgroup accounting, or baseline-adjusted host accounting |
| `cpu_usage_delta_usec` | `cpu.stat usage_usec` delta or equivalent user plus system CPU delta; combined Nix scope sums client and daemon deltas |
| `cpu_budget_utilization` | `cpu_usage_delta_usec / (max(1, heavy_interval_usec) * effective_cpu_budget)` |
| `peak_memory_bytes` | Peak sampled combined memory for the declared scope, or maximum baseline-adjusted reduction in host `MemAvailable` |
| `peak_admitted_cpu_slots` | Maximum sum of scheduler-admitted CPU quotas in an active frontier |
| `peak_scope_tasks` | Maximum hierarchical `pids.current` value for cgroup scopes, or recursive process/thread count for a declared fallback scope |
| `memory_events_delta` | Cgroup `high`, `max`, `oom`, `oom_kill`, and `oom_group_kill` deltas when available |
| `memory_psi` | Deltas of `some total` and `full total` microseconds over the heavy interval |
| `swap_io_bytes` | Baseline-adjusted `pswpin` plus `pswpout` page deltas converted with the host page size |
| `external_contention` | Closed reason code such as `none`, `nix-daemon-shared`, or `host-busy`; never free-form process data |

### State transitions

```text
prepared -> running -> passed
                    -> failed
                    -> invalidated
```

- `passed` samples contribute to the median.
- `failed` samples prove failure behavior but do not contribute to performance.
- `invalidated` samples are repeated and retain the invalidation reason.
- The median CPU-budget utilization of the three representative warm samples
  is not accepted below 80%, unless the evidence names the non-CPU bottleneck
  and proves viable concurrency for that interval was exhausted.
- A sample fails resource acceptance on orchestration-attributable OOM,
  sustained memory-pressure stalls, swap thrashing, workers beyond the
  declared bound, peak memory beyond the calculated envelope, or an active
  CPU-quota frontier above the effective budget.
- A CPU-heavy leaf is one assigned a nonzero scheduler CPU quota. The heavy
  interval starts when the first such leaf is admitted and ends when the last
  such leaf completes.
- A run with no admitted CPU-heavy leaf is invalidated as `no-heavy-work`.
  Sub-microsecond clock resolution uses a one-microsecond divisor.
- A sustained memory-pressure stall means `memory.pressure` `some total`
  increases by more than 10% or `full total` increases by more than 1% of
  heavy-interval wall time. When only host PSI is readable, the sample is valid
  only without external contention.
- Swap thrashing means baseline-adjusted swap I/O exceeds both 64 MiB total and
  1 MiB per second over the heavy interval. Any `oom`, `oom_kill`, or
  `oom_group_kill` increase fails acceptance; a `max` or `high` increase is
  reported and fails when accompanied by either sustained-stall threshold.
- `peak_admitted_cpu_slots` MUST NOT exceed `effective_cpu_budget`.
- Rust uses target-cgroup `cpu.stat`, hierarchical `pids.current`, and memory
  counters when available. Nix combines client-scope evaluation with sampled
  Nix-daemon-cgroup build activity when readable, including the sum of client
  and daemon CPU deltas and the peak of their sampled combined memory. Because
  the daemon is shared, unrelated daemon activity invalidates the sample.
  When daemon counters are unreadable, Nix uses host CPU, PSI, swap, and memory
  deltas after an idle baseline; any concurrent external activity invalidates
  and repeats the sample.

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
