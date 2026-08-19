# Bazel and BuildBuddy

d2b uses one Bazel graph for the enforcing Layer-1 checks. Cargo manifests
and the root `Cargo.lock` remain authoritative for Rust package membership,
dependencies, features, and direct Cargo workflows. `rules_rs` supplies the
Bazel-side Cargo integration; BUILD files record first-party edges and
maintained rule exceptions only.

The normal entry point is:

```bash
make check
```

## One execution graph

Bazel owns Layer-1 target selection, dependency ordering, parallelism, test
caching, retry classification, and aggregation. Make targets are public
compatibility aliases over fixed Bazel target sets. CI runs the same fixed
sets with the local profile and exposes one stable required `check` result.

The primary aliases remain available:

```bash
make check-tier0
make check-inventory
make test-lint
make test-rust
make test-proofs
make test-flake
make test-nix-unit
make test-policy
make test-drift
make test-runtime-ledger
make test-fixture-contracts
make test-unit
make check
```

Each alias invokes Bazel once. Underlying labels remain directly runnable for
focused reruns. The complete aggregate is also available as
`make bazel-check`.

Do not add a second Cargo lock, exhaustive first-party source or dependency
inventory, discovery job, or repository-owned scheduler. Add Rust files and tests in Cargo-conventional
locations so the graph follows Cargo metadata and standard source globs.

## Local and CI profiles

The committed `.bazelrc` defines:

| Profile | Purpose |
| --- | --- |
| `local` | Local execution with no remote executor, cache, or BES |
| `remote` | Developer BuildBuddy execution and cache |
| `trusted-seed` | Protected `v3` cache seeding with synchronous uploads |

Remote profiles use the BuildBuddy Linux worker contract, Ubuntu GCC
toolchain, minimal output downloads, compressed cache blobs, zero Bazel remote
retries, and a bounded job count. Nix, fixture, hardware, and other local-only
actions are tagged so they remain local and remote-disabled.

GitHub Layer-1 jobs set `D2B_BAZEL_PROFILE=local` and
`D2B_BAZEL_UNTRUSTED=1`; they receive no BuildBuddy credential. The fixed job
set is committed in `.github/workflows/pr-l1-static-fast.yml` and must remain
aligned with the public Make aliases.

## Credentials and trust selection

Store the developer API key as one line in the protected file named by
`D2B_BUILDBUDDY_CREDENTIAL_FILE`. The default is:

```text
~/.config/d2b/buildbuddy-api-key
```

`tests/tools/bazel-check` is Bazel's credential helper and execution facade.
It reads the key only for Bazel's credential-helper request and writes it
only as the helper response. Never add `--remote_header`, `--bes_header`, an
API key, or a bearer value to `.bazelrc`, command arguments, action
environments, platform properties, or committed evidence.

The facade preserves the selected target set when execution changes:

- missing or withheld credentials select `local` before Bazel starts;
- untrusted GitHub jobs always select `local`;
- `trusted-seed` requires `D2B_BAZEL_TRUSTED=1`, `GITHUB_REF=refs/heads/v3`,
  and an allowlisted security digest;
- a clearly pre-dispatch missing-credential, authentication, or endpoint
  failure permits one identical local retry; worker and transport failures
  require explicit pre-dispatch evidence, except for a remote gRPC deadline;
- analysis, policy, build, test, and post-dispatch failures fail closed.

The trusted security digest covers the committed remote profile, module lock,
platform, remote policy, and credential-helper inputs listed in
`tests/golden/bazel/cache-policy.json`. Refresh it only after reviewing those
bytes:

```bash
cargo run --quiet --locked -p xtask -- bazel-evidence security-digest
cargo run --quiet --locked -p xtask -- bazel-evidence check-security
```

## Redaction and failure output

The facade writes redacted logs and BEP output below
`.scratch/bazel-check/`. `bazel-evidence redact-log` rejects credential keys,
authorization values, header authentication fields, and configured sentinel
values before evidence is published while preserving safe failure and dispatch
hints for classification. The same redaction applies to local fallback output.

`bazel-evidence classify-failure` is the typed fallback classifier. It
distinguishes positively pre-dispatch infrastructure failures, plus a remote
gRPC deadline, that permit the one local retry from ambiguous, post-dispatch,
or check failures that must fail closed. A successful Bazel invocation must
also emit at least one `testResult` event in its BEP.

Reproduce a failure through the same alias and profile:

```bash
D2B_BAZEL_PROFILE=local make bazel-check
D2B_BAZEL_PROFILE=local make test-rust-main
```

This keeps target exclusions, tags, credential handling, redaction, and
fallback behavior identical to the normal graph.

## Updating the graph

For an ordinary Rust change:

1. Update Cargo source, tests, or manifests.
2. Run the focused Cargo and Bazel labels.
3. Run the affected public Make alias.

For a dependency change, update Cargo manifests and the root lock first.
Refresh `MODULE.bazel.lock` only for an intentional Bazel module change; the
normal `.bazelrc` lockfile mode fails closed on stale resolution.

Keep Nix assertions Nix-native through the existing Bazel adapters. Keep
doctests, harness-free binaries, feature variants, fixtures, policy checks,
and advisory leaves as explicit graph members. Do not replace them with a
shell rollup or a second scheduler.
