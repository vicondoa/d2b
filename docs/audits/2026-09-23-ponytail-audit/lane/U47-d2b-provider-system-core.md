# U47 d2b-provider-system-core

net: 0 lines, 0 deps (Lean already. Ship.)

- Nothing new to cut. Head contains exactly six modules (error 1155, host 546, lib 84, ownership 41, testing ~23+fixtures, user - crate totals ~1,200 LOC); the 17-file / -1,624-line deletion (#P4) and the zone-side handler-status landing (#P5, kept with the live zone_status.rs → `SystemCoreStatusEmitter` caller at d2bd/zone_status.rs:149, constructed at d2bd/resources 3216/3616/4957) are at HEAD - honored, not re-flagged.

## Consistency notes
No cross-crate divergence surfaced for this crate: `error.rs` (1155) is the sole shared error taxonomy home, consumed workspace-wide by d2b-provider-toolkit + d2bd (verified by caller grep over packages/placement-8 provider crates, tests, nixos-modules, docs/reference/policy for each `SystemCoreError` variant; only the live categories have production callers). `host.rs`/`user.rs` adhered double - host.rs:39/389 + user.rs:52 emit-status doubles negotiated under ledger row #P5 partial, both remain live (SystemCoreHostStatusEmitter/SystemCoreUserStatusEmitter constructed at d2bd 3216/3616).

## Checked
Read all six src files (error.rs 1155, host.rs 546, lib.rs 84, ownership.rs 41, testing.rs + fixtures, user.rs). Workspace-wide caller search per candidate symbol (method: grep pattern rooted at packages;tests;nixos-modules;docs/reference/policy; hit-count check; cross-crate paths via `::{SYMBOL}`) - all remaining pub items live. No zero-caller item found. No stdlib duplication newly found. Ledger honored per U1 U47 rows.
