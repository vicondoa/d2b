# How to add a test

d2b Layer-1 tests are invoked through fixed **Bazel targets** exposed by
public `make` aliases (one per test type). The
single rule:

> **Focused evidence is the review requirement.** Run the smallest relevant
> target and test, and explain deliberate omissions. `make check` is an
> available aggregate, not a prerequisite; integration, host, and hardware
> lanes are conditional on the changed surface. New tests must be wired into
> the owning crate or Nix-surface Bazel target.

## Decision tree - which kind of test?

Pick the row that matches what you are asserting. The group sets the `make`
target and where the test lives.

| If you are asserting… | group | `make` target | lives in |
| --- | --- | --- | --- |
| Rust logic / argv / DTO behaviour, or a **fake-backed** kernel/broker canary, or KVM-free runtime integration (sockets, `unshare` netns) | **A** | `test-rust` | `#[test]`, integration test, or doctest in the owning crate |
| Generated artifact == its **shipped, committed** copy (schema / docs / CLI / manpage) | **B** | `test-drift` | `bazel run //packages/xtask:xtask -- gen-*` plus the existing drift target |
| A property of a **rendered bundle artifact** (privileges/host/processes/minijail JSON) | **C** | `test-fixture-contracts` | the consuming crate or provider's fixture-backed integration test |
| A **pure-Nix value / option / internal-config** fact | **D** | `test-nix-unit` | `nix-unit` over an introspection fixture |
| That a **misconfig is rejected** at eval | **E** | `test-nix-unit` | `nix-unit` (Bucket-A value over `config.assertions`; Bucket-B `expectedError`) |
| That a config **builds** / a schema is strict | **F** | `test-flake` | `flake.checks` (realized via `nix build`) |
| One of the four global policy classes: source hygiene, workspace/lock integrity, supply chain, or changelog | **H** | `test-policy` | the corresponding narrow global policy target |
| Foreign-userland portability for static binaries | **G-container** | `test-integration` | `tests/integration/containers/*.sh` under rootless podman; local host/manual pre-PR, not the PR pipeline |
| Real-kernel runtime behaviour with **no physical device** (broker sockets, cgroups, pidfd, store, network, audit, ACL, swtpm) | **G-host** | `test-host-integration` | `tests/host-integration/*.nix` runNixOSTest VM checks; local NixOS/KVM host/manual pre-PR, not the PR pipeline |

### Group F resource caveat

Nix evaluation and realization stay in fixed Bazel targets with declared
inputs and explicit local-only tags. Keep resource-heavy or fixture-producing
checks in their existing fixed suite; do not add dynamic discovery or a new
workflow matrix.

Default when unsure: push the assertion into the owning crate or Nix surface.
Ad-hoc bash that shells out to `nix eval` or Cargo is rejected - use a declared
Bazel target.

## Fast inner loop (one assertion)

```bash
# Rust logic and compile-fail documentation (A):
bazel test //packages/<crate>:unit //packages/<crate>:doctest

# Binary or fixture-backed integration (A/C):
bazel test //packages/<crate>:integration

# Nix value (D/E): add a case to the nix-unit suite and run `make test-nix-unit`
```

Do not add migration ledgers, successor pins, or evidence shell scripts.

## Before you open a PR

Record focused evidence for the changed test and explain deliberate omissions.
Broader lanes are conditional:

- Run the smallest relevant `make` target and the test itself.
- Run `make test-integration` for container behavior and
  `make test-host-integration` for NixOS, daemon, or host behavior.
- Physical-device validation is manual operator work; do not add an evidence
  script for it.
- New/changed tests are wired into a `make` target.
- Docs (`docs/**`, `AGENTS.md`, `tests/README.md`) and `.github/workflows/*`
  updated in lockstep.

See [`AGENTS.md` → "Build & validate"](../../AGENTS.md) for the full target
table and [`tests/AGENTS.md`](../../tests/AGENTS.md) for the placement rules.
