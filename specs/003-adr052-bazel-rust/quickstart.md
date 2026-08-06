# Quickstart: Validate Spec 003 Under ADR 0054

This guide is for implementation and review waves. Commands do not exist until
their named wave lands. Run them from a committed scope-owned worktree.

## Prerequisites

```bash
set -euo pipefail
export D2B_WORKTREE=/absolute/path/to/the/worktree
cd "$D2B_WORKTREE"
test -z "$(git status --porcelain --untracked-files=all)"
nix develop
rustc --version
```

Expected stable Rust is the pin in `packages/rust-toolchain.toml`. Entering
`packages/` is load-bearing for contributor Cargo and xtask commands:

```bash
set -euo pipefail
cd packages
cargo --version
cd ..
```

Do not use parked historical `spec003-w0-*` or `spec003-w0` branches as
implementation input. Before spec003w0:

```bash
set -euo pipefail
git merge-base --is-ancestor a7093601 HEAD
test -f packages/d2b-priv-broker/Cargo.lock
test -f packages/d2b-guest-shell-runner/Cargo.lock
for path in \
  .bazelversion \
  .bazelrc \
  .bazelignore \
  MODULE.bazel \
  MODULE.bazel.lock \
  BUILD.bazel \
  bazel
do
  test ! -e "$path"
done
```

These checks describe the pre-implementation base. They are expected to invert
after spec003w0.

## spec003w0 product workspace

Run:

```bash
set -euo pipefail
assert_clean() {
  git diff --exit-code -- "$@"
  git diff --cached --exit-code -- "$@"
  test -z "$(git status --porcelain --untracked-files=all -- "$@")"
}
assert_clean packages/Cargo.lock
(cd packages && cargo generate-lockfile --offline)
assert_clean packages/Cargo.lock
(cd packages && cargo metadata --locked --offline --format-version 1) \
  > .scratch/spec003-product-metadata.json
```

Verify one resolver-v2 product workspace and the required members:

```bash
set -euo pipefail
jq -e '
  .resolve != null and
  ([.workspace_members[] as $id
    | .packages[]
    | select(.id == $id)
    | .name]
    | index("d2b-priv-broker") != null) and
  ([.workspace_members[] as $id
    | .packages[]
    | select(.id == $id)
    | .name]
    | index("d2b-guest-shell-runner") != null)
' .scratch/spec003-product-metadata.json

python - <<'PY'
from pathlib import Path
import tomllib

root = tomllib.loads(Path("packages/Cargo.toml").read_text())
assert root["workspace"]["resolver"] == "2"
for path in (
    "packages/d2b-priv-broker/Cargo.toml",
    "packages/d2b-guest-shell-runner/Cargo.toml",
):
    manifest = tomllib.loads(Path(path).read_text())
    assert "workspace" not in manifest
    assert not any(key.startswith("profile") for key in manifest)

guest = tomllib.loads(
    Path("packages/d2b-guest-shell-runner/Cargo.toml").read_text()
)
assert guest["features"]["default"] == []
assert guest["features"]["real-libshpool"] == []
assert guest["dependencies"]["libshpool"] == "0.11.0"
PY

test ! -e packages/d2b-priv-broker/Cargo.lock
test ! -e packages/d2b-guest-shell-runner/Cargo.lock
test -e packages/Cargo.lock
test -e packages/Cargo.guest.lock
test -e tests/tools/no-bash-ast-walker/Cargo.lock
```

The tracked lock inventory must contain the product root lock, walker lock,
and generated guest closure lock, but no nested product lock:

```bash
set -euo pipefail
expected=$(
  printf '%s\n' \
    packages/Cargo.guest.lock \
    packages/Cargo.lock \
    tests/tools/no-bash-ast-walker/Cargo.lock
)
actual=$(
  git ls-files \
    packages/Cargo.lock \
    packages/Cargo.guest.lock \
    packages/d2b-priv-broker/Cargo.lock \
    packages/d2b-guest-shell-runner/Cargo.lock \
    tests/tools/no-bash-ast-walker/Cargo.lock \
    | sort
)
test "$actual" = "$expected"
```

`packages/Cargo.guest.lock` remaining tracked is expected. Treating it as a hub
is not. Lab, proof, and compile-fixture locks are outside this scoped authority
inventory.

## spec003w0 selected Cargo contexts

From `packages/`:

```bash
set -euo pipefail
CARGO_TARGET_DIR=d2b-priv-broker/target \
cargo test --locked -p d2b-priv-broker \
  --no-default-features -- --test-threads 1

CARGO_TARGET_DIR=d2b-priv-broker/target-layer1 \
cargo test --locked -p d2b-priv-broker \
  --no-default-features --features layer1-bootstrap -- --test-threads 1

CARGO_TARGET_DIR=d2b-priv-broker/target-fakebackends \
cargo test --locked -p d2b-priv-broker \
  --no-default-features --features fake-backends -- --test-threads 1

cargo fmt -p d2b-guest-shell-runner --check

CARGO_TARGET_DIR=d2b-guest-shell-runner/target \
cargo clippy --locked -p d2b-guest-shell-runner \
  --no-default-features --features real-libshpool \
  --all-targets -- -D warnings

CARGO_TARGET_DIR=d2b-guest-shell-runner/target \
cargo nextest run --locked -p d2b-guest-shell-runner \
  --no-default-features --features real-libshpool

cargo clippy --locked --workspace --all-targets \
  --exclude d2b-priv-broker \
  --exclude d2b-guest-shell-runner -- -D warnings

cargo nextest run --locked --workspace \
  --exclude d2b-contract-tests \
  --exclude d2b-priv-broker \
  --exclude d2b-guest-shell-runner
```

