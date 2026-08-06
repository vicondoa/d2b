# Quickstart: Validate Spec 003 Under ADR 0054

This guide is for implementation and review waves. Commands do not exist until
their named wave lands. Run them from a committed scope-owned worktree.

## Prerequisites

```bash
set -euo pipefail
test "$(git rev-parse --show-toplevel)" = "$(pwd -P)"
test -z "$(git status --porcelain --untracked-files=all)"
mkdir -p .scratch
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

Validate the plan structure before a plan panel:

```bash
set -euo pipefail
perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test
perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
```

Expected:

```text
PASS: 67 validator self-tests; positive fixture accepted; 47 independent negative fixtures cover noncanonical unchecked-list forms, census declarations, task parsing, ownership, dependency, adjacency, section, cycle, and conflict fixtures rejected; full stderr byte-matched against independent literals; physical census/mismatch and adjacency rows and bounded numeric, none, and overflow locators verified; actual temp-dir, path-resolution, make-path, copy, mkdir, open3, and subprocess failures and warnings emit only their seam-specific fixed setup diagnostics after sentinel output is discarded; actual unreadable-source status 1 and unsupported-argument status 2 subprocesses verified; self-test-contract is reserved for validator contract failures
PASS: 120 unique tasks with exact canonical headers and owned paths; dependencies exist and precede consumers; adjacency matches; graph is acyclic; concurrently ready ownership is disjoint
```

The self-tests include unordered, ordered-dot, ordered-paren, indented,
blockquoted, nested-blockquoted, zero-task, whole-task-omission, malformed
census, and every isolated validation branch. The main check compares parsed
IDs with the independent exact census in `tasks.md`. Every negative expectation
is an independent literal for complete stderr and runs through the injectable
entrypoint. Adjacency cases independently scan the physical fixture row;
census, section, and mismatch locations use actual offsets and ordinals.
Oversized inputs assert the closed `overflow` bound. Actual task omitted from
census and malformed/unbalanced census markers have isolated exact fixtures.
Temp-dir, path-resolution, make-path, copy, mkdir, open3, and subprocess
capture/wait failures and warnings are injected at their actual operation
seams and call `run_cli_entrypoint --self-test` after the runner writes
sentinel stdout/stderr. No case passes an expected reason to a generic setup
wrapper. Each asserts status 1, empty stdout, exact seam-specific fixed
setup-class stderr and remedy, and absence of sentinel, raw exception/path,
or task-rewrite content. The exact `self-test-contract` case is limited to an
invalid validator self-test result. Actual
unreadable-source and unsupported-argument subprocesses assert empty stdout
plus status 1 and 2.
The only actionable location is the fixed repository-relative source plus a
bounded 1-based numeric record/line locator or closed `none`/`overflow`
sentinel. No diagnostic may contain a
task ID, dependency ID, owned path, contents, count, or operator-derived value.
Every code has one exact remedy and rerun command.

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

## spec003w0 sequential toolchain gate

Before any Bazel generator command, finish the dedicated patched-Bazel and
static execution-supervisor Nix scope, regenerate all three Nix-unit presence
pins, and run the evaluator:

```bash
set -euo pipefail
make nix-unit-pin
git diff --exit-code -- tests/unit/nix/pinned/common.txt \
  tests/unit/nix/pinned/x86_64-linux.txt \
  tests/unit/nix/pinned/aarch64-linux.txt
git diff --cached --exit-code -- tests/unit/nix/pinned/common.txt \
  tests/unit/nix/pinned/x86_64-linux.txt \
  tests/unit/nix/pinned/aarch64-linux.txt
