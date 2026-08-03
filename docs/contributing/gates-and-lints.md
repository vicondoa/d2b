# Gates and lints

Detailed reference for the heavy-lane semaphore and the policy lints whose
exemption rules are easy to get wrong. The binding summary, the Layer-1 job
list, and the enforcing/advisory rule live in
[`../../AGENTS.md`](../../AGENTS.md) under "Build and validate"; read that
first. This file explains the parts that need more than a rule.

`tests/layer1-jobs.json` remains authoritative for the job list and its
enforcement classification. Where this file disagrees with that manifest or
with the `Makefile`, those win.

## Build and validate, in detail

Use the top-level `Makefile` targets. The shell scripts under `tests/`
are implementation details unless a target or `tests/AGENTS.md` tells
you to run one directly.

`nix develop` gives you the toolchain every gate expects - the pinned Rust
release, plus sccache, cargo-nextest, cargo-deny, cargo-audit, shellcheck
and jq. The gate scripts each re-enter a nix shell and bootstrap a private
toolchain when those are missing, so working inside the dev shell skips
that setup. Normal dev/test profiles retain line tables for panic locations but
omit full dependency DWARF; use `cargo build --profile debugging` or
`cargo test --profile debugging` when a debugger needs full symbols.

Rust tests run under `cargo-nextest`. Two surfaces are not nextest surfaces
and get explicit companion runs, so do not "simplify" them away: **doctests**
(several `compile_fail` ones are capability seals) and **`harness = false`
binaries** (`d2b-core-smoke` carries real fail-closed minijail assertions).
The harness-free set is derived from `nextest list` rather than pinned. The
privileged broker workspace deliberately stays on `cargo test`: its tests
are not process-per-test safe, and it runs 528 tests in about 1.4 s.

`make test-runtime-ledger` also stays on `cargo test`, and that is load
bearing. It enforces an aggregate process-CPU budget, and nextest's
one-process-per-test model costs about 1.9x the CPU for the same census
(measured: 1.2 s against 2.3 s). Porting it would mean roughly doubling the
budget and losing that much sensitivity, for no speedup.

When a failure only reproduces inside the gate's own toolchain environment,
use `tests/tools/repro-rust-gate-env.sh <command>` rather than re-running
`make test-rust`.

```bash
# Focused Layer-1 jobs, in tests/layer1-jobs.json local phase order.
# Read each job's current enforcement classification from that manifest.
make check-tier0
make check-inventory
make test-lint
make test-changelog
make test-rust
make test-proofs
make test-flake
make test-nix-unit
make test-policy
make test-drift
make test-runtime-ledger
make test-performance-budgets
make test-fixture-contracts

# Post-preflight Layer-1 development umbrella. This runs the manifest jobs
# outside its preflight phase; `make check` also runs the preflight jobs.
make test-unit

# PR-equivalent Layer-1 gate. Uses tests/layer1-jobs.json to run
# the current enforcing and advisory jobs with bounded parallelism.
make check

# Legacy/full-static monolithic gate retained for explicit use.
make check-static

# Local Layer 1 + container integration. Still run the explicit
# host/manual pre-PR targets below before opening an agent-owned PR.
make test
```

`tests/layer1-jobs.json` is authoritative for both the job list and its
classification. A job is enforcing unless it carries `"enforcement":
"advisory"`; an advisory entry pairs that field with `advisoryReason` explaining
why its successful result is not enforcing evidence. Advisory means the
command is still launched and a nonzero result still fails the run, but a
guarded skip is permitted. Therefore an advisory result must not be cited as
validation evidence for a change.

The manifest currently classifies `check-tier0`, `check-inventory`,
`test-lint`, `test-changelog`, `test-rust`, `test-proofs`, `test-flake`,
`test-nix-unit`, `test-policy`, `test-drift`, `test-runtime-ledger`, and
`test-fixture-contracts` as enforcing. It classifies
`test-performance-budgets` as advisory. Always re-read the manifest rather than
assuming this split is fixed.

The performance canary prints `SKIP` and enforces no latency budget unless
`D2B_PERF_STABLE=1`. Promoting it requires a pinned self-hosted runner, setting
that variable on the runner, and then removing the advisory classification and
reason from the manifest. The project does not currently have such a runner.

