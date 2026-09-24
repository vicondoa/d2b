# U92 d2b-host-activation-helper

net: -434 lines, -0 deps

- `delete:` `nixos-modules/host-activation-helper/` - a dead twin source tree
  (same crate name `d2b-host-activation-helper`, same single-verb
  `chgrp-by-numeric-gid` binary, 409-line `src/main.rs` + 9-line Cargo.toml +
  16-line Cargo.lock). **Not** a cargo workspace member (root `Cargo.toml`
  `[workspace].members` names only `packages/d2b-host-activation-helper`), no
  `BUILD.bazel`/Bazel target of any kind, not staged by the Makefile stage
  list / rust-host-tools.nix / bazel-host-tools / rust-host-tools benchmark
  inventories, and not referenced by any caller in the workspace. The live
  crate is `packages/d2b-host-activation-helper` (structurally-identical
  purpose, staged and built). The twin also carries a **duplicate crate-name
  collision** - `packages/rust-host-tools.nix:121,249` and
  `nixos-modules/rust-host-tools.nix` name only the `packages/` workspace
  member, so any future name-keyed tooling (bazel binary-name resolution,
  cargo workspace-member walks, xtask crate-layout census) would resolve the
  name ambiguously across the two trees. Deleting the twin removes the
  collision at the source, alongside the duplicate LOC. [nixos-modules/host-activation-helper/] (leaf)
- `delete:` Similarly the crate's **dead sibling twin tree**
  `nixos-modules/host-activation-helper/src/` is the same deletion - see the
  row above; a single path covers both. (consolidated into finding 1)
- `yagni:` `--no-follow-symlinks` CLI flag accepted-and-ignored in the live
  crate's arg parse (`packages/d2b-host-activation-helper/src/main.rs:62`
  `"--no-follow-symlinks" => {}`) and printed in the usage banner
  (`main.rs:20` `[--no-follow-symlinks]`). The walk unconditionally never
  follows symlinks (entries are stat'ed with `AT_SYMLINK_NOFOLLOW` and
  skipped when `is_symlink`), and no call site in the workspace passes the
  flag. Drop the parse arm and the banner token; documented-but-inert CLI
  surface. [packages/d2b-host-activation-helper/src/main.rs:62] (leaf)

## Consistency notes

N/A - host-side binary crate, not a types-layer crate (U2-U11).

## Reopened refusals

None. U92 had no prior refusal-ledger rows (fresh audit of this crate).

## Checked

Read `packages/d2b-host-activation-helper/src/main.rs` in full (435 lines:
`Config`/parse_args with `--root`/`--legacy-gids`/`--target-gid`/
`--skip-while-lock-held`/`--fail-closed`/`--no-follow-symlinks` arms, fd-walk
via `walk_dir` using `AT_SYMLINK_NOFOLLOW` fstatat + `is_symlink` skip +
`openat(O_NOFOLLOW)` recursion, `scan_for_leftovers` fail-closed postscan,
`main`; test module with fail-closed/held-lock/TestDir+tempdir coverage) plus
the twin tree `nixos-modules/host-activation-helper/` in full (409-line
main.rs + Cargo.toml + Cargo.lock). Caller/reference verification
(workspace-wide, all text kinds incl. `.nix`, `.bazel`, `.bzl`, `BUILD*`,
Makefile, `*.md`, Cargo.lock, `.gitignore`-respecting ripgrep): the literal
path `nixos-modules/host-activation-helper` appears in **zero** tracked or
ignored files outside the twin tree itself; the packages tree appears only in
the live workspace-member/staging surfaces. `[workspace].members` in root
Cargo.toml lists `packages/d2b-host-activation-helper` only. Runtime flag
caller census: `--no-follow-symlinks` is passed by no caller in the workspace
(retained-guard check on the live parse arm is verified: walk always no-follows
independent of the flag). LOC measured, not estimated; deps unmoved (twin is
not a workspace member, so 0 workspace deps change; the live crate's libc +
tempfile stays).

## U2 execution (2026-09-24)

- re-verified census at HEAD: literal path `nixos-modules/host-activation-helper` appears in zero tracked files outside the twin tree (only .git/index internals + this plan/lane docs); root Cargo.toml/Makefile/rust-host-tools.nix reference only `packages/d2b-host-activation-helper`.
- applied: `nixos-modules/host-activation-helper/` tree deleted wholesale (Cargo.toml + Cargo.lock + src/) - covers findings 1+2 (consolidated).
- applied: dropped `--no-follow-symlinks` parse arm + usage-banner token from the live crate; no caller in workspace passes the flag (re-verified).
- tests: cargo test -p d2b-host-activation-helper 2 passed.
