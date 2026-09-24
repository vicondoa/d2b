# U79 d2b-provider-zone-link

net: -0 lines, -0 deps

Ship. Read-only. All nine src files verified workspace-wide caller-by-caller (driver.rs, error.rs, guest.rs, host.rs, lib.rs, metadata.rs, registrar.rs, zone_links.rs, testing.rs + lib.rs re-exports + tests/ + nix/): every pub item has at least one live Rust caller in packages;tests;nixos-modules;docs/reference/policy at HEAD, and the sole zero-production-path surfaces are the two committed wire/plane seals (foundation boundary, admission gates, wire vocabulary) that prior ledger rows already keep — no new evidence reopens them.

## Honored rows (U1 ledger, verbatim; no re-flag)

- #PR15 [refused] stays refused — systemic name-substituted duplications (zone-URI/plane vocabulary) need cross-crate shared vocabulary reachable by this crate (owned elsewhere, dossier-ledger #PR15); no new in-crate duplication found at HEAD. Not re-flagged.
- #P1/#P2 [applied] 10 name-substituted copies of one metadata driver + registration test — shared declaration-only driver live via d2b-resource-runtime/src/metadata.rs:535 + shared registration assertions; zone-link's driver.rs is 20 lines (crate now holds error/guest/host/lib/zone_links/testing + one 20-line Driver). Verified present at HEAD, not re-flagged.
- #P9 [refused] integration/*.rs + README scaffolds in ten metadata crates — crate-layout policy requires an integration/*.rs for every crate not on the README-only ratchet (xtask/src/provider_crate_policy.rs). Not re-flagged.
- #P10 [#P9-adjacent ledger, not re-flagged] zone-side copy of the shared declaration-only metadata driver kept; caller identity re-export module stays (live consumers: d2b-resource-types foundation_seed -> d2bd seed path).

## Newly-checked, no-find

ZoneLinkZoneLinkSession state machine + ZoneLinkSessionRootToken (zone_links.rs) — all constructed/consumed live (d2bd composition wiring, plane zero, broker zone-link handlers), zero dead candidates. driver.rs/error.rs/metadata.rs read + caller-searched: no zero-caller pub. `zone_links.rs` 3,787 LOC: ZoneLinkRecord/ZoneLinkCursor/ZoneLinkController/ZoneLinkRouteBinding/route admission chain all live with production callers. No hand-rolled stdlib duplication found (route digests/uid renderers all single-sourced to shared d2b-resource-types).

## Ledger
No refusals reopened; no new-evidence refusals; all prior rows honored. Crate lean at HEAD.
