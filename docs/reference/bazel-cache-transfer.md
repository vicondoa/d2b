# Local Bazel cache-transfer model

This tool measures potential content-addressable storage (CAS) transfer before
any BuildBuddy cache or remote-execution profile is enabled. It consumes an
unsorted Bazel execution log containing `SpawnExec` records and the checked-in
eligibility metadata.

## Repeatable command

The repository facade consumes a representative fixture by default:

```bash
make bazel-cache-transfer-report
```

For a clean local graph, point the same command at the execution log emitted
by the local Bazel invocation:

```bash
bazel test //... --config=local \
  --execution_log_json_file=.scratch/bazel-cache-transfer/local.json \
  --repo_contents_cache=
D2B_BAZEL_CACHE_TRANSFER_LOG=.scratch/bazel-cache-transfer/local.json \
D2B_BAZEL_CACHE_TRANSFER_ELIGIBILITY=tests/golden/bazel/eligibility.json \
D2B_BAZEL_CACHE_TRANSFER_REPORT=.scratch/bazel-cache-transfer/local-report.json \
D2B_BAZEL_CACHE_TRANSFER_CONFIGURATION=local \
D2B_BAZEL_CACHE_TRANSFER_PLATFORM=linux-x86_64 \
D2B_BAZEL_CACHE_TRANSFER_TOOLCHAIN=rules_rust \
make bazel-cache-transfer-report
```

The analyzer also runs directly:

```bash
cargo run --locked -p xtask -- bazel-cache-transfer \
  --execution-log .scratch/bazel-cache-transfer/local.json \
  --eligibility tests/golden/bazel/eligibility.json \
  --configuration local \
  --platform linux-x86_64 \
  --toolchain rules_rust \
  --output .scratch/bazel-cache-transfer/local-report.json
```

The input may be a Bazel JSON array, concatenated Bazel protobuf-JSON
`SpawnExec` messages, an object with `records`, `events`, or `spawns`, a single
`SpawnExec` object, or JSON lines. Protobuf-JSON `int64` size strings and
digestless empty files or unresolved symlinks are accepted. Records are
normalized before aggregation, so input order does not affect the report.
Configuration, platform, and toolchain identity must come from log metadata or
explicit command-line overrides; comparisons reject missing identity.

## Classification

The eligibility file maps each target label to `eligible`. An eligible action
with `remotable: true` is classified as `rbe`. An eligible action with
`remoteCacheable: true` but not `remotable` is `remote-cache-only`. A local
cacheable action without the explicit remote-cacheable signal remains
`fully-local`. Remote classes are rejected when their target is not eligible.
Missing target labels, action class signals, digests, sizes, duplicate records,
malformed JSON, failed actions, conflicting digest sizes, and arithmetic
overflow fail closed. Eligibility entries are authoritative for listed targets;
unlisted dependency owners are recorded in `source.unlistedTargets` and use
explicit Bazel `remotable` or `remoteCacheable` signals for conservative
classification.

Local-only KVM, VM, hardware, Nix, fixture, and image actions remain in the
whole-graph and fully-local sections, but do not enter RBE or remote-cache-only
totals unless their same-path output is consumed by a remote-class action.
Such a producer is then reported as a `local-to-remote` boundary crossing. The
reverse direction is reported as `remote-to-local`. Digest-only aggregation
still deduplicates equal content across paths for CAS byte bounds.

## Metrics

Every action, mnemonic, execution class, and the whole graph reports:

- gross input bytes: the pessimistic cold-executor bound, counting every
  occurrence;
- unique input bytes: the optimistic warm-executor bound, deduplicated by
  digest;
- output bytes;
- action and artifact counts;
- digest fan-out and the gross-to-unique fan-out ratio; and
- responsible target labels.

Each scope also preserves its largest input and output artifacts and highest
digest exposure. The report additionally preserves every local-to-remote or
remote-to-local boundary. Neither byte estimate is provider billing.
Provider-accounted transfer is a later qualification measurement.

## Baseline and optimized reports

The checked-in machine-readable examples are:

- `tests/golden/bazel/cache-transfer-schema.json`
- `tests/golden/bazel/cache-transfer-baseline.json`
- `tests/golden/bazel/cache-transfer-optimized.json`
- `tests/golden/bazel/cache-transfer-representative.json`

An optimized report may carry a measured delta by passing the baseline report
to the analyzer:

```bash
cargo run --locked -p xtask -- bazel-cache-transfer \
  --execution-log .scratch/bazel-cache-transfer/optimized.json \
  --eligibility tests/golden/bazel/eligibility.json \
  --configuration local \
  --platform linux-x86_64 \
  --toolchain rules_rust \
  --baseline .scratch/bazel-cache-transfer/baseline-report.json \
  --output .scratch/bazel-cache-transfer/optimized-report.json
```

To compare existing reports, use:

```bash
cargo run --locked -p xtask -- bazel-cache-transfer compare \
  --baseline .scratch/bazel-cache-transfer/baseline-report.json \
  --optimized .scratch/bazel-cache-transfer/optimized-report.json
```

A representative local log of `//packages/d2b-core:d2b_core_test` and
`//packages/d2b-contracts:d2b_contracts_test` measured 207 actions, about
163 GB gross inputs, 1.03 GB unique inputs, and a 157x fan-out. The largest
remote set that still fits the 80 GB working budget of the 100 GB monthly
allowance is `ExtractCargoTomlEnvVars` plus `TestRunner` (59 actions, 467 MB
gross, 82 MB unique, about 171 cold runs per month). Adding
`CargoBuildScriptRun` stays local because it dominated measured remote-cache
bytes. `Rustc` runs on BuildBuddy: client cache traffic stayed low while
internal executor CAS hydration is large. Enabling rules_rust pipelined
compilation on that same pair increased action count, gross inputs, and
fan-out, so it is not part of the default configuration.

Comparison rejects schema, graph, eligibility, configuration, platform, and
toolchain mismatches. There are no arbitrary transfer thresholds in this
milestone. Remote-cache read and write bytes are the provider evidence used to
qualify BuildBuddy. Keep credentials and header flags out of local
report generation.
