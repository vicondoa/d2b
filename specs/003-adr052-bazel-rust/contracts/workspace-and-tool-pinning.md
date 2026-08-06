# Workspace, Hub, Package Policy, and Tool Pinning Contract

## Product and walker authorities

| Authority | Manifest | Cargo lock | Bazel-side lock |
| --- | --- | --- | --- |
| Product | `packages/Cargo.toml` | `packages/Cargo.lock` | `bazel/cargo/product.lock` |
| Walker | `tests/tools/no-bash-ast-walker/Cargo.toml` | `tests/tools/no-bash-ast-walker/Cargo.lock` | `bazel/cargo/walker.lock` |

The product manifest uses resolver version 2 and contains the existing main
members, `d2b-priv-broker`, and `d2b-guest-shell-runner`. Broker and guest have
no nested `[workspace]`, workspace profile, or lock.

`packages/Cargo.guest.lock` is a generated static-guest closure input only. It
is not a Cargo workspace authority, a `crate_universe` hub, a product repin
input, or a first-party target source.

No synthetic splice manifest, generated standalone product workspace, or
forwarding lock may be created.

## Selected Cargo contexts

Run after entering `nix develop` at repository root and `cd packages`.

Broker:

```text
cargo test --locked -p d2b-priv-broker --no-default-features -- --test-threads 1
cargo test --locked -p d2b-priv-broker --no-default-features --features layer1-bootstrap -- --test-threads 1
cargo test --locked -p d2b-priv-broker --no-default-features --features fake-backends -- --test-threads 1
```

Guest:

```text
cargo fmt -p d2b-guest-shell-runner --check
cargo clippy --locked -p d2b-guest-shell-runner --no-default-features --features real-libshpool --all-targets -- -D warnings
cargo nextest run --locked -p d2b-guest-shell-runner --no-default-features --features real-libshpool
```

Generic main:

```text
cargo clippy --locked --workspace --all-targets --exclude d2b-priv-broker --exclude d2b-guest-shell-runner -- -D warnings
cargo nextest run --locked --workspace --exclude d2b-contract-tests --exclude d2b-priv-broker --exclude d2b-guest-shell-runner
```

`cargo fmt` is package-only. It takes neither `--locked` nor feature
selectors, because formatting does not resolve the selected dependency graph.
Every dependency-resolving command uses `--locked`. Broker and guest
dependency-resolving commands name the package, disable default features, and
name the exact feature set. Generic main Clippy uses exactly the two package
exclusions above; generic main tests and their companion discovery use exactly
the three exclusions above.

Broker contexts run serially in these gate-owned target directories:

```text
packages/d2b-priv-broker/target
packages/d2b-priv-broker/target-layer1
packages/d2b-priv-broker/target-fakebackends
```

Guest dependency-resolving commands and companions use
`packages/d2b-guest-shell-runner/target`. The gate sets each directory through
an explicit `CARGO_TARGET_DIR`; none is a Cargo workspace root.

## Exact selected-context oracle

No selected closure is inferred from the shared lock, an unfiltered workspace
metadata result, a synthetic manifest, or a splice. The oracle is a three-way
join over the real root workspace, and no view is authoritative beyond the
columns it owns:

1. Locked offline, target-filtered root Cargo metadata
   (`cargo metadata --locked --offline --format-version 1 --filter-platform
   <target>`) supplies package identities, sources, target-filtered candidate
   edges, and each edge's dependency kind and `cfg` predicate from
   `resolve.nodes[].deps[].dep_kinds`. Measured metadata objects carry no
   `checksum` field, and `resolve.root` is null for a workspace, so
   metadata supplies neither checksums nor the selected root.
2. `packages/Cargo.lock` plus the committed git archive pin supplies every
   registry and git checksum: registry entries supply `checksum`, and the
   git dependency's exact `rev` and output hash come from the committed pin.
3. Package-selected stable `cargo tree` traversals supply the exact root,
   dependency-kind reach, and resolved features.

Plain `cargo tree` output is not machine-readable and is never parsed. Every
traversal pins its parser input exactly:

```text
cargo tree --locked --offline --manifest-path Cargo.toml \
  -p <package> --target <target> --no-default-features \
  --features <exact list> --edges <kinds> \
  --charset ascii --prefix depth --no-dedupe \
  --format '|{p}|{f}|'
```

Measured behavior forces that flag set: `--prefix depth` emits the depth
integer with no separator before the format string, so the repository-pinned
`--format` must begin with a delimiter; `{p}` abbreviates git revisions and
prints no source at all for registry packages, so tree output is never a
source or checksum authority; `{f}` is the resolved-feature column.

Production and dev-inclusive edges are traversed separately wherever dependency
kind matters: `--edges normal,build` for the production closure and
`--edges normal,build,dev` for the policy closure. Policy retains root dev
edges and their complete normal/build closure. Post-filtering one traversal to
synthesize the other is not permitted.

The oracle requires one root, a nonempty exact traversal per variant, identity
equality between every traversal row and metadata, dependency-kind and `cfg`
agreement with `dep_kinds`, and lock-supplied checksum coverage for every
non-path identity. A target, package, edge-kind, default-feature, feature,
identity, or checksum mismatch fails before closure policy.