make test-nix-unit
```

The first pin command commits any toolchain-presence changes; the displayed
block is the required clean second run. T020 may later regenerate the same
three files after later Nix-policy cases land. Do not start the generator until
this early `make test-nix-unit` passes.

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

assert_no_ere() {
  pattern=$1
  shift
  if grep -Eq -- "$pattern" "$@"; then
    return 1
  else
    status=$?
    test "$status" -eq 1
  fi
}
assert_no_fixed() {
  pattern=$1
  shift
  if grep -Fq -- "$pattern" "$@"; then
    return 1
  else
    status=$?
    test "$status" -eq 1
  fi
}
assert_no_recursive_fixed() {
  pattern=$1
  shift
  if grep -RFq -- "$pattern" "$@"; then
    return 1
  else
    status=$?
    test "$status" -eq 1
  fi
}
test -r MODULE.bazel
assert_no_ere 'name = "(main|broker|guest)"' MODULE.bazel
assert_no_fixed 'Cargo.guest.lock' MODULE.bazel
test -d bazel
assert_no_recursive_fixed 'crate.spec' MODULE.bazel bazel packages
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
(cd packages && cargo generate-lockfile --offline \
  --manifest-path ../tests/tools/no-bash-ast-walker/Cargo.toml)
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
assert_no_fixed() {
  pattern=$1
  shift
  if grep -Fq -- "$pattern" "$@"; then
    return 1
  else
    status=$?
    test "$status" -eq 1
  fi
}
test -r tests/test-rust.sh
assert_no_fixed 'packages/d2b-priv-broker/Cargo.lock' tests/test-rust.sh
assert_no_fixed 'packages/d2b-guest-shell-runner/Cargo.lock' tests/test-rust.sh
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
test -r "$pinned_tool"
assert_no_fixed() {
  pattern=$1
  shift
  if grep -Fq -- "$pattern" "$@"; then
    return 1
  else
    status=$?
    test "$status" -eq 1
  fi
}
assert_no_ere() {
  pattern=$1
  shift
  if grep -Eq -- "$pattern" "$@"; then
    return 1
  else
    status=$?
    test "$status" -eq 1
  fi
}
assert_no_fixed 'packages/d2b-priv-broker/Cargo.lock' "$pinned_tool"
assert_no_ere \
  'assert-pinned-broker-lock|broker_lock_backup|restore_broker_lock' \
  "$pinned_tool"
assert_no_ere 'trap[[:space:]]+[^#]*EXIT' "$pinned_tool"
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
assert_no_ere() {
  pattern=$1
  shift
  if grep -Eq -- "$pattern" "$@"; then
    return 1
  else
    status=$?
    test "$status" -eq 1
  fi
}
for name in \
  kernel-canaries \
  usbip-firewall-skeleton \
  host-prepare-network \
  broker-socket-acl \
  broker-export-audit
do
  path="tests/golden/pinned/$name.txt"
  test -r "$path"
  assert_no_ere '^#.*broker[- ]workspace' "$path"
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

List exactly six native checks for each system:

```bash
set -euo pipefail
for system in x86_64-linux aarch64-linux; do
  for check in \
    broker-production-dependency-policy \
    guest-shell-runner-static-dependency-policy \
    broker-production-package-policy \
    guest-real-libshpool-package-policy \
    broker-host-artifact-contract \
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
  .#checks.x86_64-linux.broker-host-artifact-contract \
  .#checks.x86_64-linux.guest-static-elf
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
  .#checks.aarch64-linux.broker-host-artifact-contract \
  .#checks.aarch64-linux.guest-static-elf
make test-rust-supply-chain
```

Neither block may set `--system`, `--builders`, or a remote builder. The
generated CI must retain job ID `test-flake-aarch64`, use
`ubuntu-24.04-arm`, use a 60-minute timeout, and run
`make test-rust-supply-chain` after the six native realizations. The renderer
regression and both native results must refer to one unchanged stable PR head.

Inspect `tests/golden/bazel-rust-artifact-baselines.json`. It must contain
exactly four rows: broker and guest for each native system. Each row comes from
the realized derivation, carries a null initial `sizeGrowthAuthorization` and
no row-level allowance field, exact binary bytes, a recursive closure count and
digest but no Nix store path, and the exact selected-policy digest. Broker rows
additionally carry the exact ELF
interpreter basename and sorted `DT_NEEDED` SONAMEs; guest rows require
`ET_DYN`, the native machine, no interpreter, and no `DT_NEEDED`.

Run the unchanged-size-without-authorization and exact-approved-growth
positive fixtures. The authorization's prior bytes must equal the baseline row
and its new bytes must equal the realized artifact measurement. Run missing,
denied, stale, replayed, wrong-system/artifact, wrong-prior-baseline,
wrong-realized-new-bytes, arithmetic-mismatch, absolute-rationale, and
size-plus-one authorization negatives, plus closure-add/remove,
cross-artifact, unrelated-sibling, broker-linkage, static-broker,
dynamic-guest, non-PIE, and wrong-machine mutations. A nonzero growth requires
the closed authorization as the only allowance source, binding measured
old/new bytes to the row baseline and realized artifact, exact positive delta,
repository-relative rationale, system/artifact, candidate-content SHA-256, and
review-record SHA-256 in the same change. Qualification must reference all
four row digests and every nonzero authorization digest.

```bash
set -euo pipefail
jq -e '(.rows | length) == 4' \
  tests/golden/bazel-rust-artifact-baselines.json
