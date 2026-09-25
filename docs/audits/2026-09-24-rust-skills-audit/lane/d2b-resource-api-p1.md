# d2b-resource-api-p1 - d2b-resource-api - part 1/2
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 6971 (excl. src/generated/**) | modules: service.rs, adapter.rs, manager_backend.rs, client.rs, store.rs, watch.rs
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 1/2: src/service.rs, src/adapter.rs, src/manager_backend.rs, src/client.rs, src/store.rs, src/watch.rs (part 2 owns src/authz.rs, src/manager_backend/**, src/admission.rs, src/error.rs, src/identity.rs, src/lib.rs)

## idiom
- clean: seeds `for \w+ in 0\.\.` = 1 (test-only case-index loop, service.rs:2834), `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 0, `let mut \w+ = (String|Vec)::new\(\)` = 2 (manager_backend.rs:490 cursor key build, covered by perf finding 11; manager_backend.rs:1243 bounded batch loop, MAX_BATCH_MUTATIONS = 32). Hand-written `Clone` on `CheckedResourceStore` (store.rs:85) avoids an unwanted `S: Clone` derive bound and the hand-written `Debug` impls redact secrets - both deliberate per the idiom card's false-positive list.

## own
- d2b-resource-api-p1#1 sev=low blast=leaf effort=S verdict=actionable - every bus scoped commit clones the full assignment-mutation list (`transport.mutations().to_vec()`) even though the whole chain only borrows it - fix: change `ResourceApiClient::scoped_commit_batch` (client.rs:110) and `ResourceService::commit_scoped_batch` (service.rs:852) to take `&[ScopedResourceMutation]` and pass `transport.mutations()` directly at adapter.rs:425 - [adapter.rs:425, client.rs:110, service.rs:852]
  evidence: seed `\.clone\(\)` = 50 hits, `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = 52; census: `scoped_commit_batch` over packages = no production caller outside d2b-resource-api, so the signature change breaks no caller; the remaining clones are explainable (Arc clones of shared service, cursor/claims ownership transfers).
- clean: seeds `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<|Cow<` = 0; no refcounted interior mutability or Cow in production code.

## type
- d2b-resource-api-p1#2 sev=low blast=leaf effort=S verdict=actionable - `attach_scoped_query_frame` takes a bare `watch: bool` mode flag selecting List versus Watch rewriting, a boolean state the type lens names - fix: replace the parameter with `enum ScopedQueryMethod { List, Watch }` and update the d2b-bus call site (packages/d2b-bus/src/router.rs:2914) - [adapter.rs:124]
  evidence: seed `(mode|kind|state): String` = 0, `is_\w+: bool|\w+_flag: bool` = 0; the `watch: bool` parameter is the one boolean mode in the lane; a wrong bool fails closed at runtime (method mismatch), an enum makes it a compile error.

## api
- d2b-resource-api-p1#3 sev=low blast=leaf effort=S verdict=actionable - one-variant `ResourceApiReachability` enum plus `RESOURCE_API_REACHABILITY` const have no production consumer; the only assertion compares the const to its own definition and cannot fail - fix: delete both and the assertion in the 13-method test, or wire the const to a real reachability check - [adapter.rs:251-256, adapter.rs:1208-1211]
  evidence: census: `RESOURCE_API_REACHABILITY|ResourceApiReachability` over packages/nixos-modules/tests/docs/reference/labs/BUILD.bazel = 3 hits (adapter.rs definition, adapter.rs:1209 test, lib.rs:19 re-export); no production consumer.
- d2b-resource-api-p1#4 sev=low blast=leaf effort=S verdict=actionable - `commit_configuration_batch` is pub on both `ResourceApiClient` and `ResourceService` but nothing calls it (declared "internal Core path", unwired) - fix: wire it into d2bd bundle ingestion (packages/d2bd/src/resource_plane_v3.rs:3445 area) or reduce to pub(crate) until a caller exists - [client.rs:98, service.rs:837]
  evidence: census: `commit_configuration_batch` over packages/nixos-modules/tests/labs = 2 hits, both in d2b-resource-api itself; zero external callers.
- d2b-resource-api-p1#5 sev=low blast=leaf effort=S verdict=actionable - three `manager_backend` helpers are pub in a pub module with no production caller outside the crate: `wire_revision` (internal-only), `api_subject` (internal-only), `resource_owner_subject` (test-only, doc says U9/U10 wires it) - fix: make `wire_revision` and `api_subject` pub(crate); keep `resource_owner_subject` pub only when the U9/U10 caller lands - [manager_backend.rs:81, manager_backend.rs:208, manager_backend.rs:227]
  evidence: census: `wire_revision` over packages = 2 hits (both in manager_backend.rs), `api_subject` = 2 in-crate hits plus one doc mention (d2b-resource-runtime/src/manager.rs:82) and one test hit, `resource_owner_subject` = 1 test hit (manager_backend/tests.rs:1433); `nix_bundle_subject` and `manager_row_stored` are wired (d2bd/src/resource_plane_v3.rs:3445, d2b-provider-wayland-policy/src/effects_service.rs:627) and stay pub. watch.rs (`WatchFrame`/`WatchSink`) is the recorded B1 kept half (U1 (d) 6 refusal: `d2b-resource-api/src/watch.rs`) - cited, not re-flagged.

## err
- d2b-resource-api-p1#6 sev=low blast=leaf effort=S verdict=actionable - an empty batch is rejected with the reason "batch mutation count exceeds its bound", misstating the failure (empty is not over-bound) - fix: use a distinct reason such as "batch mutation count is zero" for the empty case and keep the bound reason for the `MAX_BATCH_MUTATIONS` check - [service.rs:867-868]
  evidence: seed `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 7 (6 test-only `unreachable!` in adapter.rs, 1 guarded `unreachable!` at manager_backend.rs:561); the reason string is wire-visible but not pinned in docs/reference/error-codes.md (0 hits for the string), so the kind stays the contract.
- clean: seed `\.unwrap\(\)|\.expect\(` = 39 (11 outside tests); every non-test expect names a real invariant (`row_uid` shape, validated manager keys, nonzero generations, non-empty truncated page, nibble range, authenticated Zone ref, bounded collection product) and no unwrap sits on input-derived data - acceptable per the panic-policy audit. seed `let _ = |\.ok\(\);` = 3 (one doc-comment line, two deliberate grant discards after authorization checks at service.rs:434 and service.rs:470).

## serde
- d2b-resource-api-p1#7 sev=medium blast=leaf effort=S verdict=actionable - `render_envelope` parses the row's metadata with `serde_json::from_slice(metadata).unwrap_or(serde_json::Value::Null)`, silently rendering a degraded envelope (empty annotations, epoch fallback timestamps, provenance-derived managedBy) when the metadata is malformed, while every other parse in the file fails closed with `envelope_invalid()` - fix: map the parse error to `envelope_invalid()` like the sibling parses so a corrupt row surfaces as a schema error instead of an invisible degraded read - [manager_backend.rs:744-745]
  evidence: seed `serde_json::from_|serde_json::to_` = 10; the manager_backend.rs:744-745 parse is the only non-fail-closed parse in the file (compare stamp_envelope, extract_metadata, apply_finalizers, project_resource, reseal_envelope); the sibling `rendered_owner_ref` `.ok().and_then` at manager_backend.rs:336-339 is a deliberate documented fallback and is not flagged.

## obs
- d2b-resource-api-p1#8 sev=low blast=leaf effort=S verdict=actionable - `parse_create_payload` writes an unstructured `eprintln!` to daemon stderr on every failed create-envelope validation, bypassing the tracing pipeline (no level, no filter, no named fields) from a library crate - fix: replace with `tracing::debug!(error = %error, "create envelope validation failed")` (the failure is already returned to the caller as `ResourceSchemaInvalid`) - [service.rs:1992]
  evidence: seed `\bprintln!\(|\beprintln!\(` = 1 (service.rs:1992); seed `tracing::|log::` = 1, the one event in the lane (manager_backend.rs:194) is already structured with a named field (`failure = %other`), so this is the only unstructured emission.

## docs
- d2b-resource-api-p1#9 sev=medium blast=leaf effort=M verdict=actionable - pub `Result`-returning items carry no `# Errors` sections stating which conditions produce which failure: the `ResourceService` constructors (StoreBindingError cases), `ResourceStoreBackend` methods (StoreError classes), the frame helpers (`attach_scoped_commit_frame`/`attach_scoped_query_frame`/`reject_scoped_commit_frame`), `manager_row_stored`, and `admit_guest_lifecycle` - fix: add `# Errors` sections naming the failure classes (binding already taken, invalid frame, unsupported capability, envelope invalid) - [service.rs:198, store.rs:39, adapter.rs:71, manager_backend.rs:625]
  evidence: seed `-> Result<` = 45; none of the pub Result-returning items in the lane has a `# Errors` section (seed `/// # (Examples|Errors|Panics|Safety)` = 0 in the lane).
- d2b-resource-api-p1#10 sev=low blast=leaf effort=M verdict=actionable - several pub items have no doc comment at all: `TrustedRequest::request`, `ResourceService::new`, the thirteen RPC forwarding methods on `ResourceApiClient` (client.rs:45-135) and `ResourceService` (service.rs:503-1115), and the `ScopedCommitFrameError`/`ScopedQueryFrameError` variants - fix: add one-line first sentences, linking docs/reference/daemon-api.md where the wire contract lives - [service.rs:70, service.rs:198, client.rs:45, adapter.rs:32-56]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 42; the undocumented items above are the gap; the RPC contract is documented in docs/reference/daemon-api.md, so a link suffices and no missing_docs lint is proposed.

## perf
- d2b-resource-api-p1#11 sev=low blast=leaf effort=S verdict=actionable - `encode_list_cursor` hex-encodes each cursor key byte with a per-byte `format!("{byte:02x}")` allocation, and duplicates the hex encoder already present as the local `hex` closure in `list_selector_digest` - fix: extract one `fn hex(bytes: &[u8]) -> String` (with `String::with_capacity(bytes.len() * 2)` and `write!`/`char::from_digit`) and call it from both sites - [manager_backend.rs:495, manager_backend.rs:449-455]
  evidence: static (unmeasured); seed `format!\(` = 20, the per-byte loop at manager_backend.rs:495 is the only format-in-loop site in the lane (cursor encoding runs on every truncated LIST page).
- d2b-resource-api-p1#12 sev=medium blast=leaf effort=S verdict=actionable - `commit_mutation` clones the full canonical resource (up to 256 KiB) on every UpdateSpec/UpdateMetadata before the byte-identical no-op check, so a no-op update pays the whole copy - fix: compare `mutation.canonical_resource.as_deref() == Some(row.spec.as_slice())` first and return the committed view early, cloning only when the bytes actually differ - [manager_backend.rs:1006]
  evidence: static (unmeasured); `MAX_RESOURCE_ENVELOPE_BYTES = 256 * 1024` (packages/d2b-contracts-resource/src/v3/limits.rs:5); the clone at manager_backend.rs:1006 runs on the per-mutation hot path before the documented no-op short-circuit at manager_backend.rs:1008-1014.
- d2b-resource-api-p1#13 sev=medium blast=family effort=M verdict=actionable - `owner_key_for` resolves a mutation's owner by listing the entire Zone row set (`manager.list(ResourceSelector::default())`) and linear-searching for the owner uid, on every Delete and every owner-less UpdateSpec/UpdateMetadata/UpdateFinalizers - fix: expose a manager-side uid-to-key lookup on `ResourceManagerClient` (d2b-resource-runtime) or return the owner key from `get_row`, and call it instead of the full-zone list - [manager_backend.rs:1081-1103, manager_backend.rs:1090]
  evidence: static (unmeasured); the full-zone list at manager_backend.rs:1090 is called from `owner_for_update` (manager_backend.rs:1065) and the Delete arm (manager_backend.rs:1046) on every mutation that does not carry an explicit owner; the doc comment claims the manager's uid index resolves the owner, but the implementation re-derives it by scanning all rows.

## conc
- clean: seeds `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 15, `Atomic\w+|Ordering::` = 12, `thread_local!|unsafe impl (Send|Sync) for` = 0; every Mutex and atomic hit is test-only (`tokio::sync::Mutex` fakes, SeqCst counters in `FakeStore`/`RecordingStore`), production code in the lane has no locks, threads, or atomics.

## async
- clean: seeds `async fn|async move|\.await` = ~130, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 0, `tokio::sync::(Mutex|RwLock|Notify)` = 15 (all test fakes), `#\[tokio::(main|test)\]|Runtime::block_on` = 14 (all `#[tokio::test]`); no blocking calls in async context, no guards held across `.await` in production code, and both async traits (`ResourceStoreBackend`, `UpgradeDispatcher`) use RPITIT with `+ Send` bounds.

## unsafe
- N/A: seeds `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0, `// SAFETY:` = 0, `transmute|from_raw|MaybeUninit|mem::zeroed` = 0; the crate manifest forbids unsafe (`unsafe_code = "forbid"` at packages/d2b-resource-api/Cargo.toml:7), so the lens is not applicable.

## ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0; no FFI surface in the lane.

## macro
- d2b-resource-api-p1#14 sev=low blast=leaf effort=S verdict=actionable - `response_error!` generates thirteen identical one-line functions that differ only in the response type, a case a generic function covers without a macro - fix: replace the macro with `fn error_response<T: protobuf::Message>(error: ResourceError) -> T` (type inferred from each RPC method's return type) and delete the thirteen `response_error!` invocations - [service.rs:2245-2267]
  evidence: seed `macro_rules!` = 3 (service.rs:1262, service.rs:1322, service.rs:2245); `impl_mutation_request!` and `impl_strict_mutation_request!` are genuine impl-per-type generation (one of the skill's three legitimate answers) and are not flagged; seed `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0.

## test
- d2b-resource-api-p1#15 sev=low blast=leaf effort=S verdict=actionable - `status_owner_matching_generation_is_representable` asserts only that `ControllerGeneration::new(11)` and `ResourceGeneration::new(11)` succeed on literals, restating the type system rather than a behavior contract - fix: delete it, or convert the representability claim into the wire-compatibility test it is meant to document (asserting the status-owner comparison path with a real mismatch) - [service.rs:3377]
  evidence: seed `#\[test\]|#\[tokio::test\]` = 23, `assert_eq!\(|assert_ne!\(|assert!\(` = ~90; the test body (service.rs:3377-3384) contains no behavior under test; the suite is otherwise behavioral (dispatch sentinels, redaction markers, authorization-before-validation ordering, byte-bound enforcement).

## Coverage
- idiom: clean (seeds: 1/0/2; only non-test hit is the cursor build covered by perf finding 11; hand-written Clone/Debug impls deliberate)
- own: 1 finding(s)
- type: 1 finding(s)
- api: 3 finding(s); watch.rs kept-half (B1) refusal cited per U1 (d) 6, not re-flagged
- err: 1 finding(s)
- serde: 1 finding(s)
- obs: 1 finding(s)
- docs: 2 finding(s)
- perf: 3 finding(s)
- conc: clean (seeds: 0/15/12/0; all hits test-only fakes and counters)
- async: clean (seeds: ~130/0/15/14; no blocking, no guards across await, Send bounds on both async traits)
- unsafe: N/A (seeds: 0/0/0 all zero; manifest `unsafe_code = "forbid"`)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: 1 finding(s)
- test: 1 finding(s)