The broker commands remain serial and the explicit target directories are
gate-owned. Formatting is package-only and intentionally has no lock or
feature selector.

Inspect the exact selected-context oracle. It is a three-way join over the real
root workspace, never a synthetic manifest or splice. Locked offline
target-filtered metadata supplies package identities, sources, candidate edges,
and each edge's dependency kind and `cfg`; `packages/Cargo.lock` plus the
committed git archive pin supplies every registry and git checksum;
package-selected stable tree traversals supply the exact root, dependency-kind
reach, and resolved features:

```bash
set -euo pipefail
cd packages
cargo metadata --locked --offline --format-version 1 \
  --filter-platform x86_64-unknown-linux-gnu \
  > ../.scratch/spec003-oracle-metadata.json
jq -e '
  all(.packages[]; has("checksum") | not) and
  (.resolve.root == null) and
  ([.resolve.nodes[].deps[].dep_kinds[]] | length) > 0
' ../.scratch/spec003-oracle-metadata.json

for kinds in normal,build normal,build,dev; do
  cargo tree --locked --offline --manifest-path Cargo.toml \
    -p d2b-priv-broker --target x86_64-unknown-linux-gnu \
    --no-default-features --features layer1-bootstrap \
    --edges "$kinds" \
    --charset ascii --prefix depth --no-dedupe \
    --format '|{p}|{f}|' > "../.scratch/spec003-oracle-broker-$kinds.txt"
  test -s "../.scratch/spec003-oracle-broker-$kinds.txt"

  cargo tree --locked --offline --manifest-path Cargo.toml \
    -p d2b-guest-shell-runner --target x86_64-unknown-linux-musl \
    --no-default-features --features real-libshpool \
    --edges "$kinds" \
    --charset ascii --prefix depth --no-dedupe \
    --format '|{p}|{f}|' > "../.scratch/spec003-oracle-guest-$kinds.txt"
  test -s "../.scratch/spec003-oracle-guest-$kinds.txt"
done
cd ..
```

That flag set is pinned by measured behavior, not preference: `--prefix depth`
emits the depth integer with no separator, so the repository-pinned
`--format` must begin with a delimiter; `{p}` abbreviates git revisions and
prints no source for registry packages, so no traversal row is an identity,
source, or checksum authority on its own; `{f}` is the resolved-feature column.
Production (`--edges normal,build`) and dev-inclusive
(`--edges normal,build,dev`) closures are separate traversals, never one
traversal post-filtered into the other.

The owning tests cross-check every traversal identity against metadata and
`packages/Cargo.lock`, require lock-supplied checksum coverage for every
non-path identity, require dependency-kind and `cfg` agreement with
`resolve.nodes[].deps[].dep_kinds`, and compare the joined result with the
generated policy inputs. The feature canary is an unrelated workspace member
that enables an otherwise-absent feature on a dependency shared with broker or
guest: the feature appears in a whole-workspace union and must remain absent
from the `{f}` column of both selected traversals. The tests also prove
generic Cargo and Nix build/test and Clippy contexts exclude both packages
while dedicated contexts retain exact selection.

## spec003w0 hubs and generation

```bash
set -euo pipefail
bazel --version
cat .bazelversion

assert_clean() {
  git diff --exit-code -- "$@"
  git diff --cached --exit-code -- "$@"
  test -z "$(git status --porcelain --untracked-files=all -- "$@")"
}
assert_clean .bazelignore bazel packages MODULE.bazel.lock
(cd packages && cargo xtask gen-bazel --check)
assert_clean .bazelignore bazel packages MODULE.bazel.lock
assert_clean packages/policy-inputs
(cd packages && cargo xtask gen-package-policy-inputs --check)
assert_clean packages/policy-inputs
```

Inspect `MODULE.bazel`:

```bash
set -euo pipefail
test "$(grep -c 'name = "product"' MODULE.bazel)" -eq 1
test "$(grep -c 'name = "walker"' MODULE.bazel)" -eq 1
test "$(grep -c 'skip_cargo_lockfile_overwrite = True' MODULE.bazel)" -eq 2
test "$(grep -Ec '^[[:space:]]*cargo_lockfile[[:space:]]*=' MODULE.bazel)" -eq 2
test "$(grep -Ec '^[[:space:]]*lockfile[[:space:]]*=' MODULE.bazel)" -eq 2

! grep -Eq 'name = "(main|broker|guest)"' MODULE.bazel
! grep -F 'Cargo.guest.lock' MODULE.bazel
! grep -R 'crate.spec' MODULE.bazel bazel packages/*/BUILD.bazel
```

Supported repins run from `packages/`, and the module lock is always last:

```bash
set -euo pipefail
assert_clean() {
  git diff --exit-code -- "$@"
  git diff --cached --exit-code -- "$@"
  test -z "$(git status --porcelain --untracked-files=all -- "$@")"
}
assert_clean bazel/cargo/product.lock
(cd packages && cargo xtask bazel-repin --hub product)
assert_clean bazel/cargo/product.lock
assert_clean bazel/cargo/walker.lock
(cd packages && cargo xtask bazel-repin --hub walker)
assert_clean bazel/cargo/walker.lock
assert_clean MODULE.bazel.lock
(cd packages && cargo xtask bazel-module-refresh)
assert_clean MODULE.bazel.lock
```