The feature canary is an unrelated workspace member that enables an
otherwise-absent feature on a dependency shared with broker or guest. That
feature appears in a whole-workspace union but must remain absent from the
`{f}` column of both package-selected traversals. Connecting the canary to
either selected root must change that context and fail its exact census.

Generic Cargo and Nix build/test and Clippy contexts exclude
`d2b-priv-broker` and `d2b-guest-shell-runner` exactly. Dedicated contexts
retain their exact package, target, default-feature, feature, and edge-kind
selection. Tests cover both directions so broad generic contexts and weakened
dedicated selectors cannot pass.

## Libshpool

The guest manifest contract is:

```toml
[features]
default = []
real-libshpool = []

[dependencies]
libshpool = "0.11.0"
```

Existing code activation stays behind `cfg(feature = "real-libshpool")`.
`crate.spec` is forbidden in generated and hand-written Bazel files.

## Dependency hubs

`crate_universe` declares exactly:

```text
product
walker
```

The product hub is an accepted third-party package and feature union. It does
not define actual first-party dependencies. Every first-party product crate is
a native Bazel target, and each broker and guest configured context declares
its direct first-party dependencies, `@product` dependencies, cfg values, and
features.

Every selected-context guard runs in this order:

1. assert one selected root;
2. materialize a nonempty complete context closure;
3. assert the exact generated census;
4. prove hub and lock containment;
5. prove direct first-party dependencies, cfgs, and features;
6. prove no cross-context edge and no unrelated first-party sibling.

## Hub regeneration

The only commands are:

```text
cargo xtask bazel-repin --hub product
cargo xtask bazel-repin --hub walker
```

Every hub sets `lockfile`, `cargo_lockfile`, and
`skip_cargo_lockfile_overwrite = True`.

The command:

- requires one accepted hub;
- refuses ambient `CARGO_BAZEL_REPIN`, `REPIN`, or
  `CARGO_BAZEL_REPIN_ONLY`;
- sets repin controls only on its one Bazel child;
- uses the wrapper's absolute startup options;
- changes only the selected Bazel-side lock;
- exits zero with no change when current;
- is unreachable from Make and workflows.

On the fresh spec003w0 tree only, `MODULE.bazel.lock` does not yet exist.
Repin therefore adds command-local `--lockfile_mode=off` while generating the
two initial hub locks. That exception neither creates nor updates the module
lock and is refused once `MODULE.bazel.lock` exists. After product and walker
locks exist, `bazel-module-refresh` creates the final module lock last. Every
ordinary repin then runs under global `--lockfile_mode=error`. Tests cover the
fresh bootstrap, refuse `off` after bootstrap, and prove each bootstrap repin
changes only its selected hub lock.

Retired hubs fail before Bazel starts with exactly:

```text
Hub 'main' is retired; after entering nix develop, run from packages/: cargo xtask bazel-repin --hub product
Hub 'broker' is retired; after entering nix develop, run from packages/: cargo xtask bazel-repin --hub product
Hub 'guest' is retired; after entering nix develop, run from packages/: cargo xtask bazel-repin --hub product
```

Tests use an injected non-mutating executor and require:

```text
argv = ["cargo", "xtask", "bazel-repin", "--hub", "product"]
cwd = "packages/"
```

A `cd packages` argv element or `packages/` command prefix is refused as a
duplicated `packages/packages` path. The tests never run a real repin. No final
newline contract exists.

## Module lock and pinned tools

- Bazel is 8.6.0 from pinned nixpkgs.
- `rules_rust` is pinned to the reviewed compatible version.
- `cargo-bazel` is pinned by URL and sha256; source bootstrap is refused.
- `MODULE.bazel.lock` runs under `common --lockfile_mode=error`.
- Direct module disagreement runs under
  `common --check_direct_dependencies=error`.
- `cargo xtask bazel-module-refresh` is the only module-lock update path.
- `.bazelrc` contains no startup line and no global Rust channel flag.
- Wrapper-supplied startup options are absolute and byte-identical across all
  commands selecting the Bazel server.

### Module-lock refresh

`cargo xtask bazel-module-refresh` is a repository-owned, no-argument
contributor mutation. Its test lands before its implementation and plants real
module drift against the pinned Bazel. The command:

1. refuses every argument and every ambient `CARGO_BAZEL_REPIN`, `REPIN`, or
   `CARGO_BAZEL_REPIN_ONLY` control before starting Bazel;
2. runs the measured
   `bazel mod deps --lockfile_mode=update` child with the same absolute
   `--output_user_root` and `--output_base` startup options used by every other
   server-selecting command;
3. permits exactly `MODULE.bazel.lock` to change and fails while listing any
   other changed path repository-relative;
4. exits zero with no change on an already-current candidate;
5. is absent from `Makefile` and every workflow; and
6. uses only the exact `D2B-BZLDRIFT-MODULE` remediation row below beside a
   real `--lockfile_mode=error` refusal.

The implementation test proves exact lock-only mutation, second-run
idempotence, startup-option identity, refusal of unrelated mutations, and the
exact remediation beside the upstream failure. A mutation that runs bare
`bazel mod`, changes another tracked file, names `bazel-repin`, or exposes the
command through Make or a workflow must fail.