if grep -Fq '/nix/store/' \
  tests/golden/bazel-rust-artifact-baselines.json
then
  exit 1
fi
```

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
  ([.governedSources[].path] - [.declaredInputs[].path] | length) == 0 and
  ([.declaredInputs[].path] - [.governedSources[].path] | length) == 0 and
  ([.spawnSites[].source] - [.governedSources[].path] | length) == 0 and
  ([.scanResults[].source] - [.governedSources[].path] | length) == 0 and
  ([.governedSources[].path] - [.scanResults[].source] | length) == 0 and
  (.scanResults | length) == (.governedSources | length) and
  ([.scanResults[].source] | unique | length) ==
    (.governedSources | length) and
  ([.spawnSites[] | select(.shellInvocation)] | length) == 0 and
  ([.scanResults[] | select(.status != "scanned")] | length) == 0
' bazel/generated/no-shell-inventory.json
```

`gen-bazel --check` freshly rediscovers spawn sites and compares their stable
source/span/program keys with the committed `spawnSites` set in both
directions. A governed source with no spawn construct remains present through
one successful zero-site `scanResults` row. Empty governed input is a refusal,
not a vacuous pass. The mandatory plants
(`no-shell-inventory-empty`, `no-shell-inventory-missing-entry`,
`no-shell-inventory-extra-entry`, `no-shell-inventory-unguarded-spawn`,
`no-shell-inventory-missing-zero-site-record`, and
`no-shell-inventory-planted-shell`) must each fail at their own diagnostic.
Scopes produce `.scratch/` previews only; the integrator commits the generated
inventory.

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
- forbidden-value absence and committed byte/record bounds across JUnit,
  sanitized `test.log`, emitted evidence, and exporter diagnostics;
- exact schema, no-bash, companion, API, and pinned-test censuses.
- two independent nonempty schema generations, with empty and mismatch plants;
- stub missing-executable, wrong-identity, and runtime-state plants, with no
  socket-denial plant assigned to the stub carrier;
- pinned inventory empty, missing, and extra plants;
- prior-evidence invalidation, multi-carrier attribution, sorted atomic
  success/failure/interruption manifest v1 evidence, original-status
  preservation, ignored-case fidelity, and typed complete/degraded publication;
- one planted failed result containing every forbidden redaction value before
  sanitization and absent from every emitted sink;
- no shell in repository-owned runner paths;
- invalid `D2B_RUST_BUDGET`, scheduler-only, suite-only, and combined-limit
  mutations.

Inspect the closed sink-retention rows:

```text
junit-v1                 14 days   128 per slice output root
test-log-v1              14 days   128 per slice output root
evidence-v1              30 days    32 per workflow and head digest
exporter-diagnostic-v1    7 days    64 per workflow and head digest
```

Injected-clock/filesystem tests must cover just-inside, exact-age, expired,
count-minus-one, exact-count, count-plus-one, newest retention, unowned/link
refusal, and expiry failure with no publication. Complete and degraded
evidence status variants must each carry exactly their required fields beneath
one common `sinkKind` and policy-matching `retentionClass`; neither variant may
repeat them. A degraded publication must still emit schema-valid unchanged
manifest v1.

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
`--runs_per_test=20`.