On a current committed tree all three commands exit zero with no change.
Refresh authority follows the manifest that changed. After a product dependency
change, product repin changes only `bazel/cargo/product.lock` and the
following module refresh changes only `MODULE.bazel.lock`, while
`tests/tools/no-bash-ast-walker/Cargo.lock` and `bazel/cargo/walker.lock`
stay byte-identical. After a walker manifest or lock change, walker repin
changes only `bazel/cargo/walker.lock` and the following module refresh
changes only `MODULE.bazel.lock`, while `packages/Cargo.lock` and
`bazel/cargo/product.lock` stay byte-identical.

The module command takes no arguments, changes only `MODULE.bazel.lock` when
stale, and changes nothing on its second run. The drift refusal uses this exact
ordered repository remediation:

```text
From the repository root, run: nix develop
Then run: cd packages
cargo xtask bazel-module-refresh
Review and commit MODULE.bazel.lock.
Rerun cargo xtask bazel-module-refresh, then rerun the failed command.
```

`Makefile` and every workflow must contain none of `bazel-module-refresh`,
`bazel-repin`, `bazel-yanked-refresh`, `bazel-yanked-check`,
`gen-package-policy-inputs`, `bazel-evidence`,
`bazel-qualification-validate`, or `cargo generate-lockfile`.

The retired inputs must refuse before a Bazel child starts:

```bash
set -euo pipefail
cd packages
for hub in main broker guest; do
  if cargo xtask bazel-repin --hub "$hub"; then
    echo "retired hub unexpectedly succeeded: $hub" >&2
    exit 1
  fi
done
cd ..
```

Unit tests, not this shell loop, bind exact diagnostics, zero executor calls,
argv, and cwd. They use the injected non-mutating executor and must show:

```text
argv = ["cargo", "xtask", "bazel-repin", "--hub", "product"]
cwd = "packages/"
```

No test runs a genuine repin.

When a manifest changes, generation order is mandatory and depends on which
authority changed. A product manifest change is exactly:

```bash
set -euo pipefail
before_walker=$(sha256sum \
  tests/tools/no-bash-ast-walker/Cargo.lock bazel/cargo/walker.lock)
(cd packages && cargo generate-lockfile --offline)
(cd packages && cargo xtask bazel-repin --hub product)
(cd packages && cargo xtask bazel-module-refresh)
test "$before_walker" = "$(sha256sum \
  tests/tools/no-bash-ast-walker/Cargo.lock bazel/cargo/walker.lock)"
```

A walker manifest or lock change is exactly:

```bash
set -euo pipefail
before_product=$(sha256sum packages/Cargo.lock bazel/cargo/product.lock)
(cd tests/tools/no-bash-ast-walker && cargo generate-lockfile --offline)
(cd packages && cargo xtask bazel-repin --hub walker)
(cd packages && cargo xtask bazel-module-refresh)
test "$before_product" = \
  "$(sha256sum packages/Cargo.lock bazel/cargo/product.lock)"
```

Initial or combined setup is exactly:

```bash
set -euo pipefail
(cd packages && cargo xtask bazel-repin --hub product)
(cd packages && cargo xtask bazel-repin --hub walker)
(cd packages && cargo xtask bazel-module-refresh)
```

Commit and review those outputs in that order, with `MODULE.bazel.lock`
always last, then rerun each command under the clean assertions above. Prove
byte identity by comparing recorded hashes of the untouched authority before
and after, never by reading a diff summary. Module and hub locks, Nix pins,
BUILD files, and coverage/query goldens are generated and committed only by the
integrator.

## spec003w0 package policy inputs

Expected generated directories:

```bash
set -euo pipefail
find packages/policy-inputs -mindepth 4 -maxdepth 4 -type d | sort
```

For each system, require broker GNU and guest musl contexts:

```bash
set -euo pipefail
for system in x86_64-linux aarch64-linux; do
  case "$system" in
    x86_64-linux)
      gnu=x86_64-unknown-linux-gnu
      musl=x86_64-unknown-linux-musl
      ;;
    aarch64-linux)
      gnu=aarch64-unknown-linux-gnu
      musl=aarch64-unknown-linux-musl
      ;;
  esac

  for path in \
    "packages/policy-inputs/$system/$gnu/broker-production" \
    "packages/policy-inputs/$system/$musl/guest-real-libshpool"
  do
    test -s "$path/production/closure.json"
    test -s "$path/production/Cargo.lock"
    test -s "$path/policy/metadata.json"
    test -s "$path/policy/Cargo.lock"
  done
done
```

Run the generator check from `packages/`:

```bash
set -euo pipefail
cd packages
cargo xtask gen-package-policy-inputs --check
cd ..
```

Review one context at a time. Before accepting deny or audit output, verify the
checker reported:

- one selected root;
- nonempty production and policy graphs;
- exact system and target;
- exact edge kinds, cfgs, and features;
- exact metadata and filtered-lock identity equality;
- exact selected-source count;
- no missing, extra, or unreadable source;
- every registry checksum and git rev/archive checksum verified.

The package audit output must show a pinned database and `--no-fetch`.

## spec003w0 Cargo gate supply-chain and pinned inventory

The nested broker and guest locks are deleted in the same wave, so the Cargo
gate keeps no nested authority. Its package supply-chain surfaces read the
native-system selected policy inputs, and the aggregate root and guest closure
checks stay independent:

```bash
set -euo pipefail
! grep -Fq 'packages/d2b-priv-broker/Cargo.lock' tests/test-rust.sh
! grep -Fq 'packages/d2b-guest-shell-runner/Cargo.lock' tests/test-rust.sh
grep -Fq 'broker-production/policy/Cargo.lock' tests/test-rust.sh
grep -Fq 'broker-production/policy/metadata.json' tests/test-rust.sh
grep -Fq 'guest-real-libshpool/production/Cargo.lock' tests/test-rust.sh
grep -Fq 'guest-real-libshpool/production/closure.json' tests/test-rust.sh
grep -Fq 'guest-real-libshpool/policy/Cargo.lock' tests/test-rust.sh
grep -Fq 'guest-real-libshpool/policy/metadata.json' tests/test-rust.sh
grep -Fq 'packages/Cargo.lock' tests/test-rust.sh
grep -Fq 'packages/Cargo.guest.lock' tests/test-rust.sh
grep -Fq -- '--no-fetch' tests/test-rust.sh
```