### ADR-0054 drift and refusal messages

Every row exits nonzero. The exact first two remediation steps are shared and
ordered:

```text
From the repository root, run: nix develop
Then run: cd packages
```

The remaining exact sequence is:

| Code | Condition | Command | Review and commit | Final rerun |
| --- | --- | --- | --- | --- |
| `D2B-CARGODRIFT-PRODUCT` | `packages/Cargo.lock` stale | `cargo generate-lockfile --offline` | `Review and commit packages/Cargo.lock.` | `Rerun cargo generate-lockfile --offline; run cargo xtask bazel-repin --hub product and review and commit bazel/cargo/product.lock; run cargo xtask bazel-module-refresh and review and commit MODULE.bazel.lock; then rerun the failed command.` |
| `D2B-CARGODRIFT-WALKER` | walker `Cargo.lock` stale | `cargo generate-lockfile --offline --manifest-path ../tests/tools/no-bash-ast-walker/Cargo.toml` | `Review and commit tests/tools/no-bash-ast-walker/Cargo.lock.` | `Rerun the walker cargo generate-lockfile command; run cargo xtask bazel-repin --hub walker and review and commit bazel/cargo/walker.lock; run cargo xtask bazel-module-refresh and review and commit MODULE.bazel.lock; then rerun the failed command.` |
| `D2B-BZLDRIFT-PRODUCT-HUB` | `bazel/cargo/product.lock` stale | `cargo xtask bazel-repin --hub product` | `Review and commit bazel/cargo/product.lock.` | `Rerun cargo xtask bazel-repin --hub product, then rerun the failed command.` |
| `D2B-BZLDRIFT-WALKER-HUB` | `bazel/cargo/walker.lock` stale | `cargo xtask bazel-repin --hub walker` | `Review and commit bazel/cargo/walker.lock.` | `Rerun cargo xtask bazel-repin --hub walker, then rerun the failed command.` |
| `D2B-BZLDRIFT-MODULE` | `MODULE.bazel.lock` stale | `cargo xtask bazel-module-refresh` | `Review and commit MODULE.bazel.lock.` | `Rerun cargo xtask bazel-module-refresh, then rerun the failed command.` |
| `D2B-BZLDRIFT-GENERATOR` | generated Bazel output stale | `cargo xtask gen-bazel` | `Review and commit the listed repository-relative generated paths.` | `Rerun cargo xtask gen-bazel --check, then rerun the failed command.` |
| `D2B-BZLDRIFT-PACKAGE-POLICY` | package-policy output stale | `cargo xtask gen-package-policy-inputs` | `Review and commit the generated changes under packages/policy-inputs/.` | `Rerun cargo xtask gen-package-policy-inputs --check, then rerun the failed command.` |
| `D2B-BZLDRIFT-YANKED` | yanked snapshot stale | `cargo xtask bazel-yanked-refresh` | `Review and commit bazel/supply_chain/yanked-snapshot.json.` | `Rerun cargo xtask bazel-yanked-check, then rerun the failed command.` |
| `D2B-BZL-AMBIENT-REPIN` | a repin control is present | `unset CARGO_BAZEL_REPIN REPIN CARGO_BAZEL_REPIN_ONLY` | `Review the requested contributor command and its selected hub; no file is changed by this refusal.` | `Rerun the exact refused command from the closed contributor-command set.` |
| `D2B-BZL-UNEXPECTED-MUTATION` | a mutation changed an unapproved tracked path | `git status --short --untracked-files=all` | `Review every listed repository-relative path; commit the intended generated change or remove the unintended change.` | `Rerun the exact refused command from the closed contributor-command set.` |

Each rendered message is the code and condition sentence, the two shared
steps, then the row's command, review sentence, and rerun sentence. A closed
command enum supplies the exact refused command for the final two rows; no
free-form command or path is interpolated.

Table-driven tests assert exact bytes, nonzero status, repository-relative
paths, and the correct row-specific remedy. Wrong-code, missing-step,
wrong-command, borrowed-remedy, absolute-path, worktree, user ID, process ID,
token, and rejected ambient-value plants must fail. The three retired-hub
diagnostics above are byte-unchanged and are not rewritten into this table.

## Action network boundary

ADR 0052's no-network action invariant remains literal. Rust build and test
actions may open no network or Unix socket, including a loopback listener or
connection. `CARGO_NET_OFFLINE=1` and a Linux network namespace are defense in
depth, not proof. A network namespace does not deny socket creation.

The proof is the repository-owned Nix package
`pkgs/bazel-8.6.0-seccomp/default.nix`. It pins Bazel 8.6.0 and applies exactly
`linux-sandbox-seccomp.patch`. The installed fixed policy and the Bazel
executable share one immutable output. A committed identity record binds the
exact upstream source, patch, policy, and capability ABI plus separate
`x86_64-linux` and `aarch64-linux` output NAR and executable hashes. Gates use
the matching native Bazel output directly; Bazelisk, foreign-system output,
and ambient Bazel are not accepted.

