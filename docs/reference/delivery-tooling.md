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
| GitHub CLI | `2.92.0` |
| Git Town | `23.0.1` |
| `cargo-udeps` | `0.1.61`, run with nightly `2025-12-01` |
| `cargo-semver-checks` | `0.47.0` |
| project Rust toolchain | `1.94.1` |

Every tool source and dependency set is fixed in `pkgs/delivery-tools.nix`.
GitHub CLI and Git Town are repository-owned source builds rather than aliases
to the versions in a consumer-followed nixpkgs input. Nothing downloads a tool
when the shell or a command starts. The focused packages can also be built
directly:

```console
nix build .#gh .#git-town .#cargo-udeps-nightly .#cargo-semver-checks
```

Use `cargo udeps` in the shell. Its wrapper selects the pinned nightly compiler
and Cargo only for that subprocess and provides `sccache`; ordinary `cargo`
remains the project's pinned stable toolchain. The shell also provides a C
toolchain, CMake, `pkg-config`, OpenSSL, Protobuf, and `sccache` for workspace
crates with native build dependencies.

## Stack authority

Git Town is the only stack topology, propose, and synchronization mutator. It
owns parent relationships, restacking, PR creation/update, and retargeting.
The delivery `xtask` independently reconstructs the parent chain with
`git-town config get-parent` and reads ordinary GitHub PR authority. Active
branches must match exact local refs. Merged prefixes instead retain their
historical PR head, merge base, merge commit, and merge tree, so a deleted or
advanced local branch ref does not erase merge authority. The checked-in
manifest remains the expected ordered graph. `xtask` never accepts a
caller-authored graph, edits PR topology, or rewrites branches.

The historical authority remains `delivery/manifest.json`. New independent
delivery lines select a checked-in `delivery/manifests/w<N>.json` path. The file
name must match its declared wave, its own path must appear in
`contract_fingerprints`, and only one tracked authority may declare a given
wave. Selecting a per-wave path does not relax graph checks: the ordered
branches, ordinary PR numbers, active terminal integration ref, and every
immediate Git Town parent must still match exactly.

The capability probe verifies the supported Git Town major, noninteractive
propose flags, GitHub authentication, repository read access, and the
ordinary pull-request API:

```console
nix run .#delivery -- stack capability \
  --repository github.com/example/d2b
```

The equivalent direct workspace invocation, run from `packages/`, is:

```console
cargo xtask delivery stack capability \
  --repository github.com/example/d2b
```

The result is typed JSON. It accepts supported Git Town `23.x`
binaries and fails closed when Git Town, required noninteractive flags, GitHub
authentication, repository read access, or the ordinary PR API is absent or
unverifiable. It does not require special GitHub enrollment.

The configured Git Town main branch is the perennial root; feature branches
form one immediate-parent chain above it. Git Town must verify every configured
parent, and dirty worktrees or missing local parent refs are hard failures.
A direct `git push` may only publish commits on a branch whose parent is already
configured and verified by Git Town; it cannot create, change, restack, or
retarget topology. Git Town owns every ordinary PR create/update and every
topology or synchronization change. Merging remains the delivery `xtask`
exact-base-and-head compare-and-swap path or the GitHub merge queue.

For the noninteractive setup, propose, update, and retarget procedure, see
[Manage stacked wave pull requests with Git Town](../how-to/manage-stacked-wave-prs.md).

## Wave ownership authority

The post-W4 W5, W6, and W7 branches are checked by tooling built from their
trusted immediate parent, not by the candidate's copy of `xtask`. Keep a clean
worktree at the exact parent commit corroborated by Git Town and the candidate's
ordinary GitHub PR, then run:

```console
make -C "$TRUSTED_PARENT_ROOT" wave-policy-check \
  CANDIDATE_ROOT="$WAVE_WORKTREE"
```

Do not run this target from the wave worktree. The trusted command accepts no
`--wave` or `--base`: it derives the wave from the candidate's canonical branch
stem, reads the immediate parent with `git-town config get-parent`, discovers
the unique open ordinary PR in the policy-pinned repository, and requires its
local and GitHub base/head refs and OIDs to match. It walks every W5/W6 ancestor
and corroborates each Git Town edge with that branch's ordinary PR through the
shared root. Its own clean source worktree must be checked out at the
candidate's exact immediate base commit. A base equal to its branch `HEAD`
fails.

The checker reads `delivery/shared-contracts.json` from the verified parent Git
object, reads the selected per-wave manifest from the candidate `HEAD` object,
and computes a no-rename parent-to-head path diff. Candidate edits to the
checker, delivery implementation, Make target, or policy are therefore judged
by the parent policy and cannot exempt themselves.

The policy's implementation partition is fail-closed across waves:

| Wave | Owned implementation |
| --- | --- |
| W5 | Core CLI/client/daemon, realm, guest, provider-agent, broker, host, and allocator crate prefixes |
| W6 | Userd, systemd-user/shell, clipboard, notify/wlcontrol, Wayland, security-key, activation, TTY, and retained-helper crate prefixes |
| W7 | `nixos-modules/`, `pkgs/`, `examples/`, and `templates/` |

Each wave records the exact union of the other waves' prefixes as foreign.
Shared-root contracts and tooling stay protected for every wave; W7 retains
only the policy's narrow provider-registry and `flake.nix` exceptions. Before
linearization all three waves use the shared root as parent. In the final
W5-to-W6-to-W7 chain, W6 uses W5 and W7 uses W6 as the exact trusted parent;
partial linearization is rejected.

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
  --payload "$PAYLOAD" --repo "d2b=$CHECKOUT"
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
to the reviewed tree. The app bundles pinned `git`, `gh`, and `git-town`
executables so status inspection does not depend on an ambient command version.

The non-generated pull-request workflows emit a small GitHub step summary bound
to the exact candidate SHA reported by `git rev-parse HEAD` and to step
outcomes. A summary is status metadata, not validation or panel evidence, and
cannot satisfy a seal. The merge-eligibility command independently reads
GitHub's check rollup and fails closed for a missing, duplicate, pending,
failed, or malformed required check.

## Validation ownership

When realized, the `delivery-tooling` flake check verifies the
repository-pinned GitHub CLI, Git Town, Rust toolchains, Cargo tools, offline
workspace metadata, and native OpenSSL build inputs. On x86_64 Linux it is
discovered and instantiated as its own hosted CI flake-check shard. `make check`
performs the same no-build instantiation through the bounded local flake shards;
`D2B_FLAKE_CHECK=delivery-tooling make test-flake` selects it directly. Build
`.#checks.x86_64-linux.delivery-tooling` to realize the check. The committed
x86_64 flake-check matrix pin makes additions or removals fail the drift gate
until `make flake-matrix-pin` is reviewed.
