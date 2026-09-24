# U12 d2b-bus

net: -4,214 lines, -1 dep (island 7 mods = 4,198; four duplicate-path re-export shims = 16; retires the crate's ONLY `d2b_audit` import - the workspace's only consumer of the `d2b_audit` crate at HEAD. `d2b_telemetry` STAYS (live metrics.rs:10/593 - NoopBusTelemetry constructed at router.rs:1250/1280/4898 via NoopBusTelemetry from crate::metrics; telemetry has live producers); `d2b_contracts-zone-session`/`d2b-session` stay - canonical live session path (session/mod.rs:89-148) re-imports them)

## Delete: the relay island - 7 modules, 4,198 lines, a closed island

- `relay.rs` (832), `zone_route.rs` (1,495), `service_router.rs` (120), `audit.rs` (300), `routing.rs` (266), `transport/{mod,credit,unix}.rs` (14 + 528 + 643 = 1,185). Total 4,198.

**Evidence - zero-caller, workspace-wide, every symbol path checked:**
- `crate::census` across the six live core files (`router.rs`, `streams.rs`, `registry.rs`, `operations.rs`, `wire.rs`, `session/mod.rs`): each imports NONE of the seven island modules by any path (`crate::relay`, `crate::zone_route`, `crate::service_router`, `crate::audit`, `crate::routing`, `crate::transport` = 0 refs in every live file - full grep on each).
- Island-internal wiring: the island's only reach-in is `crate::operations::OperationId` (zone_route.rs:39) and its only reach-out is nothing (relay.rs:1-11 + zone_route.rs:1-11 both claim to BE the canonical transmitter, duplicating session/mod.rs:25's canonical claim - three module-level claims of canonical ownership for the same wire contract = exact consistency-class flag). Island modules link only to each other: `service_router→{audit,routing}`, `relay→zone_route`.
- Workspace-wide (packages/, nixos-modules/, tests/, BUILD.bazel, *.bzl, docs/reference/): zero refs to `d2b_bus::(relay|zone_route|service_router|audit|routing|transport)::` by any path, in any crate, any config, any doc, any BUILD target.
- The six live core files use `d2b_session_unix` items directly (router.rs:42 `use d2b_session_unix::{PeerCredentials, VerifiedUnixPeer}` re-imports the transport surface directly - NOT through crate::transport) and `d2b_contracts_zone_session::v3::zone_session` via the canonical session path. So the canonical session/wire surface is unchanged; the island is not on any documented canonical path.

**Per-finding test handling:** every island file carries a `#[cfg(test)]` body (relay: 16, zone_route: 28, service_router: 2, audit: 3, routing: 2). These tests only pin island-internal behavior; they are deleted WITH the island (project test-deletion rule: remove tests that only pinned the removed production). No test is kept as an "isolated harness," consistent with the plan's per-finding "test-only half deleted with the production half."

## Delete: four duplicate-path re-export shims (16 lines)

- `lifecycle.rs` (6), `error.rs` (4), `engine.rs` (3), `driver.rs` (3). Each is a pure `pub use d2b_session::{…}` assembly whose every item is already canonical at `d2b_bus::session::{…}` (session/mod.rs re-exports the whole family: `SessionLifecycle`, `SessionPhase`, `KeepaliveAction`, `SessionEngine`, `SessionEvent`, `ComponentSessionDriver`/`SessionDriverHandle`, `SessionError*`, `Result`, `TransportError`). Zero workspace callers under the four top-level paths `d2b_bus::(lifecycle|error|engine|driver)::`, and zero in-crate `crate::(lifecycle|error|engine|driver)::` refs outside the files themselves. Deleting them with their four `pub mod` arms in lib.rs retires four duplicate-path entry points a caller could otherwise grow a second FSM through.

## Consistency notes

- **Canonical session home = `session/`.** The live, caller-backed surface is `d2b_bus::session::{…}` - the only path with production callers (router.rs, session/mod.rs). The four top-level shims and relay.rs/zone_route.rs each claim in their module doc to be the single canonical path for the same item families (lifecycle:6, error:4, engine:3, driver:3 each say "canonical session path"; relay.rs:1-11 and zone_route.rs:1-11 both claim the canonical transmitter duty). Three-plus module-level claims of one canonical owner for the same session/wire surface is the exact class the plan's consistency feed flags: duplicate canonical-home claims. The canonical home that stays is `session/`; everything else is a duplicate-path claim over the dead island.
- `streamlets.rs`/`streamlets/` - the scout census tallied a `streamlets.rs` at 791 lines as an island member, but **no such file or directory exists on disk** (verified: `find` across `packages/d2b-bus/src` returns nothing; `wc` errors ENOENT). It is a census artifact - not counted as removable, not refused, not part of the net. Noted for the scout's own refusal ledger.
- All seven island files are `pub mod` (compile + have tests), so they are NOT "unused-code-let-me-delete" without the caller census - the census above IS that evidence. The island is a real, closed, zero-caller cluster, removable as one family.

## Checked

- `find`/`ls`/`wc` across `packages/d2b-bus/src` (crate = 7/8 modules island).
- Workspace-wide symbol census with grep across `packages/`, `nixos-modules/`, `tests/`, `docs/reference/`, `BUILD.bazel`, `*.bzl`, `*.nix`: zero callers of `d2b_bus::(relay|zone_route|service_router|audit|routing|transport|lifecycle|error|engine|driver)::` by any path (probe + per-name grep).
- In-crate: `crate::<island>::` grep against the six live core files - all zero.
- Verified the four shims' items are canonical at `d2b_bus::session::{…}` (session/mod.rs re-export block) and that lib.rs declares `pub mod` for each island module but publishes zero of their items on the documented surface.