The patch carries the fixed policy through the Linux sandbox runner into the
sandbox child. After sandbox construction and before exec of the action
command, the child rejects inherited sockets and every io_uring ring including
SQPOLL and registered/fixed-socket state, sets `no_new_privs`, verifies and
loads the fixed filter, then execs the complete action command. Compile/build
commands, Bazel `test-setup.sh` or equivalent setup, tests, and all descendants
therefore inherit the policy. No action wrapper or `--run_under` is used or
credited.

The filter denies the complete socket-operation set, all socket domains
through `socket`, `socketpair`, descriptor import through `pidfd_getfd`, and
all three io_uring entry points. The exact syscall and eight-plant inventories
are closed in `coverage-map.md`. Every action receives tools, sources, yanked snapshot, and the pinned RustSec
database as declared inputs. Configured-target, `aquery`, and strategy
inventories cover every stable/nightly action kind. Governed execution accepts
only the patched Linux `sandboxed` strategy and rejects `process`, `local`,
`standalone`, `worker`, `remote`, `no-sandbox`, a network-enabling tag, or any
fallback.

The startup probe runs against the exact Nix output before a Bazel server
starts and requires the capability ABI plus a real fixed-denial result.
Missing/wrong output, patch removal, policy mismatch, failed filter load, and
any strategy fallback fail closed. Evidence persists hashes and the capability
version, never a complete Nix store path.

Repository fetches remain outside governed Rust actions. Gates operate
offline; registry archives use the URL and checksum from `packages/Cargo.lock`;
`wl-proxy` uses its pinned revision and committed archive sha256. No action may
read a live package index, advisory database, external URL, or external
socket, and no unpinned repository fetch exists.

Enforcement includes:

- exact Nix output identity, startup capability, configured-target, `aquery`,
  and strategy inventories proving every governed action kind and both Rust
  toolchains use the patched Linux sandbox and every fetch stays outside the
  governed action;
- patch-removal, wrong-output, policy-mismatch, filter-load, and
  setup-before-payload plants;
- inherited socket, ordinary-ring, SQPOLL-ring, and registered-fixed-socket
  ring plants that refuse before filter load;
- the eight IPv4, IPv6, netlink, packet, pathname Unix, abstract Unix,
  socketpair, and io_uring pre-action plants, each observing the fixed policy
  errno, plus separate compile/build, test, and descendant placement plants;
- a planted build/test action that attempts external egress and must fail;
- a live-index plant that refuses before name resolution or socket use; and
- an exact generated census of committed mandatory socket-using tests that
  remain under their current Cargo compatibility carriers, with same-commit
  non-advisory verdicts attributed to their existing surface IDs.

Qualification contains exact package identity and startup capability,
patch-removal/filter-load/setup placement, strategy inventory, all eight
socket/io_uring plants, external-egress and live-index results, the exact
offline repository-fetch inventory, fresh PID-namespace containment,
crash-stage/long-lived-descendant plant results, and the complete Cargo
compatibility census. Promotion describes
affected surfaces as hybrid. The Cargo compatibility carriers cannot be
retired until a separate authorized design changes the no-network invariant.

## Immutable execution supervisor pin

`pkgs/d2b-bazel-exec-supervisor/default.nix` statically builds the one reviewed
`tests/tools/d2b-bazel-exec-supervisor/supervisor.c` source and installs
exactly one `d2b-bazel-exec-supervisor` executable. This is a dedicated
build/test-tooling derivation. It is not a Rust crate, is absent from
`packages/Cargo.toml`, and cannot enter the product dependency hub.
`tests/golden/bazel-exec-supervisor.json` records:

- the exact C source SHA-256;
- the Nix expression and fixed protocol-schema SHA-256;
- the exact native compiler, static libc, headers, and every other derivation
  dependency identity and hash;
- separate `x86_64-linux` and `aarch64-linux` derivation and dependency-closure
  hashes;
- separate output NAR and matching native executable SHA-256 values;
- static ELF evidence with no interpreter or dynamic `NEEDED` entry; and
- the fixed private executable fd, supervisor status fd, single-record
  exec-error shape and overlong rule, framed `D2BS`
  `READY`/`EXECUTED`/`EXITED`/`SIGNALED` header/version/type/length shapes,
  27-byte retained decoder bound, signal allowlist, block-first initialization,
  ignored `SIGPIPE`, waitable default `SIGCHLD`, pre-`READY` termination
  ownership, pre-`EXECUTED` queuing with empty-EOF priority and no false
  execution/audit publication, fixed post-exec external-TERM escalation,
  absolute-deadline transport, and
  protocol version.

The safe typed Rust consumer embeds the exact helper store path from that Nix
toolchain artifact and accepts no path parameter or environment override. It
verifies output identity before spawn. Missing output, wrong output, copied
binary, symlink rebind, runfiles path, worktree path, altered source or
derivation dependency, dynamic output, or digest mismatch refuses before the
verified descriptor is mapped. Committed records persist no complete Nix store
path.

