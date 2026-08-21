# Bazel and BuildBuddy

d2b uses one Bazel graph for the enforcing Layer-1 checks. Cargo manifests
and the root `Cargo.lock` remain authoritative metadata for Rust package
membership, dependencies, and features consumed by rules_rs. BUILD files
record first-party edges and maintained rule exceptions only; Cargo is not a
contributor or CI gate.

The normal entry point is:

```bash
make check
```

Run any public `make check*` or `make test*` alias directly from a
Nix-enabled host. The Makefile detects the pinned d2b shell contract and
re-enters `nix develop --no-write-lock-file .#bazel` exactly once when the
contract is absent. It does not trust an unrelated Nix shell or a bare
`IN_NIX_SHELL`; the shell must provide `D2B_PROJECT_SHELL=d2b` and an
executable `D2B_BAZEL_BIN`. Multiple goals, `-j` parallelism, target-specific
variables, the working directory, profile/trust variables, and the final exit
status are preserved across re-entry. A re-entry with an incomplete contract
fails closed instead of recursing.

For an interactive session, use the complete shell:

```bash
nix develop
```

The focused shell is self-contained for Make/facade/Bazel commands:

```bash
nix develop --no-write-lock-file .#bazel
nix develop --no-write-lock-file .#bazel -c bazel test //packages/<crate>:<owner-test>
```

Direnv is optional; it may enter `nix develop` automatically but is not part
of the contributor or CI contract. CI installs Nix and invokes the same
public Make aliases with `D2B_BAZEL_PROFILE=local` and
`D2B_BAZEL_UNTRUSTED=1`.

## One execution graph

Bazel owns Layer-1 dependency ordering, parallelism, test caching, retry
classification, and aggregation. Make targets are public thin aliases over
fixed Bazel target patterns and owner-local suites. CI runs the same fixed sets
with the local profile and exposes one stable required `check` result.

The primary aliases remain available:

```bash
make check-tier0
make test-lint
make test-rust
make test-proofs
make test-flake
make test-nix-unit
make test-policy
make test-drift
make test-fixture-contracts
make test-unit
make check
```

Each alias invokes Bazel once after any single shell re-entry. Underlying
labels remain directly runnable for focused reruns through the focused shell.
The complete aggregate is also available as
`make bazel-check`.

Do not add a second Cargo lock, exhaustive first-party source or dependency
inventory, discovery job, or repository-owned scheduler. Add Rust files and
tests in their owner-local Cargo-conventional locations so rules_rs metadata
and Bazel BUILD targets remain aligned.

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
actions are tagged `local` or `no-remote-exec`; `no-remote-cache` alone does
not make an action local. Heavy, container, VM, live-host, hardware, fixture,
and performance lanes remain explicit local lanes.

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
bazel run //packages/xtask:xtask -- bazel-evidence security-digest
bazel run //packages/xtask:xtask -- bazel-evidence check-security
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

1. Update owner-local Rust source/tests or the Cargo metadata consumed by
   rules_rs.
2. Run the focused Bazel label.
3. Run the affected public Make alias.

For a dependency change, update Cargo manifests and the root lock first.
Refresh `MODULE.bazel.lock` only for an intentional Bazel module change; the
normal `.bazelrc` lockfile mode fails closed on stale resolution.

Keep Nix assertions Nix-native through the existing Bazel adapters. Keep
doctests, harness-free binaries, feature variants, fixtures, policy checks,
and advisory leaves as explicit graph members. Do not replace them with a

## Action-locality checks

Use bounded Bazel queries when changing graph ownership. These checks describe
the expected dependency shape without adding a second scheduler or inventory:

| Representative change | Query | Expected shape |
| --- | --- | --- |
| USBIP Provider implementation | `bazel query 'rdeps(//packages/..., //packages/d2b-provider-device-usbip:d2b_provider_device_usbip)'` | The Provider's tests, `d2bd` composition/final link, and direct `d2b` integration tests only; no `d2bd-runtime`, `d2b-guestd`, sibling Provider, or foundational-contract target. |
| Broker contract | `bazel query 'rdeps(//packages/..., //packages/d2b-contracts-broker:d2b_contracts_broker)'` | Broker/control-plane consumers only; resource-only and desktop interaction Providers remain absent. |
| Network Nix surface | `bazel query 'kind("source file", deps(//bazel/checks/nix:nix-unit-network))'` | Only the network surface expression, its selected case, network modules, shared Nix evaluator helpers, and declared tools. Sibling Nix surfaces remain separate labels. |
| Documentation-only | `bazel query 'rdeps(//..., //:docs/reference/bazel-buildbuddy.md)'` | Documentation/source-hygiene owners only; no product Rust test or Nix surface target. |
| Cargo manifest or lock | `bazel query 'rdeps(//..., //:Cargo.lock)'` | Legitimate rules_rs/fixture consumers plus workspace-lock and supply-chain policies; unrelated documentation and Nix-unit labels remain absent. |

Use `bazel aquery` for the affected owner label when action-level confirmation
is needed. Keep any experiment in `.scratch/`; never commit a changed-file
selector, cache-evidence pin, or generated action inventory. Do not replace them with a
shell rollup or a second scheduler.
