# How to add a test

nixling tests are invoked through **`make` targets** (one per test type). The
single rule:

> **`make check` is the done-gate.** A change is not finished until `make check`
> passes. CI runs the same targets. New tests must be classified in
> `tests/migration-ledger.toml` (`make check-inventory` fails closed otherwise).

## Decision tree — which kind of test?

Pick the row that matches what you are asserting. The group sets the `make`
target and where the test lives.

| If you are asserting… | group | `make` target | lives in |
| --- | --- | --- | --- |
| Rust logic / argv / DTO behaviour, or a **fake-backed** kernel/broker canary, or KVM-free runtime integration (sockets, `unshare` netns) | **A** | `test-rust` | `#[test]` in the owning crate (`cargo nextest`) |
| Generated artifact == its **shipped, committed** copy (schema / docs / CLI / manpage) | **B** | `test-drift` | `xtask gen-* && git diff` (canonical) + `insta` |
| A property of a **rendered bundle artifact** (privileges/host/processes/minijail JSON) | **C** | `test-contract` | `packages/nixling-contract-tests` — parse the fixture into a `nixling-core` DTO + assert |
| A **pure-Nix value / option / internal-config** fact | **D** | `test-nix-unit` | `nix-unit` over an introspection fixture |
| That a **misconfig is rejected** at eval | **E** | `test-nix-unit` | `nix-unit` (Bucket-A value over `config.assertions`; Bucket-B `expectedError`) |
| That a config **builds** / a schema is strict | **F** | `test-flake` | `flake.checks` (realized via `nix build`) |
| A **source/doc cross-reference** or structural-policy invariant | **H** | `test-policy` | the policy scanner / a focused gate |
| Real-kernel runtime behaviour with **no physical device** (broker sockets, cgroups, pidfd, store, network, audit, ACL, swtpm) | **G-ci** | `test-integration` | `runNixOSTest` VM (runs in CI on a KVM job + local NixOS) |
| Real **device passthrough** (GPU/YubiKey/hardware-TPM) or a **full microVM boot** | **G-hw** | `test-hardware` | a NixOS host **with the devices** — **not runnable in CI** |

Default when unsure: if it can be expressed as an assertion over a rendered
artifact, it is **C** (Rust contract test). Ad-hoc bash that shells out to
`nix eval` / `cargo test` is **rejected** by the placement gate — use a target.

## Fast inner loop (one assertion)

```bash
# Contract (C) — reuse the smoke fixture, run one test:
make test-fixtures                       # builds the fixture, prints NL_FIXTURES
NL_FIXTURES=<that path> cargo nextest run -p nixling-contract-tests -E 'test(my_new_case)'

# Rust logic (A): cargo nextest run -p <crate> -E 'test(my_new_case)'
# Nix value (D/E): add a case to the nix-unit suite and run `make test-nix-unit`
```

No ledger/mutation ceremony is required for a *new* test — that machinery is
migration-scoped. You only must: (1) put the test behind a `make` target, and
(2) keep `make check-inventory` green (add a ledger row if you add a script).

## Before you open a PR

The PR template checklist is mandatory:

- `make check` passes (CI also runs `make check-ci`).
- If you touched GPU/YubiKey/hardware-TPM or a full microVM boot, run
  `make test-hardware` on a NixOS host **with the devices** and paste results
  (CI cannot — hosted runners have KVM but no devices).
- New/changed tests are wired into a `make` target and `make check-inventory`
  is green.
- Docs (`docs/**`, `AGENTS.md`, `tests/README.md`) and `.github/workflows/*`
  updated in lockstep.

See [`AGENTS.md` → "Build & validate"](../../AGENTS.md) for the full target
table and the three CI tiers.