Rust parent spawn and descriptor mapping use the exact reviewed safe
`command-fds` pin from `packages/Cargo.lock`. Under the one process-wide
serialization guard, the spawning thread uses the already reviewed safe
`nix::sys::signal::SigSet` API to capture its exact mask, block the full
managed set before `Command::spawn`, and attempt restoration of the captured
mask after successful or failed spawn before unlocking. Injected
capture/block/poison/restoration failures and overlapping launches prove the
shared guard and restore-before-unlock. The C supervisor is the only
fork owner. It is single-threaded and inherits that blocked set. Its first
setup operation inspects every managed disposition and fails with the typed
ignored-disposition recovery code before fork if any is `SIG_IGN`; only after
verification may it normalize dispositions, install synchronous consumption,
and establish the final mask. Parent and child both perform `setpgid`, the
parent confirms the exact group before `READY`, and
managed signals stay blocked through that confirmation. It uses the fixed
protocol described in `runner-environment.md`: before `EXECUTED`, any managed
signal becomes one typed helper-owned setup termination with no forwarding,
grace, `EXECUTED`, target terminal, or target-executed audit event, including
when the child dies with empty exec-pipe EOF. No Rust unsafe exception, Rust helper crate,
runfiles/worktree helper, numeric Rust PID/PGID signal, or fallback exists.
The patched Bazel PID-namespace monitor, not Rust, is the abnormal-teardown
owner. Its patch, userspace ceiling, `pending-kernel-cleanup` quarantine,
no-success/no-reuse rules, canonical monitor identity digest, and ordinary
plus beyond-ceiling crash plants are bound by
`tests/golden/bazel-toolchain.json`. The patched sandbox owns every
`SANDBOX_*` renderer and live exact test. Its pending-cleanup diagnostic links
to the governed
`docs/contributing/critical-subsystems.md#bazel-pending-kernel-cleanup-quarantine`
runbook. The original live monitor remains sole wait owner through consuming
reap; no reboot, retry-before-release, replacement waiter, or manual release
exists. A closed invocation-site policy
permits exactly one Rust source location to spawn this exact output and rejects
every other Rust, Bazel, Make, workflow, or documentation command site.

## Package policy generation

After entering `nix develop` at repository root and `cd packages`:

```text
cargo xtask gen-package-policy-inputs
cargo xtask gen-package-policy-inputs --check
```

The target matrix is:

| Nix system | Broker GNU target | Guest musl target |
| --- | --- | --- |
| `x86_64-linux` | `x86_64-unknown-linux-gnu` | `x86_64-unknown-linux-musl` |
| `aarch64-linux` | `aarch64-unknown-linux-gnu` | `aarch64-unknown-linux-musl` |

Generated paths:

```text
packages/policy-inputs/<system>/<gnu-target>/broker-production/production/closure.json
packages/policy-inputs/<system>/<gnu-target>/broker-production/production/Cargo.lock
packages/policy-inputs/<system>/<gnu-target>/broker-production/policy/metadata.json
packages/policy-inputs/<system>/<gnu-target>/broker-production/policy/Cargo.lock
packages/policy-inputs/<system>/<musl-target>/guest-real-libshpool/production/closure.json
packages/policy-inputs/<system>/<musl-target>/guest-real-libshpool/production/Cargo.lock
packages/policy-inputs/<system>/<musl-target>/guest-real-libshpool/policy/metadata.json
packages/policy-inputs/<system>/<musl-target>/guest-real-libshpool/policy/Cargo.lock
```

Production contains selected normal and build closure. Policy adds root dev
edges and the complete transitive normal and build closure reached from those
dev packages.

Every artifact binds root, system, target, package identity, version, source,
checksum, edge kind, cfg, and resolved features. Drift output lists every stale
path repository-relative and renders the exact
`D2B-BZLDRIFT-PACKAGE-POLICY` row above:

```text
From the repository root, run: nix develop
Then run: cd packages
cargo xtask gen-package-policy-inputs
Review and commit the generated changes under packages/policy-inputs/.
Rerun cargo xtask gen-package-policy-inputs --check, then rerun the failed command.
```

## Exact selected-source census

For each production and policy graph, derive the exact sorted non-path
`(name, version, source)` set from metadata and independently from the filtered
lock. They must be equal.

Before deny or audit:

- the selected root exists exactly once;
- the graph and source census are nonempty;
- the materialized source set has the exact derived count;
- no expected source is missing or unreadable;
- no extra source is present;
- each registry source URL and checksum equals the filtered lock;
- each git URL and rev equals metadata and its archive checksum equals the
  committed pin;
- metadata and filtered-lock identity sets are equal.

Only after all checks pass may policy tools run. A missing source cannot make a
license scan pass by scanning fewer packages.

The implementation reuses ADR 0052's pinned source materialization. It does
not create a second vendor authority.

## Package policy

Package deny consumes root-dev-inclusive policy metadata without
`--exclude-dev` and checks bans, licenses, and sources.

Package audit consumes the policy filtered lock and the pinned RustSec database
with `--no-fetch`.

- Broker ignore set is empty.
- Guest ignore set is exactly `RUSTSEC-2024-0384`.

The guest license update is exactly:

```text
bindgen        BSD-3-Clause
instant        BSD-3-Clause
inotify        ISC
inotify-sys    ISC
libloading     ISC
notify         CC0-1.0
```

These are package-scoped exceptions in
`packages/d2b-guest-shell-runner/deny.toml`. Adding BSD-3-Clause, ISC, or
CC0-1.0 to its global allowlist is forbidden. A planted different package with
each license must still fail.

Existing root union and `packages/Cargo.guest.lock` deny and audit checks remain
independent and enforcing.

