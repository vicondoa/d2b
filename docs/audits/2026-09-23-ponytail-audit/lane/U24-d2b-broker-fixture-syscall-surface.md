# U24 d2b-broker-fixture-syscall-surface
Lean already. Ship. (55 LOC fixture, one source file `src/lib.rs`; nothing to cut and nothing eligible: every element is a live scan target of the broker dependency-surface audit.)

## Checked
Read the entire crate: `src/lib.rs` (55 lines, the only source; no `tests/`, no `nix/`, no `build.rs`, no `benches/`), `Cargo.toml`, `BUILD.bazel`. Cross-referenced every code element against the real consumer — `packages/d2b-broker-composition`'s dependency-surface audit (`src/dependency_surface.rs`, `src/seam.rs`), which both unit-chain it and assert on it:

- `raw_syscall_close` x86_64 `asm!` body → `raw-asm` source probe, asserted non-empty in `seam.rs:631-641` (`the_syscall_surface_fixture_crate_fails_the_source_surface_probe`).
- `FIXTURE_MARKER` `#[used]` + `#[unsafe(link_section = ".d2b_fixture_marker")]` → fires both `used-static` and `link-section-entry-point` probes from one static; both asserted in `seam.rs:636-641`.
- `install_isolation_silencer` (`std::panic::set_hook`) → `panic-hook-registration` probe, asserted in `seam.rs:636-641`.
- `libc` normal dependency → `forbidden_dependencies` containing `"libc"`, asserted in `dependency_surface.rs:469-478` (`the_syscall_surface_fixture_crate_fails_the_full_audit`) and refused at registration in `seam.rs:630-646`.
- non-x86_64 `libc::close` fallback arm → keeps the fixture compiling on non-x86_64 scan hosts (audit probes sources on the build host regardless of target); the `#[cfg]` split is intentional and documented in-crate.

Workspace-wide reference search: `grep d2b-broker-fixture-syscall-surface` (and `_`-spelled form) across `**/*.rs`, `BUILD.bazel`, `Cargo.toml`/`Cargo.lock`/`Cargo.guest.lock`, `nixos-modules/`, `docs/`, `flake.nix`, `closure.json`/`metadata.json`. Every hit is (a) the composition audit using it as a live scan target, or (b) mechanical packaging/policy membership (workspace `Cargo.toml:74`, root `BUILD.bazel:370`, `flake.nix:81`, lockfile/closure/metadata entries). No production caller — by deliberate design: dev-dependency of the composition root only, never linked into the broker binary.

Not a types-layer crate (U2–U11), so no consistency-notes section. U24 ledger has no prior findings; nothing gained reopens. Nothing cut, no deps removed: `net: -0 lines, -0 deps`.