The guest static dependency policy reads only
`production/{closure.json,Cargo.lock}`. Package deny reads the dev-inclusive
policy metadata, and audit reads the policy filtered lock with the pinned
RustSec database and `--no-fetch`. A `--no-fetch` audit cannot fail
transiently, so the owning tests prove no retry wrapper surrounds it.

The pinned inventory selects packages from the root lock and never mutates the
tree:

```bash
set -euo pipefail
pinned_tool=tests/tools/assert-pinned-tests.sh
! grep -Fq 'packages/d2b-priv-broker/Cargo.lock' "$pinned_tool"
! grep -Eq 'assert-pinned-broker-lock|broker_lock_backup|restore_broker_lock' \
  "$pinned_tool"
! grep -Eq 'trap[[:space:]]+[^#]*EXIT' "$pinned_tool"
grep -Fq -- 'cargo nextest list --locked --workspace' "$pinned_tool"
grep -Fq -- 'cargo nextest list --locked -p d2b-priv-broker' "$pinned_tool"

assert_clean() {
  git diff --exit-code -- "$@"
  git diff --cached --exit-code -- "$@"
  test -z "$(git status --porcelain --untracked-files=all -- "$@")"
}
assert_clean .
tests/tools/assert-pinned-tests.sh
assert_clean .
```

`cargo nextest list --locked` is non-mutating by construction, which is why
the snapshot, restore function, scratch path, and `EXIT` trap are deleted
rather than hardened.

The five pinned comment files whose headers describe the retired nested
workspaces must describe the selected root contexts instead, with no pinned
entry changed:

```bash
set -euo pipefail
for name in \
  kernel-canaries \
  usbip-firewall-skeleton \
  host-prepare-network \
  broker-socket-acl \
  broker-export-audit
do
  ! grep -Eq '^#.*broker[- ]workspace' "tests/golden/pinned/$name.txt"
done
```

## spec003w0 guest license policy

The guest policy must contain package-scoped exceptions for exactly:

```text
bindgen        BSD-3-Clause
instant        BSD-3-Clause
inotify        ISC
inotify-sys    ISC
libloading     ISC
notify         CC0-1.0
```

It must not add those licenses to the global `licenses.allow` list.

Run:

```bash
set -euo pipefail
make test-rust-supply-chain
make test-policy
```

Read the planted different-package cases in the owning test. Each must prove a
different package under BSD-3-Clause, ISC, or CC0-1.0 is still denied.

## spec003w0 Nix package and architecture realization

Both dedicated derivations must retain the same exact git output hash:

```bash
set -euo pipefail
expected='sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8='
test "$(grep -F \
  'outputHashes."wl-proxy-0.1.2" = "'"$expected"'";' \
  nixos-modules/host-broker.nix | wc -l)" -eq 1
test "$(grep -F \
  'outputHashes."wl-proxy-0.1.2" = "'"$expected"'";' \
  flake.nix | wc -l)" -ge 1
```

List the four package checks and guest ELF check for each system:

```bash
set -euo pipefail
for system in x86_64-linux aarch64-linux; do
  for check in \
    broker-production-dependency-policy \
    guest-shell-runner-static-dependency-policy \
    broker-production-package-policy \
    guest-real-libshpool-package-policy \
    guest-static-elf
  do
    nix eval --raw ".#checks.$system.$check.name" >/dev/null
  done
done
```

On a native x86_64-linux runner:

```bash
set -euo pipefail
nix build --no-link \
  .#checks.x86_64-linux.broker-production-dependency-policy \
  .#checks.x86_64-linux.guest-shell-runner-static-dependency-policy \
  .#checks.x86_64-linux.broker-production-package-policy \
  .#checks.x86_64-linux.guest-real-libshpool-package-policy \
  .#checks.x86_64-linux.guest-static-elf \
  .#checks.x86_64-linux.rust-deny \
  .#checks.x86_64-linux.rust-audit
make test-rust-supply-chain
make test-policy
```

On a native aarch64-linux runner:

```bash
set -euo pipefail
nix build --no-link \
  .#checks.aarch64-linux.broker-production-dependency-policy \
  .#checks.aarch64-linux.guest-shell-runner-static-dependency-policy \
  .#checks.aarch64-linux.broker-production-package-policy \
  .#checks.aarch64-linux.guest-real-libshpool-package-policy \
  .#checks.aarch64-linux.guest-static-elf \
  .#checks.aarch64-linux.rust-deny \
  .#checks.aarch64-linux.rust-audit
make test-rust-supply-chain
```

Neither block may set `--system`, `--builders`, or a remote builder. The
generated CI must retain job ID `test-flake-aarch64`, use
`ubuntu-24.04-arm`, use a 60-minute timeout, and run
`make test-rust-supply-chain` after the five native realizations. The renderer
regression and both native results must refer to one unchanged stable PR head.

For every native guest artifact, the automated check must report `ET_DYN`, the
native expected machine (`EM_X86_64` or `EM_AARCH64`), no `PT_INTERP`, and no
`DT_NEEDED`. Run the owning mutation tests and require both a non-PIE/`ET_EXEC`
plant and a wrong-machine plant to fail.

Regenerate both system inventories:

```bash
set -euo pipefail
assert_clean() {
  git diff --exit-code -- "$@"
  git diff --cached --exit-code -- "$@"
  test -z "$(git status --porcelain --untracked-files=all -- "$@")"
}
assert_clean tests/golden/flake-check-matrix
make flake-matrix-pin
assert_clean tests/golden/flake-check-matrix
make test-drift
```

## spec003w0 complete validation

```bash
set -euo pipefail
assert_clean() {
  git diff --exit-code -- "$@"
  git diff --cached --exit-code -- "$@"
  test -z "$(git status --porcelain --untracked-files=all -- "$@")"
}
assert_clean packages/Cargo.lock
(cd packages && cargo generate-lockfile --offline)
assert_clean packages/Cargo.lock
assert_clean .bazelignore bazel packages MODULE.bazel.lock
(cd packages && cargo xtask gen-bazel --check)
assert_clean .bazelignore bazel packages MODULE.bazel.lock
(cd packages && cargo xtask gen-package-policy-inputs --check)
assert_clean .bazelignore bazel packages MODULE.bazel.lock
(cd packages && cargo xtask bazel-repin --hub product)
assert_clean .bazelignore bazel packages MODULE.bazel.lock
(cd packages && cargo xtask bazel-repin --hub walker)
assert_clean .bazelignore bazel packages MODULE.bazel.lock
(cd packages && cargo xtask bazel-module-refresh)
assert_clean .bazelignore bazel packages MODULE.bazel.lock
assert_clean tests/unit/nix/pinned
make nix-unit-pin
assert_clean tests/unit/nix/pinned
make check-tier0
make test-lint
make test-rust-main
make test-rust-broker
make test-rust-guest-shell-runner
make test-rust-schema
make test-rust-inventory
make test-rust-supply-chain
make test-rust
make test-policy
make test-drift
make test-flake
make test-nix-unit
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

Confirm `make test-rust` remains Cargo-authoritative through spec003w4.

Also inspect `.github/workflows/release-host-binaries.yml` and the two retained
drift gates. The workflow must use the root manifest with `--locked`, explicit
package/bin/default-feature selectors, copy the broker from
`packages/target/release`, declare only `packages -> target` as its workspace
cache mapping, and list the explicit gate target directories. Both
`tests/unit/gates/flake-check-matrix-sync.sh` and
`tests/unit/gates/ci-rust-cache-sync.sh` must enforce the new shape and remain
present.

The future spec003w0 binding-doc diff includes `AGENTS.md`, `tests/AGENTS.md`,
`CONTRIBUTING.md`, `docs/contributing/gates-and-lints.md`,
`docs/contributing/workflow.md`,
`docs/contributing/critical-subsystems.md`, and
`packages/d2b-contract-tests/tests/policy_modules.rs`, plus the stale
ADR-status surfaces `docs/adr/0052-bazel-rust-build-and-test.md`,
`docs/adr/README.md`, and `changelog.d/adr0054-broker-hub.md`. It does not
edit dated ADR 0038; the new text states that ADR 0054 governs the unified
product workspace.

## spec003w1 Bazel aggregate

```bash
set -euo pipefail
make test-bazel-rust
make test-bazel-rust-main
make test-bazel-rust-api
make test-bazel-rust-broker
make test-bazel-rust-aux
```

All six shadow names must be approved in the same wave that introduces them:

```bash
set -euo pipefail
for target in \
  test-bazel-rust \
  test-bazel-rust-main \
  test-bazel-rust-api \
  test-bazel-rust-broker \
  test-bazel-rust-aux \
  bazel-shutdown
do
  grep -Fq "\"$target\"" packages/xtask/tests/policy_ci.rs
  grep -Eq "^$target:" Makefile
done
(cd packages && cargo test -p xtask --test policy_ci)
```

`APPROVED_MAKE_TARGETS` in `packages/xtask/tests/policy_ci.rs` is the only
allowlist the ci-uses-make guard reads, so an unlisted shadow target would let a
shadow workflow escape it. The owning tests carry a positive case for each of
the six names, a negative for a workflow step calling an unapproved
`test-bazel-rust-<name>`, and a negative for an approved name with no Makefile
rule.

The no-shell rule is bound to a generated, drift-checked inventory:

```bash
set -euo pipefail
(cd packages && cargo xtask gen-bazel --check)
test -s bazel/generated/no-shell-inventory.json
jq -e '
  (.governedSources | length) > 0 and
  (.declaredInputs | length) > 0 and
  (.spawnSites | length) > 0 and
  ([.governedSources[].path] - [.declaredInputs[].path] | length) == 0 and
  ([.declaredInputs[].path] - [.governedSources[].path] | length) == 0 and
  ([.spawnSites[].source] - [.governedSources[].path] | length) == 0 and
  ([.governedSources[].path] - [.spawnSites[].source] | length) == 0 and
  ([.spawnSites[] | select(.shellInvocation)] | length) == 0 and
  ([.governedSources[] | select(.scanned | not)] | length) == 0
' bazel/generated/no-shell-inventory.json
```

`gen-bazel --check` freshly rediscovers spawn sites and compares their stable
source/span/program keys with the committed `spawnSites` set in both
directions. An empty inventory is a refusal, never a vacuous pass. The four mandatory
plants (`no-shell-inventory-empty`, `no-shell-inventory-missing-entry`,
`no-shell-inventory-extra-entry`, and `no-shell-inventory-planted-shell`)
must each fail at their own diagnostic. Scopes produce `.scratch/` previews
only; the integrator commits the generated inventory.

Capture execution evidence:

```bash
set -euo pipefail
D2B_SKIP_FIXTURE_BUILD=1 \
D2B_EXECUTION_MANIFEST=.scratch/spec003-bazel-pass.json \
make test-bazel-rust