## spec003w0 Cargo gate supply-chain inputs

The nested broker and guest locks are deleted by the same wave, so the Cargo
gate has no nested authority left. Its package supply-chain surfaces read the
native-system selected policy inputs instead:

```text
packages/policy-inputs/<native-system>/<native-gnu-target>/broker-production/policy/metadata.json
packages/policy-inputs/<native-system>/<native-gnu-target>/broker-production/policy/Cargo.lock
packages/policy-inputs/<native-system>/<native-musl-target>/guest-real-libshpool/production/closure.json
packages/policy-inputs/<native-system>/<native-musl-target>/guest-real-libshpool/production/Cargo.lock
packages/policy-inputs/<native-system>/<native-musl-target>/guest-real-libshpool/policy/metadata.json
packages/policy-inputs/<native-system>/<native-musl-target>/guest-real-libshpool/policy/Cargo.lock
```

The guest static dependency policy consumes only the production closure and
production filtered lock. Package deny consumes the dev-inclusive policy
metadata, and audit consumes the policy filtered lock with the pinned RustSec
database and `--no-fetch`. A `--no-fetch` audit
cannot fail transiently, so no retry wrapper surrounds it. The aggregate
`packages/Cargo.lock` and `packages/Cargo.guest.lock` deny and audit checks
stay independent and enforcing on both the Cargo gate and the Nix side; the two
deleted nested-lock inputs are removed from the aggregate flake audit and from
the guest-shell-runner static dependency policy.

## Pinned test inventory

`tests/tools/assert-pinned-tests.sh` selects packages from the one root lock
and backs up, restores, or otherwise mutates no lock file. It performs exactly
two listings from `packages/`:

```text
cargo nextest list --locked --workspace --message-format oneline
cargo nextest list --locked -p d2b-priv-broker --no-default-features \
  --features layer1-bootstrap,fake-backends --message-format oneline
```

The workspace listing keeps `d2b-contract-tests` and is a superset of the
tests each lane executes. The broker listing selects the package from the root
workspace and enables the union of the executed broker feature sets, so every
executed broker test remains guarded; a pinned broker entry the selection
cannot reach is a refusal resolved by correcting the selection, never by
deleting the pin.

Measured `cargo nextest list` accepts `--locked`, `--offline`,
`--frozen`, `--workspace`, `--no-default-features`, and package
selection. `--locked` makes listing non-mutating by construction: if
resolution would change the lock, the listing refuses instead of writing, and
the remediation is the contributor lock-regeneration command, never an
in-gate mutation. The snapshot-and-restore trap, its scratch path, and its
`EXIT` handler are deleted.

The five `tests/golden/pinned/*.txt` comment headers that describe the nested
broker workspace (`kernel-canaries.txt`,
`usbip-firewall-skeleton.txt`, `host-prepare-network.txt`,
`broker-socket-acl.txt`, and `broker-export-audit.txt`) are updated in the
same change to describe root-lock package selection. Comment-only edits never
change a pinned entry.

## Cargo supply-chain equivalence

Before spec003w1 merge and again before promotion, run the current Cargo
`cargo deny check` executor and the decomposed Bazel union over three contexts:

| Context | Cargo authority | Bazel comparison authority |
| --- | --- | --- |
| Main | Full product root workspace and lock. | Full product deny, audit, and yanked set. |
| Broker | Root workspace selected as broker with its exact target/default-feature/feature policy context. | Exact broker policy projection for deny, audit, and yanked. |
| Guest | Root workspace selected as guest real-libshpool with its exact musl policy context. | Exact guest policy projection for deny, audit, and yanked. |

For each context, record the current Cargo raw enforcing exit status and a
sorted normalized finding set. The finding key is
`(class, package, version, source, finding_id, detail)`, where `class` is one
of `ban`, `license`, `source`, `advisory`, or `yanked`; `detail` is a stable
policy token, never raw tool output. The decomposed runner emits the same
status convention as current `cargo deny check`: zero for no enforcing
finding and the measured Cargo policy-finding status for a nonempty union.
Operational errors remain distinct and are never coerced into equivalence.

Equality requires both the raw enforcing status and the complete normalized
set to match. Missing, extra, reclassified, ignored, or differently attributed
findings block spec003w1 and promotion. Mutations plant one finding in each class,
remove each union component in turn, swap broker and guest projections, alter
one status, and add one extra finding. Every mutation must fail the comparison.

## Post-merge yanked authority

There is exactly one committed lock-bounded snapshot:
`bazel/supply_chain/yanked-snapshot.json`. Its exact sorted
`(name, version)` key set is derived only from `packages/Cargo.lock`.
`tests/tools/no-bash-ast-walker/Cargo.lock` and
`packages/Cargo.guest.lock` are excluded from snapshot authority.

`rust-deny-main` evaluates the full product snapshot.
`rust-deny-broker` and `rust-deny-guest` evaluate exact projections of the
broker and guest root-dev-inclusive package-policy graph identities against
that same snapshot. A projection with a missing, extra, or wrong-context key
fails before deny runs.

The contributor-only commands remain separate:

```text
cargo xtask bazel-yanked-refresh
cargo xtask bazel-yanked-check
```

