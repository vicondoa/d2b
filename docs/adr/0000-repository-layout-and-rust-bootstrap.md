# 0000. Repository layout and Rust bootstrap

- Status: Accepted
- Date: 2026-05-25
- Wave: W0a
- Plan slice: "Decide where Rust crates live: confirmed at `packages/` per `docs/reference/cli-contract.md`, with sibling-flake compatibility preserved."
- Companion ADRs: [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md)

## Context

The v0.4.0 baseline is a NixOS-host framework organized around
`nixos-modules/`, `pkgs/`, `examples/`, `templates/`, `tests/`,
`docs/`, and the root `flake.nix`. Those paths are already consumed by
NixOS users and sibling flakes, so W0a must add Rust structure without
moving or renaming the existing public surface.

The existing bash `d2b` CLI and systemd-backed lifecycle remain the
only user-facing implementation before W0a. The portability plan needs a
Rust control-plane bootstrap, but the W0a binaries are only stubs that
prove layout, packaging, and gates; they must not create sockets,
request authorization, or write `/run/d2b` artifacts.

The plan also calls out `docs/reference/cli-contract.md` as the stable
contract a future Rust CLI must match. Keeping Rust crates under a
single `packages/` workspace lets Nix packaging, CI, and local commands
refer to one manifest while preserving the repository's existing
Diataxis documentation and Nix module layout.

Architecture decisions are currently spread across plan text and
reference docs. W0a creates `docs/adr/` so later waves can cite stable,
accepted decisions instead of relying on one large portability plan.

## Decision

1. Rust crates live under `packages/`, matching the future CLI location
   referenced by `docs/reference/cli-contract.md`.
2. The Cargo workspace manifest is `packages/Cargo.toml`,
   `rust-toolchain.toml` lives at `packages/rust-toolchain.toml`, and
   every cargo invocation in CI and docs uses
   `--manifest-path packages/Cargo.toml` or `(cd packages && cargo ...)`.
3. W0a workspace members are `d2b-core`, `d2b-contracts`, `xtask`,
   `d2b`, and `d2bd`.
4. The workspace sets `unsafe_code = "forbid"`; per-crate exceptions are
   allowed only through future ADRs.
5. The W0a `d2b` and `d2bd` binaries are version-stub binaries
   with no sockets, no authorization, and no `/run/d2b` artifacts.
6. The v0.4.0 bash CLI remains the only user-facing d2b entry point,
   and the flake exposes Rust crates only as
   `checks.<system>.rust-*` until W2.
7. `docs/adr/` is the repository home for architecture decision records.

## Consequences

1. Positive: Later waves can expand the Rust control plane without
   moving crates or changing the cargo invocation shape.
2. Positive: Existing consumers keep using the v0.4.0 bash CLI and Nix
   module surface while Rust work appears only in flake checks.
3. Positive: Stub binaries give CI a concrete Rust build target without
   implying daemon, socket, or privilege behavior.
4. Negative: Contributors must remember to run cargo through
   `packages/Cargo.toml`; invoking cargo at the repository root is not a
   supported W0a workflow.
5. Neutral: The AGENTS.md repo-layout table needs an integrator-owned
   `packages/` row, and the existing `docs/` row needs to mention
   `docs/adr/` alongside the Diataxis tree.

## Alternatives considered

- Put Rust crates at the repository root: rejected because it would mix
  cargo metadata with the existing Nix flake surface and confuse root
  cargo invocations.
- Put Rust crates under `src/`: rejected because the plan and CLI
  contract already reserve `packages/` for this workspace.
- Expose Rust binaries as public flake packages immediately: rejected
  because W0a stubs do not yet match the v0.4.0 CLI surface.
- Delay ADR creation until W0b: rejected because W0a layout and
  toolchain decisions are prerequisites for later work.

## References

- plan.md, "W0a: Project structure review and Rust bootstrap"
- plan.md, "Cargo workspace"
- plan.md, "Per-wave parallel scopes"
- AGENTS.md, "Repo layout"
- AGENTS.md, "Worktrees for parallel agents"
- [docs/reference/cli-contract.md](../reference/cli-contract.md)
- [docs/explanation/design.md](../explanation/design.md)