nix shell --quiet --inputs-from . nixpkgs#check-jsonschema --command \
  check-jsonschema \
  --schemafile docs/reference/schemas/test-execution-manifest-v1.json \
  .scratch/spec003-bazel-pass.json

jq -e '
  .version == 1 and
  .target == "test-rust" and
  .run_status == "passed" and
  (.completed_leaves | length) == 18 and
  (.failed_surfaces | length) == 0
' .scratch/spec003-bazel-pass.json
```

Inspect:

- product and walker containment;
- exact native configured broker and guest target censuses;
- no broker-to-guest or unrelated first-party edge;
- main and guest per-case topology;
- broker process-per-binary topology;
- result document redaction and raw `test.log` availability;
- exact schema, no-bash, companion, API, and pinned-test censuses.
- two independent nonempty schema generations, with empty and mismatch plants;
- stub missing-executable, wrong-identity, runtime-state, and forbidden
  undeclared-listener plants;
- pinned inventory empty, missing, and extra plants;
- prior-evidence invalidation, multi-carrier attribution, sorted atomic
  success/failure/interruption manifest v1 evidence, original-status
  preservation, ignored-case fidelity, and enforcing JUnit publication;
- one planted failed result containing every forbidden redaction value, absent
  from JUnit and present only in `test.log`;
- no shell in repository-owned runner paths;
- invalid `D2B_RUST_BUDGET`, scheduler-only, suite-only, and combined-limit
  mutations.

Require broker and network isolation:

```bash
set -euo pipefail
test "$(grep -Fc 'tags = ["exclusive"]' bazel/carriers/broker.bzl)" -eq 3
(cd packages && cargo test -p xtask --test bazel_action_network)
(cd packages && cargo test -p xtask --test bazel_yanked)
(cd packages && cargo xtask bazel-yanked-check)
```

The broker tests must include a tag-removal mutation that overlaps a planted
ordinary test. Qualification, not this smoke block, runs each context with
`--runs_per_test=20`. The action-network test includes canonical declared
loopback TCP and Unix socket positives, a forbidden external-egress plant, and
a live-index plant. The committed yanked snapshot key set comes only from
`packages/Cargo.lock`; main checks the full set, while broker and guest check
exact projections of their selected package-policy graphs. The walker and
`Cargo.guest.lock` must not contribute keys.

Keep adjacent authority green:

```bash
set -euo pipefail
make test-rust
make test-policy
make test-drift
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

The supply-chain qualification test compares Cargo's current raw enforcing
status and normalized finding set with the decomposed deny, audit, and yanked
union for full-product main and exact broker/guest projections. Run its
finding-class, missing-union-leg, projection-swap, extra-finding, and
status-difference mutations. Any difference blocks this wave.

## spec003w2 operational safety

```bash
set -euo pipefail
make test-rust-main
make test-policy
make check-tier0
make test-drift
make test-bazel-rust
D2B_CLEAN_DRY_RUN=1 make clean
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

Run targeted tests for:

- both cleanup resolution routes;
- leaf and intermediate link refusal;
- descriptor inheritance;
- replacement race and external decoy;
- deadline grammar and conservative rounding;
- repeated non-consuming nonblocking
  `waitid(EXITED|NOWAIT|NOHANG)` observations throughout the independently
  timed full SIGTERM grace, informational only, unconditional group SIGKILL,
  then direct-child reap;
- blocking-wait, early-reap, shortened-grace, and conditional-SIGKILL
  mutations;
- descendant death and sibling survival;
- missing process-group creation, wrapper-group, group-zero, group-minus-one,
  and PID-file-decoy mutations, with the sibling and decoy left alive;
- exact per-code recovery commands, redaction, and wrong-remedy mutations;
- synchronous trim before size measurement;
- one startup option set for every server-selecting command.
- provider `O_RDONLY|O_CLOEXEC`,
  `RESOLVE_NO_MAGICLINKS` without `RESOLVE_BENEATH` or
  `RESOLVE_NO_SYMLINKS`, permissive fallback leaf semantics without provider
  leaf `O_NOFOLLOW`, `ENOSYS`
  refusal, same-descriptor `execveat(AT_EMPTY_PATH)`, and behavioral
  close-on-exec checks for every auxiliary descriptor.
- a provider mutation that reintroduces `RESOLVE_BENEATH`, while strict
  result and cleanup paths retain
  `RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS`.

No test should fill a disk, require a privileged mount, sleep to reach expiry,
or write a stale executable into `packages/target/`.

Inspect exact recovery bytes:

- `D2B-BZLCLEAN-TRACKED`: `D2B_CLEAN_DRY_RUN=1 make clean`, remove or
  relocate the unexpected tracked entry from `.scratch/bazel/`, then
  `make clean`.
- `D2B-BZLCLEAN-SYMLINK` and `D2B-BZLCLEAN-ESCAPE`: the same dry run, remove
  only the offending link, magic link, or escaping layout under
  `.scratch/bazel/`, then `make clean`; external content stays untouched and
  needs separate ownership verification.
- `D2B-BZLCLEAN-LIVE`: close other clients, run `make bazel-shutdown`, then
  `make clean`; no dry run or tree correction first.
- `D2B-BZLSERVER-STUCK`: close other clients and run
  `make bazel-shutdown`; never delete `.scratch/bazel/` or signal a process
  identifier manually.

Run the ADR-0054 drift/refusal table tests for stale product and walker hub
locks, module lock, generator output, package-policy output, yanked snapshot,
ambient repin controls, and unexpected tracked mutation. Each exact nonzero
message must begin with:

```text
From the repository root, run: nix develop
Then run: cd packages
```

It must then carry its exact command, repository-relative review/commit step,
and rerun sequence from `workspace-and-tool-pinning.md`. Exact-message,
wrong-remedy, missing-step, absolute-path, secret, identifier, and echoed-value
plants must fail. Retired-hub diagnostics remain byte-unchanged.

## spec003w3 shadow workflow

Before review:

```bash
set -euo pipefail
make test-bazel-rust-main
make test-bazel-rust-api
make test-bazel-rust-broker
make test-bazel-rust-aux
make test-rust-main
make test-policy
make test-lint
make check-tier0
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

