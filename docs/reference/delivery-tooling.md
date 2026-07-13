# Delivery tooling

The repository's Nix flake provides the tools used to inspect and deliver
dependent pull requests on both supported Linux systems. Enter the reproducible
shell with:

```console
nix develop
```

The shell contains these exact tools:

| Tool | Pin |
| --- | --- |
| GitHub CLI | `2.92.0` from the locked nixpkgs input |
| official `github/gh-stack` | `0.0.7` |
| `cargo-udeps` | `0.1.61`, run with nightly `2025-12-01` |
| `cargo-semver-checks` | `0.47.0` |
| project Rust toolchain | `1.94.1` |

The source and dependency hashes are fixed in `pkgs/delivery-tools.nix`.
Nothing downloads a tool when the shell or a command starts. The focused
packages can also be built directly:

```console
nix build .#gh-stack .#cargo-udeps-nightly .#cargo-semver-checks
```

Use `cargo udeps` in the shell. Its wrapper selects the pinned nightly compiler
and Cargo only for that subprocess and provides `sccache`; ordinary `cargo`
remains the project's pinned stable toolchain. The shell also provides a C
toolchain, CMake, `pkg-config`, OpenSSL, Protobuf, and `sccache` for workspace
crates with native build dependencies.

## Stack authority and private preview

Official `gh-stack` is the only stack mutator. It owns stack creation,
restacking, submission, and retargeting. The delivery `xtask` imports the
checked-in manifest and declared graph while creating an immutable snapshot;
it never accepts a caller-authored manifest, substitutes `gh pr edit`, performs
custom API mutations, or rewrites local branches for a failed `gh stack`
operation.

Before creating a stack, verify that the repository has the private preview:

```console
nix run .#delivery -- stack capability \
  --repository github.com/example/d2b
```

The equivalent direct workspace invocation, run from `packages/`, is:

```console
cargo xtask delivery stack capability \
  --repository github.com/example/d2b
```

The capability check accepts only official `gh-stack` `0.0.7` and performs a
read-only query against GitHub's private-preview endpoint. Private-preview
availability is mandatory and fail-closed: if the preview is disabled, the
token cannot access it, the response is malformed, or the tool version differs,
the result is **cannot operate**. There is no fallback stack mutation. Enable
the preview or stop the stacked delivery.

After the check succeeds, mutate the stack only with `gh stack`, for example:

```console
gh stack init --base main feature-contracts feature-runtime
gh stack submit
```

## External evidence and check summaries

The Make-independent delivery app exposes the implemented tree-bound commands
below. Select a secure external state directory explicitly. The flake app
already selects the delivery executable, so its command starts with `wave`:

```console
install -d -m 0700 "$XDG_STATE_HOME/d2b/delivery"
nix run .#delivery -- wave snapshot \
  --authority-repository github.com/example/d2b \
  --authority-ref refs/heads/main \
  --manifest-path "$MANIFEST" \
  --repo "d2b=$CHECKOUT" \
  --state-dir "$XDG_STATE_HOME/d2b/delivery"
nix run .#delivery -- wave validation-import \
  --snapshot "$SNAPSHOT" --artifact "$ARTIFACT" --bundle "$BUNDLE" \
  --payload "$PAYLOAD" --repo "d2b=$CHECKOUT"
nix run .#delivery -- wave verify \
  --seal "$SEAL" --repo "d2b=$CHECKOUT"
nix run .#delivery -- wave eligibility \
  --seal "$SEAL" --target "$TARGET" --repo "d2b=$CHECKOUT"
```

`--payload` is optional for `validation-import`. To invoke the same CLI from
the Rust workspace instead, run from `packages/` and retain the mandatory
`delivery` namespace:

```console
cargo xtask delivery wave validation-import \
  --snapshot "$SNAPSHOT" --artifact "$ARTIFACT" --bundle "$BUNDLE" \
  --repo "d2b=$CHECKOUT"
cargo xtask delivery wave verify \
  --seal "$SEAL" --repo "d2b=$CHECKOUT"
cargo xtask delivery wave eligibility \
  --seal "$SEAL" --target "$TARGET" --repo "d2b=$CHECKOUT"
```

Use `$XDG_STATE_HOME/d2b/delivery` when `XDG_STATE_HOME` is available. Otherwise,
pass `--state-dir` an explicitly selected, operator-owned external directory
with mode `0700`. Snapshots, evidence, panel records, and seals stay outside
every reviewed checkout and outside every Git directory or common directory.
Git metadata is never delivery state, and these artifacts must never be added
to the reviewed tree. The app bundles pinned `git`, `gh`, and `gh-stack`
executables so status inspection does not depend on an ambient command version.

The non-generated pull-request workflows emit a small GitHub step summary bound
to the exact candidate SHA reported by `git rev-parse HEAD` and to step
outcomes. A summary is status metadata, not validation or panel evidence, and
cannot satisfy a seal. The merge-eligibility command independently reads
GitHub's check rollup and fails closed for a missing, duplicate, pending,
failed, or malformed required check.

## Validation ownership

When realized, the `delivery-tooling` flake check verifies the locked GitHub
CLI, `gh-stack`, Rust toolchains, Cargo tools, offline workspace metadata, and
native OpenSSL build inputs. On x86_64 Linux it is discovered and instantiated
as its own hosted CI flake-check shard. `make check` performs the same no-build
instantiation through the bounded local flake shards;
`D2B_FLAKE_CHECK=delivery-tooling make test-flake` selects it directly. Build
`.#checks.x86_64-linux.delivery-tooling` to realize the check. The committed
x86_64 flake-check matrix pin makes additions or removals fail the drift gate
until `make flake-matrix-pin` is reviewed.
