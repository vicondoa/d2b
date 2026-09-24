# U93 d2b-unsafe-local-helper

net: -35 lines, -1 dep

- delete: `ScopeRuntime::with_paths` + `with_paths_and_executable` (18 lines) — zero callers in crate or workspace; only `with_paths_executable_and_proxy` is used (by `new`). [packages/d2b-unsafe-local-helper/src/runtime.rs:285-302] (leaf)
- delete: `ManagerEnvironment::state_home` (8 lines) — zero callers in crate or workspace. [packages/d2b-unsafe-local-helper/src/environment.rs:95-102] (leaf)
- shrink: `hex` byte-identical copy in runtime.rs:579 and systemd.rs:379 (11 lines) — one shared `pub(crate) fn hex`; both call sites stay. [packages/d2b-unsafe-local-helper/src/runtime.rs:579] (leaf)
- delete: unused `d2b-core` dependency — declared in Cargo.toml:24 and BUILD.bazel:24, zero `d2b_core::` references in src. [packages/d2b-unsafe-local-helper/Cargo.toml] (leaf)

## Checked

Read all src files (lib 4, main 50, environment 282, protocol 523, systemd 458, runtime 1958), Cargo.toml, BUILD.bazel; no tests/ or integration/ dirs; crate is a standalone helper binary consumed only by nixos-modules/rust-host-tools.nix (hostPackages + mkMainPackage) and xtask BUILD file list — no Rust workspace consumer. Caller verification method: workspace-wide `grep -rn` over packages/ + nixos-modules/ + labs/ + docs for every flagged symbol (with_paths, with_paths_and_executable, state_home, d2b_core), plus per-variant census of RuntimeError/ProtocolError/ScopeError/EnvironmentError (all variants constructed or matched), per-method usage scan of ManagerEnvironment/ScopeRuntime/HelperClient/SystemdUserScopeManager, and dependency usage counts per Cargo.toml entry (clap, d2b-contracts, d2b-contracts-control, d2b-core, getrandom, nix, rustix, serde, serde_json, sha2, socket2, uzers, zbus — all others referenced in src). No zero-caller claim left unverified.

## Honored rows (U1 ledger, verbatim; no re-flag)

- "no prior findings" — no ledger rows exist for U93; nothing to reopen or re-flag.

## Reopened refusals

None.