The action-network test first verifies that the invoked Bazel is the exact
repository Nix package: Bazel 8.6.0 source, Linux sandbox patch, fixed policy,
output NAR, executable, and capability-ABI hashes must match
`tests/golden/bazel-toolchain.json`. The startup probe must pass before the
Bazel server starts. Patch-removal, wrong-output, and filter-load fixtures must
refuse before any governed action.

Configured-target, `aquery`, and strategy inventories cover stable/nightly
Rustc, metadata, Clippy, rustdoc, doctest compile/run, rustfmt, unpretty,
build-script, repository, setup, and test actions. Every governed action uses
the patched Linux `sandboxed` strategy. Process, local, standalone, worker,
remote, and every fallback are rejected. The sandbox child loads the fixed
filter before exec of the complete action command, so compile/build commands,
Bazel `test-setup.sh` or equivalent setup, tests, and descendants inherit it.
Do not credit an action wrapper for setup coverage.

Preflight plants pass inherited sockets, an ordinary ring, an SQPOLL ring, and
a ring with a registered fixed socket; each must refuse before load. Setup
before payload, compile/build, test, and descendant plants plus IPv4, IPv6,
netlink, packet, pathname Unix, abstract Unix, socketpair, and io_uring must
each observe the fixed policy errno. External-egress and live-index are
additional plants. Inspect every fixed-code stage diagnostic and exact slice
remedy; leak tests reject descriptor numbers, runtime paths, OS text, raw
output, and dynamic identifiers. Mandatory socket-using Rust tests remain on their
exact same-commit non-advisory Cargo compatibility carriers; inspect the
generated census and verify no namespace output claims socket creation was
denied. The committed yanked
snapshot key set comes only from `packages/Cargo.lock`; all repository fetches
remain outside governed actions, offline, and checksum/revision pinned. Main checks the full
set, while broker and guest check exact projections of their selected
package-policy graphs. The walker and `Cargo.guest.lock` must not contribute
keys.

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
  refusal, private-CLOEXEC same-open-file-description
  `execveat(AT_EMPTY_PATH)`, and behavioral
  close-on-exec checks for every auxiliary descriptor.
- a provider mutation that reintroduces `RESOLVE_BENEATH`, while strict
  result and cleanup paths retain
  `RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS`.
- compiler-derived API snapshots showing `VerifiedExecutable` has an empty
  public inherent API, empty locally-authored explicit-trait allowlist, and
  exact auto/blanket set, plus focused rustdoc compile-fail examples for
  construction, descriptor extraction/access, Deref/Borrow/fd traits,
  formatting/serialization, conversion, duplication/default, and minting;
- co-location of `VerifiedExecutable` and its only consuming public API in one
  dependency-leaf crate; reviewed safe `command-fds` mapping of the consumed
  verified description to a private fd; preserved stdin/stdout/stderr; and
  exactly one Rust invocation site for the exact immutable Nix store path.
  Under the one process-wide guard, the spawning thread uses reviewed safe
  `nix::sys::signal::SigSet` calls to block the full managed set before spawn
  and attempts restoration of its exact mask after successful and failed
  spawn before unlock. Capture, block, poisoned-guard, restoration, and
  overlapping-launch mutations prove one shared guard and restore-before-unlock;
- the dedicated tiny single-threaded C supervisor, statically built outside
  the product Rust workspace, with exact source, derivation-dependency,
  protocol, output NAR, executable, static ELF, and native-system hashes;
