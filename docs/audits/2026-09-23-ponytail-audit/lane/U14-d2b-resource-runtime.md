# U14 d2b-resource-runtime

net: -303 lines, -0 deps

- `dead` packages/d2b-resource-runtime/src/context.rs:157: delete trait method `ManagerEndpoint::cancel_watch` + manager.rs:1487,1593 impls + context.rs:1218 stub + the `CancelWatch` msg (context.rs:776,939,996 arms; manager.rs CancelWatch constructors only inside impl bodies) + the five tests that pin them; in resource.rs keep local `Unwatch` path (resource.rs:815). Verified workspace-wide: grep `cancel_watch` over packages;tests;nixos-modules;docs/reference/policy: ~30 hits, ALL trait/impl declarations (context.rs 157,776,939,996,1218 + manager.rs 1487,1593 + 15 provider test-manager impls + forward_rendezvous.rs:2699); ZERO call sites - even in-module stub returns `Err("cancel_watch not exercised")` (context.rs:1219). Dead whole cancel-watch path (B6 removed the ChannelManagerEndpoint that would carry it; trait serf remains uncalled). (leaf)
- `dead` packages/d2b-resource-runtime/src/context.rs:196: delete `LookupPlane::Store` variant + `RowLookup::plane()` (245) + `LookupPlane::plane()` (196-236) - grep `LookupPlane::Store` over packages;tests: all 4 hits in context.rs tests (1342,1359,1362,1370); every production lookup uses `LookupPlane::Manager` (context.rs 566,622,828,844 + guest effects_service + plane_controller_bridge). `RowLookup::plane()` zero production callers. (leaf)
- `dead` packages/d2b-resource-runtime/src/context.rs:174: delete `LookupDisposition`/`LookupDisposition::as_str()` + `RowLookup::disposition()` (270) - grep `.disposition(` over packages;tests: only in-module test asserts (227-233,277-283) call it; production manual-matches the enum. Tests-only classifier helper. (leaf)
- `dead` packages/d2b-resource-runtime/src/context.rs:529: delete `ResourceContext::get` (529) + `ResourceContext::lookup_view` (620) + `ServiceResourceContext::lookup` (826) + `lookup_view` (842); keep `view` only (800) + `fail_closed`. Grep `\.lookup_view\(|\.lookup\(|\.get\(` over packages+d2bd+resource-api: only the `view` (+) getter has production callers (effects services, forward_rendezvous 2719, plane_controller_bridge); `lookup`, `lookup_view` have only in-module test callers (1343,1349,1359,1370 etc.); `ResourceContext::get` has tests-only callers. 3 of 5 service reads dead; keep only `view`. (leaf)
- `dead` packages/d2b-resource-runtime/src/context.rs:74: delete `RowLookup::Unavailable` variant? - NO; refused: `RowLookup::Unavailable` live (context.rs:1351-1370 tests + guest effects_service:663-685 + plane_controller_bridge:412 + volume binding tests). Not flagged. (checked)
- `dead` packages/d2b-resource-runtime/src/context.rs:378: delete write-only `target` / `target_binding` fields + `with_target_binding` (381) + `target()`/`target_binding()` getters - grep `.target()`/`.target_binding()` over packages;tests: only in-module tests call them (context.rs:1342-1370); actor wires via `with_target_binding` (resource.rs:573,733) but the attached binding is never read. (leaf)

### U14 (dead getters; ContextProvider lane)

- `dead` packages/d2b-resource-runtime/src/provider.rs:134: `ProviderDirectory::register` (factory-only) - grep `\.register\(` over packages;tests: all 6 hits are `register_driver` (provider tests + provider_lifecycle.rs:342 + resource_plane_v3.rs:3098 + foundation_seed.rs:1104-1125). `register` never called in production; delete + keep `register_driver`. (leaf)

## Consistency notes

- packages/d2b-resource-runtime/src/lib.rs:65-84: 13 `MODULE_NAME` consts (not 12; count includes manager.rs:51, context.rs:4, driver.rs:3, error.rs:32, guest_target.rs:23, identity.rs:8, metadata.rs:44, provider.rs:3, resource.rs:36, revision.rs:37, spec_store.rs:43, target.rs:24, watch.rs:29; no schema.rs, revision included) + `smoke_tests` module asserting them - pending #A5 ledger item (not applied); re-verified present at HEAD, not re-flagged.

## Reopened refusals

- #PR15 [refused] stays refused - systemic duplications (NullRequeue/recording doubles + duplicated resource_uid + otel copy) - cross-crate owned elsewhere, still present at HEAD, no new evidence. Not re-flagged.
- #B6 [partial] - ManagerCall/ChannelManagerEndpoint deleted; audit-log history read path kept (spec_store.rs:700-800) + identity re-export kept (context.rs:10). The cancel-watch dead path above is NEW (B6 removed the channel endpoint; the approach is a separate dead caller).
- #B8 [applied] - BindingChildReconciler absent at HEAD (grep 0 hits); not re-flagged.

## Checked

Scanned all 15 source files of packages/d2b-resource-runtime (manager.rs 3707, resource.rs 1639, context.rs 1862, provider.rs 508, driver.rs 316, metadata.rs 487, watch.rs 263, guest_target.rs 1630, target.rs 832, spec_store.rs 1249, error.rs, revision.rs 53, schema.rs, identity.rs 37, spec_store.rs). Caller verification: workspace-wide grep (packages;tests;nixos-modules;docs/reference/policy) per candidate symbol with hit-count reporting. Verified live: ResourceDriver/ResourceManager family, WatchHub/ManagerEndpoint, deterministic_uid (callers resource_plane_v3.rs:6083 etc.), binding-child reconciler path in resource.rs, metadata.rs shared driver (live via d2b-resource-types/metadata.rs:72). src/generated included only as evidence. Finding candidates verified zero-caller or tests-only with search-method notes. Nothing edited.