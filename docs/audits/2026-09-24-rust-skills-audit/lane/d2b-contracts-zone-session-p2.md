# d2b-contracts-zone-session-p2 - d2b-contracts-zone-session - part 2/2
Baseline: 6ebdd4cec | LOC audited: 6403 (excl. src/generated/**) | modules: v3/zone_routing.rs, v3/resource_bundle.rs, v3/zone_session.rs, v3/zone.rs, v3/role_binding.rs, v3/services.rs, v3/emergency_policy.rs
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 2/2 per U1 section f (7 files, no item-range splits)

## idiom
- clean: seeds 0/1/1 - the single hand-written Default (emergency_policy.rs:170) preserves the drain-deadline invariant a field-wise derive would break (card false positive), and the one Vec::new accumulation (zone_routing.rs:1554) is test code whose loop body asserts per item, where a plain loop is the right shape.

## own
- d2b-contracts-zone-session-p2#1 sev=low blast=leaf effort=S verdict=actionable - ZoneLinkRouteWithdrawal::new clones the entire route-id vec only to detect duplicates - fix: sort the owned vec in place and check windows(2), mirroring the crate's own dedup pattern in RoleBindingSpec::with_facets (role_binding.rs:273-276) - [zone_routing.rs:883]
  evidence: seed \.clone\(\) = 81 hits; this is the only non-test clone in a validation path, and the in-crate sort-plus-windows pattern at role_binding.rs:273-276 proves the clone is avoidable.
- d2b-contracts-zone-session-p2#2 sev=low blast=leaf effort=S verdict=actionable - reject_runtime_or_private_fields clones the whole spec object (object.clone().into_inner()) just to wrap it for the walk, on every BundleResource::new call - fix: make walk iterate the CanonicalJsonObject map directly (and a sibling fn for arrays) so no CanonicalJsonValue wrapper or clone is built - [resource_bundle.rs:944, resource_bundle.rs:149]
  evidence: seed \.clone\(\) = 81 hits; the clone is on the success path of every BundleResource::new (call site resource_bundle.rs:149), and walk only needs Object and Array cases it can take by reference.
- d2b-contracts-zone-session-p2#3 sev=low blast=leaf effort=S verdict=actionable - ServiceDescriptor::new clones each method String for BoundedText::parse, which takes impl Into<String> - fix: pass method.as_str() (String: From<&str> satisfies the bound) - [services.rs:216]
  evidence: seed \.clone\(\) = 81 hits; the clone is required only by the argument position, and as_str() removes it without changing the parse allocation.
- clean: seeds 81/11/0/0 - remaining clones are test fixtures, wire-rendering to_owned on literals (schema_name, protocol constants), and the ResourceRef::new pair at resource_bundle.rs:689-690 which is required by the owned signature (same shape at d2b-contracts-resource/src/v3/resource.rs:718); no Rc/RefCell/Arc/Cow anywhere.

## type
- d2b-contracts-zone-session-p2#4 sev=low blast=leaf effort=S verdict=actionable - narrowing_set_is_subset takes a bare empty_allowed_is_unrestricted: bool that flips the empty-allowed semantics between call sites - fix: replace the bool with a two-variant enum (or split into named fns) so the three call sites state the policy they mean - [role_binding.rs:179-182, role_binding.rs:149-162]
  evidence: seed is_\w+: bool = 5 hits; the bool is passed both true (subresources, execution_refs) and false (zones) at role_binding.rs:149-162, so it is a real semantic switch, not a constant.
- clean: the remaining seed hits are cross-value validators (validate_self_resource, validate_finalizer, validate_scope_against_role, validate_zone, ZoneEnrollmentIdentity::validate) that check identity/ownership relations no parsed type can carry, and ZoneEnrollmentIdentity::validate is enforced at both decode boundaries (zone_session.rs:448, 502); no stringly-typed state and no flag-soup structs (EmergencyScope's four booleans are independent actions with all combinations valid).

## api
- d2b-contracts-zone-session-p2#5 sev=low blast=leaf effort=S verdict=actionable - EmergencyPolicySpec::default_values is a pub method with zero callers outside its own Default impl - fix: delete it and let Default::default() be the single entry (or make it private) - [emergency_policy.rs:142, emergency_policy.rs:170]
  evidence: census: default_values over packages/ = 4 hits (zone_link.rs:96, 128; emergency_policy.rs:142, 172), all within the two definitions and their Default delegation; no external caller.
- d2b-contracts-zone-session-p2#6 sev=low blast=leaf effort=S verdict=actionable - ZoneSpec::validate always returns Ok and has no callers, a dead always-succeeding validation on the exported surface - fix: remove the method (ZoneSpec is the empty spec; its Deserialize gate already enforces the only invariant) - [zone.rs:74]
  evidence: census: ZoneSpec over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 1 file (src/v3/zone.rs); the method is never invoked, only defined.
- clean: seeds 292/0/2 - the pub surface is the deliberate wide wire vocabulary of a contract crate (needs-contract to narrow); the two pub use arms (zone_session.rs:89, 104) are the documented house re-export pattern for the component-session taxonomy; no Arc/Rc/Box/RefCell in any public signature.

## err
- d2b-contracts-zone-session-p2#7 sev=medium blast=leaf effort=S verdict=actionable - from_component_session on EndpointPurpose and ServicePackage panics via expect("preserved component-session tag") on a wire-derived value, enforcing the cross-taxonomy totality only at runtime - fix: replace the tag lookup with an exhaustive match over base::EndpointPurpose / base::ServicePackage variants so adding a component-session variant becomes a compile error, or return Result like EndpointRole::from_component_session already does (zone_session.rs:314) - [zone_session.rs:297, zone_session.rs:332]
  evidence: seed \.unwrap\(\)|\.expect\( = 227 hits; 225 are in #[cfg(test)] or on literally-built values (card false positives), and these two are the only non-test expects on input-derived values in the lane.
- clean: seeds 227/1/0/5 - no panic!/unreachable!/todo!/unimplemented! anywhere; the single let _ = (zone.rs:84) discards a Wire value while propagating via ?; the five error enums are Copy unit-variant taxonomies with prose-documented Display, and wire refusal codes (ZoneEnrollmentRefusal) are returned, not panicked.

## serde
- d2b-contracts-zone-session-p2#8 sev=medium blast=family effort=M verdict=actionable - seventeen hand-written Deserialize impls repeat the identical Wire-struct shape (local #[derive(Deserialize)] Wire with deny_unknown_fields, then new() plus map_err(serde::de::Error::custom)) - fix: consolidate behind a shared macro in the style of parsed_deserialize! at d2b_contracts_resource::v3::execution_policy, or #[serde(try_from = "Wire")] with TryFrom, keeping each new() gate as the admission check - [zone_routing.rs:558, zone_routing.rs:624, zone_routing.rs:818, zone_routing.rs:932, zone_routing.rs:1043, zone_routing.rs:1142, zone_routing.rs:1246, resource_bundle.rs:101, resource_bundle.rs:185, resource_bundle.rs:457, zone.rs:79, zone.rs:172, zone.rs:327, role_binding.rs:192, role_binding.rs:382, services.rs:265, emergency_policy.rs:176]
  evidence: seed impl .*Deserialize.*for = 0 hits (the card regex misses impl<'de> forms); dedicated search impl<'de> Deserialize<'de> for = 20 blocks in the lane, 17 of them the Wire-struct shape; this is the recorded not-applied row C4 (docs/explanation/over-engineering-audit-record.md:475), so re-proposing is actionable with this citation.
- clean: seeds 52/90/0/45 - deny_unknown_fields is applied per type on every Wire gate, rename_all conventions are consistent (camelCase structs, kebab-case enums, PascalCase method vocabularies), optionality is deliberate (skip_serializing_if on BundleResourceMetadata and ProcessTemplateBinding, pinned null spellings on RoleBindingSpec round-trip goldens), and no live admission gate is bypassed by a direct derive.

## obs
- N/A: seeds 0/0/0/0 all zero; the crate declares no tracing/log dependency (Cargo.toml), so there is no telemetry surface to judge.

## docs
- d2b-contracts-zone-session-p2#9 sev=low blast=leaf effort=M verdict=actionable - Result-returning pub constructors and the two panicking lifts document failure modes in prose but carry no canonical # Errors or # Panics sections anywhere in the lane - fix: add # Errors to the wire constructors (ZonePath::new, ZoneLinkRouteAdvertisement::new, RoleBindingSpec::with_facets, EmergencyPolicySpec::new, ServiceDescriptor::new) listing their PrimitiveSpecError/ContractError variants, and # Panics to from_component_session noting the totality invariant - [zone_routing.rs:210, zone_routing.rs:685, role_binding.rs:254, emergency_policy.rs:112, services.rs:200, zone_session.rs:296]
  evidence: seed /// # (Examples|Errors|Panics|Safety) = 0 hits while -> Result< = 73 hits; every pub item carries a prose doc comment (checked all 40 candidates flagged by the lookback scan), so the gap is the canonical-section shape, not missing docs.
- clean: all pub items documented with one-line first sentences; module-level docs present (zone_routing.rs:1-16); magic values carry the why (MAX_ZONE_ADVERTISEMENT_LIFETIME_SECONDS, ZONE_ROUTE_INITIAL_HOP_BUDGET); no doctests exist and none are needed for pure contract types.

## perf
- d2b-contracts-zone-session-p2#10 sev=low blast=leaf effort=S verdict=actionable - ProcessTemplateBinding validation builds a fresh String via format!("/bin/{}", ...) on every construction to run an ends_with check - fix: binary_path.strip_suffix(binary_ref.as_str()).is_some_and(|prefix| prefix.ends_with("/bin/")) which is allocation-free - [resource_bundle.rs:382]
  evidence: seed format!\( = 38 hits; 37 are tests or the one-shot fingerprint renderer (services.rs:297); this is the only success-path allocation in the lane. static (unmeasured).
- clean: seeds 38/42/0 - the Vec::new/BTreeMap::new hits are the empty-case constructor (resource_bundle.rs:592) and test fixtures; no to_string() sites; no loops with per-iteration allocation.

## conc
- N/A: seeds 0/0/0/0 all zero; no threads, locks, atomics, or manual Send/Sync in the lane.

## async
- N/A: seeds 0/0/0/0 all zero; no async fn, await, tokio, or block_on in the lane.

## unsafe
- N/A: seeds 0/0/0/0 all zero; no unsafe blocks, fns, impls, or SAFETY comments; the crate inherits unsafe_code = "forbid" via [lints] workspace = true (Cargo.toml).

## ffi
- N/A: seeds 0/0/0/0 all zero; no extern "C", no_mangle, repr(C), catch_unwind, or CStr/CString in the lane.

## macro
- clean: seeds 2/0/0/0 - opaque_routing_token! (zone_routing.rs:117) and zone_closed_enum! (zone_session.rs:121) are by-example impl-per-type generators (a genuine macro case), use narrow fragment specifiers (meta/ident/literal), are invoked only in their defining modules so hygiene holds, and zone_closed_enum!'s local redeclaration is documented as deliberately cheaper than exporting the component_session macro (zone_session.rs:114-120).

## test
- d2b-contracts-zone-session-p2#11 sev=medium blast=leaf effort=S verdict=actionable - EmergencyPolicySpec's contract branches have no tests: the file holds exactly one test covering only the union behavior - fix: add table tests for new() rejecting deadline 0 and > MAX_EMERGENCY_DRAIN_DEADLINE_SECONDS, reason > MAX_EMERGENCY_REASON_BYTES, and control characters; plus effective_scope returning None when no policy is enabled, and the serde default round-trip - [emergency_policy.rs:236, emergency_policy.rs:112]
  evidence: seeds #[test] = 50, assert = 255, proptest/insta/rstest = 0, #[ignore] = 0; emergency_policy.rs has 1 test for 4 validation branches plus the effective_scope all-disabled case, the widest untested contract surface in the lane.
- clean: seeds 50/255/0/0 - the other six modules carry strong in-module tests: golden canonical wire vectors with hand-written expectations (zone_routing.rs:1450-1462, zone_session.rs:787-908), tamper rejection (resource_bundle.rs:1110-1124), bounds at every MAX constant, secret-marker echo checks (zone_routing.rs:2052), and integration coverage of the re-exported session surface in tests/contracts.rs; no test restates implementation, no ignored tests.

## Coverage
- idiom: clean (seeds ran: 0/1/1)
- own: 3 finding(s)
- type: 1 finding(s)
- api: 2 finding(s)
- err: 1 finding(s)
- serde: 1 finding(s)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency in Cargo.toml)
- docs: 1 finding(s)
- perf: 1 finding(s)
- conc: N/A (seeds: 0/0/0/0 all zero; no threads/locks/atomics)
- async: N/A (seeds: 0/0/0/0 all zero; no async fn)
- unsafe: N/A (seeds: 0/0/0/0 all zero; no unsafe blocks and no unsafe_code = "allow" manifest)
- ffi: N/A (seeds: 0/0/0/0 all zero; no FFI surface)
- macro: clean (seeds ran: 2/0/0/0)
- test: 1 finding(s)