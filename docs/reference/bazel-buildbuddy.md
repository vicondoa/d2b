# Bazel and BuildBuddy

d2b pins the unmodified upstream Bazel 9.2.0 release and uses Bazel as an
enforcing execution path inside `make check`. BuildBuddy supplies remote
execution, action caching, and invocation results for eligible developer
work. Tests that need host devices, privileged host state, nested build tools,
or non-hermetic fixtures remain in the existing local, preflight, or
integration lanes.

The normal contributor entry point is:

```bash
make check
```

No Bazel profile argument is needed. Local Layer-1 execution defaults to the
BuildBuddy `remote` profile and uses the `local` profile automatically when a
developer credential is unavailable. GitHub Actions deliberately uses Bazel
with the `local` profile and receives no BuildBuddy credential.

## Graph ownership

Cargo remains the source of truth for Rust workspace membership and dependency
resolution:

- `Cargo.toml` and `Cargo.lock` define the production Rust workspace.
- `MODULE.bazel` and `MODULE.bazel.lock` pin Bazel modules and
  `rules_rs` resolution.
- `@crates//:defs.bzl` derives third-party Cargo dependencies from the root
  lockfile through `all_crate_deps()` and `aliases()`.
- First-party `BUILD.bazel` targets use Cargo-conventional source globs.
  Explicit entries remain only for first-party edges and true rule exceptions.

Rust source and tests stay in Cargo-standard crate paths. Each crate has its
own Bazel package and test targets, so Bazel schedules, caches, and reports
crate tests separately instead of treating the workspace as one opaque test
command.

## How `make check` executes Bazel

`tests/layer1-jobs.json` is the shared local and CI scheduler manifest. During
a local `make check`, its parallel phase runs both `bazel-check` and
`test-rust`:

```text
make check
  bazel-check
    tests/tools/bazel-check --profile remote --leaf rest
  test-rust
    main workspace
      tests/tools/bazel-check --profile remote --leaf main
    privileged broker
      tests/tools/bazel-check --profile remote --leaf broker
    guest shell runner
      tests/tools/bazel-check --profile remote --leaf guest
    no-bash AST, schema, inventory, and supply-chain leaves
      local Cargo, Nix, and repository tools
  test-fixture-contracts
    separate local Cargo and Nix fixture lane
```

The target leaves prevent duplicate package test execution:

| Leaf | Scheduled by | Bazel target set |
| --- | --- | --- |
| `rest` | `make bazel-check` | Non-crate tests under `//tests/...` and `//bazel/checks/{nix,policy,fixtures}/...`; Gas City fixtures are excluded |
| `main` | `make test-rust-main` and `make test-rust` | `//packages/...` plus `//bazel/checks/rust/...`, excluding broker, guest shell runner, and compile-fail UI suites |
| `broker` | `make test-rust-broker` and `make test-rust` | `//packages/d2b-priv-broker/...` |
| `guest` | `make test-rust-guest-shell-runner` and `make test-rust` | `//packages/d2b-guest-shell-runner/...` |
| `all` | Manual diagnostics only | The combined graph, with the documented UI and Gas City exclusions |

`rest` does not rerun Rust package tests, and `main` does not rerun the broker
or guest shell runner. The manual `all` leaf intentionally overlaps the
scheduled leaves and must not be added to `make check`.

The facade adds `--build_tests_only`, prints test errors, and filters targets
tagged `local`, `manual`, `gpu`, or `kvm`. Targets tagged `exclusive` run only
from the broker leaf. Compile-fail UI suites remain on Cargo, and Gas City
fixture genrules remain outside this aggregate because they need a
user-namespace FHS environment.

## Local and CI commands

Use the Make targets for normal work:

```bash
# Full Layer-1 gate. BuildBuddy is the local default.
make check

# Non-crate Bazel leaf only.
make bazel-check

# Complete Rust DAG. Main, broker, and guest package tests use Bazel.
make test-rust

# Focused Bazel-backed Rust leaves.
make test-rust-main
make test-rust-broker
make test-rust-guest-shell-runner

# Explicitly disable BuildBuddy for a local reproduction.
D2B_BAZEL_PROFILE=local make check
```

For direct facade or Bazel commands, enter the focused shell so the exact
upstream binary and action shell are selected:

```bash
nix develop --no-write-lock-file .#bazel
bazel --version
tests/tools/bazel-check --profile local --leaf rest
tests/tools/bazel-check --profile remote --leaf main
```

The shell exports `D2B_BAZEL_BIN` and `BAZEL_SH`; do not substitute an ambient
Bazel installation when validating the repository graph.

CI is generated from `tests/layer1-jobs.json`. It runs separate
`bazel-check`, `test-rust-main`, `test-rust-broker`, and
`test-rust-guest-shell-runner` jobs with:

```text
D2B_BAZEL_PROFILE=local
D2B_BAZEL_UNTRUSTED=1
```

The `test-rust` CI job is a rollup over those Bazel jobs and the remaining
local Rust jobs. It does not execute the crate tests again.

## BuildBuddy profiles and caching

The committed `.bazelrc` defines four profiles:

| Profile | Purpose |
| --- | --- |
| `local` | Same selected target leaf with no remote executor, cache, or Build Event Service |
| `remote` | Normal developer remote execution and cache namespace |
| `trusted-seed` | Protected `v3` cache seeding with synchronous uploads |
| `qualification` | Isolated measurement and provider-evidence runs |

The remote profiles use:

- `grpcs://d2b.buildbuddy.io` for execution, action cache, CAS, and BES;
- credential-helper authentication through `tests/tools/bazel-check`;
- the BuildBuddy Linux x86_64 platform and Ubuntu GCC toolchain;
- remote Rust compilation and build-script actions when they are eligible;
- `--remote_download_outputs=minimal` and remote cache compression;
- up to 50 Bazel jobs;
- zero Bazel remote retries, because the facade owns the single permitted
  local fallback; and