Inspect one pull-request run. It must:

- have four attributed slices and one rollup;
- be non-required;
- call approved Make targets only;
- use credentialless checkout;
- request only `contents: read`;
- execute zero cache actions;
- emit no qualification record.

Separately inspect a protected-`v3` push produced by a merged pull request. Its
qualification record must carry explicit `bazelRestoreCount`,
`bazelSaveCount`, and `bazelPublicationCount` of zero and four complete
`sliceDurationsSeconds` entries. Those four camelCase names are canonical
everywhere; no snake_case spelling is accepted.

## spec003w4 qualification audit

The typed validator is the authority. Run it first and require success; the
`jq` block below is informational only and cannot qualify a record:

```bash
set -euo pipefail
cd packages
cargo xtask bazel-qualification-validate
cd ..
```

It reads the fixed repository-relative record path, derives every threshold
from the record's immutable evidence references, and refuses omitted, forged,
duplicate, inconsistent, and wrong-candidate references. A record cannot
qualify through a trusted boolean: any boolean that disagrees with the derived
verdict is a refusal.

```bash
set -euo pipefail
jq -e '
  .status == "qualified" and
  .coverage.exact_surface_count == 18 and
  .coverage.unmapped_count == 0 and
  (.seeded_failures | length) == 18 and
  .package_policy.context_count == 4 and
  .package_policy.package_check_wrapper_count == 8 and
  .package_policy.all_selected_source_censuses_exact and
  .package_policy.all_checksums_verified and
  .package_policy.all_audits_no_fetch and
  .package_policy.guest_license_exception_count == 6 and
  .architecture.x86_native_five_checks_passed and
  .architecture.aarch64_native_five_checks_passed and
  .architecture.aarch64_supply_chain_passed_on_same_stable_head and
  .broker.all_contexts_exclusive and
  .broker.all_contexts_twenty_consecutive and
  .action_network.canonical_local_sockets_passed and
  .action_network.external_egress_plant_refused and
  .action_network.live_index_plant_refused and
  .supply_chain.all_three_contexts_equal and
  .architecture.all_guest_elf_et_dyn and
  .architecture.all_guest_elf_machine_matches and
  .architecture.non_pie_plant_refused and
  .architecture.wrong_machine_plant_refused and
  .runner.manifest_junit_contract_passed and
  .runner.combined_budget_mutations_refused and
  .yanked.authority_lock == "packages/Cargo.lock" and
  .yanked.walker_excluded and
  .yanked.guest_lock_excluded and
  ([.qualification_records[]
    | (has("bazelRestoreCount") and
       has("bazelSaveCount") and
       has("bazelPublicationCount"))]
    | all) and
  ([.qualification_records[]
    | (.bazelRestoreCount == 0 and
       .bazelSaveCount == 0 and
       .bazelPublicationCount == 0)]
    | all) and
  ([.qualification_records[]
    | select(.cold_ci == true)
    | (.sliceDurationsSeconds | length) == 4]
    | all) and
  (.no_shell_inventory.nonempty and
   .no_shell_inventory.source_projections_bidirectional and
   .no_shell_inventory.spawn_sites_bidirectional and
   (.no_shell_inventory.plants | length) == 4)
' specs/003-adr052-bazel-rust/evidence/qualification.json
```

Every record carries all three counts, derived from the records themselves
rather than from a self-asserted aggregate; every cold record additionally
carries four `sliceDurationsSeconds` entries.

Also require ten consecutive matching qualification records, five topology
proofs, twenty consecutive executions per broker context, complete locator
evidence, three valid performance sets, the committed no-shell inventory digest
with its four plant results, and all planted guard results.

## spec003w5 promotion and rollback

Before merge:

```bash
set -euo pipefail
assert_clean() {
  git diff --exit-code -- "$@"
  git diff --cached --exit-code -- "$@"
  test -z "$(git status --porcelain --untracked-files=all -- "$@")"
}
assert_clean .github/workflows/pr-l1-static-fast.yml
make layer1-workflow
assert_clean .github/workflows/pr-l1-static-fast.yml
make test-drift
make check
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

After promotion:

- required context is still `test-rust`;
- eighteen surfaces use Bazel;
- fixture surfaces use the existing path;
- generated CI calls only `test-rust-slice-main`, `test-rust-slice-api`,
  `test-rust-slice-broker`, and `test-rust-slice-aux`;
- all eight public Rust leaf names remain;
- each public leaf maps to its exact carrier subset and `test-rust-main` keeps
  conditional fixture behavior;
- every Bazel compatibility alias prints its exact stderr replacement line,
  forwards to `test-rust` or the matching `test-rust-slice-*`, and preserves
  status;
- action and download caches are separate;
- output base is not cached;
- one protected-`v3` writer publishes after synchronous trim and two headroom
  checks.
- each primary key is unique for its successful protected-`v3` run;
- restore prefixes contain neither run ID nor commit SHA;
- maintenance retains the newest complete generation for each authorized
  prefix.
- the table-driven cache test mutates every bound input and changes every
  applicable action/repository key without collapsing namespaces.

The alias replacements are exactly:

```text
test-bazel-rust -> test-rust
test-bazel-rust-main -> test-rust-slice-main
test-bazel-rust-api -> test-rust-slice-api
test-bazel-rust-broker -> test-rust-slice-broker
test-bazel-rust-aux -> test-rust-slice-aux
```

Each stderr line has the exact form
`make: <old> is deprecated; use <new>`.

The promotion integrator records the spec003w5 parent, integrates or squashes
all spec003w5 scope results into exactly one atomic candidate, and asserts the
complete path diff relative to that parent. Promotion docs and the semantic
changelog list all five Bazel alias replacements. The same change updates
`AGENTS.md`, `tests/AGENTS.md`,
`docs/contributing/gates-and-lints.md`, `tests/README.md`, and
`docs/reference/test-execution-manifest.md`, because the last two also
describe the eight CI jobs:

```bash
set -euo pipefail
for doc in \
  AGENTS.md \
  tests/AGENTS.md \
  docs/contributing/gates-and-lints.md \
  tests/README.md \
  docs/reference/test-execution-manifest.md