- the supervisor's close-on-exec nonblocking child exec-error pipe, sole fork,
  ignored `SIGPIPE` with typed `EPIPE`, waitable default `SIGCHLD`, normalized
  masks/dispositions only after first-operation refusal of any inherited
  managed `SIG_IGN`, one close-on-exec group-confirmation pipe, child and
  supervisor `setpgid` calls, exact live-group confirmation before `READY` or
  managed-signal consumption, handoff-window/normalization-time/
  pre-confirmation `SIGTERM`, typed `ESRCH`/`EPERM`/early-exit cleanup,
  pre-`READY` termination ownership, child stdio installation,
  executable-fd CLOEXEC, same-open-file-description
  `execveat(AT_EMPTY_PATH)`, explicit framed `READY` then `EXECUTED`,
  deterministic post-`READY` pre-exec signal queuing for every managed signal,
  helper-owned group kill/reap including child-death empty EOF, no forwarding
  or grace and no false `EXECUTED`/target terminal/audit publication before
  exec, continued supervision, fixed post-`EXECUTED` signal allowlist,
  external-TERM escalation without a case deadline, terminal record,
  direct-child reap, and exact normal/signaled target status;
- the patched Linux sandbox's fresh PID-namespace monitor as the sole abnormal
  teardown owner, with namespace kill/reap, one fixed 10,000 ms userspace
  escalation and close-or-quarantine ceiling, consuming outer-monitor reap,
  and real helper-crash-before-`READY`,
  crash-after-`READY`, crash-after-`EXECUTED`, crash-during-grace, and
  direct/double-forked long-lived-descendant plants. A beyond-ceiling plant
  proves typed `pending-kernel-cleanup`, owned quarantine, no reaped claim,
  no success/reuse, and eventual consuming reap by the same original live
  monitor while the action remains failed. The pending diagnostic links to
  `docs/contributing/critical-subsystems.md#bazel-pending-kernel-cleanup-quarantine`;
  its file/anchor and consuming-reap release bytes resolve exactly. Namespace,
  teardown-patch, ceiling, quarantine, false-reap, reboot-remedy,
  retry-before-release, replacement-waiter, manual-release, success/reuse, and
  fallback mutations fail; Cargo tests use containment mocks only;
- no runfiles/worktree/copied helper path, second Rust invocation, fd-0
  executable transport, Rust `pre_exec`, Rust raw fork, reopen,
  `/proc/self/fd`, `fexecve`, path fallback, provider-fd leak, or first-party
  Rust unsafe allowance or numeric Rust PID/PGID signal;
- complete Rust-parent and C-supervisor prepare, identity, map, adopt,
  normalize, pipe, fork, child-setup, execveat, exec-result, supervise, wait,
  terminal, cleanup, and reap tables, with injected held-open writer,
  closed-reader `EPIPE`, exact
  single-record exec-error `EINTR`/`EAGAIN`/short/partial/overlong transport,
  fragmented/coalesced framed status and malformed/duplicate/order negatives,
  descriptor absence,
  private-fd identity, helper crash/EOF before `EXECUTED`, fast target exit
  with the same status as the crash, inherited ignored/`SA_NOCLDWAIT`
  `SIGCHLD`, capture/block/guard-poison/restoration failure coverage,
  overlapping-launch restore-before-unlock mutation, inherited managed
  `SIG_IGN` refusal, handoff-window/normalization-time/blocked SIGTERM,
  post-`READY` pre-exec signals and empty-EOF priority, setpgid races and typed
  confirmation failures, target-ignore-TERM, signal
  forwarding, target-status mismatch, and every cleanup/wait/reap failure.

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
- `D2B-BZLEVIDENCE-SANITIZE`, `D2B-BZLEVIDENCE-LIMIT`,
  `D2B-BZLEVIDENCE-RETENTION`, `D2B-BZLEVIDENCE-PUBLISH`, and
  `D2B-BZLEVIDENCE-NO-VERDICT`: name the stable repository-relative input or
  carrier/workflow, the corrective action, and one literal
  `make test-bazel-rust-{main,api,broker,aux}` or fixed qualification command.
  They preserve `testVerdict`, emit the structurally valid degraded variant,
  and contain no planted value.

Before alias removal these byte-exact diagnostics use command version 1 and
name existing `test-bazel-rust*` targets. The alias-removal change must
atomically update every renderer and exact-message test to command version 2,
which names only `make test-rust` or
`make test-rust-slice-{main,api,broker,aux}`. No intermediate or merged state
may name a target that does not exist.

