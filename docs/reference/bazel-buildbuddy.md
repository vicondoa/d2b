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
of the contributor or CI contract. CI installs Nix and invokes the trusted
`v3` Make aliases from the protected checkout. Remote-eligible jobs use the
brokered BuildBuddy credential; local-only jobs do not receive it.

## One execution graph

Bazel owns Layer-1 dependency ordering, parallelism, test caching, retry
classification, and aggregation. `bazel/checks/BUILD.bazel` is the public
suite facade: package-level Rust suites and component suites compose each
public Make target without duplicating a fixed label graph in Make. CI runs the
same nested suites with remote-eligible Rust and policy actions on BuildBuddy,
while Nix, fixture, hardware, and explicitly local actions remain local. It
exposes one stable required `check` result.

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

Each alias invokes one matching `//bazel/checks:<target>` suite after any
single shell re-entry. Underlying owner labels remain directly runnable for
focused reruns through the focused shell.
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

The credential-bearing PR gate is a `pull_request_target` workflow owned by
protected `v3`; push seeding is also limited to `v3`. Each job checks out the
event's immutable base into `trusted` and the immutable tested merge/push
commit into `workspace`. The workflow executes the `v3` Makefile and trusted
shell/bootstrap only. Remote jobs use `D2B_BAZEL_PROFILE=remote` and
`D2B_BAZEL_REQUIRE_REMOTE=1`; local-only jobs use `local`. The fixed job set is
committed in `.github/workflows/pr-l1-static-fast.yml` and must remain aligned
with the public Make aliases.

## Developer invocation metadata

For developer `remote` and protected `trusted-seed` runs, `tests/tools/bazel-check`
derives one checkout-bound metadata contract before invoking Bazel:

```text
REPO_URL=https://github.com/vicondoa/d2b
COMMIT_SHA=<40-hex commit at HEAD>
BRANCH_NAME=<validated symbolic branch name>
```

The facade reads all three values from the repository root it will test. It
clears inherited Git repository-selection and configuration environment,
verifies the discovered top-level directory, and reads the origin from local
repository configuration. It accepts only canonical d2b Git remotes, a full
commit object id, and a Git-valid local branch under `refs/heads/`. The branch
and commit come from one Git status snapshot, and the facade requires two
identical snapshots before publishing them. It passes the values as Bazel
invocation metadata (`--build_metadata=...`) to both developer profiles; local
execution omits them. The contract uses no `GITHUB_*`, `CI_*`, user, host,
credential, or workspace-path environment values, and never forwards a remote
URL containing credentials.

If Git is unavailable, the origin is not canonical, `HEAD` cannot be resolved,
the checkout is detached, or the branch and commit change while the tuple is
collected, the facade emits an explicit diagnostic and omits all three fields
rather than publishing a partial or misleading revision. This is not a local
retry condition.

`--build_metadata` is Build Event Service invocation metadata, not an action
input. It therefore does not change ordinary action keys or invalidate
reusable outputs, and the global `--stamp=no` contract remains unchanged.

Trusted CI additionally validates the protected base, PR head, tested merge,
trusted checkout, run id, PR number, branch, workflow reference, and event
before invoking Bazel. It publishes those immutable OIDs and run/linkage
fields as BuildBuddy metadata, and derives a cache instance namespace from the
PR number and head SHA (`d2b/pr/<number>/<head-sha>/...`). Pushes use the
separate trusted `v3` namespace. A stale checkout or mismatched event fails
closed.

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

For GitHub Actions, add the repository secret
`D2B_BUILDBUDDY_API_KEY`. The trusted workflow passes it only over stdin to
`tests/tools/bazel-check-bootstrap`, which stores it in an anonymous memfd and
execs the trusted `v3` command with a descriptor number. The key is not placed
in ordinary action environment variables, arguments, repository files, Bazel
rc files, or test environments. Credential-bearing CI has no local fallback:
missing credentials, authentication failure, endpoint failure, or remote
execution failure fails the job instead of producing a reduced local gate.

The facade stages the tested source with trusted copies of `.bazelrc`,
`MODULE.bazel`, `MODULE.bazel.lock`, remote/platform BUILD files, and the
credential/shell helpers. It disables system, home, workspace, and PR
`.bazelrc.user` configuration and pins the endpoint and helper path on the
Bazel command line. PR cache writes never use the trusted seed namespace.

The trusted security digest covers the committed workflow, Makefile, module
lock, platform, remote policy, bootstrap, shell, and credential-helper inputs listed in
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