`bazel-yanked-refresh` is the reviewed networked updater and writes only the
snapshot. `bazel-yanked-check` is the offline, no-write exact-key validator,
does not construct the network client, and is the implementation all three
deny carriers call. Both are unreachable from Make and workflows. Unit tests
inject all-clear, yanked, missing-key, extra-key, missing-revision, malformed,
and transport cases; no unit test reaches the live index. The live-index plant
proves the offline check refuses a network source without resolving it.

## Dedicated Nix derivations

Broker source packaging:

- consumes the root packages source;
- removes the broker-local `sourceRoot`;
- uses `packages/Cargo.lock`;
- retains
  `cargoLock.outputHashes."wl-proxy-0.1.2" =
  "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8="`;
- selects package and binary `d2b-priv-broker`;
- disables default features.

Guest static packaging:

- consumes the root packages source;
- uses source root `d2b-rust-src/packages`;
- uses `packages/Cargo.lock`;
- retains
  `cargoLock.outputHashes."wl-proxy-0.1.2" =
  "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8="`;
- selects package and binary `d2b-guest-shell-runner`;
- disables default features and enables `real-libshpool`.

Both retain dedicated derivations and independently enforced binary-size and
closure evidence. Broker remains a host dynamic binary. Every guest artifact
must be ELF `ET_DYN` PIE, must report the `e_machine` expected by the native
Nix system (`EM_X86_64` for `x86_64-linux`, `EM_AARCH64` for
`aarch64-linux`), and must have no `PT_INTERP` and no `DT_NEEDED`. Nix-unit and
Rust contract tests assert the exact hash key and value in both derivations
and carry missing-key, wrong-value, one-derivation-only, non-PIE/`ET_EXEC`, and
wrong-machine mutations.

`tests/golden/bazel-rust-artifact-baselines.json` is the committed artifact
authority. It contains exactly four rows: broker and guest for each of
`x86_64-linux` and `aarch64-linux`. The integrator generates it only after
realizing the actual derivations. Each row records:

- the measured executable byte size;
- the exact ELF type and machine;
- for the broker, the exact interpreter basename and sorted `DT_NEEDED`
  SONAME set;
- for the guest, absent interpreter and empty `DT_NEEDED`;
- the recursive Nix closure count and SHA-256 computed transiently from the
  exact sorted path set, but no path;
- the exact selected package-policy graph digest; and
- the measurement command and candidate commit.

No fixed byte threshold is invented in prose. Size allowance exists only in
`sizeGrowthAuthorization`; there is no row-level allowance field. An unchanged
or smaller artifact has `sizeGrowthAuthorization = null`. A positive delta
passes only with this closed authorization object in the same change:

```text
system
artifact
priorBinaryBytes
newBinaryBytes
deltaBytes
rationalePath
candidateContentSha256
reviewRecordSha256
decision = "approved"
```

`priorBinaryBytes` must equal the row's `binaryBytes`; `newBinaryBytes` must
equal the realized artifact's measured bytes; `deltaBytes` must equal
`newBinaryBytes - priorBinaryBytes` and must be positive;
`rationalePath` is normalized and repository-relative; both digests bind the
candidate and review record; and the system/artifact pair must equal the row.
A later accepted baseline folds the authorized delta into `binaryBytes`,
then removes the authorization. A first native baseline is accepted only with
its realized artifact and a null authorization.

Positive fixtures cover unchanged size without authorization and exact growth
with a matching approved authorization. Negative fixtures cover missing
authorization, a denied decision, wrong system or artifact, stale candidate or
review digest, wrong prior baseline, wrong realized new bytes, absolute
rationale, arithmetic mismatch, replay against another row, and one byte
beyond the authorized allowance. Qualification references
all four baseline-row digests and every nonzero authorization digest and
refuses a row whose authorization was not part of the candidate review.
Closure-add/remove, cross-artifact, unrelated-sibling, changed broker
SONAME/interpreter, static-broker, dynamic-guest, non-PIE, and wrong-machine
mutations remain.

Artifact refusals use only `D2B-BZLARTIFACT-IDENTITY`,
`D2B-BZLARTIFACT-LINKAGE`, `D2B-BZLARTIFACT-CLOSURE`, or
`D2B-BZLARTIFACT-SIZE-AUTH`. A message names the repository-relative baseline
row and candidate/measurement SHA-256 only, followed by the exact rerun
`make test-flake`. It never contains `$!`, an absolute or Nix store path, an
exact closure member, raw tool output, a process identifier, or an opaque
handle. Table-driven tests cover every code and reject a borrowed remedy.

The native realized set therefore contains the four package-policy checks,
`broker-host-artifact-contract`, and the extended `guest-static-elf` check.
Those final two realize the actual dedicated derivations and validate linkage,
size, closure, and selected-policy binding. Qualification references all four
baseline rows, all four artifact realization results, every size-authorization
result, every mutation, and the same stable head.

The product lock is regenerated only by the contributor command
`cargo generate-lockfile --offline` from `packages/`. It is not a Make target
and no workflow may name it. Validation surrounds that command with clean-diff
assertions and fails if the committed candidate changes.

## Dual-system checks

For both `x86_64-linux` and `aarch64-linux`, the authoritative native
inventory contains exactly six checks:

