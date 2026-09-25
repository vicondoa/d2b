# d2b-provider-display-wayland-p1 - d2b-provider-display-wayland - part 1/2
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 9342 (excl. src/generated/**) | modules: wayland_proxy/{bridge,clipboard,decoration,diag,dmabuf,filter,identity,mod,policy,readiness}
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 1/2 = src/wayland_proxy/** (part 2 = src/controller.rs, src/runtime.rs, src/process.rs, src/bin/**, src/spec.rs, src/policy.rs, src/session_children.rs, src/principal.rs, src/lib.rs)

## idiom
- d2b-provider-display-wayland-p1#1 sev=low blast=leaf effort=S verdict=actionable - handoff_via_bridge re-wraps the bound `error` into a fresh `HandoffStatus::Failed(error)` and immediately matches it back out with a `_ => unreachable!()` arm that can never fire; the outer match arm already binds the value - fix: delete the `let status = ...` / `let error = match status {...}` round-trip and use the arm-bound `error` directly in filter.rs:620-628 - [packages/d2b-provider-display-wayland/src/wayland_proxy/filter.rs:620, packages/d2b-provider-display-wayland/src/wayland_proxy/filter.rs:624]
  evidence: err seed 3 `panic!\(|unreachable!\(|todo!\(|unimplemented!\(` = 2; static read of filter.rs:606-631 shows the re-match is on a value constructed two lines earlier
- d2b-provider-display-wayland-p1#2 sev=low blast=leaf effort=S verdict=actionable - handle_bind dispatches per-interface handler installation through a nested `match try_downcast::<XdgWmBase>() { Some => ..., _ => match try_downcast::<WlEglstreamDisplay>() { Some => ..., _ => { if let ... } } }` while the same function already uses edition-2024 if-let chains for viewporter and dmabuf, mixing two dispatch styles in one body - fix: flatten the nested match into `if let Some(wm_base) = ... else if let Some(eglstream) = ... else if let Some(compositor) = ...` chains, keeping the early `return` arms - [packages/d2b-provider-display-wayland/src/wayland_proxy/filter.rs:1272, packages/d2b-provider-display-wayland/src/wayland_proxy/filter.rs:1300]
  evidence: idiom seed 1 `for \w+ in 0\.\.` = 12; static read of filter.rs:1191-1332 (the fn mixes nested match with `if let ... && let ...` chains at filter.rs:1296-1304)
- d2b-provider-display-wayland-p1#3 sev=low blast=leaf effort=S verdict=actionable - filter_format_table iterates `for (index, entry) in table.chunks_exact(16).enumerate()` but the index is never used except `let _ = index;` inside the overflow branch, an ignore that exists only to silence the unused variable - fix: drop `.enumerate()` and remove `let _ = index;` - [packages/d2b-provider-display-wayland/src/wayland_proxy/dmabuf.rs:790, packages/d2b-provider-display-wayland/src/wayland_proxy/dmabuf.rs:801]
  evidence: idiom seed 1 `for \w+ in 0\.\.` = 12; static read of dmabuf.rs:790-806 (the `Ok` branch derives `new_index` from `filtered.len() / 16`, never from `index`)
- d2b-provider-display-wayland-p1#4 sev=low blast=leaf effort=S verdict=actionable - sanitize_label calls `out.chars().count()` on every loop iteration, a quadratic re-count of the output string that grows with the label length (bounded at 64 chars, so cheap, but the shape invites the same mistake at a larger bound) - fix: track a `let mut written = 0usize;` counter incremented per pushed char and compare against `MAX_LABEL_CHARS` - [packages/d2b-provider-display-wayland/src/wayland_proxy/decoration.rs:150]
  evidence: idiom seed 3 `let mut \w+ = (String|Vec)::new\(\)` = 8; static read of decoration.rs:146-171
- d2b-provider-display-wayland-p1#5 sev=low blast=leaf effort=S verdict=actionable - four comment blocks are mangled prose: a non-ASCII full stop (`\u3002`) at dmabuf.rs:820, a stray `**` at filter.rs:769, two `; no` joins missing the space after the semicolon at filter.rs:770 and filter.rs:2801, and misindented two-line comment pairs at decoration.rs:1804-1805, dmabuf.rs:819-820 and filter.rs:2799-2801 where the continuation line sits at 4-space indent inside the fn - fix: rewrite the four comments as plain ASCII with normal spacing and consistent indent - [packages/d2b-provider-display-wayland/src/wayland_proxy/dmabuf.rs:820, packages/d2b-provider-display-wayland/src/wayland_proxy/filter.rs:769, packages/d2b-provider-display-wayland/src/wayland_proxy/filter.rs:770, packages/d2b-provider-display-wayland/src/wayland_proxy/filter.rs:2801, packages/d2b-provider-display-wayland/src/wayland_proxy/decoration.rs:1804]
  evidence: static (grep for `\u3002|\uff1b` and `\*\*|; no` over the module = 1 and 3 hits respectively; all four sites read)

## own
- d2b-provider-display-wayland-p1#6 sev=low blast=leaf effort=S verdict=actionable - FilterPolicy carries `dmabuf_filters: std::sync::Arc<DmabufFilterList>` (built at policy.rs:316, cloned into DmabufHandler at filter.rs:1305) while the entire proxy is a single-threaded `Rc` graph - no thread or `'static` boundary justifies Arc, and the conc seeds are all zero - fix: switch the field and `DmabufHandler::new` parameter to `Rc<DmabufFilterList>` - [packages/d2b-provider-display-wayland/src/wayland_proxy/policy.rs:186, packages/d2b-provider-display-wayland/src/wayland_proxy/policy.rs:316]
  evidence: own seed 3 `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` = 295; this is the only `Arc<` in the module (conc seeds 1-4 all 0); Rc clones at handler boundaries are the explainable wayland-rs pattern
- clean: own seeds 1-4 ran (119 / 79 / 295 / 0); the 119 `.clone()` sites and 79 `to_owned` family sites are explainable (Rc protocol-object clones at handler installs, identity/mime copies into owned metadata and log closures, `Rc::downgrade` weak refs); no `Cow<` anywhere

## type
- clean: type seeds 1-3 ran (2 / 0 / 0); the two `validate_*` fns (bridge.rs:83, bridge.rs:95) are boundary checks on CLI-derived path input executed once inside `BridgeConfig::from_identity_parts`, the single construction path - parse-once is already applied, and the bridge component itself is a SHA-256 digest that cannot be invalid

## api
- d2b-provider-display-wayland-p1#7 sev=medium blast=leaf effort=M verdict=actionable - `#[allow(missing_docs)] pub mod wayland_proxy` (lib.rs:14) exposes the whole 9,342-line proxy implementation as public library surface whose only consumer is the crate's own binary target, which cannot reach crate-private items - fix: move the module into the binary target (declare `#[path = "../wayland_proxy/mod.rs"] mod wayland_proxy;` in src/bin/d2b-wayland-proxy.rs and rewrite the `crate::wayland_proxy::` paths to `wayland_proxy::`), keeping the lib surface to the re-exported controller/policy/process/runtime/spec items - [packages/d2b-provider-display-wayland/src/lib.rs:14]
  evidence: api seed 1 `\bpub (fn|struct|enum|trait|type|const|mod) ` = 160; census: `d2b_provider_display_wayland::wayland_proxy` over packages/, nixos-modules/, tests/, docs/reference/ = 4 hits, all in src/bin/d2b-wayland-proxy.rs; labs/window-chrome/proxy is a separate workspace whose disposition ADR 0047 owns (docs/adr/0047-window-identity-chrome.md)
- d2b-provider-display-wayland-p1#8 sev=low blast=leaf effort=S verdict=actionable - `pub use policy::{FilterPolicy, GlobalAction, PolicyInput, PolicyWarning};` in wayland_proxy/mod.rs re-exports four items at a second path with zero consumers; the bin imports them via `wayland_proxy::policy::...` - fix: delete the re-export line so each item has one path - [packages/d2b-provider-display-wayland/src/wayland_proxy/mod.rs:12]
  evidence: census: `wayland_proxy::(FilterPolicy|GlobalAction|PolicyInput|PolicyWarning)` over packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel = 0 hits

## err
- clean: err seeds 1-4 ran (63 / 10 / 2 / 1); production `unwrap`/`expect` sites are all justified - invariant-named expects on private state (bridge.rs:249), static memfd `CString` literals (dmabuf.rs:823, decoration.rs:1808), and the literally-built serde frame (bridge.rs:363, the U1 false-positive class); the `let _ =` sites are deliberate fd-close/status-ignore at terminal handoff steps; the one dead `unreachable!()` (filter.rs:624) is covered by idiom#1 and the documented registry-exhaustion `unreachable!` (filter.rs:1093) names its invariant

## serde
- d2b-provider-display-wayland-p1#9 sev=medium blast=family effort=S verdict=actionable - the bridge receive path detects the clipd refresh frame by scanning raw bytes for the substring `"type":"refresh_selection"` (filter.rs:817-824) instead of deserializing the typed frame the way the send side serializes it (bridge.rs:321-365); the producer emits a hand-written byte literal (d2b-clipd.rs:1690), so any whitespace or key-order change in the shared JSON shape silently disables clipboard refresh with no error - fix: define a `#[serde(tag = "type", rename_all = "snake_case")]` inbound frame enum mirroring `bridge_frame`'s `Frame`, deserialize each newline frame with `serde_json::from_str`, and match the `RefreshSelection` variant; add a test feeding `{"type":"refresh_selection"}` through the drain path - [packages/d2b-provider-display-wayland/src/wayland_proxy/filter.rs:817, packages/d2b-provider-display-wayland/src/wayland_proxy/filter.rs:819]
  evidence: serde seed 4 `serde_json::from_|serde_json::to_` = 5 (to_ side only in this module); wire peer emits the byte literal at packages/d2b-provider-clipboard-wayland/src/bin/d2b-clipd.rs:1690; no test covers the refresh path (test seed 1 = 130, none exercises drain_bridge_messages refresh)

## obs
- clean: obs seeds 1-4 ran (0 / 1 / 0 / 20); no println in the module; the single message-only macro hit is the lazy `log::warn!("{}", msg())` at diag.rs:147; all `log::` events use the crate's consistent `[d2b-wlproxy] target=... event=... reason=...` key=value scheme with closure-built messages evaluated only when enabled, and diag.rs:1-5 documents the bounded-metadata policy (no titles, payloads, or raw app-ids); the ADR 0010/0028 redaction gate covers identifier-in-log

## docs
- d2b-provider-display-wayland-p1#10 sev=medium blast=leaf effort=M verdict=actionable - `#[allow(missing_docs)]` on `pub mod wayland_proxy` (lib.rs:14) exempts the crate's largest public surface from the `#![deny(missing_docs)]` contract that every other module honors, leaving non-obvious items undocumented (FilterPolicy::build layering, BridgeReconnectMachine state transitions, DmabufFilterList::normalize semantics, ProxyIdentity::log_label) - fix: either document the exported items or, per api#7, move the module into the binary target where missing_docs does not apply - [packages/d2b-provider-display-wayland/src/lib.rs:14]
  evidence: docs seed 1 `^\s*pub (fn|struct|enum|trait|const|type)` = 160; docs seed 2 `/// # (Examples|Errors|Panics|Safety)` = 0; lib.rs:1-5 carries the deny
- d2b-provider-display-wayland-p1#11 sev=low blast=leaf effort=S verdict=actionable - public Result-returning items lack `# Errors` sections describing which conditions fail: `BridgeConfig::from_identity_parts`, `path_for_user_identity`, `parse_filter`, and the three `ReadinessReporter` methods - fix: add `# Errors` sections naming `BridgeConfigError` variants, the parse failure modes, and the io errors - [packages/d2b-provider-display-wayland/src/wayland_proxy/bridge.rs:37, packages/d2b-provider-display-wayland/src/wayland_proxy/bridge.rs:73, packages/d2b-provider-display-wayland/src/wayland_proxy/dmabuf.rs:162, packages/d2b-provider-display-wayland/src/wayland_proxy/readiness.rs:30]
  evidence: docs seed 3 `-> Result<` = 9; docs seed 2 = 0 (no canonical sections anywhere in the module)

## perf
- clean: perf seeds 1-3 ran (53 / 31 / 11); every `format!` site is a cold path (lazy diag closures, error details, startup policy messages, identity labels built once per instance); no `format!` or allocation sits in a per-message hot loop; the rail pixel buffer is cached behind `FrameKey` (decoration.rs:1011) and only rebuilt when the key changes; the one bounded-quadratic `chars().count()` is covered by idiom#4; all findings here are static (unmeasured)

## conc
- N/A (seeds: 0/0/0/0 all zero; the module declares no threads, mutexes, atomics, or thread_local - it is a single-threaded `Rc`/`RefCell` handler graph, so the lens has nothing to judge)

## async
- N/A (seeds: 0/0/0/0 all zero; no async fn, await, spawn, or tokio sync primitive in the module - the proxy is a synchronous poll-driven loop and every blocking call site carries the sanctioned `#[allow(clippy::disallowed_methods, reason = "synchronous path")]` marker: readiness.rs:29, filter.rs:771, filter.rs:2880, bridge.rs:280, dmabuf.rs:822, decoration.rs:1806)

## unsafe
- N/A (seeds: 1-3 zero after false positives; the 2 `from_raw` hits are the safe `std::io::Error::from_raw_os_error` at filter.rs:991 and filter.rs:2824, not raw-pointer conversion; no `unsafe` block, fn, or impl exists in the module and the crate forbids unsafe_code at lib.rs:5)

## ffi
- N/A (seeds: 0/0/0/4; the four `CString` hits are memfd name arguments at libc call sites (memfd_create at dmabuf.rs:823 and decoration.rs:1808), which never cross a foreign caller - the U1 ffi false-positive class)

## macro
- d2b-provider-display-wayland-p1#12 sev=low blast=leaf effort=S verdict=actionable - the local `macro_rules! entry!` (policy.rs:420-437) is a two-arm table-filling shorthand whose `max=` arm only omits one field; it is not variadic, does not generate impls per type, and is not a DSL, so a plain function is the cheaper answer - fix: replace the macro with `fn entry(m: &mut HashMap<String, PolicyEntry>, iface: &str, action: GlobalAction, class: Classification, max: Option<u32>)` and update the ~70 call rows; the catalog content stays byte-identical (the hand-written catalog itself is Nix-pinned and refused at docs/explanation/over-engineering-audit-record.md:352, 427-429 - this finding touches only the mechanism) - [packages/d2b-provider-display-wayland/src/wayland_proxy/policy.rs:420]
  evidence: macro seed 1 `macro_rules!` = 1 (sole definition in the module); static read of policy.rs:417-465

## test
- d2b-provider-display-wayland-p1#13 sev=high blast=leaf effort=S verdict=actionable - two registry-handler tests cannot fail on any behavior change: `filtered_globals_preserve_original_global_names` (filter.rs:3213-3224) inserts entries into `advertised_globals`/`hidden_globals` and asserts their presence - pure setup restatement with no function under test - and `standard_clipboard_global_is_advertised_as_synthetic` (filter.rs:3264-3283) admits in its comment that the real path is untested and then asserts only `interface.name()` plus the map content it just inserted - fix: delete the first test and rewrite the second to exercise `prepare_global(11, ObjectInterface::WlDataDeviceManager, 3)` and assert the synthetic `GlobalAdvertisement` decision, as the neighboring `prepare_global_hides_*` tests already do - [packages/d2b-provider-display-wayland/src/wayland_proxy/filter.rs:3213, packages/d2b-provider-display-wayland/src/wayland_proxy/filter.rs:3264]
  evidence: test seeds over src/wayland_proxy + tests/: `#\[test\]` = 130, `assert_*` = 301, proptest/insta/rstest = 0, `#\[ignore\]` = 0; static read of filter.rs:3213-3224 and filter.rs:3264-3283 (assertions restate the inserted state)

## Coverage
- idiom: 5 finding(s)
- own: 1 finding(s)
- type: clean (seeds ran: 2/0/0; boundary validation already parse-once at the single construction path)
- api: 2 finding(s)
- err: clean (seeds ran: 63/10/2/1; every production panic site is an invariant-named expect or a U1 false-positive class)
- serde: 1 finding(s)
- obs: clean (seeds ran: 0/1/0/20; no println, consistent key=value event scheme with lazy closures, bounded-metadata policy documented)
- docs: 2 finding(s)
- perf: clean (seeds ran: 53/31/11; format!/allocation sites are cold or lazy, rail redraw cached by FrameKey)
- conc: N/A (seeds: 0/0/0/0 all zero; single-threaded Rc/RefCell handler graph)
- async: N/A (seeds: 0/0/0/0 all zero; synchronous poll-driven proxy with sanctioned synchronous-path allows)
- unsafe: N/A (seeds: 0/0/2/0 with the 2 from_raw hits being safe from_raw_os_error; no unsafe blocks; crate forbids unsafe_code)
- ffi: N/A (seeds: 0/0/0/4; CString hits are memfd name args at libc call sites, not a foreign-caller boundary)
- macro: 1 finding(s)
- test: 1 finding(s)