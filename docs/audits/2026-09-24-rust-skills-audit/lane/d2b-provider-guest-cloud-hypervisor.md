# d2b-provider-guest-cloud-hypervisor - d2b-provider-guest-cloud-hypervisor
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 10440 (excl. src/generated/**) | modules: adoption, bootstrap_graph, config, controller, controller_session, descriptor, guest_local, health, identity, lib, shutdown, state
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate

## idiom
- d2b-provider-guest-cloud-hypervisor#6 sev=low blast=leaf effort=S verdict=actionable - `CloudHypervisorController` stores `_config` (controller.rs:1712) that is never read; only `config.validate()` at 1739 uses the value - fix: drop the `_config` field and its initializer, keeping the validate() call in `from_verified_descriptor` - [controller.rs:1712, controller.rs:1744]
  evidence: census `_config` over packages/ = 2 hits (field declaration + initializer), zero reads
- d2b-provider-guest-cloud-hypervisor#7 sev=low blast=leaf effort=S verdict=actionable - `observed_process_status` is controller state used only inside one `reconcile` invocation (reset at 1860, set at 2013/2016, read at 2042), a field masquerading as a local - fix: make it a local variable in `reconcile` and delete the struct field - [controller.rs:1722, controller.rs:1860, controller.rs:2042]
  evidence: census `observed_process_status` over the crate = 5 hits, all inside one reconcile() body
- d2b-provider-guest-cloud-hypervisor#8 sev=low blast=leaf effort=S verdict=actionable - `deletion_rank` (shutdown.rs:487) and `upgrade_rank` (shutdown.rs:666) are byte-identical match arms duplicated across two free functions - fix: one `ChildRole::rank()` method (or single free fn) used by both planners - [shutdown.rs:487-494, shutdown.rs:666-673]
  evidence: idiom seed 2 (`impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for`) = 0 hits; the two fn bodies are identical by static comparison
- d2b-provider-guest-cloud-hypervisor#9 sev=low blast=leaf effort=S verdict=actionable - `BootstrapGraph::readiness()` (bootstrap_graph.rs:131) hardcodes `bindings_ready`/`setup_ready` to true while the `bindings` field doc says fenced binding readiness gates VMM start; only tests call it - fix: delete the wrapper and update the test (bootstrap_graph.rs:417) to call `vmm_readiness` with explicit booleans - [bootstrap_graph.rs:131-139, bootstrap_graph.rs:417-422]
  evidence: census `\.readiness\(|vmm_readiness|vmm_lifecycle` over packages/ = production call at controller.rs:662 uses vmm_lifecycle with real values, all other calls are in bootstrap_graph.rs tests
- d2b-provider-guest-cloud-hypervisor#13 sev=low blast=leaf effort=S verdict=actionable - `GuestControlEndpoint::uid()` and `endpoint_uid()` (guest_local.rs:113-121) are identical accessors with identical doc text, and `endpoint_uid()` has no caller in this crate - fix: keep one accessor and drop the other (mirror the choice in the sibling copy under finding #14) - [guest_local.rs:113-121]
  evidence: census `endpoint_uid` over packages/ = 2 hits (this definition and the sibling copy's own test in d2b-resource-client/src/zone_client.rs:1067)

## own
- clean: seeds ran `\.clone\(\)` = 127, `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = 31, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<|Cow<` = 0; every clone inspected is explainable (request-payload ownership into `CloudHypervisorResourceRequest` variants, sort keys in `plan_children`/`plan_upgrade`, builder-method clones), and no shared-ownership types appear in src (Arc appears only in the `AuthenticatedResourceApiAdapter` session seam and test fakes).

## type
- d2b-provider-guest-cloud-hypervisor#10 sev=medium blast=leaf effort=S verdict=actionable - `derive_private_runtime_scope` (identity.rs:857) and `CloudHypervisorController::private_runtime_scope` (controller.rs:1808) take `role: &str` validated against exactly the four `ChildRole` variants, stringly-typed state where the enum exists - fix: take `ChildRole` and use `role.suffix()` inside; update the test callers (tests/controller.rs:249, tests/redaction_test.rs:62-104, tests/guest_spec_validation_test.rs:107-110) - [identity.rs:857-865, controller.rs:1808-1818]
  evidence: type seed 3 (`(mode|kind|state): String`) = 0 hits; the role whitelist at identity.rs:863 duplicates `ChildRole::suffix()` values
- d2b-provider-guest-cloud-hypervisor#11 sev=medium blast=leaf effort=M verdict=needs-contract - `CloudHypervisorConfig::default_machine_type` reuses the credential vocabulary type `OpaqueAzureRef` (d2b_contracts_provider::v3::credential) for a machine type that `validate()` restricts to "q35"|"microvm" - fix: introduce a `MachineType` enum (serde kebab-case) in place of `OpaqueAzureRef`, updating the committed root-config.schema.json and provider config wire - [config.rs:20, config.rs:53]
  evidence: serde seed 1 = 51 hits; root-config.schema.json (committed in this crate) defines the OpaqueAzureRef wire shape, so the field type is wire-pinned
- d2b-provider-guest-cloud-hypervisor#12 sev=medium blast=leaf effort=M verdict=actionable - `BootstrapGraph::vmm_readiness`/`vmm_lifecycle` take five positional booleans (bootstrap_graph.rs:142-176) where a swapped argument compiles and silently changes VMM start gating - fix: pass one readiness snapshot struct (the five facts already exist as `GuestDependencySnapshot` accessors) instead of five bools - [bootstrap_graph.rs:142-176, controller.rs:662-670]
  evidence: type seed 2 (`is_\w+: bool|\w+_flag: bool`) = 0 hits; static reading of the signatures and their call sites

## api
- d2b-provider-guest-cloud-hypervisor#1 sev=medium blast=leaf effort=S verdict=actionable - `repair_children` takes `committed: &BTreeMap<ResourceRef, CommittedChild>` whose only call site passes an always-empty map (`let committed = BTreeMap::new()` at controller.rs:2140), making the `committed.get(target)` branch at 2890 unreachable - fix: drop the parameter and the dead branch, delete the empty-map local - [controller.rs:2140, controller.rs:2148-2151, controller.rs:2876-2895]
  evidence: census `repair_children` over packages/,nixos-modules/,tests/,docs/reference/,labs/ = 2 hits (definition + the single call site with the empty map)
- d2b-provider-guest-cloud-hypervisor#2 sev=medium blast=leaf effort=S verdict=actionable - `CloudHypervisorResourceApi::assess_update` takes `children` that the production adapter discards (`let _ = children;` at controller.rs:1366, the request carries no children) while `reconcile` allocates a Vec just to drop it - fix: remove the `children` parameter from the trait method, the adapter override, and the call site (controller.rs:1907-1909) - [controller.rs:1361-1376, controller.rs:1907-1909]
  evidence: err seed 2 (`let _ = |\.ok\(\);`) hit at controller.rs:1366; census of `assess_update` call sites = 1 production call plus test fakes
- d2b-provider-guest-cloud-hypervisor#3 sev=low blast=leaf effort=S verdict=actionable - `ChildMutation::expected_uid()` (identity.rs:554) always returns `None` because the UID-free batch is structurally UID-free; the only consumers are tests asserting the None (bootstrap_graph.rs:340, tests/controller.rs:206, tests/guest_spec_validation_test.rs:181) - fix: delete the accessor and the assert-None assertions - [identity.rs:554-556, tests/controller.rs:206]
  evidence: census `expected_uid\(\)` over packages/ = 8 hits; the 4 ChildMutation hits are all assert-None, the rest are `ChildSpecUpdate::expected_uid` in d2bd (a different type with a real value)
- d2b-provider-guest-cloud-hypervisor#4 sev=low blast=leaf effort=S verdict=actionable - `GuestUpgradePlan::preserve_state()` (shutdown.rs:576) returns a literal `true`; its only consumer is the tautological assertion in finding #5 - fix: delete the accessor together with the assertion - [shutdown.rs:576-578]
  evidence: census `preserve_state\(\)` over packages/ = 1 hit (the test assertion at finalize_ordering_test.rs:286)
- d2b-provider-guest-cloud-hypervisor#14 sev=medium blast=family effort=M verdict=actionable - `GuestControlEndpoint` is declared byte-identically in this crate (guest_local.rs:49) and in d2b-resource-client (zone_client.rs:129), the not-applied ledger row C1 with no refusal reason - fix: keep one declaration (d2b-resource-client is the consumer-facing home; d2bd/src/composition.rs:10723 constructs it) and re-export from the other - [guest_local.rs:49-166, packages/d2b-resource-client/src/zone_client.rs:129-256]
  evidence: census `GuestControlEndpoint` over packages/ = 2 struct declarations plus consumers; ledger row C1 at docs/explanation/over-engineering-audit-record.md:472 (not applied, no refusal); both sites confirmed byte-identical at this baseline

## err
- clean: seeds ran `\.unwrap\(\)|\.expect\(` = 108, `let _ = |\.ok\(\);` = 2, `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 1, `enum \w*Error` = 10; every unwrap/expect hit is inside `#[cfg(test)]` modules or the sanctioned "fixed ..." class (`role.purpose().expect("fixed Endpoint purpose")` identity.rs:610, `expect("fixed child plan role")` controller.rs:2949, `expect("fixed owner limits")` controller.rs:2869), the two `let _ =` sites are the dropped `assess_update` parameter (finding #2) and a test abort-await, and the error taxonomy (CloudHypervisorError wrapping Descriptor/ResourceApi/LifecyclePlan via From) needs no string matching by callers.

## serde
- clean: seeds ran `derive\([^)]*(De)?[Ss]erialize|serde\(...` = 51, `impl .*Deserialize.*for|serde_json::from_|serde_json::to_` = 6; the hand-written `Deserialize` impls (descriptor.rs, identity.rs) are live admission gates for the signed setup descriptor and child bodies, the recorded-refusal class (over-engineering-audit-record.md, refused Deserialize gates) so not re-flagged; `deny_unknown_fields` is applied on config, status, and every Wire admission struct, and the enum representations (internal tag on ChildCreateBody, transparent on OpaqueDescriptorSignature/ChildRoleSet) are deliberate wire pins covered by guest_spec_validation_test.rs.

## obs
- clean: seeds ran `\bprintln!\(|\beprintln!\(|(info|debug|warn|error|trace)!\("` = 0, `\.instrument\(|#\[instrument|tracing::|log::` = 53; every tracing event uses named fields (zone/resource/error/stage), errors are logged once at the boundary that handles them via `inspect_err`, no secrets reach fields (identity-bearing Debug impls redact), and no subscriber is installed by the library.

## docs
- d2b-provider-guest-cloud-hypervisor#15 sev=low blast=leaf effort=M verdict=actionable - no canonical `# Errors`/`# Examples` sections exist anywhere in the crate (seed 2 = 0) although 114 pub items return `Result<`; e.g. `GuestSetupDescriptor::from_canonical_bytes` (descriptor.rs:423) and `GuestChildBatch::from_descriptor` (identity.rs:590) document neither failure conditions nor a usage example - fix: add `# Errors` sections to the wire-boundary constructors first (descriptor.rs, identity.rs, health.rs), then the remaining Result-returning pub items - [descriptor.rs:423-429, identity.rs:590-634]
  evidence: docs seeds: `^\s*pub (fn|struct|enum|trait|const|type)` = 339, `/// # (Examples|Errors|Panics|Safety)` = 0, `-> Result<` = 114; the crate denies missing_docs (lib.rs:3) so every item has a first sentence, but failure contracts are absent

## perf
- d2b-provider-guest-cloud-hypervisor#16 sev=low blast=leaf effort=S verdict=actionable - `child_role_for_ref` (shutdown.rs:505) builds `format!("-{}", role.suffix())` inside the per-role loop, four String allocations per call on the per-child planning path (`plan_upgrade` at controller.rs:2340, `project_status` at controller.rs:2940) - fix: use `name.rsplit_once('-')` and compare the suffix, or a static suffix table - [shutdown.rs:505-513]
  evidence: perf seed 1 `format!\(` = 6 hits, this is the only non-test production hit; static (unmeasured)

## conc
- N/A: seeds `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 0, `Atomic\w+|Ordering::` = 0 (one doc-comment word "atomically" is a false positive), `thread_local!|unsafe impl (Send|Sync) for` = 0; the crate declares no threads, locks, atomics, or manual Send/Sync.

## async
- clean: seeds ran `async fn|async move|\.await` = 125, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(|tokio::sync::(Mutex|RwLock|Notify)|#\[tokio::(main|test)\]|Runtime::block_on` = 2, `tokio::sync::(Mutex|RwLock|Notify)` = 0; the fd10 bootstrap handshake and assignment-stream loop in controller_session.rs are the refused row 7 (over-engineering-audit-record.md:125, G7) so not re-flagged; `Runtime::block_on` at the process entry is the sanctioned CLI-only path with `#[allow(clippy::disallowed_methods, reason = "CLI-only path")]` (controller_session.rs:58), tokio::spawn appears only in tests, no guard is held across `.await` in src, and no blocking call sits inside an async fn.

## unsafe
- N/A: seeds `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0, `// SAFETY:` = 0, `transmute|from_raw|MaybeUninit|mem::zeroed` = 0; the single `unsafe_code` hit is `#![forbid(unsafe_code)]` at lib.rs:4, which alone does not make the lens applicable.

## ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0; the crate crosses no foreign boundary.

## macro
- N/A: seeds `macro_rules!` = 0, `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0; the crate defines no macros.

## test
- d2b-provider-guest-cloud-hypervisor#5 sev=medium blast=leaf effort=S verdict=actionable - `assert!(plan.preserve_state())` (finalize_ordering_test.rs:286) cannot fail because `preserve_state()` returns a literal `true` (shutdown.rs:577), an assertion of implementation rather than behavior - fix: delete the assertion together with the accessor (finding #4) - [finalize_ordering_test.rs:286]
  evidence: test seeds: `#\[test\]|#\[tokio::test\]` = 52, `assert_eq!\(|assert_ne!\(|assert!\(` = 221, `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0; the asserted accessor body is a constant

## Coverage
- idiom: 5 findings
- own: clean (seeds ran: 127/31/0)
- type: 3 findings
- api: 5 findings
- err: clean (seeds ran: 108/2/1/10; all hits tests or sanctioned "fixed" expects)
- serde: clean (seeds ran: 51/6; admission-gate Deserialize impls are recorded-refusal class)
- obs: clean (seeds ran: 0/53)
- docs: 1 finding
- perf: 1 finding
- conc: N/A (seeds: 0/0/0/0; no threads, locks, atomics, or TLS)
- async: clean (seeds ran: 125/2/0/0; G7-refused handshake not re-flagged)
- unsafe: N/A (seeds: 0/0/0/1; only the forbid(unsafe_code) attribute)
- ffi: N/A (seeds: 0/0/0/0)
- macro: N/A (seeds: 0/0/0/0)
- test: 1 finding