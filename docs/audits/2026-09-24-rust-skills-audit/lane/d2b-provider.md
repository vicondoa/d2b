# d2b-provider - d2b-provider
Baseline: 6ebdd4cec | LOC audited: 3,252 (incl. tests/runtime.rs 667; no src/generated/**) | modules: whole crate (agent, context, descriptor, error, identity, instance, lib, operation_ledger, registry, session; tests/runtime.rs)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate

## idiom
- clean: seeds ran: 0/2/0;; the two hand-written `Default` impls (operation_ledger.rs:152, registry.rs:76)preserve invariantsa field-wise derive would break (ledger capacity=MAX_OPERATION_LEDGER_ROWS; registry caps=256/32) - the U1-listed deliberate class;; no index loops,no statement-style accumulation in src/**

## own
- d2b-provider#1 sev=low blast=leaf effort=S verdict=actionable - `ProviderAgent::dispatch` clones the full canonical-JSON request per dispatch (agent.rs:290)even though only `request.method` and `request.timeout_ms` are used after the `timeout`, both Copy - fix: extract `let method = request.method;` before the `timeout)...)`, move `request` into `self.service.dispatch)...)` instead of `request.clone()`, and use `method` in the audit record - [packages/d2b-provider/src/agent.rs:290, packages/d2b-provider/src/agent.rs:299-304]
  evidence: `\.clone\(\)` = 26 hits over src/**,all other 25 reviewed as required shared-ownership or owned-wrapper clones (Arc<Semaphore>, Arc<RegistryInner>, watch Sender, cancellation tokens, instance/descriptor/subject clones into owned values);`SpecifiedProviderMethod` derives Copy (d2b-contracts-provider/src/v3/provider.rs:2758-2763),so the partial move compiles

## type
- clean: seeds ran:  0/0/0;; no `validate_`/`check_` fns,no bool flags,no string-typed state;;`RegistryLimits::validate`/`RegistryDrainPolicy::validate` run at their consuming boundaries (builder.limits, shutdown/publish),so there is no validate-at-every-callsite spread to encode away

## api
- clean: seeds ran: ~163/1/6;; single-surface `pub use` house pattern (lib.rs:40-65),private module tree;; the one `Arc<ProviderRegistry<I>>` return (registry.rs:618 `current()`)is justified shared ownership:callers hold the generation Arc across awaits while `ProviderRegistryManager::publish` swaps it (registry.rs:626-648; U1-listed evaluation class); no `Rc`/`Box`/`RefCell` in pub signatures

## err
- clean: seeds ran:  12/0/0/4;; all 12 `.unwrap()`/`.expect(` sites are inside `#[cfg(test)]` modules (agent.rs:349-403,registry.rs:726-727,U1-listed acceptable class); no swallowed Results,no panic macros;; the four error enums are closed Copy code-book types printing kebab wire codes (ProviderAgentError, RegistryBuildError, ProviderRuntimeError, OperationLedgerError);; agent's HandlerFailed/DispatchTimeout mapping is a deliberate wire-boundary conversion,not a swallowed chain

## serde
- N/A: seeds ran:  0/0/0/0 all zero;; crate crosses no wire - no serde dependency in Cargo.toml,no serde attributes,no serde_json anywhere in src/**

## obs
- N/A: seeds ran:  0/0/0/0 all zero;; no `println!`/`eprintln!`,,no tracing/log macros,no instrument,no tracing/log dependency in Cargo.toml - the card's N/A criterion (all seeds zero and no tracing/log dep)holds

## docs
- d2b-provider#2 sev=medium blast=leaf effort=M verdict=actionable - Public Result-returning APIs carry no `# Errors` sections,naming which conditions produce which error variants - fix: add `# Errors` sections to the ~28 pub Result-returning fns (agent.rs:40,270; context.rs:63; descriptor.rs:55,108,196,232; identity.rs:98,120,142; instance.rs:28; operation_ledger.rs:179,195; registry.rs:64,99,329,412; session.rs:36,94),listing each reachable variant per fn - [packages/d2b-provider/src/agent.rs:270, packages/d2b-provider/src/descriptor.rs:232, packages/d2b-provider/src/registry.rs:412, packages/d2b-provider/src/session.rs:36]
  evidence: docs seed2 `/// # (Examples|Errors|Panics|Safety)` = 0 hits over src/**,while seed3 `-> Result<` = 33 hit sites;; surface doc coverage itself is enforced (`#![deny(missing_docs)]` at lib.rs:5),so the gap is doc-contract shape (canonical sections),not absence of docs

## perf
- clean: seeds ran:  0/3/0;; the 3 `BTreeMap::new()` sites are cold one-shot builders (operation_ledger.rs:172,184; registry.rs:261); the agent audit deque preallocates with `with_capacity` (agent.rs:226); no `format!`/`to_string` in src/**;; no hot-path allocation site identified statically

## conc
- clean: seeds ran:  0/1/~33/0;; the tokio `Mutex<VecDeque<ProviderAgentAuditEvent>>` (agent.rs:213)is held across no `.await` (agent.rs:257-262); Acquire/Release atomics with compare_exchange loops and documented lock-free rationale (registry.rs:446-448,546-560); the Notify drain handoff has a lost-wakeup regression test (registry.rs:698-727); no `std::thread`,no manual `Send`/`Sync` claims

## async
- d2b-provider#3 sev=medium blast=leaf effort=S verdict=actionable - `ProviderAgent::serve` awaits each `dispatch` serially (agent.rs:316-324):a slow handler near the 900s timeout bound (`MAX_AGENT_TIMEOUT_MS`)stalls the whole session queue,and the `MAX_AGENT_IN_FLIGHT=64` Semaphore bound can never be exceeded by the serve loop itself - fix: spawn each dispatch (`tokio::spawn(async move { let result = self.dispatch(request).await; let _ = response_tx.send(result.await; })`) with a cloned `response_tx`,letting the already-acquired Semaphore permit cap concurrency; state whether per-session response ordering is a contract) - [packages/d2b-provider/src/agent.rs:316-324]
  evidence: async seeds = 36/1/0/4 (seed2 hit: registry.rs:709 test `tokio::spawn`; seed4: 4 `#[tokio::test]` in src/**);`census: ProviderAgent over packages/; nixos-modules/; tests/; docs/reference/; labs/; BUILD.bazel = lib.rs re-export + toolkit `FakeProvider` impl (d2b-provider-toolkit/src/testing/fixture.rs:380)+ own tests,so the serialization defect is latent until a session wires the kept B2 dispatcher (not a removal proposal)

## unsafe
- N/A: seeds ran:  0/0/0 all zero;; no `unsafe` block/fn/impl/SAFETY site in src/**;;`unsafe_code` appears only as the inherited workspace `forbid` (Cargo.toml [lints] workspace = true),which is not a site per the card

## ffi
- N/A: seeds ran:  0/0/0/0 all zero;; no extern "C",no no_mangle,no repr(C)/repr(transparent),,no CStr/CString/c_char anywhere in src/**

## macro
- N/A: seeds ran:  0/0/0/0 all zero;; no `macro_rules!`,no proc-macro/syn/quote,no `$crate`,no compile-error machinery anywhere in src/**

## test
- clean: seeds ran:  26/75+/0/0;;26 tests (18 `#[test]` + 8 `#[tokio::test]`:4 in-agent/registry src tests,22 in tests/runtime.rs)assert behavior and error variants (never Display strings),pin the redaction contract (tests/runtime.rs:480-488),use hermetic fixed-value helpers (no network,no clock reads; drain tests await only the in-process Notify path); no `#[ignore]`,no proptest/insta/rstest

## Coverage
- idiom: clean (seeds ran:  0/2/0)
- own:  1 finding(s) (seeds ran:  26/0/0/0)
- type: clean (seeds ran:  0/0/0)
- api: clean (seeds ran: ~163/1/6)
- err: clean (seeds ran:  12/0/0/4)
- serde: N/A (seeds:  0/0/0/0 all zero; crate crosses no wire)
- obs: N/A (seeds:  0/0/0/0 all zero; no tracing/log dep)
- docs:  1 finding(s) (seeds ran: ~165/0/33)
- perf: clean (seeds ran:  0/3/0)
- conc: clean (seeds ran:  0/1/~33/0)
- async:  1 finding(s) (seeds ran:  36/1/0/4)
- unsafe: N/A (seeds:  0/0/0 all zero)
- ffi: N/A (seeds:  0/0/0/0 all zero)
- macro: N/A (seeds:  0/0/0/0 all zero)
- test: clean (seeds ran:  26/75+/0/0)