Also inspect the provider table in `contracts/runner-environment.md`: every
provider refusal names the declared repository-relative provider key, its
specific correction, and one exact closed slice rerun command without a
generic placeholder, absolute runfiles location, or descriptor. Run the
reason-by-slice exact-message matrix.

Qualification and release tables must likewise render fixed codes and exact
remedies. Qualification and release query errors are typed degradation, never
absence. Reject any message containing `$!`, descriptor numbers, runtime,
socket, absolute, or Nix store paths, OS text, raw child/API/tool output,
argv/environment values, dynamic identifiers, raw cursors/handles, or
free-form/borrowed commands.

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

Run the fixture-backed validator tests in spec003w3:

```bash
set -euo pipefail
(cd packages && cargo test -p xtask --test bazel_qualification)
```

Do not run the no-argument `cargo xtask bazel-qualification-validate` command
yet. Its fixed `evidence/qualification.json` input is initialized and
completed only in spec003w4. The fixture suite must cover a protected push
where either or both workflows have no verdict and require a bounded degraded
record that preserves available test verdicts and resets the streak.

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
verdict is a refusal. It also requires every one of the seven closed
PID-namespace containment stages, every validator mutation result, matching
sandbox patch and canonical monitor identity digests, legal cleanup/quarantine
states, and the pending-cleanup no-success/no-reuse proof. Raw PIDs,
descriptors, paths, process output, and opaque identities refuse.

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
  .architecture.x86_native_six_checks_passed and
  .architecture.aarch64_native_six_checks_passed and
  .architecture.aarch64_supply_chain_passed_on_same_stable_head and
  .broker.all_contexts_exclusive and
  .broker.all_contexts_twenty_consecutive and
  .action_network.patched_bazel_identity_exact and
  .action_network.startup_capability_probe_passed and
  .action_network.sandbox_load_before_action_exec and
  .action_network.inherited_capability_preflight_exact and
  .action_network.stable_nightly_action_inventory_exact and
  .action_network.sandbox_strategy_inventory_exact and
  .action_network.no_process_local_standalone_worker_remote_fallback and
  .action_network.setup_before_payload_plant_denied and
  .action_network.all_eight_socket_io_uring_plants_denied and
  .action_network.external_egress_plant_refused and
  .action_network.live_index_plant_refused and
  .action_network.cargo_compatibility_census_exact and
  .action_network.compatibility_verdicts_same_head_non_advisory and
  (.containment.results | length) == 7 and
  .containment.stage_census_exact and
  .containment.recovery_classes_exact and
  .containment.patch_monitor_digests_exact and
  .containment.cleanup_quarantine_results_legal and
  .containment.pending_cleanup_no_success_no_reuse and
  .containment.all_validator_mutations_passed and
  .containment.forbidden_field_count == 0 and
  .supply_chain.all_three_contexts_equal and
  .architecture.all_guest_elf_et_dyn and
  .architecture.all_guest_elf_machine_matches and
  .architecture.non_pie_plant_refused and
  .architecture.wrong_machine_plant_refused and
  .architecture.artifact_baseline_row_count == 4 and
  .architecture.size_authorizations_valid and
  .architecture.persisted_store_path_count == 0 and
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
  (.no_shell_inventory.governed_and_declared_nonempty and
   .no_shell_inventory.governed_declared_equal and
   .no_shell_inventory.spawn_sources_governed and
   .no_shell_inventory.raw_scan_record_count ==
     .no_shell_inventory.governed_source_count and
   .no_shell_inventory.unique_scan_source_count ==
     .no_shell_inventory.governed_source_count and
   .no_shell_inventory.zero_site_scan_records_complete and
   .no_shell_inventory.spawn_sites_bidirectional and
   (.no_shell_inventory.plants | length) == 6) and
  .evidence_sinks.all_forbidden_values_absent and
  .evidence_sinks.all_bounds_hold and
  .evidence_sinks.all_retention_classes_enforced and
  .evidence_sinks.manifest_v1_unchanged and
  .enforcement.required_jobs_non_advisory