- separate developer, trusted-seed, and qualification instance names.

Bazel computes an action key from declared inputs, tools, command line,
environment, and execution platform. An unchanged action can therefore reuse
the BuildBuddy action cache and CAS even when another crate or test changed.
The developer instance name is intentionally shared across branches; branch
and commit names are not part of the namespace. Trust level, platform,
toolchain, worker contract, output mode, and module lock remain part of the
namespace or action identity so incompatible results cannot collide.

Minimal output download keeps successful intermediate artifacts in BuildBuddy
unless a downstream action or the user requests them. BES still publishes the
invocation graph, target results, and logs to:

```text
https://d2b.buildbuddy.io/invocation/
```

## Credentials and fallback

Store the developer API key as a single line in the protected file named by
`D2B_BUILDBUDDY_CREDENTIAL_FILE`. The default is:

```text
~/.config/d2b/buildbuddy-api-key
```

Use owner-only file permissions. The helper reads the key from the file and
returns it only through Bazel's credential-helper protocol. Never add
`--remote_header`, `--bes_header`, an API key, or a bearer value to `.bazelrc`,
the command line, an action environment, a platform property, or committed
evidence.

The facade preserves the selected target leaf when it changes execution mode:

- A missing or withheld credential selects `local` before Bazel starts.
- An untrusted GitHub job always selects `local`.
- A missing-credential, authentication, endpoint, worker, or transport failure
  before remote dispatch permits one retry of the identical leaf locally.
- An analysis failure, policy failure, test failure, build failure, or any
  post-dispatch uncertainty fails closed and is not retried locally.

Trusted seed and qualification profiles additionally require
`D2B_BAZEL_TRUSTED=1`, `GITHUB_REF=refs/heads/v3`, and an allowlisted security
digest from `tests/golden/bazel/cache-policy.json`.

## Logs and troubleshooting

The facade writes redacted output below `.scratch/bazel-check/`:

```text
<profile>.<leaf>.log
<profile>.<leaf>.bep.json
```

On failure it prints the captured log to stderr after redaction, including a
local fallback log when that fallback also fails. A successful run prints only
the facade result and the normal Make job summary.

Reproduce failures through the same leaf before calling Bazel directly:

```bash
D2B_BAZEL_PROFILE=local make bazel-check
D2B_BAZEL_PROFILE=local make test-rust-main
```

This preserves the target exclusions, tags, test environment, redaction, and
fallback policy used by `make check`.

## Updating first-party targets

For an ordinary Rust crate or test change:

1. Update the Cargo-standard source, test, and manifest files.
2. Run the focused Cargo and Bazel targets, then `make check`.

Do not add a repository-owned generator or dependency inventory. Keep
exceptional checked-in rules explicit and reviewable.

## Updating dependencies and modules

Cargo dependency changes start in Cargo manifests and locks. Bazel module or
toolchain dependency changes start in `MODULE.bazel`. Refresh the Bzlmod lock
only for an intentional dependency change:

```bash
bazel mod deps --lockfile_mode=update
```

Normal commands use `.bazelrc` with `--lockfile_mode=error`, so a stale or
implicitly rewritten `MODULE.bazel.lock` fails rather than changing during a
test run.

Because `MODULE.bazel` and `MODULE.bazel.lock` are trusted-injection inputs,
review their resolved sources and refresh the allowlisted security digest:

```bash
cargo run --quiet --locked -p xtask -- bazel-evidence security-digest
```

Copy a newly reviewed digest into
`tests/golden/bazel/cache-policy.json`, then verify it:

```bash
cargo run --quiet --locked -p xtask -- bazel-evidence check-security
```

Never allowlist an unexplained digest only to make the check pass.

## Updating Bazel itself

Bazel is an exact upstream binary, not a nixpkgs wrapper and not a patched
fork. A version update must keep all version-bearing surfaces in sync:

- `.bazelversion`;
- `pkgs/bazel-<version>/default.nix`, including official x86_64 and aarch64
  release URLs and hashes;
- the Bazel provider, package, shell, and check bindings in `flake.nix`;
- `tests/unit/smoke/bazel-provider.nix`;
- the provider source path in the root `BUILD.bazel`;
- the fallback binary selection in `tests/tools/bazel-check`; and
- version references in contributor documentation and changelog entries.

Use `rg` for both the old version and its compact Nix binding name before
declaring the update complete. Verify the official release bytes and the
focused shell:

```bash
nix build --no-link .#checks.x86_64-linux.bazel-9_2_0-provider-smoke
nix develop --no-write-lock-file .#bazel -c bazel --version
```

Rename the check attribute for a new version instead of leaving
`bazel-9_2_0-provider-smoke` behind.

## Updating remote policy or CI mapping

Changes to `.bazelrc`, module locks, platforms, remote policy, or the facade
change trusted cache identity. After reviewing the change:

1. Recompute and allowlist the security digest as described above.
2. Regenerate cache-transfer evidence when the action graph, platform,
   toolchain, or remote policy changes. Follow
   [Local Bazel cache-transfer model](bazel-cache-transfer.md).
3. Run `cargo run --quiet --locked -p xtask -- bazel-evidence check-u9`.
4. If `tests/layer1-jobs.json` changed, regenerate and verify CI:

   ```bash
   make layer1-workflow
   make layer1-workflow-check
   ```

5. Run the focused leaf and bare `make check`.

The cache-transfer and qualification tools measure remote suitability; they do
not replace the Make entry points or create a second scheduler.