```text
broker-production-dependency-policy
guest-shell-runner-static-dependency-policy
broker-production-package-policy
guest-real-libshpool-package-policy
broker-host-artifact-contract
guest-static-elf
```

Each check reads only its exact system-and-target policy path. Before graph
work, it checks embedded system, target, runner, and edge kinds.

Native x86 and native arm lanes realize their own checks. They set no foreign
`--system`, no `--builders`, and no remote builder. Aarch64 broker realization
does not claim runtime support.

## Workspace boundary

Local Bazel output, action cache, repository cache, and convenience links live
under `.scratch/bazel/`. Generated `.bazelignore` covers `.scratch/` and every
Cargo target directory, including the root product target, broker isolated
targets, guest isolated target, walker target, proof targets, and lab targets.

No stale package-local workspace path may appear in graph discovery, cache
workspace declarations, generated BUILD ownership, or cleanup logic.

## Workspace/module generation ownership

spec003w0 prep creates and registers green runner and locator skeleton manifests and
crate roots with complete future dependencies before their first tests. spec003w1 and
spec003w2 prep own their relevant crate-root `lib.rs` and xtask dependency and
contract seams without declaring not-yet-present implementation modules. spec003w5
prep does the same for cache and promotion. Scope tests load the scope-owned
module through a test-local path. After every dependent scope merges, only the
integrator wires completed modules into the prep-owned roots.

Lock refresh follows the authority that changed, and `MODULE.bazel.lock` is
always refreshed and committed last.

A product manifest change is exactly:

1. regenerate `packages/Cargo.lock`;
2. run product repin and commit `bazel/cargo/product.lock`;
3. run module refresh and commit `MODULE.bazel.lock`;
4. run each command again as a clean no-op;
5. prove `tests/tools/no-bash-ast-walker/Cargo.lock` and
   `bazel/cargo/walker.lock` byte-identical to their pre-change bytes.

A walker manifest or lock change is exactly:

1. regenerate `tests/tools/no-bash-ast-walker/Cargo.lock`;
2. run walker repin and commit `bazel/cargo/walker.lock`;
3. run module refresh and commit `MODULE.bazel.lock`;
4. run each command again as a clean no-op;
5. prove `packages/Cargo.lock` and `bazel/cargo/product.lock`
   byte-identical to their pre-change bytes.

Initial or combined setup is exactly: commit `bazel/cargo/product.lock`, then
`bazel/cargo/walker.lock`, then `MODULE.bazel.lock`, then rerun each
command as a clean no-op.

Byte-identity is proved by comparing recorded hashes of the untouched files
before and after the refresh, not by reading a diff summary.

`MODULE.bazel.lock`, both hub locks, Nix pins, generated BUILD files, and
coverage/query goldens are integrator-generated only. Slices may create scratch
previews but never commit those outputs.

The spec003w0 release workflow uses `packages/Cargo.toml`, `--locked`, explicit
package/bin/default-feature selectors, and copies every product binary from
`packages/target/release`. Its Rust cache declares `packages -> target` as the
only workspace mapping plus the explicit broker and guest gate target
directories. `tests/unit/gates/flake-check-matrix-sync.sh` and
`tests/unit/gates/ci-rust-cache-sync.sh` are updated to enforce the new shape;
neither is deleted or retired.

The broker release command and copy source are exact:

```text
cargo build --release --locked --manifest-path packages/Cargo.toml \
  --package d2b-priv-broker --bin d2b-priv-broker --no-default-features
cp packages/target/release/d2b-priv-broker dist/bin/
```

## Seeded refusals

Each applicable checker has one isolated plant for:

```text
missing-root
duplicate-root
empty-closure
wrong-system
wrong-runner
wrong-target
wrong-edge-kind
omitted-normal-edge
omitted-build-edge
omitted-root-dev-edge
omitted-dev-normal-edge
omitted-dev-build-edge
wrong-cfg
wrong-feature
cross-context-edge
unrelated-first-party-sibling
product-hub-containment
walker-hub-containment
product-lock-containment
walker-lock-containment
broker-x86_64-target-edge
guest-x86_64-target-edge
broker-aarch64-target-edge
guest-aarch64-target-edge
stale-bazel-output
source-missing
source-extra
source-unreadable
checksum-mismatch
source-identity-mismatch
metadata-lock-mismatch
forbidden-production-class
forbidden-license
forbidden-source
forbidden-ban
advisory
stale-policy-output
x86_64-foreign-system
x86_64-remote-builder
aarch64-foreign-system
aarch64-remote-builder
unrelated-member-feature-union
generic-context-includes-broker
generic-context-includes-guest
dedicated-context-loses-selector
guest-non-pie
guest-wrong-machine
supply-chain-status-difference
supply-chain-finding-missing
supply-chain-finding-extra
supply-chain-projection-swap
tree-format-unpinned
tree-identity-not-in-metadata
metadata-checksum-assumed
dev-edges-derived-by-post-filter
module-lock-refreshed-before-hub
untouched-hub-input-changed
pinned-inventory-lock-mutation
pinned-inventory-nested-lock-input
```

Each plant must fail at its own predicate and exact diagnostic. Reaching a
later predicate, reusing another case's diagnostic, or returning zero fails the
harness.