' specs/003-adr052-bazel-rust/evidence/qualification.json
```

Every record carries all three counts, derived from the records themselves
rather than from a self-asserted aggregate; every cold record additionally
carries four `sliceDurationsSeconds` entries.

```bash
set -euo pipefail
if grep -Fq '/nix/store/' \
  specs/003-adr052-bazel-rust/evidence/qualification.json
then
  exit 1
fi
```

Also require ten consecutive matching qualification records, five topology
proofs, twenty consecutive executions per broker context, complete locator
evidence, three valid performance sets, the committed no-shell inventory digest
with all six plant results and equal raw/unique/governed scan counts, all seven
containment results and every containment-validator mutation result, and all
planted guard results.

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
- cache deletion authority comes only from the closed committed typed prefix
  set; mixed authorized/unauthorized pagination preserves every unauthorized
  entry and every authorization refusal records zero delete calls;
- each primary key is unique for its successful protected-`v3` run;
- restore prefixes contain neither run ID nor commit SHA;
- maintenance retains the newest complete generation for each authorized
  prefix.
- the table-driven cache test mutates every bound input and changes every
  applicable action/repository key without collapsing namespaces.
- `test-flake-aarch64`, all four Rust slices, and the `test-rust` rollup are
  non-advisory; advisory-classification mutations fail.

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
changelog list all five Bazel alias replacements and every exact surface ID
from `cargoCompatibilityCarriers`. They call those surfaces permanently hybrid
under this specification, list the retained socket-using Cargo cases and
public executor, and state that separate authorization is required before
retirement. The same change updates
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
assert_no_fixed() {
  pattern=$1
  shift
  if grep -Fq -- "$pattern" "$@"; then
    return 1
  else
    status=$?
    test "$status" -eq 1
  fi
}
test -r tests/README.md
test -r docs/reference/test-execution-manifest.md
assert_no_fixed 'eight CI leaf targets' tests/README.md
assert_no_fixed \
  'runs API, main, broker, guest, no-bash, schema, inventory and supply chain' \
  docs/reference/test-execution-manifest.md
```

Those two literals are the current committed sentences that assert eight Rust
CI jobs; both must be gone, and all eight public leaf names must remain
documented.

Run the enforcing hybrid-disclosure policy after the promotion fragment and
all five fixed docs are present:

```bash
set -euo pipefail
make test-policy
```

`policy_bazel_hybrid_docs` derives the exact nonempty
`cargoCompatibilityCarriers` census from the coverage map, retaining each
case's surface ID, Cargo selector, test identity, and socket class, and
requires every governed semantic block and the present fragment to equal all
entries in both directions. Distinct cases sharing one surface remain
distinct. Run isolated empty-census, missing, extra, malformed-block,
duplicate-block, malformed-identity, duplicate-identity, stale-attribution,
and governed-document-mismatch fixtures; each must fail at its own predicate.
The same test governs the alias-removal and Cargo-retirement fragments when
those files are present.

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

After the promotion merge and record creation, run:

```bash
set -euo pipefail
(cd packages && cargo xtask bazel-promotion-record-validate)
```

It must bind the record to the actual protected-`v3` pull-request merge commit
and re-derive the exact sealed `spec003w5` candidate, content, and snapshot
identities. Run old-SHA, candidate-SHA, wrong-seal, unsealed-merge, and wrong
PR merge-SHA negatives before `spec003w5fu1` seals.

## spec003w6 and spec003w7 independent checks

Alias removal requires a containing published semantic release tag, not any
tag. First run the fixed-code checker:

```bash
set -euo pipefail
(cd packages && cargo xtask bazel-promotion-record-validate)
(cd packages && cargo xtask bazel-release-containment-validate)
```

The validator transiently enumerates only tags matching
`^v[0-9]+\.[0-9]+\.[0-9]+$`, proves promotion ancestry, compares peeled local
and origin objects, and requires present non-draft/non-prerelease release
metadata. This repository already carries two-component tags such as `v1.0`,
so containment alone is not an entry condition. It persists only a successful
tag-reference digest.

