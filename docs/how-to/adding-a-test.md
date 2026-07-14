# How to add a test

d2b tests are invoked through **`make` targets** backed by the checked
Layer-1 manifest. The delivery rule is:

> Run focused preflight, open the PR, and then run the final CI, validator, and
> panel lanes concurrently against one immutable delivery snapshot. A change is
> not mergeable until every manifest-required result and the tree-bound seal
> pass. Reviewers inspect the test and external status; they do not execute
> tests.

## Decision tree — which kind of test?

Pick the row that matches what you are asserting. The group sets the `make`
target and where the test lives.

| If you are asserting… | group | `make` target | lives in |
| --- | --- | --- | --- |
| Rust logic / argv / DTO behaviour, or a **fake-backed** kernel/broker canary, or KVM-free runtime integration (sockets, `unshare` netns) | **A** | `test-rust` | `#[test]` in the owning crate (`cargo nextest`) |
| Generated artifact == its **shipped, committed** copy (schema / docs / CLI / manpage) | **B** | `test-drift` | `xtask gen-* && git diff` (canonical) + `insta` |
| A property of a **rendered bundle artifact** (privileges/host/processes/minijail JSON) | **C** | `test-contract` | `packages/d2b-contract-tests` — parse the fixture into a `d2b-core` DTO + assert |
| A **pure-Nix value / option / internal-config** fact | **D** | `test-nix-unit` | `nix-unit` over an introspection fixture |
| That a **misconfig is rejected** at eval | **E** | `test-nix-unit` | `nix-unit` (Bucket-A value over `config.assertions`; Bucket-B `expectedError`) |
| That a config **builds** / a schema is strict | **F** | `test-flake` | `flake.checks` (realized via `nix build`) |
| A **source/doc cross-reference** or structural-policy invariant | **H** | `test-policy` | the existing Rust policy scanner or closed meta-gate set |
| Foreign-userland portability for static binaries | **G-container** | `test-integration` | `tests/integration/containers/*.sh` under rootless podman; final validator lane after the PR opens, not GitHub CI |
| Real-kernel runtime behaviour with **no physical device** (broker sockets, cgroups, pidfd, store, network, audit, ACL, swtpm) | **G-host** | `test-host-integration` | `tests/host-integration/*.nix` runNixOSTest VM checks; final local NixOS/KVM validator lane after the PR opens, not GitHub CI |
| Real **device passthrough** (GPU/YubiKey/hardware-TPM) or a **full microVM boot** | **G-hw** | `test-hardware` | a NixOS host **with the devices** — **not runnable in CI** |

### Group F hosted-runner caveat

The PR workflow discovers hosted-runner `test-flake` shards with
`make test-flake-list`, not by blindly sharding every
`flake.checks.x86_64-linux.*` key. The full static check set remains pinned by
`tests/golden/flake-check-matrix/x86_64-linux.txt`; the hosted matrix may
intentionally filter checks that are too large or unstable for GitHub-hosted
runners.

Today `fixture-smoke-full` is one such check. It is the feature-rich contract
fixture used by local `make test-unit` / contract-test validation, but the
nested NixOS graph can make hosted-runner Nix evaluators segfault before they
produce a typed Nix error. Keep it in `flake.checks` and the pin, validate it
locally with `make test-unit` (or directly with
`D2B_FLAKE_CHECK=fixture-smoke-full make test-flake` on a sufficiently large
host), and only add it back to the hosted dynamic matrix after the evaluator
failure mode is gone.

Default when unsure: if it can be expressed as an assertion over a rendered
artifact, it is **C** (Rust contract test). Ad-hoc bash that shells out to
`nix eval` / `cargo test` is **rejected** by the placement gate — use a target.

## Fast inner loop (one assertion)

```bash
# Contract (C) — reuse the smoke fixture, run one test:
make test-fixtures                       # builds the fixture, prints D2B_FIXTURES
D2B_FIXTURES=<that path> cargo nextest run -p d2b-contract-tests -E 'test(my_new_case)'

# Rust logic (A): cargo nextest run -p <crate> -E 'test(my_new_case)'
# Nix value (D/E): add a case to the nix-unit suite and run `make test-nix-unit`
```

No ledger/mutation ceremony is required for a new Rust, nix-unit, contract, or
flake test; that machinery is migration-scoped. Do not add a standalone
`tests/*.sh` gate. Follow the closed taxonomy in
[`tests/AGENTS.md`](../../tests/AGENTS.md), keep `make check-inventory` green,
and edit `tests/layer1-jobs.json` only when Layer-1 job membership changes.

Validate and regenerate that graph through Rust `xtask`:

```bash
cd packages
cargo xtask layer1 validate
cargo xtask layer1 workflow write
cargo xtask layer1 workflow check
```

## Open the PR before final gates

The PR template checklist is mandatory. Use this order:

1. Commit the candidate and run the smallest focused test that can reject an
   obviously broken change.
2. Open or update the PR and stack, then create the immutable delivery snapshot
   for that exact open PR state.
3. Run `make check` in the required CI or validator lane. Run applicable
   `make test-integration`, `make test-host-integration`, and
   `make test-hardware` commands in the final validator lane after the PR opens.
   GitHub CI, validators, and the panel proceed concurrently.
4. Import validator and panel attestations into external delivery state and
   seal the candidate before merge.

Run `cd packages && cargo xtask delivery wave help` for the canonical
machine-readable delivery command and option index.

Summarize pass/fail/pending status in the PR and link external status when
useful. Never paste raw command output, evidence payloads, panel records,
seals, or run/model provenance into the PR or repository. New/changed tests
must remain in the closed taxonomy; generated workflow and docs changes land
in lockstep.

See [`AGENTS.md` → "Build & validate"](../../AGENTS.md) for the full target
table and [`tests/AGENTS.md`](../../tests/AGENTS.md) for the placement rules.
