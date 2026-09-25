# d2b-provider-zone-link - d2b-provider-zone-link
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 3805 (excl. src/generated/**) | modules: whole crate (lib.rs, driver.rs, zone_links.rs, zonelink.rs; tests/registration.rs)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: none (single-part lane)

## idiom
- d2b-provider-zone-link#1 sev=medium blast=family effort=S verdict=actionable - the frozen cryptoperiod defaults `BOOTSTRAP_PSK_TTL_MS_DEFAULT` (300_000) and `KK_SESSION_MAX_LIFETIME_MS_DEFAULT` (86_400_000) are defined identically in two crates with no shared home, so a drift silently desynchronizes the child-local handler from the bus-side enrollment machine - fix: move both constants to `d2b_contracts_zone_session` (the crate both `d2b-provider-zone-link` and `d2b-bus` already depend on) and re-export from both sites; this is not the refused ZoneLink enrollment-machine merge, only the two constants - [packages/d2b-provider-zone-link/src/zone_links.rs:60, packages/d2b-provider-zone-link/src/zone_links.rs:63, packages/d2b-bus/src/session/enrollment.rs:42, packages/d2b-bus/src/session/enrollment.rs:49]
  evidence: census: `BOOTSTRAP_PSK_TTL_MS_DEFAULT|KK_SESSION_MAX_LIFETIME_MS_DEFAULT` over packages/nixos-modules/tests/docs/reference/labs = 2 defining sites with identical values (zone_links.rs:60,63 and d2b-bus/src/session/enrollment.rs:42,49); the refused class in docs/explanation/over-engineering-audit-record.md is the enrollment-machine merge, not these constants
- clean: seeds ran 2/2/1; the two `impl Default` hits (zone_links.rs:305,338) deliberately preserve frozen nonzero defaults a field-wise derive would break; the `Vec::new()` accumulation (zone_links.rs:1422) is the match-arms planner pattern; the two `for .. in 0..` loops (zone_links.rs:2012,2828) are test drivers

## own
- d2b-provider-zone-link#2 sev=low blast=leaf effort=S verdict=actionable - `plan()` clones `record.route_binding` in the `RoutePolicyCommitted` and `SessionGenerationAdvanced` arms only to mutate it and store it back, where a `route_binding.as_mut()` borrow would work (no other borrow of the record is live in either arm) - fix: replace `let Some(mut binding) = record.route_binding.clone() else ...` with `let Some(binding) = record.route_binding.as_mut() else ...` and mutate through the borrow in both arms - [packages/d2b-provider-zone-link/src/zone_links.rs:1692, packages/d2b-provider-zone-link/src/zone_links.rs:1706]
  evidence: seed `\.clone\(\)` = 50 hits; the 1663/1682 clones are required (binding moves into `ZoneLinkRouteAdmissionContext`, operation id is re-inserted after comparison), the 1418 record clone is the deliberate copy-on-write pass design, the 815-820 clones feed an owned wire struct, 1269 feeds an owned status projection; only 1692/1706 are avoidable
- d2b-provider-zone-link#3 sev=low blast=leaf effort=S verdict=actionable - `plan()` clones `record.enrollment` in the `EnrolledSessionEstablished` arm solely to compare the key fingerprint before mutating disjoint record fields - fix: take `record.enrollment.as_ref()`, compare `enrollment.key_fingerprint() != &peer_key_fingerprint` (fingerprint tokens are Copy), and let the borrow end before the `record.link_epoch += 1` mutation - [packages/d2b-provider-zone-link/src/zone_links.rs:1526]
  evidence: seed `\.clone\(\)` = 50 hits; the arm mutates only `link_epoch`/`connected`/`child_authorized`/`reconnect_attempts`/`advertised_routes`, all disjoint from `enrollment`, so the clone buys nothing
- clean: seeds ran 50/3/0; the three `to_*` hits and the remaining clones are test fixtures (zonelink.rs:350,367,401,429-523; zone_links.rs:1948-2135); no `Rc`/`RefCell`/`Arc<Mutex>`/`Cow` anywhere; the `AtomicU64` owner-token static is a process-wide counter with no single owner

## type
- clean: seeds ran 1/0/0; the single `fn validate_` hit (`validate_commit_proof`, zone_links.rs:1402) is an internal invariant check on an opaque token with no public constructor, not input validation a parsed type could replace; no boolean-flag soup or stringly-typed state (the record's `disabled`/`connected`/`child_authorized` booleans mirror pinned spec fields - schema-mirroring false positive); the commit-before-effect protocol is already typestate-enforced via `ZoneLinkPass` (no Clone/Copy) and `ZoneLinkCommitProof` (no public constructor)

## api
- d2b-provider-zone-link#4 sev=low blast=leaf effort=S verdict=actionable - `ZoneLinkMetricSample` and `ZONE_LINK_METRIC_LABEL_KEYS` are exported pub (and re-exported through `zonelink`) but have zero consumers outside the crate, so the metric vocabulary is promised surface nobody wires - fix: either consume them from `d2bd`'s metrics path or reduce to `pub(crate)` until a consumer exists - [packages/d2b-provider-zone-link/src/zone_links.rs:1783, packages/d2b-provider-zone-link/src/zone_links.rs:89, packages/d2b-provider-zone-link/src/zonelink.rs:13]
  evidence: census: `ZoneLinkMetricSample|ZONE_LINK_METRIC_LABEL_KEYS` over packages/nixos-modules/tests/docs/reference/labs = 0 hits outside the crate (only in-crate tests at zone_links.rs:3092-3107 and the re-export)
- d2b-provider-zone-link#5 sev=low blast=leaf effort=S verdict=actionable - `transport_error_is_quarantine` is a `pub const fn` with zero callers anywhere, including in-crate tests, so it is dead exported surface - fix: make it private or delete it until the quarantine mapping is actually consumed - [packages/d2b-provider-zone-link/src/zonelink.rs:281]
  evidence: census: `transport_error_is_quarantine` over packages/nixos-modules/tests/docs/reference/labs = 0 hits outside its definition
- d2b-provider-zone-link#6 sev=low blast=leaf effort=S verdict=actionable - `ZoneLinkCursorAuthority` is `pub` but is only reached through `ZoneLinkController` in the same module and the module's own tests, so its publicity is wider than its use - fix: reduce to `pub(crate)` - [packages/d2b-provider-zone-link/src/zonelink.rs:178]
  evidence: census: `ZoneLinkCursorAuthority` over packages/nixos-modules/tests/docs/reference/labs = 0 hits outside the crate; `d2bd/src/composition.rs:770,891` consumes `ZoneLinkController` only
- clean: seeds ran 133/0/5; no `Arc`/`Rc`/`Box`/`RefCell` in any public signature; the lib.rs `pub use ...::*` arms are the house single-surface pattern; the `pub(crate)` + `#[cfg(test)]` accessors on `ZoneLinkRouteAdmissionContext` and `ZoneLinkHandler::route_admission_context` are correctly scoped; the crate root surface is consumed by `d2bd/src/composition.rs` (86-896, 1118-1172) and `d2bd/src/resource_plane_v3.rs:153`

## err
- clean: seeds ran 115/0/0/2; all 115 `unwrap`/`expect` hits sit in `#[cfg(test)] mod tests` helpers (zone_links.rs:1817-1945, zonelink.rs:350-527) - the card's test-code false positive; zero panic macros, zero swallowed `Result`s, zero production unwraps; the two error enums (`ZoneLinkError` 26 variants, `ZoneLinkAdoptionError` 4 variants) are closed, Copy, and carry stable kebab-case `label()` tokens asserted bounded by `every_error_label_is_a_bounded_lowercase_token` (zone_links.rs:3131); the cross-crate label reuse via `ZoneRouteFailClosedReason` (zone_links.rs:224,232) avoids duplicated wire tokens

## serde
- clean: seeds ran 1/1/0/2; the only serde surface is the private durable envelope `ZoneLinkRouteAdmissionDedupWire` (zone_links.rs:697-704) with `rename_all = "camelCase"` + `deny_unknown_fields`, a version field checked on recovery, canonical-bytes enforcement (zone_links.rs:822-826, 845-847), and identity binding; round-trip, version-mismatch, and identity-mismatch paths are covered by `multiple_committed_route_ids_survive_versioned_restart_recovery` (zone_links.rs:2036) and `aborted_route_ids_are_reusable_but_recreated_identity_is_isolated` (zone_links.rs:2097); no hand-written `Deserialize`, no `flatten`, no untagged

## obs
- N/A: seeds 0/0/0/0 all zero; the crate has no `tracing`/`log` dependency (Cargo.toml deps: d2b-contracts-resource, d2b-resource-types, serde, d2b-contracts-zone-session, serde_json; tokio is dev-only), and the module is a pure planner with no telemetry surface

## docs
- d2b-provider-zone-link#7 sev=low blast=leaf effort=M verdict=actionable - 21 public `Result`-returning items document their failure modes only in prose, with zero `# Errors` sections, so the error contract (which `ZoneLinkError` variant fires) is not in the canonical place a caller reads - fix: add `# Errors` sections naming the `ZoneLinkError`/`ZoneLinkAdoptionError` variants to the public `Result` items, starting with `ZoneLinkLimits::new`, `ZoneLinkHandler::{begin,commit,release_effects,issue_route_admission}`, `ZoneLinkRecord::{with_route_binding,encode_route_admission_dedup,with_route_admission_dedup}`, `ZoneLinkOwnerProof::{new,from_digest}`, `ZoneLinkCursorAuthority::{adopt,cursor}` - [packages/d2b-provider-zone-link/src/zone_links.rs:267, packages/d2b-provider-zone-link/src/zone_links.rs:1300, packages/d2b-provider-zone-link/src/zonelink.rs:197]
  evidence: seeds ran 131/0/21 (131 public items, 0 canonical sections, 21 `-> Result<`); every public item carries a first-sentence doc comment, module docs exist in all four files, and the redaction `Debug` impls are deliberate (tested at zone_links.rs:3115)
- clean: seeds ran 131/0/21; no undocumented public item found; the `ZoneLinkKeyPolicy` "locked six-field schema" doc (zone_links.rs:335-337) is accurate - the ZoneLink spec has exactly six fields (childZoneName, disabled, limits, transportCredentials, transportProviderRef, transportSettings per docs/reference/schemas/v3/core.d2b.org_ZoneLink.schema.json)

## perf
- clean: seeds ran 3/1/2; all six hits are test-only (`format!` at zone_links.rs:1963,3119 and zonelink.rs:350; `to_string` at zone_links.rs:1977,3168 and zonelink.rs:350; `Vec::new` at zone_links.rs:1422); production has no `format!`/`to_string` and the only allocation in the reconcile path is the deliberate copy-on-write record clone in `plan()` (cold per-event path); `static (unmeasured)` - no benchmark exists for this crate

## conc
- clean: seeds ran 0/0/3/0; the three hits are the `AtomicU64` owner-token generator (zone_links.rs:35,48,52) using `Ordering::Relaxed` on a counter nobody synchronizes on - the weakest correct ordering per the skill; no `Mutex`/`RwLock`, no threads, no `thread_local!`, no manual `Send`/`Sync` impls

## async
- N/A: seeds 0/0/0/0 all zero over src/; the crate is a synchronous planner - no `async fn`, no `tokio::spawn`, no `tokio::sync` in src; tokio appears only as a dev-dependency for the single `#[tokio::test]` registration shim (tests/registration.rs:11), which is the test lens's territory

## unsafe
- N/A: seeds 0/0/0/0 all zero; no `unsafe` blocks/fns/impls, no `transmute`/`from_raw`/`MaybeUninit`, no `// SAFETY:` sites; the manifest sets `[lints.rust] unsafe_code = "forbid"` (Cargo.toml:7), and the crate is not on the (d) 8 exception list

## ffi
- N/A: seeds 0/0/0/0 all zero; no `extern "C"`, no `no_mangle`, no `repr(C)`/`repr(transparent)`, no `CStr`/`CString` - the crate crosses no foreign boundary

## macro
- N/A: seeds 0/0/0/0 all zero; no `macro_rules!`, no proc-macro/syn/quote, no `$crate`, no `to_compile_error` - the crate defines no macros

## test
- d2b-provider-zone-link#8 sev=low blast=leaf effort=S verdict=actionable - three table-driven loops assert without a per-case failure message, so a failing row reports only a line number, not which state/error/key failed - fix: add messages naming the loop variable (`"state: {state:?}"`, `"key: {key}"`, `"error: {error:?}"`) to the loops at zone_links.rs:2513-2519, 3092-3094, and 3133-3167 - [packages/d2b-provider-zone-link/src/zone_links.rs:2519, packages/d2b-provider-zone-link/src/zone_links.rs:3093, packages/d2b-provider-zone-link/src/zone_links.rs:3162]
  evidence: seed `assert_eq!\(|assert_ne!\(|assert!\(` = 166 hits; the three loops are the only table-driven asserts without messages (the metric-label loops at 3093/3109 do carry messages)
- clean: seeds ran 43/166/0/0 (209 hits); 42 `#[test]` + 1 `#[tokio::test]` (tests/registration.rs:11, the policy-required registration shim - documented pattern, not flagged); assertions target error variants and durable state, not `Display` strings (the only `to_string` assertions pin the label contract at zone_links.rs:3168); the suite is deterministic (explicit `now_ms`, no clock/network), covers restart replay, capacity ceilings, redaction, and identity isolation; no `#[ignore]`, no proptest/insta/rstest - none needed for this state machine; no test found that cannot fail

## Coverage
- idiom: 1 finding(s)
- own: 2 finding(s)
- type: clean (seeds ran: 1/0/0)
- api: 3 finding(s)
- err: clean (seeds ran: 115/0/0/2)
- serde: clean (seeds ran: 1/1/0/2)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency in Cargo.toml)
- docs: 1 finding(s)
- perf: clean (seeds ran: 3/1/2)
- conc: clean (seeds ran: 0/0/3/0)
- async: N/A (seeds: 0/0/0/0 all zero; no async fn or tokio in src; tokio is dev-dep only)
- unsafe: N/A (seeds: 0/0/0/0 all zero; manifest unsafe_code = "forbid")
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: 1 finding(s)