A two-component tag, a local-only tag, a divergent same-named local and remote
tag, a draft release, and a prerelease each fail entry, and the owning
interface test carries all five as negatives. Inspect the fixed
`D2B-BZLRELEASE-NO-TAG`, `D2B-BZLRELEASE-UNPUSHED`,
`D2B-BZLRELEASE-DIVERGENT`, `D2B-BZLRELEASE-NO-RELEASE`, and
`D2B-BZLRELEASE-NOT-FINAL` refusals and the
`D2B-BZLRELEASE-RECORD-QUERY`, `D2B-BZLRELEASE-LOCAL-QUERY`,
`D2B-BZLRELEASE-ORIGIN-QUERY`, and
`D2B-BZLRELEASE-METADATA-QUERY` degradations and their exact remedies from
`make-target-compatibility.md`. A failed query must never appear as absence,
and no candidate/tag/object identifier or raw command output may be
substituted. Run
both no-argument validators before this containment check.

In the alias-removal candidate, verify the diagnostic transition atomically:

```bash
set -euo pipefail
if grep -R 'make test-bazel-rust' \
  packages/d2b-bazel-exec/src/provider.rs \
  packages/d2b-bazel-exec/src/execute.rs \
  packages/d2b-bazel-runner/src/lib.rs \
  packages/d2b-bazel-runner/src/coverage.rs \
  packages/d2b-bazel-runner/src/diagnostic.rs \
  packages/d2b-bazel-runner/src/junit.rs \
  packages/d2b-bazel-runner/src/manifest.rs \
  packages/d2b-bazel-runner/src/recovery.rs \
  packages/xtask/src/main.rs \
  packages/xtask/src/bazel_evidence.rs \
  packages/xtask/src/bazel_qualification.rs \
  packages/xtask/src/hermeticity.rs \
  AGENTS.md tests/AGENTS.md tests/README.md \
  docs/contributing/gates-and-lints.md \
  docs/reference/test-execution-manifest.md \
  changelog.d/adr052-bazel-alias-removal.md \
  specs/003-adr052-bazel-rust/evidence/post-promotion.json
then
  printf '%s\n' 'stale shadow target in promoted diagnostic surface' >&2
  exit 1
else
  grep_status=$?
  if [ "$grep_status" -ne 1 ]; then
    exit "$grep_status"
  fi
fi
make test-rust
make test-rust-slice-main
make test-rust-slice-api
make test-rust-slice-broker
make test-rust-slice-aux
```

The exact-message tests must prove every provider, sandbox-policy,
qualification threshold/table, evidence/publication, cleanup, and recovery
renderer, both module roots, every governed doc, the evidence record, and the
semantic changelog now use command version 2. The pre-change fixture is the
only version-1 record and may name only shadow targets that all exist in its
fixture Makefile. The grep accepts only status 1 as absence; status 2 or any
other error propagates and fails the check.

Cargo implementation retirement:

```bash
set -euo pipefail
(cd packages && cargo xtask bazel-promotion-record-validate)
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

The API stream is complete but transient. `post-promotion.json` persists only
`paginationState = "complete"`, page/stream counts, the fixed stream digest,
and final ten normalized units, with an attempt-history count and digest per
unit. It persists no raw cursor. Verify its schema byte and record bounds and
atomic replacement, and use an oversized transient fixture to prove the
bounded record yields the same decision as the complete in-memory oracle.

```bash
set -euo pipefail
jq -e '
  .paginationState == "complete" and
  (.pageCount | type) == "number" and
  (.streamCount | type) == "number" and
  (.streamSha256 | test("^[0-9a-f]{64}$")) and
  (has("finalCursor") | not) and
  (has("cursor") | not)
' specs/003-adr052-bazel-rust/evidence/post-promotion.json
```

After either change, every public Rust Make name, fixture mode, and mandatory
socket-test Cargo compatibility carrier must still work. spec003w7
qualification and code preparation may run first, but its shared documentation
and evidence task waits for merged spec003w6, rebases, reruns complete
validation, and receives a new panel verdict.
The spec003w6 and spec003w7 docs and semantic changelog fragments repeat the
exact hybrid surface/case inventory and separate authorization requirement.
Do not cite container, VM, live-host, hardware, or deployed-host tiers for this
internal build scheduler.
