# How to add a test

d2b Layer-1 tests are invoked through fixed **Bazel targets** exposed by
public `make` aliases (one per test type). The
single rule:

> **Focused evidence is the review requirement.** Run the smallest relevant
> target and test, and explain deliberate omissions. `make check` is an
> available aggregate, not a prerequisite; integration, host, and hardware
> lanes are conditional on the changed surface. New tests must be wired into a
> `make` target and classified in `tests/migration-ledger.toml` when the test
> model requires it (`make check-inventory` fails closed otherwise).

## Decision tree - which kind of test?

Pick the row that matches what you are asserting. The group sets the `make`
target and where the test lives.

| If you are asserting… | group | `make` target | lives in |
| --- | --- | --- | --- |
| Rust logic / argv / DTO behaviour, or a **fake-backed** kernel/broker canary, or KVM-free runtime integration (sockets, `unshare` netns) | **A** | `test-rust` | `#[test]` in the owning crate (`cargo nextest`) |
| Generated artifact == its **shipped, committed** copy (schema / docs / CLI / manpage) | **B** | `test-drift` | `xtask gen-* && git diff` (canonical) + `insta` |
| A property of a **rendered bundle artifact** (privileges/host/processes/minijail JSON) | **C** | `test-contract` | `packages/d2b-contract-tests` - parse the fixture into a `d2b-core` DTO + assert |
| A **pure-Nix value / option / internal-config** fact | **D** | `test-nix-unit` | `nix-unit` over an introspection fixture |
| That a **misconfig is rejected** at eval | **E** | `test-nix-unit` | `nix-unit` (Bucket-A value over `config.assertions`; Bucket-B `expectedError`) |
| That a config **builds** / a schema is strict | **F** | `test-flake` | `flake.checks` (realized via `nix build`) |
| A **source/doc cross-reference** or structural-policy invariant | **H** | `test-policy` | the policy scanner / a focused gate |
| Foreign-userland portability for static binaries | **G-container** | `test-integration` | `tests/integration/containers/*.sh` under rootless podman; local host/manual pre-PR, not the PR pipeline |
| Real-kernel runtime behaviour with **no physical device** (broker sockets, cgroups, pidfd, store, network, audit, ACL, swtpm) | **G-host** | `test-host-integration` | `tests/host-integration/*.nix` runNixOSTest VM checks; local NixOS/KVM host/manual pre-PR, not the PR pipeline |
| Real **device passthrough** (GPU/YubiKey/hardware-TPM) or a **full microVM boot** | **G-hw** | `test-hardware` | a NixOS host **with the devices** - **not runnable in CI** |

### Group F resource caveat

Nix evaluation and realization stay in fixed Bazel targets with declared
inputs and explicit local-only tags. Keep resource-heavy or fixture-producing
checks in their existing fixed suite; do not add dynamic discovery or a new
workflow matrix.

Default when unsure: if it can be expressed as an assertion over a rendered
artifact, it is **C** (Rust contract test). Ad-hoc bash that shells out to
`nix eval` / `cargo test` is **rejected** by the placement gate - use a target.

## Fast inner loop (one assertion)

```bash
# Contract (C) - build the enforcing fixture lane, then run one test:
make test-fixture-contracts
D2B_FIXTURES=<that path> cargo nextest run -p d2b-contract-tests -E 'test(my_new_case)'

# Rust logic (A): cargo nextest run -p <crate> -E 'test(my_new_case)'
# Nix value (D/E): add a case to the nix-unit suite and run `make test-nix-unit`
```

No ledger/mutation ceremony is required for a *new* test - that machinery is
migration-scoped. You only must: (1) put the test behind a `make` target, and
(2) keep `make check-inventory` green (add a ledger row if you add a script).

## Before you open a PR

Record focused evidence for the changed test and explain deliberate omissions.
Broader lanes are conditional:

- Run the smallest relevant `make` target and the test itself.
- Run `make check-inventory` when adding or retiring a test script.
- Run `make test-integration` for container behavior and
  `make test-host-integration` for NixOS, daemon, or host behavior.
- If you touched GPU/YubiKey/hardware-TPM or a full microVM boot, run
  `make test-hardware` on a NixOS host **with the devices** and paste results
  (CI cannot - hosted runners have KVM but no devices).
- New/changed tests are wired into a `make` target.
- Docs (`docs/**`, `AGENTS.md`, `tests/README.md`) and `.github/workflows/*`
  updated in lockstep.

See [`AGENTS.md` → "Build & validate"](../../AGENTS.md) for the full target
table and [`tests/AGENTS.md`](../../tests/AGENTS.md) for the placement rules.
