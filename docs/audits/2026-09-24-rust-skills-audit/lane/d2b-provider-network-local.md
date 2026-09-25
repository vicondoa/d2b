# d2b-provider-network-local - d2b-provider-network-local
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 11234 (excl. src/generated/**, src 9346 + tests 1888) | modules: whole crate (artifact, bridge_port, broker, controller, diagnostics, driver, effects_service, facets, ifname, netlink, nftables, observe, operations, plan, routes, test_support)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: n/a (single lane)

## idiom
- d2b-provider-network-local#1 sev=low blast=leaf effort=S verdict=actionable - octet-to-string conversion collects a Vec of four Strings and joins it, where one format! suffices - fix: destructure the parsed octets (`let [a, b, c, d] = octets; Some(format!("{a}.{b}.{c}.{d}"))`) instead of `.collect::<Vec<_>>().join(".")` - [src/controller.rs:298-302]
  evidence: idiom seed `let mut \w+ = (String|Vec)::new\(\)` count 10; the site is the collect-then-convert shape the skill names (reviewed statically)
- d2b-provider-network-local#2 sev=low blast=leaf effort=S verdict=actionable - declared_dependency_refs accumulates into `let mut refs = Vec::new()` with a nested if-push, where a filter_map pipeline fits - fix: `spec.pointer("/spec/attachments").and_then(Value::as_array).into_iter().flatten().filter_map(|a| a.get("executionRef").and_then(Value::as_str).and_then(|v| ResourceRef::parse(v.ok()()).collect()` - [src/driver.rs:377-393]
  evidence: idiom seed `let mut \w+ = (String|Vec)::new\(\)` count 10 (direct hit at driver.rs:378)

## own
- d2b-provider-network-local#3 sev=low blast=leaf effort=S verdict=actionable - collision-detection BTreeSet stores owned Strings from borrowed &str keys, though the set never outlives the borrow - fix: `let mut unique_interfaces = BTreeSet::new();` and insert `ifname.as_str()` (a set of `&str` borrowing interface_names for its whole short life)) - [src/controller.rs:416-417]
  evidence: own seed `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` count 85; sampled:	 50 of 206 own hits (every 5th; the borrow lives only inside the fn, no caller census needed)

## type
- clean: seeds: `fn validate_\w+|fn check_\w+` 14, `is_\w+: bool|\w+_flag: bool` 1), `(mode|kind|state): String` 0; every validator takes already-parsed types (TapRole, BridgePortFlagSet, DefaultRouteState, ReconcileInput)...) and the one bool is a single config flag, not flag soup; no illegal-state combos found

## api
- d2b-provider-network-local#4 sev=medium blast=leaf effort=S verdict=actionable - two pub route validators are exported with zero production callers (only crate-internal unit tests), and the wrapper carries a stale `#[allow(dead_code)]` on a pub item - fix: lower both to `pub(crate)` (unit tests still reach them)and remove the dead_code allow, or wire them into BrokerNetworkEffectPort::apply_routes/remove_routes which currently resolve intents without these checks - [src/routes.rs:244-279, src/routes.rs:264-265]
  evidence: census: `validate_network_route_intent` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 7 hits (all in src/routes.rs: def 245, wrapper call 276, tests 525/534, wrapper def 265, wrapper tests 568/579); `validate_network_route_intent_with_provenance` = 3 hits (all in src/routes.rs; sampled:   50 of 328 api hits

## err
- d2b-provider-network-local#5 sev=low blast=leaf effort=S verdict=actionable - the sole non-test unwrap (SHA-256 word slice conversion() carries no named invariant, though the 4-byte length is statically known - fix: `u32::from_be_bytes(chunk[offset..offset + 4].try_into().expect("4-byte chunk word"))` or a slice-pattern destructure - [src/nftables.rs:588]
  evidence: err seed `\.unwrap\(\)|\.expect\(` count 155 (154 are cfg(test) fixtures or literal canonical-ref expects at operations.rs:139-202; this singleton is production code)

## serde
- d2b-provider-network-local#6 sev=medium blast=leaf effort=M verdict=actionable - the stored-spec parse maps serde failure to `()` unit, dropping the deserialization reason before the toolkit's SpecInvalid terminal - fix: log the serde error (add a tracing::debug/warn at driver.rs:289 before the map)) or return `Result<NetworkSpec, serde_json::Error>` and let the driver surface the reason; do not touch the pinned SharedProviderDeclarationError enum - [src/driver.rs:282-289, src/driver.rs:190]
  evidence: serde seed `serde_json::from_|serde_json::to_` count 29 (this site maps from_value failure to ()); `derive)...Serialize...)` 0
- d2b-provider-network-local#7 sev=low blast=leaf effort=S verdict=actionable - provenance serialization failures are silently `.ok()`-swallowed into a missing wire field at four payload builders, while the sibling update-hosts path propagates with map_err - fix: match broker.rs:1368: `.map(serde_json::to_value).transpose().map_err)...)` at all four sites (operations.rs maps to OperationFailure::with_detail(KERNEL_REFUSED, ...)) - [src/broker.rs:1432, src/broker.rs:1453, src/operations.rs:408, src/operations.rs:429]
  evidence: serde seed `serde_json::to_` count 29 (4 of which are `.ok()`-swallowed; sibling at broker.rs:1368 uses map_err)

## obs
- clean: seeds: `\bprintln!\(|\beprintln!\(` 0,, `(info|debug|warn|error|trace)!\("` 0,, `\.instrument\(|#\[instrument` 0,, `tracing::|log::` 4; all four tracing events carry named fields (`broker_kind = %code`, `provider = "network-local"`, `network_uid = ...`) and no secret or interpolated message

## docs
- d2b-provider-network-local#8 sev=low blast=leaf effort=M verdict=actionable - Result-returning pub items (~151 sites() carry no `# Errors` section naming their failure conditions, despite `#![deny(missing_docs)]` giving every item a first sentence - fix: add canonical `# Errors` sections to the boundary-facing Result fns (at least: resolve_net_vm_system_artifact, validate_readback, validate_network_route_intent, observe_host_network, NetworkReconciler::reconcile/finalize,and the broker kernel adapters)) - [src/artifact.rs:67, src/bridge_port.rs:151, src/routes.rs:245, src/observe.rs:255, src/controller.rs:1096]
  evidence: docs seeds: `^\s*pub (fn|struct|enum|trait|const|type)` 302,, `/// # (Examples|Errors|Panics|Safety)` 0,, `-> Result<` 151; sampled:	  50 of 453 docs hits (every 10th; first-sentence quality checked across all modules - mostly strong, no magic values left unexplained)

## perf
- d2b-provider-network-local#9 sev=low blast=leaf effort=S verdict=actionable - FirewallDigest::to_hex formats each byte into its own String (32 heap allocations per call), though the output size is known - fix: `let mut out = String::with_capacity(64); for byte in &self.0 { use std::fmt::Write; write!(out, "{byte:02x}").expect("writing to String is infallible"); } out` - [src/nftables.rs:266-268]
  evidence: static (unmeasured); perf seed `format!\(` count 54; to_hex is cross-crate used (d2bd resource_plane_v3.rs:322/533 socket identity keys, process_provider_runtime.rs:3041 log field))
- d2b-provider-network-local#10 sev=low blast=leaf effort=S verdict=actionable - observed-address parse allocatesa fresh String per entry via `format!("{local}/{prefix}")`, inside the host-observation parse path - fix: build the CIDR text into a reused buffer or add a two-part Ipv4Cidr constructor to the contracts crate - [src/observe.rs:305]
  evidence: static (unmeasured); perf seeds: `format!\(` 54,, `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` 61,, `\.to_string\(\)` 10

## conc
- clean: seeds: `std::thread::|thread::spawn|thread::scope` 1 (broker.rs:1662 test poll-loop yield_now), `\bMutex<|\bRwLock<` 7 (all test fixtures; the parking_lot sites carry async-gate-allow markers - recorded exceptions, clippy.toml:40-43,82-84), atomics/Ordering 0,, thread_local/unsafe-Send-Sync 0; no shared-state or atomic-ordering claims in production code

## async
- d2b-provider-network-local#11 sev=low blast=leaf effort=S verdict=actionable - observe_host_network awaits three independent `ip` observations sequentially, where tokio::join! would run them concurrently - fix: `let (links, addresses, routes)= tokio::join!(run_ip(&["-j", "-d", "link", "show"]), run_ip(&["-j", "-4", "addr", "show"]), run_ip(&["-j", "-4", "route", "show", "table", "all"]));` then parse - [src/observe.rs:255-260]
  evidence: async seed `async fn|async move|\.await` count 123 (the three sequential process awaits at observe.rs:256-258 are independent - no data dependency); spawn/JoinSet 0; tokio::sync::* 6 (test fixtures); tokio::test 5; block_on 0 in src; elsewhere kernel invocations run through async kernel_seat::run (operations.rs:249-264) and process calls carry timeouts (observe.rs:420-437); no lock is held across an await in production code, and test-support parking_lot locks carry async-gate-allow markers (test_support.rs:68-80) - deliberate exceptions

## unsafe
- clean: seeds: `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` 0,, `// SAFETY:` 0,, `transmute|from_raw|MaybeUninit|mem::zeroed` 0,, `unsafe_code` 0 in src; lens N/A (no unsafe code; manifest forbids - Cargo.toml:5))

## ffi
- clean: seeds: `extern "C"|no_mangle|unsafe\(link_section` 0,, `catch_unwind` 0,, `repr\(C\)|repr\(transparent\)` 0,, `CStr|CString|c_char` 0; lens N/A (no FFI surface in the crate)

## macro
- clean: seeds: `macro_rules!` 1 (bridge_port.rs:157 local field-check macro - acceptable impl-per-field generation for nine flags, hygiene trivial), `proc_macro|syn::|quote!` 0,, `\$crate` 0,, `to_compile_error|new_spanned` 0; no macro needs rework

## test
- clean: seeds: `#\[test\]|#\[tokio::test\]` 95,, `assert_eq!\(|assert_ne!\(|assert!\(` 252 (over src+tests), `proptest!|insta::assert|rstest` 0,, `#\[ignore\]` 0;; sampled:	 50 of 347 test hits; all 6 test files read - table-driven cases (broker.rs:1844), error variants asserted not Display strings (netlink.rs:437, observe.rs:730), deterministic, no network; fd-passing test exercisesa real socketpair with rustix ScmRights (network_family.rs:200-220); block_onin plain #[test] harnesses is the sanctioned pattern

## Coverage
- idiom: 2 finding(s)
- own:	 1 finding(s)
- type: clean (seeds ran: 14/1/0)
- api:	 1 finding(s)
- err:	 1 finding(s)
- serde:	 2 finding(s)
- obs: clean (seeds ran:	 0/0/0/4)
- docs:	 1 finding(s)
- perf:	 2 finding(s)
- conc: clean (seeds ran:	 1/7/0/0)
- async:	 1 finding(s)
- unsafe: N/A (seeds:	 0/0/0/0 all zero; no unsafe code, manifest forbids)
- ffi:	 N/A (seeds:	 0/0/0/0 all zero; no FFI surface)
- macro: clean (seeds ran:	 1/0/0/0)
- test:	 clean (seeds ran:	 95/252/0/0)