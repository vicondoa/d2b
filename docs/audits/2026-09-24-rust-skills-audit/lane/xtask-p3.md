# xtask-p3 - xtask - part 3/5
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 8936 (excl. src/generated/**) | modules: delivery/recovery, delivery/command, delivery/snapshot, resource_type_authority, provider_packaging, semantic_service_schemas, deadcode, authority_common, bin/manifest_v04_check
Lenses: idiom own type api err serde obs docs perf conc async unsafe ffi macro test | Partitions: part 3/5 (U1 section f: the nine files above)

## idiom
- xtask-p3#1 sev=low blast=leaf effort=S verdict=actionable - resource_type_authority.rs carries misindented statements (`errors.push(format!(` at column 0, `out.push_str("// @generated\n");` at column 0, `fn drop` under-indented by 4) that rustfmt would reflow; the repo runs no fmt gate, so the drift is committed - fix: reindent the statements at the three sites (or run rustfmt over the file once) - [packages/xtask/src/resource_type_authority.rs:704, packages/xtask/src/resource_type_authority.rs:940, packages/xtask/src/resource_type_authority.rs:1083]
  evidence: idiom seed 3 (`let mut \w+ = (String|Vec)::new\(\)`) 15 hits, each hit neighborhood read; the misindented statements were found while reading the seed-3 hits
- xtask-p3#2 sev=low blast=family effort=S verdict=needs-contract - generator string literals in resource_type_authority.rs drop spaces, so the committed generated artifact header reads "Provenance:emitted", "; the layout check's" and "byte-for-byte,and", and the type-declared-twice diagnostic reads "declared by both {}and {}" - fix: restore the spaces in the four push_str literals and the format string, then regenerate the artifact via `cargo xtask check-provider-crate-layout --fix` so the drift gate and the committed `v3_converted_resource_types.rs` move together - [packages/xtask/src/resource_type_authority.rs:704, packages/xtask/src/resource_type_authority.rs:940, packages/xtask/src/resource_type_authority.rs:941, packages/xtask/src/resource_type_authority.rs:942, packages/d2b-contracts/src/generated/v3_converted_resource_types.rs:2]
  evidence: idiom seed 3 15 hits; the emitted strings were confirmed byte-identical in the committed generated artifact (the drift gate would reject a mismatch), so the fix touches src/generated/** and is needs-contract per U1 section d.7

## own
- xtask-p3#3 sev=low blast=leaf effort=S verdict=actionable - `nix_string_list` takes `impl IntoIterator<Item = String>`, forcing every caller to `.to_owned()` its `&'static str` fields at seven call sites only to borrow them again inside `nix_string` - fix: change the signature to `impl IntoIterator<Item = &'a str>` (or `&[&str]`) and delete the `.map(|field| (*field).to_owned())` closures at the call sites - [packages/xtask/src/provider_packaging.rs:163, packages/xtask/src/provider_packaging.rs:206, packages/xtask/src/provider_packaging.rs:222, packages/xtask/src/provider_packaging.rs:230, packages/xtask/src/provider_packaging.rs:242, packages/xtask/src/provider_packaging.rs:252, packages/xtask/src/provider_packaging.rs:263, packages/xtask/src/provider_packaging.rs:278]
  evidence: own seed 2 (`\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)`) 168 hits lane-wide; 7 of the provider_packaging.rs hits are nix_string_list call sites (signature read in full)
- xtask-p3#4 sev=low blast=leaf effort=S verdict=actionable - `resource_ref_schema(pattern: String, allowed_types: &[String])` forces `.to_owned()`/`String::from` at every call site, including static regex literals that never need an owned String - fix: change the signature to `pattern: &str` and `allowed_types: &[&str]` (both serialize into `json!` unchanged) and drop the conversions at the call sites - [packages/xtask/src/semantic_service_schemas.rs:32, packages/xtask/src/semantic_service_schemas.rs:44, packages/xtask/src/semantic_service_schemas.rs:55, packages/xtask/src/semantic_service_schemas.rs:61, packages/xtask/src/semantic_service_schemas.rs:74, packages/xtask/src/semantic_service_schemas.rs:155, packages/xtask/src/semantic_service_schemas.rs:203]
  evidence: own seed 2 168 hits lane-wide; 7 hits in semantic_service_schemas.rs are signature-forced allocations (function and call sites read in full)

## type
- clean: type seeds 11/0/9 - seed 1 (`fn validate_\w+|fn check_\w+`) 11 hits are the recovery attestation admission gate (`validate_shape`/`validate_binding`/`validate_at` family, each a parse-once check at the decode/consumption boundary of a deny_unknown_fields wire type) plus the CLI gate `check()`; seed 3 (`(mode|kind|state): String`) 9 hits are all `artifact_kind: String` wire-record fields mirroring the pinned recovery schema; both classes are the card's recorded wire-type false positives. No boolean-flag soup, stringly-typed state, or validate-at-every-callsite duplication beyond the deliberate wire admission.

## api
- clean: api seeds 171/0/0 - seed 1 (`\bpub (fn|struct|enum|trait|type|const|mod) `) 171 hits, seed 2 (`pub .*\b(Arc|Rc|Box|RefCell)<`) 0, seed 3 (`^\s*pub use `) 0. xtask is a bin-only crate (no `[lib]` target in packages/xtask/Cargo.toml), so the `pub` items are internal wiring consumed by `main.rs`, not an exported surface; over-broad `pub` visibility is already policed by the crate's own dead-code gate (`deadcode.rs` runs `cargo hawk check` for `pub` -> `pub(crate)` reductions).

## err
- xtask-p3#5 sev=medium blast=leaf effort=S verdict=actionable - `RecoveryError::Json` conflates three failure modes: an unreadable attestation file (`read_attestation` maps open/read errors to Json), canonical-JSON rejection, and a typed-parse failure whose serde detail (missing field, line, column) is discarded, so an operator debugging a rejected attestation sees only "recovery attestation shape rejected" with no way to tell a missing file from a malformed payload - fix: add a `RecoveryError::Read` variant for the fs errors and carry the bounded serde error text (field names and positions only, never payload values, keeping the enum's redaction contract) in a `Json(String)` variant, propagating through the existing `From<RecoveryError> for DeliveryError` - [packages/xtask/src/delivery/recovery.rs:384, packages/xtask/src/delivery/recovery.rs:1610, packages/xtask/src/delivery/recovery.rs:1715, packages/xtask/src/delivery/recovery.rs:1719]
  evidence: err seed 1 (`\.unwrap\(\)|\.expect\(`) 239 hits, sampled: 50 of 239 (every sampled hit is cfg(test) code or a named-invariant expect on internal catalog data); seed 2 (`let _ = |\.ok\(\);`) 4 hits (all deliberate: Drop cleanup, infallible `write!` to String, test cleanup); seed 3 (`\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(`) 3 hits (test helpers and the fail-closed golden arm); seed 4 (`enum \w*Error`) 1 hit (`RecoveryError`, read in full)

## serde
- xtask-p3#7 sev=low blast=leaf effort=S verdict=actionable - `DeclarationFile` and `TypeDeclaration` use per-field `#[serde(rename = ...)]` for their camelCase wire keys while the sibling `RoleDeclaration` in the same file uses `#[serde(rename_all = "camelCase")]`, splitting the boundary-naming convention within one file - fix: add `#[serde(rename_all = "camelCase")]` to `DeclarationFile` and `TypeDeclaration` and delete the two per-field renames - [packages/xtask/src/resource_type_authority.rs:179, packages/xtask/src/resource_type_authority.rs:182, packages/xtask/src/resource_type_authority.rs:190, packages/xtask/src/resource_type_authority.rs:192, packages/xtask/src/resource_type_authority.rs:204]
  evidence: serde seed 2 (`serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)`) 37 hits; the three declaration structs read in full (seeds 1/3/4: 26/0/32 hits, all derived wire types and boundary calls)

## obs
- clean: obs seeds 15/0/0/0 - seed 1 (`\bprintln!\(|\beprintln!\(`) 15 hits are CLI product output and gate diagnostics (`deadcode.rs` eprintln report lines, `bin/manifest_v04_check.rs` usage/error lines), the card's recorded carve-out; seeds 2-4 0 hits (no interpolated events, no spans, no tracing/log dependency). No telemetry surface exists in this scope to judge.

## docs
- xtask-p3#6 sev=low blast=leaf effort=S verdict=actionable - several pub items in the delivery modules lack the doc comment the modules' own discipline gives every sibling item: `WaveSnapshot::digests`/`program`/`wave`, `WaveCommand::as_str`/`parse`/`required_options`/`optional_options`, `WorkflowOutput::ok`/`with_digests`, `WorkflowCommandHelp`, and the `CliOptions` accessors - fix: add one-line doc comments naming each contract (mirroring the sibling wording already present) - [packages/xtask/src/delivery/snapshot.rs:87, packages/xtask/src/delivery/snapshot.rs:95, packages/xtask/src/delivery/snapshot.rs:99, packages/xtask/src/delivery/command.rs:104, packages/xtask/src/delivery/command.rs:116, packages/xtask/src/delivery/command.rs:205, packages/xtask/src/delivery/command.rs:234, packages/xtask/src/delivery/command.rs:425, packages/xtask/src/delivery/command.rs:440, packages/xtask/src/delivery/command.rs:465]
  evidence: docs seed 1 (`^\s*pub (fn|struct|enum|trait|const|type)`) 153 hits; the listed items were confirmed doc-less while reading each module's pub surface (seed 2 `/// # (Examples|Errors|Panics|Safety)` 0 hits; seed 3 `-> Result<` 73 hits)

## perf
- clean: perf seeds 138/37/5 - seed 1 (`format!\(`) 138 hits, seed 2 (`Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)`) 37 hits, seed 3 (`\.to_string\(\)`) 5 hits; every hit is a generator building a text artifact (the card's recorded xtask carve-out), a cold error/diagnostic path, a bounded artifact read, or a test. No hot loop allocates; no collection choice is wrong for its access pattern; no attacker-controlled hashing. Perf claims are static (unmeasured) per the card.

## conc
- N/A (seeds: 0/0/0/0 all zero; no threads, locks, atomics, or manual Send/Sync anywhere in the assigned files - the lane is single-threaded CLI/generator code)

## async
- N/A (seeds: 0 all zero; no async fn, await, tokio, or block_on in the assigned files - the lane is synchronous CLI/generator code)

## unsafe
- N/A (seeds: 0/0/0/0 all zero; no unsafe blocks, fns, impls, SAFETY comments, or transmute/from_raw/MaybeUninit sites; the crate manifest carries `unsafe_code = "forbid"`, and seed 4 alone does not make the lens applicable per the card)

## ffi
- clean: ffi seeds 0/1/0/0 - seed 2 (`catch_unwind`) 1 hit at command.rs:1792, a `#[test]` asserting `golden_fingerprint` fails closed for an unpinned schema version; it is a panic-behavior assertion, not a foreign boundary, so no FFI surface exists to judge (seeds 1/3/4 are 0).

## macro
- clean: macro seeds 1/0/0/0 - seed 1 (`macro_rules!`) 1 hit: `workflow_status!` (command.rs:317), a list-driven enum/wire-string/ALL-domain generator - the skill's genuine "impl-per-type generation from a list" answer; it uses the narrow `$meta:meta` fragment, needs no `$crate` (no crate paths in the expansion), and its doc comment states the drift rationale. Seeds 2-4 are 0 (no proc macros).

## test
- xtask-p3#8 sev=low blast=leaf effort=S verdict=actionable - `workflow_status_all_enumerates_every_variant` and `wave_commands_enumerates_every_stage` assert `ALL.contains(status)` for every status drawn from `ALL` itself, so the runtime assertion is tautological and can never fail; the real guard is the wildcard-free match's compile-time exhaustiveness, which the assert adds nothing to - fix: drop the `assert!` and keep the wildcard-free match (the compile-fail property), or assert a property not derived from the same enumeration - [packages/xtask/src/delivery/command.rs:1059, packages/xtask/src/delivery/command.rs:1079]
  evidence: test seed 1 (`#\[test\]|#\[tokio::test\]`) 174 hits, seed 2 (`assert_eq!\(|assert_ne!\(|assert!\(`) 501 hits; both tests read in full (their own comments document the compile-time intent)

## Coverage
- idiom: 2 finding(s)
- own: 2 finding(s)
- type: clean (seeds ran: 11/0/9; all hits are wire admission gates, the CLI gate, and schema-mirroring artifact_kind fields per the card's false positives)
- api: clean (seeds ran: 171/0/0; bin-only crate with no lib target, pub items are internal wiring, cargo-hawk gate polices visibility)
- err: 1 finding(s)
- serde: 1 finding(s)
- obs: clean (seeds ran: 15/0/0/0; all eprintln/print hits are CLI product output and gate diagnostics per the card's carve-out)
- docs: 1 finding(s)
- perf: clean (seeds ran: 138/37/5; all hits are generator text building, cold error paths, bounded reads, or tests per the card's carve-outs)
- conc: N/A (seeds: 0/0/0/0 all zero; no threading primitives in scope)
- async: N/A (seeds: 0/0/0/0 all zero; no async code in scope)
- unsafe: N/A (seeds: 0/0/0/0 all zero; no unsafe sites; manifest `unsafe_code = "forbid"` only)
- ffi: clean (seeds ran: 0/1/0/0; the single catch_unwind is a test assertion, not a foreign boundary)
- macro: clean (seeds ran: 1/0/0/0; the one macro is the list-driven workflow_status! generator, a genuine macro use)
- test: 1 finding(s)