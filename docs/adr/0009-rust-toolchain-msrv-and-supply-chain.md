# 0009. Rust toolchain, MSRV, and supply-chain policy

- Status: Accepted
- Date: 2026-05-25
- Wave: W0a
- Plan slice: "Bootstrap a minimal Rust workspace under `packages/`: workspace `packages/Cargo.toml`, `packages/rust-toolchain.toml`, and `packages/.cargo/config.toml` pinned to a published MSRV documented in ADR 0009."
- Companion ADRs: [ADR 0000](0000-repository-layout-and-rust-bootstrap.md)

## Context

D2b is moving from a NixOS-only bash and systemd orchestration model
toward a Rust control plane with a CLI, daemon, and eventually a
privileged broker. That control plane will become load-bearing for
lifecycle, authorization, manifests, and host reconciliation, so W0a
must establish reproducible Rust builds before privileged behavior
exists.

The v0.4.0 baseline remains the compatibility target. W0a Rust binaries
are only version stubs, but the same workspace, lint, toolchain, and
supply-chain rules will carry into later waves that add sockets,
privilege boundaries, and daemon-owned state.

A stable minimum supported Rust version (MSRV) is required so Nix,
local developer shells, CI, and review output all agree on the compiler
used for formatting, linting, and tests. Pinning the toolchain in
`packages/rust-toolchain.toml` also prevents accidental drift caused by
host-local rustup defaults.

Supply-chain gates need to be credible from the first Rust commit.
`cargo-deny`, RustSec auditing, a committed `Cargo.lock`, and Nix builds
that vendor from that lock give W0a a reproducible baseline even if the
RustSec advisory database integration has to start as an explicit W2
TODO stub.

## Decision

1. The Rust toolchain is pinned through `packages/rust-toolchain.toml`
   to the stable channel currently shipped by the pinned nixpkgs input
   (1.94.1 as of W0fu2).
2. The W0a MSRV equals the pinned channel; MSRV bumps require a new ADR.
3. The cargo-deny configuration lives at `packages/deny.toml`, and the
   cargo-audit RustSec advisory database is Nix-pinned through a flake
   input; if W0a must stub this because nixpkgs lacks integration, the
   stub references this ADR and is an explicit TODO scheduled for W2.
4. Workspace lints set `unsafe_code = "forbid"` at workspace scope, and
   new per-crate exceptions land only with a follow-up ADR.
5. `tests/static.sh` runs `cargo fmt --check`,
   `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`, `cargo deny check`, and `cargo audit` or
   the ADR-referenced stub; all must exit 0 to pass the gate.
6. Rust packages are exposed only through
   `checks.<system>.rust-{build,tests,clippy,deny,audit}` until the Rust
   CLI matches the v0.4.0 surface.
7. Reproducible Nix builds vendor from `packages/Cargo.lock` with a
   fixed `cargoLock` hash, and no in-Nix advisory-database fetch occurs
   outside the pinned flake input.

## Consequences

1. Positive: Local, CI, and Nix Rust checks all target the same compiler
   channel and workspace manifest.
2. Positive: `Cargo.lock`, cargo-deny, and cargo-audit become required
   parts of the W0a static gate rather than later hardening work.
3. Positive: The repository can cross-evaluate Rust checks on supported
   systems without exposing unfinished binaries as user-facing packages.
4. Negative: MSRV changes now require ADR process overhead even for
   routine compiler upgrades.
5. Neutral: Drift between a developer's local RustSec advisory database
   and the Nix-pinned advisory database is acknowledged and must be
   revisited in W2.

## Alternatives considered

- Track rustup `stable`: rejected because CI and local builds would
  drift without an auditable MSRV.
- Let each crate choose lints independently: rejected because unsafe-code
  policy must be uniform until a reviewed exception exists.
- Skip cargo-audit until daemon work starts: rejected because supply-chain
  posture should be present before privileged Rust code is added.
- Fetch the advisory database ad hoc during Nix builds: rejected because
  it breaks reproducibility and bypasses the pinned flake-input policy.

## References

- plan.md, "W0a: Project structure review and Rust bootstrap"
- plan.md, "Test gate for W0a"
- plan.md, "Cargo workspace"
- AGENTS.md, "Build & validate"
- AGENTS.md, "CI / `flake.checks`"
- [ADR 0000](0000-repository-layout-and-rust-bootstrap.md)
