# d2b-provider-volume - d2b-provider-volume
Baseline: 6ebdd4cec | LOC audited: 2,135 (excl. src/generated - none present) | modules: whole crate (driver, effects_service, facets, lib, test_support)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate

## idiom
- d2b-provider-volume#1 sev=low blast=leaf effort=S verdict=actionable - `reconcile` converts the provider facet via `serde_json::to_value(value).unwrap_or(serde_json::Value::Null)` where the value is already a `serde_json::Value`: a serialization round trip plus dead `unwrap_or` fallback for an infallible conversion - fix: replace with `envelope.base.get("provider").cloned()` - [driver.rs:607-609]
  evidence: idiom seeds 0/0/0 (index loops, hand impls, statement accumulation absent) + static read of the site;`to_value::<Value>` on a `&Value` is a deep copy the value's own `Clone` already performs, so the serialization path adds only a dead `Result`.
- clean: seeds `for \w+ in 0\.\.`=0, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for`=0, `let mut \w+ = (String|Vec)::new\(\)`=0; the crate's loops iterate (reconcile_children over `desired`, tests over rows), no hand-written derive-replaceable impls, no statement-style accumulators.



## own
- d2b-provider-volume#2 sev=low blast=leaf effort=S verdict=actionable - `decoded_spec` returns `(envelope.clone(), spec)` though the local `envelope` is never used after the clone:an avoidable `Vec<u8>` raw-spec copy on every driver op (validate, recover, reconcile, delete) - fix: return `(envelope, spec)` directly - [driver.rs:337]
  evidence: own seed `\.clone\(\)` = 35 over src; this site is the only redundant clone outside cfg(test)/test-support (redundant_clone-class; the envelope's later borrow (building `spec`) ends before the return, so ownership can move).
- d2b-provider-volume#3 sev=low blast=wide effort=M verdict=actionable - `desired_binding_intents` takes `ResourceRef` by value though it only reads it (cloning into each `BindingIntent` internally), so every production caller must clone first: driver.rs:380 and d2bd/src/resource_runtime.rs:5882,12921 - fix: change the signature to `&ResourceRef` in `d2b-provider-volume-local/src/bindings.rs:80`, drop the caller clones (callers pass `&volume_ref`) - [driver.rs:380, d2b-provider-volume-local/src/bindings.rs:80-81, d2bd/src/resource_runtime.rs:5882,12921]
  evidence: own seed `\.clone\(\)` = 35 over src + census `desired_binding_intents` over packages/ = 11 hits (3 production call sites + 8 volume-local test sites); the callee stores owned refs into each intent, so taking the arg by value buys nothing over a borrow.

- clean: seeds `\.clone\(\)`=35, `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)`=28, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<`=0, `Cow<`=0; remaining production clones are required moves into owned values (`ChildEnsure`, spawned-task `Arc`, daemon-facing facet clone), test-support recorder clones are exempt.