The fixture-contract lane runs the fixture-dependent `d2b-contract-tests`
crate and the CLI-contract cases against `D2B_FIXTURES` materialized directly
from evaluated Nix artifact data. Both the local and continuous-integration
lanes set `D2B_ENABLE_FIXTURE_BUILD=1`, so it executes and enforces; invoking it
without that variable is a hard failure rather than a silent skip. The eval-only
lane does not realize NixOS systems or patched VMM binaries. The separately
pinned video binary command-surface contract remains the narrow realized check.
The default `test-rust` includes the fixture-dependent contract and CLI
surfaces once when Nix is available. The Layer-1 graph sets
`D2B_SKIP_FIXTURE_BUILD=1`, leaving those surfaces to the separate enforcing
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts` lane; selected
hermetic policy files may still have separate enforcing entrypoints such as
`test-policy`. The focused `test-rust-main` target retains the same
conditional fixture behavior.

### The API census shard

CI runs eight independent Rust leaf jobs behind the stable required
`test-rust` rollup context: API, main workspace, broker, guest shell runner,
no-bash AST, schema, inventory and supply chain. Each focused target receives
the full runner budget and drops local-only dependency edges, so a shard does
not repeat another shard's work. `make test-rust` remains the local aggregate.

The API census is a separate shard because it shares nothing with the
workspace build: it renders through the separately pinned nightly toolchain in
`packages/d2b-api-surface/rust-toolchain.toml` into its own target directory
under `.scratch/rust-test-cache/`, so it neither consumes nor produces
artifacts that fmt, clippy or nextest use. Its cost is rustdoc rendering rather
than dependency compilation, so it does not need a cache entry of its own; do
not give it one. `test-rust-main` remains the single rust-cache writer.

### Rust budget and execution manifest

The local Rust aggregate is the GNU Make DAG behind `make test-rust`. It uses
`--keep-going` and `--output-sync=target` and keeps broker feature passes
serial. Fixture/CLI work and the API snapshot checker use isolated stable
targets below `.scratch/rust-test-cache`, so they overlap the main workspace
without sharing mutable Cargo state. The public and private rustdoc censuses
also use separate stable targets and overlap only when the API leaf has at
least two admitted Cargo jobs; their split job shares never exceed that leaf's
quota. The snapshot checker runs from Cargo's release profile because its
measured long pole is CPU-bound JSON processing, not compilation. Budgets
through nine use one job per active lane; surplus jobs above
nine are assigned to the measured API long pole while the full nine-lane
frontier stays within the effective budget. Direct calls to
`tests/test-rust.sh` require one explicit leaf mode and are not aggregate
schedulers. A passing Rust manifest retains
the exact baseline sub-surface IDs documented in the execution-manifest
reference; `D2B_SKIP_FIXTURE_BUILD=1` intentionally omits only the conditional
fixture and CLI IDs.

The local warm aggregate keeps that parallel profile. When its normal Cargo
target is absent, it selects a cold profile that reuses the workspace target
for fixture/CLI work while retaining the warm-local split API census targets
across `make clean`. A four-lane bounded prebuild frontier overlaps API, main,
broker and light independent work. Fixture, inventory and schema then run as a
full-budget dependency chain, so inventory reuses every prior build before
schema generation. CI alone
uses the shared API census target and dispatches each Rust leaf as its own job.

`D2B_RUST_BUDGET` is the supported local Rust control. It must be a positive
integer when set and is only a requested upper bound. The automatic budget is
the smaller of logical CPUs and a memory-derived cap. The memory calculation
uses `MemAvailable` and the smaller remaining finite cgroup v2
`memory.max`/`memory.high` allowance after subtracting reclaimable
`inactive_file`, reserves 2 GiB for the host, and budgets 3 GiB per heavy job.
If visible cgroup v2 controller state is unreadable, the target warns and
fails closed to budget 1. Cargo `--jobs` and nextest `--test-threads` quotas
are assigned so every active frontier remains within the effective budget,
including budget 1. Top-level Make `-j` does not replace this control.

`D2B_EXECUTION_MANIFEST=<path>` opts the Rust aggregate into the versioned
execution evidence documented in
[`../reference/test-execution-manifest.md`](../reference/test-execution-manifest.md).
The parent is anchored before the persistent lock is opened, evidence
descriptors use close-on-exec, and the lock is a current-user mode-0600
nonblocking OFD lock. A fixed `manifest-lock-contended` result identifies the
execution-manifest lock without printing its path and directs the operator to
wait and retry. Adjacent mode-0700 fragments are same-filesystem and are
atomically renamed. The prior manifest is removed before dispatch. Handled
signals stop the dedicated process group with a fixed 10-second grace, then
kill and reap survivors before idempotent partial finalization.

### The realized flake check and its cache

A **realized** flake check (currently only `video-binary-contract`, listed in
`D2B_FLAKE_REALIZED_CHECKS` in `tests/tools/flake-check-classes.sh`) is built
rather than merely instantiated, so it compiles the patched VMM packages. In
CI that shard carries its must-build inputs between runs through
`tests/tools/realized-check-cache.sh`, which publishes only the outputs
`cache.nixos.org` does not already serve - two packages, about 30 MB - rather
than a whole-store cache. Keep it that size: the Actions cache is a hard
repository-wide budget, and this shard is affordable precisely because it is a
targeted entry. Publish only each input's **default** output; a package built
with separate debug info also declares a `debug` output that no `--help`
assertion can need, and carrying those took the same entry to 175 MiB. A
carried entry can never produce a wrong result, since store paths are
content-addressed and a changed derivation simply misses and builds, so the
import is deliberately best-effort and must never fail the shard. Measured on
the gate, a hit takes that shard from 1010 s to 33 s and builds neither
package.

Two properties of that script are load-bearing and were each learned the
expensive way, so do not "simplify" either. It resolves its paths with
`nix-store --query` and restores them **by name from a manifest the export
writes**, never with `nix derivation show`-plus-jq and never with
`nix copy --all`. This tree evaluates under Lix and CI installs upstream Nix;
both of those spellings work under Lix and fail under upstream Nix, and both
fail as a silent empty result rather than as an error. Each one cost a full
run whose only symptom was a lane that never got faster. `import` therefore
decides success by re-querying the store rather than by trusting the copy's
exit status, and the shard runs `realized-check-cache.sh self-test` - which
fails closed on a reintroduced `--all` - before the restore.

Do not resolve this by deleting the check. The `--backend` and
`--vhost-user-media` flags are separately pinned in
`nixos-modules/processes-json.nix` and in the golden argv under
`tests/golden/runner-shape/`, but those pin what d2b *emits*; the realized
check pins what the binary *accepts*, which is what catches an upstream bump
dropping a flag.

Before opening an agent-owned PR, run the host/manual integration
targets on the development host; do not rely on the PR pipeline for
them:

```bash
make test-integration       # Layer 2 container tests; needs podman
make test-host-integration  # runNixOSTest VM checks; NixOS + KVM host
```

`make test-host-integration` is x86_64-linux only and may fall back to
slow TCG if `/dev/kvm` is absent. Hardware and live-host tests remain
explicit manual tiers and require a host with the matching devices or
deployed d2b state.

`make test-runtime-ledger` is the hermetic execution-budget Layer-1 job
(also run by `make test-unit` / `make check` through
`tests/layer1-jobs.json`). After a warm build (so compilation is excluded
from measurement), it records per-test wall-clock p95s as advisory
diagnostics and enforces an aggregate process-CPU p95 budget for each pinned
crate. Process CPU excludes time descheduled behind unrelated machine load,
which is why it is the enforced timing basis. The closed census in
`tests/runtime-ledger-census.json` presently pins one crate and exactly 190
tests; a vanished or extra test, an incomplete or under-repeated run, or an
aggregate crate CPU p95 over budget fails the gate. A per-test diagnostic
threshold breach does not.

The gate holds no baseline and makes no historical-regression claim. When you
legitimately add, remove or rename a census test, regenerate the pin with
`make runtime-ledger-pin` and commit the result; the pin is a closed set, so
the gate fails until it matches. The `test-runtime-ledger check` output is
authoritative for the exact advisory-report formatting and selection.
Growing the census to a real multi-crate shard inventory (with a per-shard
budget) and adding a cross-machine reference baseline for a true
historical-regression gate is the named deferred follow-up
`runtime-ledger-full-census-and-real-shards`. If its shape here diverges from
the current `Makefile` target or `tests/layer1-jobs.json`, treat those as
authoritative and flag the drift for the integrator.

## Heavy lanes

Every Layer-2, host-integration, hardware, live, and perf-heavy command
runs through **one** semaphore, invoked from the repository root as `cargo
run --manifest-path packages/Cargo.toml -p xtask -- heavy-gate`. It grants
two slots per uid via open file description locks so concurrent heavy lanes
cannot oversubscribe the shared Nix store, cargo target directory, or KVM
device. Do not add a second lock file, sleep-and-retry loop, or per-crate
guard.

The slot namespace is fixed at `/run/d2b-heavy-gates/uid-<uid>/`. The root
and per-uid directory are root-owned and non-writable by unprivileged users;
the two `slot-*` files are pre-created for the target uid at mode `0600`.
There is no runtime-directory or temporary-directory fallback. The NixOS
module provisions the root with systemd-tmpfiles, then activation provisions
directories and slots for configured lifecycle users that NSS can resolve.
An unavailable network-backed user is deferred rather than failing
activation; after that user logs in, run `make heavy-gate-provision`. Use
the same target on a host that does not consume the module. Because `/run`
is a tmpfs, run it once per boot when the gate requests it. An absent or
malformed namespace is an environment error with that provisioning
remediation, never permission to create a weaker pool. In particular,
`/run/user/<uid>` is rejected because its owner can rename slot names or
their parent and create an independent pool.

The structure is public-lane-plus-guarded-internal:

- **Public lane targets** (`make test-integration`,
  `make test-host-integration`, `make test-hardware`, `make perf`) acquire
  a slot and then delegate to a guarded internal `heavy-lane-*` target.
  Run these.
- **Internal `heavy-lane-*` targets** hold the raw work and fail closed
  through `heavy-lane-guard` if invoked outside the gate (the gate exports
  `D2B_HEAVY_GATE` across its re-exec). Do not run them directly.
- **Convenience wrappers** `make heavy-check`, `make heavy-cargo-test`,
  `make heavy-flake-check`, and the `heavy-test-*` aliases run a Layer-1
  gate, the Rust suite, the building flake check, or a public lane under
  the same semaphore.

Run a heavy lane through its public target (or, for an arbitrary command,
`cargo run --manifest-path packages/Cargo.toml -p xtask -- heavy-gate --
<command>`) whenever another heavy lane might be running; the bare internal
targets stay available only for a serial console. Live-host and hardware
tests obey the same rule: use the gated live-VM smoke entrypoints (`make
pre-tag` for the full gate, `make smoke-lite` for the lite gate) or wrap a
raw live script as `cargo run --manifest-path packages/Cargo.toml -p xtask
-- heavy-gate -- env D2B_LIVE=1 bash tests/integration/live/<name>.sh`.

The `cargo run --manifest-path packages/Cargo.toml` form is deliberate:
there is no root cargo workspace, so the bare `cargo xtask` alias resolves
only when the working directory is `packages/`, and running it from the
repository root fails with `no such command: xtask`. Because cargo config
discovery is cwd-based, invoking `xtask` from the root via `--manifest-path`
silently drops the `sccache` configuration in `packages/.cargo/config.toml`;
that is immaterial for the gate itself. When it matters for a specific
command, `cd packages && cargo xtask <command>` is the equivalent form -
pick one per command and pass file arguments relative to the directory you
run from.

Invoking a live script directly is safe but not the documented path: each
one verifies the inherited slot and re-executes itself through the semaphore
exactly once when no genuine slot is held. A bare `D2B_HEAVY_GATE` value is
not trusted, so it cannot bypass the sole-use invariant.
**A new live, hardware, or performance entrypoint must carry that same
self-guard block**, or the fail-closed inventory guard
(`every_live_and_heavy_entrypoint_routes_through_the_gate`) rejects it.

## Spec-literal lint allowlist

The ADR 0046 spec-literal lints (`policy_adr046_spec_literals.rs`) enforce
three frozen decisions across `docs/specs/**`: D103 (the single 24-byte
`YYYY-MM-DDTHH:MM:SS.sssZ` datetime spelling), D104 (the single
`.d2bus.org.` ResourceType qualifier infix), and D108 (the integer
`retryAfterMs` retry-delay scalar superseding the old `retryAfter`
duration string). The allowlist is a pinned exact exemption, not an
author-suppressible marker: an inline `d2b-lint-allow` comment is
explicitly **not** honored and will not exempt a line - the lint rejects
that escape hatch by design, because a per-line marker would let any
future author silently suppress a real violation. The **only** exemption
is the decision-register table row that *defines* the rule (the `| <code> |`
row in `docs/specs/ADR-046-decision-register.md`), and that exemption is
pinned to that one file. Everywhere else, including a rejection
illustration, must be phrased so it does not embed the exact rejected
literal; correct the example rather than trying to silence the lint.

The same policy test checks the seven canonical feasibility measurements
against every Markdown and JSON document under `docs/**` plus `CHANGELOG.md`.
It inventories class-specific measurement signatures globally, including
run and group-commit denominators, the ChangeBatch comparison count, the
crash-boundary count phrase, RSS values with units, and each p95/p99 value
with its unit. Registered sites additionally pin their exact measurement or
qualitative outcome summary. The global scan deliberately does not match bare
numbers such as `13`, `20`, or `48`, because those are common in unrelated
prose. Consequently, a new copy that preserves a canonical number-and-unit,
denominator, or class phrase is rejected even in an unregistered document; a
free paraphrase that omits every inventoried signature remains a review
concern rather than something this lint claims to detect.

## Envelope policy lint (D116) negative-example marker

Unlike the spec-literal lints above - which honor no author-suppression
marker at all - the envelope policy lint (`policy_adr046_envelopes`)
recognizes exactly one deliberately narrow exemption. That lint enforces
D116 across `docs/specs/**`: a `Host` or `Guest` whose `allowedDomains`
admits the `user` domain must name a non-null, non-empty `defaultUserRef`
(D116 is frozen in `docs/specs/ADR-046-decision-register.md`). A block that
simply omits it is a real violation and must be corrected.

The one exception is an **intentional negative example**: a fenced example
(typically a Nix block) authored to *teach* the rule by demonstrating the
eval-time failure that omitting `defaultUserRef` produces. Deleting that
counter-example would lose correct teaching content, so the lint preserves
it - but only under three exact conditions it enforces together, not the
looser "names both `d2b-lint` and `d116`" shape earlier drafts of this
section described:

- **One exact, case-sensitive marker.** A comment line **inside the fence**
  whose text, after its `#` or `//` prefix is stripped, equals the marker
  string exactly. The current spelling is `# d2b-lint: expect-d116-eval-error`;
  the match is a whole-string, case-sensitive comparison, so a paraphrase or a
  comment that merely mentions the `d2b-lint` and `d116` tokens does not
  qualify.
- **One pinned file.** The marker is honoured only in the single documenting
  file the lint pins (currently `docs/specs/ADR-046-nix-configuration.md`).
  The same comment anywhere else exempts nothing and fails closed.
- **Exactly once.** The marker must appear a single time in that file. A
  second copy makes the exemption fail closed for the whole file, so every
  D116 block there is flagged again.

This is an unambiguous authoring signal for one intentional-rejection
example, never a general suppression switch. Never reach for it to silence a
D116 failure on a shape that is meant to be valid - correct the shape
instead. `policy_adr046_envelopes` is the authority for the exact spelling,
the pinned file, and the single-occurrence scope; a concurrent hardening may
tighten them further, so if you are adding a legitimate negative example take
the current requirement from that lint, not from this paragraph.

For where tests live, when to add or retire each kind of test, and
which pins/ledgers to update, read [`tests/AGENTS.md`](../../tests/AGENTS.md).
[`tests/README.md`](../../tests/README.md) is the human quick-start for the
same test model.
