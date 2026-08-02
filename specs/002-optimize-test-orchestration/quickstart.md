# Quickstart: Validate Test Orchestration Speedups

## Prerequisites

Run from the feature worktree:

```bash
cd /path/to/d2b-test-speedup
nix develop
```

Use a committed tree before collecting Nix evidence. Record:

```bash
git rev-parse HEAD
nproc
free -h
rustc --version
cargo --version
nix --version
```

Select the evidence directory for the revision being measured:

```bash
export D2B_EVIDENCE_DIR=.scratch/test-speedup-baseline
mkdir -p "$D2B_EVIDENCE_DIR"
```

Use `.scratch/test-speedup-optimized` instead when measuring the optimized
revision.

The target contracts are documented in
[contracts/local-validation-targets.md](./contracts/local-validation-targets.md).

## Capture coverage inventories

Rust:

```bash
(
  set -euo pipefail
  (
    cd packages
    cargo nextest list --workspace --message-format oneline
    cargo test --workspace --doc -- --list

    cargo test \
      --manifest-path d2b-priv-broker/Cargo.toml \
      --workspace -- --list
    cargo test \
      --manifest-path d2b-priv-broker/Cargo.toml \
      --workspace --features layer1-bootstrap -- --list
    cargo test \
      --manifest-path d2b-priv-broker/Cargo.toml \
      --workspace --features fake-backends -- --list

    cargo nextest list \
      --manifest-path d2b-guest-shell-runner/Cargo.toml \
      --workspace --features real-libshpool \
      --message-format oneline
    cargo test \
      --manifest-path d2b-guest-shell-runner/Cargo.toml \
      --workspace --features real-libshpool --doc -- --list
  ) | LC_ALL=C sort -u > "$D2B_EVIDENCE_DIR/test-rust-inventory.txt"
)

bash tests/tools/assert-pinned-tests.sh
```

The implementation must additionally compare the discovered `harness = false`
target set used by `run_nextest_companions`; these targets do not expose a
libtest case listing.

Nix unit and flake checks:

```bash
(
  set -euo pipefail
  system=$(nix eval --raw --impure --expr builtins.currentSystem)
  flake_ref="git+file://$(git rev-parse --show-toplevel)"

  nix eval --json "${flake_ref}#checks.${system}" \
    --apply 'checks: builtins.filter (name: name == "nix-unit" || builtins.substring 0 9 name == "nix-unit-") (builtins.attrNames checks)' \
    > "$D2B_EVIDENCE_DIR/test-nix-unit-inventory.json"

  nix eval --json "${flake_ref}#checks.${system}" \
    --apply builtins.attrNames \
    > "$D2B_EVIDENCE_DIR/test-flake-inventory.json"
)
```

Static discovery proves that required tests still exist, but it does not prove
that the aggregate target executed them. Capture an execution manifest from
each optimized aggregate run:

```bash
D2B_EXECUTION_MANIFEST="$D2B_EVIDENCE_DIR/test-rust-executed.json" \
  make test-rust
D2B_EXECUTION_MANIFEST="$D2B_EVIDENCE_DIR/test-nix-unit-executed.json" \
  make test-nix-unit
D2B_EXECUTION_MANIFEST="$D2B_EVIDENCE_DIR/test-flake-executed.json" \
  make test-flake
```

For the baseline commit, retain the full command traces from the actual public
target runs and record the completed baseline leaves in the corresponding
`*-executed.json` files. Each entry must cite the trace line proving that the
leaf completed. The optimized manifests must contain every baseline leaf.

## Warm-cache benchmark

Install the external benchmark tool without adding it to the repository:

```bash
nix shell nixpkgs#hyperfine
```

Prime each target once, then collect three timed samples:

```bash
make test-rust
hyperfine --runs 3 --export-json "$D2B_EVIDENCE_DIR/test-rust.json" \
  'make test-rust'

make test-nix-unit
hyperfine --runs 3 --export-json "$D2B_EVIDENCE_DIR/test-nix-unit.json" \
  'make test-nix-unit'

make test-flake
hyperfine --runs 3 --export-json "$D2B_EVIDENCE_DIR/test-flake-direct.json" \
  'make test-flake'
```

Run the same procedure on the accepted baseline commit and the optimized
commit. The Rust and Nix unit optimized medians must be no greater than half
their matching baseline medians.

For the flake hard target, benchmark the legacy local Layer-1 path at the
baseline commit:

```bash
D2B_FLAKE_LOCAL_SHARDS=1 make test-flake
hyperfine --runs 3 --export-json "$D2B_EVIDENCE_DIR/test-flake-layer1.json" \
  'D2B_FLAKE_LOCAL_SHARDS=1 make test-flake'
```

After implementation, benchmark the optimized local path:

```bash
make test-flake
hyperfine --runs 3 --export-json "$D2B_EVIDENCE_DIR/test-flake-layer1.json" \
  'make test-flake'
```

The optimized Layer-1 median must be no greater than half the legacy shard
median. Compare the direct baseline and optimized direct measurements
separately; the optimized direct path may regress by no more than 20%.

## Resource utilization evidence

For each representative warm run, sample the target process tree and available
cgroup v2 counters at one-second intervals. Record the effective CPU budget,
the interval from the first CPU-heavy leaf starting through the last
CPU-heavy leaf completing, process-tree user plus system CPU time, peak
CPU-consuming workers, peak memory, `memory.events` deltas, memory PSI, and
swap activity in
`.scratch/test-speedup-optimized/resource-stability.json`.

Calculate CPU-budget utilization as:

```text
process CPU seconds / (CPU-heavy interval seconds * effective CPU budget)
```

The median representative warm run for each target must reach at least 80%.
A lower value is acceptable only when the evidence identifies a non-CPU
bottleneck and proves the selected candidate exhausted viable concurrency for
that interval. Reject a run or candidate if an active CPU-quota frontier
exceeds the budget, workers exceed the declared bound, peak memory exceeds the
calculated envelope, or the target causes an OOM event, sustained
memory-pressure stall, or swap thrashing.

## Cold-cache observation

Cold results are best-effort and non-blocking. Do not clear the shared Nix
store.

For Rust, use the repository's guarded cleanup while retaining the shared
compiler cache:

```bash
D2B_CLEAN_SKIP_GC=1 make clean
make test-rust
```

For Nix evaluation, use a fresh evaluator cache directory:

```bash
(
  set -euo pipefail
  cache_dir=$(mktemp -d)
  trap 'rm -rf -- "$cache_dir"' EXIT
  XDG_CACHE_HOME="$cache_dir" make test-nix-unit
  XDG_CACHE_HOME="$cache_dir" make test-flake
)
```

Repeat as required by the benchmark record. Remove only the explicitly created
temporary directory.

## Failure behavior

Introduce or select an existing controlled failing test on a disposable branch
and confirm:

- the public target returns nonzero;
- other independent leaves are allowed to finish;
- each observed failure is attributed to its leaf or check;
- output from concurrent leaves is not interleaved beyond readability.

Do not retain the intentional failure in the implementation branch.

## Final comparison

Confirm that every baseline item remains present:

```bash
(
  set -euo pipefail

  comm -23 \
    .scratch/test-speedup-baseline/test-rust-inventory.txt \
    .scratch/test-speedup-optimized/test-rust-inventory.txt

  jq -r '.[]' .scratch/test-speedup-baseline/test-nix-unit-inventory.json \
    | LC_ALL=C sort > .scratch/test-speedup-baseline/test-nix-unit-inventory.txt
  jq -r '.[]' .scratch/test-speedup-optimized/test-nix-unit-inventory.json \
    | LC_ALL=C sort > .scratch/test-speedup-optimized/test-nix-unit-inventory.txt
  comm -23 \
    .scratch/test-speedup-baseline/test-nix-unit-inventory.txt \
    .scratch/test-speedup-optimized/test-nix-unit-inventory.txt

  jq -r '.[]' .scratch/test-speedup-baseline/test-flake-inventory.json \
    | LC_ALL=C sort > .scratch/test-speedup-baseline/test-flake-inventory.txt
  jq -r '.[]' .scratch/test-speedup-optimized/test-flake-inventory.json \
    | LC_ALL=C sort > .scratch/test-speedup-optimized/test-flake-inventory.txt
  comm -23 \
    .scratch/test-speedup-baseline/test-flake-inventory.txt \
    .scratch/test-speedup-optimized/test-flake-inventory.txt

  jq -r '.completed_leaves[]' \
    .scratch/test-speedup-baseline/test-rust-executed.json | LC_ALL=C sort \
    > .scratch/test-speedup-baseline/test-rust-executed.txt
  jq -r '.completed_leaves[]' \
    .scratch/test-speedup-optimized/test-rust-executed.json | LC_ALL=C sort \
    > .scratch/test-speedup-optimized/test-rust-executed.txt
  comm -23 \
    .scratch/test-speedup-baseline/test-rust-executed.txt \
    .scratch/test-speedup-optimized/test-rust-executed.txt
)
```

Repeat the executed-manifest comparison for Nix unit and flake manifests. Each
command must produce no missing baseline items. Use `comm -13` on the same
pairs to list and classify newly added orchestration tests or leaves.

Then run the targeted infrastructure checks named in
[plan.md](./plan.md#validation-strategy).

Execution manifests preserve each target's own contract; they do not merge
separate Layer-1 jobs into `test-rust`. Run the adjacent enforcing policy and
fixture lanes explicitly:

```bash
make test-policy
make test-fixture-contracts
```