## type
- clean: seeds `fn validate_\w+|fn check_\w+`=1, `is_\w+: bool|\w+_flag: bool`=0, `(mode|kind|state): String`=0; the one gate (`check_provider`) must stay runtime glue: folding Provider support into the envelope decode would merge the two wire error codes `VOLUME_SPEC_INVALID` and `VOLUME_PROVIDER_UNSUPPORTED` that `VolumeDriverErrorKind::failure_kind` deliberately keeps apart (issue #508); no boolean/Option-pair/stringly-typed states.



## api
- d2b-provider-volume#4 sev=medium blast=wide effort=S verdict=actionable - `VolumeDriverArgs.zone: String` is never read by the factory, descriptor, or driver (`create` clones it into a throwaway args before `VolumeDriver::new` drops it; the derived rows' zone comes from the manager-keyed `ResourceContext`, so the doc's "zone identity every derived row folds in" claim has no code path) - fix: remove the field from `VolumeDriverArgs` (and its `lib.rs` re-export), drop the clone at driver.rs:282, update construction sites `d2bd/src/resource_plane_v3.rs:2975-2977` and `tests/registration.rs:25` (plus this crate's test fixtures)) - [driver.rs:246-250,282, d2bd/src/resource_plane_v3.rs:2975, tests/registration.rs:25]
  evidence: census `VolumeDriverArgs` over packages/ = 14 hits (import/construction sites only; the 3 production construction sites all pick a real zone string the plane already knows); full-file static read of driver.rs shows no `args.zone` read anywhere in the crate; the sibling credential lane records the same args-zone pattern as an X3 candidate (d2b-provider-credential#2), but there the zone is actually consumed at use sites - here it is dead.



- clean: seeds `\bpub (fn|struct|enum|trait|type|const|mod) `=21, `pub .*\b(Arc|Rc|Box|RefCell)<`=5, `^\s*pub use `=3; surface is a single path via lib.rs re-exports (house pattern); the Arc-in-signature hits are deliberate, documented shared-ownership shapes:the factory's `Arc<dyn SpecDecoder>` return matches sibling decoders in process/host/user/guest crates,and `VolumeEffectFacets.runtime` is the daemon-supplied facet set shared by the driver effects and the hosted service (U7); no leaked dependency types in public signatures.



## err
- clean: seeds `\.unwrap\(\)|\.expect\(`=35, `let _ = |\.ok\(\);`=4, `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\\(`=4, `enum \w*Error`=1; all unwrap/expect hits are in cfg(test) modules or test-support doubles(`RefusingRuntime` panics loudly by design); the only non-test `let _ =` is driver.rs:516 on a fire-and-forget completion send (an unbounded-channel send to a possibly-dropped actor mailbox, no caller remains to notify);`VolumeDriverErrorKind` is a private 6-variant taxonomy split by caller action with `class()`/`failure_kind()` mappings (R13, issue #508).



## serde
- clean: seeds `derive\([^)]*(De)?[Ss]erialize`=0, `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)`=0, `impl .*Deserialize.*for`=0, `serde_json::from_|serde_json::to_`=8; the 8 hits are boundary reads/writes over contract-crate types (`ResourceSpec`, `VolumeSpec`, fixtures), no serde attrs or hand-written deserializers in this crate, validation lives in the typed decoder plus `check_provider` (runtime gate).





## obs
- N/A: seeds `\bprintln!\(|\beprintln!\\(`=0, `(info|debug|warn|error|trace)!\(`=0, `\.instrument\(|#\[instrument`=0, `tracing::|log::`=0; crate has no tracing/log dependency, so there is no telemetry to judge.



## docs
- clean: seeds `^\s*pub (fn|struct|enum|trait|const|type)`=20, `^\s*/// # (Examples|Errors|Panics|Safety)`=0, `-> Result<`=35;`#![deny(missing_docs)]` is active in lib.rs:5, all 20 pub items carry first-sentence docs, VOLUME_RESYNC's 30-second magic is documented with the why (driver.rs:65-77), the 4 Result-returning pub trait methods document what the bool/unit contract reports (the `String` error is an opaque note the caller passes through, not a match surface, so `# Errors` would narrate nothing); no `# Examples` needs (`VolumeRuntime` has no doctests andits use is composition-root wiring, documented therein).



## perf
- d2b-provider-volume#5 sev=low blast=leaf effort=S verdict=actionable - `reconcile` performs two identical `ctx.children()` manager round-trips per pass:`reconcile_children` already fetched the owned child set after ensures (to retire obsolete),andthen `reconcile` re-fetches the same set to compute `converged` - an extra manager RPC per reconcile pass - fix: have `reconcile_children` return the fetched `Vec<StoredDesiredResource>` (or compute the converged verdict inside)and consume it there - [driver.rs:437-440,614-617]
  evidence: static (unmeasured);`ctx.children()` routes to `self.ager.list_owned)...).await` (d2b-resource-runtime/src/context.rs:586-588), a per-call manager RPC; between the two calls no other actor can mutate this owner's rows (driver-owned children only, row actor is single-threaded), so the second fetch returns identical data.



- clean: seeds `format!\\(`=5, `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)`=7, `\.to_string\(\)`=1; all hits are cfg(test) helpers (`format!` recording keys in RecordingManager, malformed-spec fixture bytes) or required empty field defaults (`ChildEnsure.metadata`), no hot-path allocation sites.



## conc
- clean: seeds `std::thread::|thread::spawn|thread::scope`=0, `\bMutex<|\bRwLock<`=3, `Atomic\w+|Ordering::`=18, `thread_local!|unsafe impl (Send|Sync) for`=0; the 3 Mutex hits are test-support recorders (`RecordingRuntime.calls`, `RecordingManager.log/rows`) sanctioned with `async-gate-allow: test-support recorder lock` markers and cfg(test)-helper allows; the AtomicBool flags use SeqCst deliberately (they publish a layout state read once per 30-s-cadence pass,and the cost is negligible per the skill's "SeqCst when unsure"), no threads are spawned by this crate (task concurrency belongs to async lens).





## async
- clean: seeds `async fn|async move|\.await`=100, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\\(`=1, `tokio::sync::(Mutex|RwLock|Notify)`=0, `#\[tokio::(main|test)\]|Runtime::block_on`=13; the single `tokio::spawn` (driver.rs:512-535) is the documented layout-effect spawn (R5/KTD12: mailbox never blocks; completion arrives as `EffectCompleted` and a degraded report flows into the actor's retryable requeue), no guards are held across awaits in src (`tokio::sync` unused), the trait bounds `Send + Sync + 'static` make the spawned future Send-safe, the send on the unbounded channel is non-blocking so the irreversible step cannot be lost to cancellation.



## unsafe
- N/A: seeds `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern`=0, `// SAFETY:`=0, `transmute|from_raw|MaybeUninit|mem::zeroed`=0, `unsafe_code`=0 (over src\); crate manifest forbids `unsafe_code`, so no unsafe sites exist.



## ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section`=0, `catch_unwind`=0, `repr\(C\)|repr\(transparent\)`=0, `CStr|CString|c_char`=0; the crate crosses no foreign boundary.



## macro
- N/A: seeds `macro_rules!`=0, `proc_macro|syn::|quote!`=0, `\$crate`=0, `to_compile_error|new_spanned`=0; no macro definitions or proc-macro usage (std macros only).



## test
- clean: seeds `#\[test\]|#\[tokio::test\]`=18 (14 src + 4 tests/registration.rs), `assert_eq!\(|assert_ne!\(|assert!\\(`=67, `proptest!|insta::assert|rstest`=0, `#\[ignore\]`=0; tests are behavioral throughout: effect-order and commit-before-spawn (F1), deterministic child identity with no churn, adoption re-validates layout idempotently, degraded layout reports exactly one retryable failure per pass, finalize-before-delete drain ordering, idempotent delete retryarry, wire-pinned canonical payload bytes (`the_has_layout_wire_payloads_are_canonical`), error variants asserted via `matches!`/eq on the enum not Display strings, registration tests pin the declaration's verbs/creations/services against human-written expectations; no property/snapshot tooling is needed for this scale (unit + integration coverage is complete for the flows named), no ignored tests.



## Coverage
- idiom: 1 finding
- own:  2 finding(s)
- type: clean (seeds ran: 1/0/0; the one gate is deliberate to keep the two wire error codes distinct)
- api:  1 finding
- err: clean)(seeds ran: 35/4/4/1; all panics are in tests/test-support; the alone `let _ =` is a fire-and-forget completion send)
- serde: clean)(seeds ran: 0/0/0/8; boundary reads over contract-crate types only)
- obs: N/A)(seeds ran: 0/0/0/0; no tracing/log dep)
- docs: clean)(seeds ran: 20/0/35;`deny(missing_docs)` active and pub items documented)
- perf:  1 finding
- conc: clean)(seeds ran: 0/3/18/0; test-support recorders + deliberate SeqCst flags)
- async: clean)(seeds ran: 100/1/0/13; single documented effect spawn; no guards across awaits)
- unsafe: N/A)(seeds ran: 0/0/0; no unsafe sites;`forbid` in manifest)
- ffi: N/A)(seeds ran: 0/0/0/0)
- macro: N/A)(seeds ran:  0/0/0/0)
- test: clean)(seeds ran: 18/67/0/0 incl. tests/; behavioral unit+registration suite, no ignored/property tests)