do
  grep -Fq 'test-rust-slice-main' "$doc"
done
! grep -Fq 'eight CI leaf targets' tests/README.md
! grep -Fq \
  'runs API, main, broker, guest, no-bash, schema, inventory and supply chain' \
  docs/reference/test-execution-manifest.md
```

Those two literals are the current committed sentences that assert eight Rust
CI jobs; both must be gone, and all eight public leaf names must remain
documented.

Rollback rehearsal in a disposable worktree, before merge. There is no
promotion record yet: `promotion-record.json` is created only after the
candidate merges, so the rehearsal resolves the candidate from the verified
current atomic candidate HEAD and the parent the integrator recorded when it
built that candidate:

```bash
set -euo pipefail
candidate_sha=$(git rev-parse --verify HEAD)
: "${D2B_SPEC003W5_PARENT:?set it to the parent the integrator recorded}"
recorded_parent=$D2B_SPEC003W5_PARENT
git rev-parse --verify "$recorded_parent^{commit}" >/dev/null
test "$(git rev-parse --verify "$candidate_sha^")" = \
  "$(git rev-parse --verify "$recorded_parent^{commit}")"
test "$(git rev-list --count "$recorded_parent..$candidate_sha")" -eq 1
test ! -e specs/003-adr052-bazel-rust/evidence/promotion-record.json
git revert --no-commit "$candidate_sha"
make test-rust
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

Discard the rehearsal worktree. `promotion-record.json` is read only after
merge; any pre-merge command that reads it is wrong by construction.

## spec003w6 and spec003w7 independent checks

Alias removal requires a containing published semantic release tag, not any
tag. This repository already carries two-component tags such as `v1.0`, so
`git tag --contains` alone is not an entry condition:

```bash
set -euo pipefail
promotion_sha=$(jq -r '.promotion_commit' \
  specs/003-adr052-bazel-rust/evidence/promotion-record.json)
tag=
while IFS= read -r candidate; do
  git merge-base --is-ancestor "$promotion_sha" "$candidate^{commit}" \
    || continue
  remote_commit=$(
    git ls-remote --exit-code --tags origin \
      "refs/tags/$candidate" "refs/tags/$candidate^{}" \
      | awk '
          $2 ~ /\^\{\}$/ { peeled = $1 }
          $2 !~ /\^\{\}$/ { direct = $1 }
          END { print peeled != "" ? peeled : direct }
        '
  ) || continue
  test -n "$remote_commit" || continue
  test "$(git rev-parse "$candidate^{commit}")" = "$remote_commit" || continue
  test "$(gh release view "$candidate" --json isDraft --jq .isDraft \
    2>/dev/null)" = "false" || continue
  tag=$candidate
  break
done < <(
  git tag --contains "$promotion_sha" \
    | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' \
    | sort -V
)
test -n "$tag"
```

A two-component tag, a local-only tag, a divergent same-named local and remote
tag, and a draft-only release each fail entry, and the owning interface test
carries all four as negatives.

Cargo implementation retirement:

```bash
set -euo pipefail
(cd packages && cargo test -p xtask --test post_promotion_observations)
```

The validator paginates every promoted protected-`v3` `test-rust` run unit.
A unit is one distinct push-created `(runId, headSha)` pair, never an
attempt. Attempts `1..max` are that unit's complete nested history: the unit
normalizes to the conclusion of its highest terminal attempt, and no further
attempt of the same unit ever increments the streak again. Units are ordered by
immutable creation order `(createdAt, runId)` and never by rerun start time,
so an old unit rerun today cannot move behind a newer failure and erase its
reset.

Each unit requires immutable run ID, head SHA, push event, `v3` branch, a
complete attempt list, a terminal highest attempt, deterministic creation
ordering, and verified promotion ancestry. Pagination gaps, missing attempts,
missing or duplicate unit identities, conflicting head or provenance across
attempts, non-v3/non-push, pre-promotion, and nonterminal units fail.
Eligibility, count, and run IDs are derived; self-asserted summary fields are
ignored. The derived final ten distinct ordered units must be successes with no
intervening failure or cancellation.

After either change, every public Rust Make name and fixture mode must still
work. spec003w7 may land before spec003w6. If both changes edit a binding doc
or `post-promotion.json`, the child that lands second rebases onto the merged
first child, reruns its complete validation, and receives a new panel verdict.
Do not cite container, VM, live-host, hardware, or deployed-host tiers for this
internal build scheduler.
