# U56 d2b-provider-credential
net: -0 lines, -0 deps

Lean already. Ship. Every in-scope surface verified live or policy-required: `credential_descriptor`/effects factory consumed by d2bd (credential_resource_runtime.rs:114,2279,3981,5279) and d2bd/src/shared_provider_effects.rs:3525; `CredentialSession`/`credential_session` read by d2bd credential_resource_runtime.rs:29,42,73,197-212; the three `CONTROLLER_PROVIDER_*_ANNOTATION` consts are all written into live status annotations (driver.rs:529-531) with the annotations themselves production-read (driver.rs:550-552 region, inbound ref check); `CREDENTIAL_TYPE_NAME` has a live reader at d2bd credential_resource_runtime.rs:88,341; `CredentialRevocationEvidence` has a production reader; `integration/credential.rs` + `tests/` are policy-required (crate-layout ratchet, not on README-only). Prior-ledger #PR14's deleted `CredentialDriverStatus::{phase,outcome_code}` stays deleted and its annotations remain the crate's live write surface; #PR15's systemic-duplication refusals re-verified: the d2bd/credential cross-crate surfaces need d2b-resource-runtime test support reachable by provider crates (owned elsewhere), and the `ResourceUid::from_bytes` fix (identity.rs:621) is net-applied. No new in-scope caller evidence found - refused stays refused.

## Consistency notes
N/A - this crate is a provider-family leaf (not a contracts/types crate).

## Reopened refusals
None. #PR15 [refused] not reopened - the cross-crate test-support blocker is unchanged at HEAD.

## Checked
Read driver.rs (1961 LOC), effects_service.rs (293), session.rs (490), facets.rs (85), lib.rs (75), test_support.rs (256); verified caller surfaces for every pub symbol: descriptor/effects factory (d2bd credential_resource_runtime.rs + shared_provider_effects.rs), session/passages (d2bd), annotation writers (driver.rs:529-531), CREDENTIAL_TYPE_NAME reader (d2bd:88). Workspace-wide rg for each zero-caller candidate named in the 2026-09-23 packet ledger. Integration/ + tests/ are policy-required scaffolds honoring the U1 ratchet classes.
