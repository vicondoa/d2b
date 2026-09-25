# xtask - unit-test audit
tests: 455 · src files: 32
net: -4 tests, -65 lines

## Findings (biggest net first)
- trivial: `filtered_lock_omits_unselected_lock_only_packages` (src/production_closure.rs:1774) - calls `filtered_lock` against the committed lock and discards the result (`let _ =`); asserts nothing observable, pins only absence of panic. Nothing lost.
- duplicate: `an_on_disk_provider_omitted_from_workspace_is_rejected` (src/provider_crate_policy.rs:9918) - covered by `empty_provider_scope_fails_closed` (src/provider_crate_policy.rs:9950). Both pin that an on-disk provider crate omitted from workspace members fails with `provider-crate-not-workspace-member`; the keeper asserts the exact error JSON, the duplicate only `contains` checks with a richer fixture that changes nothing about the exercised path.
- duplicate: `every_failure_class_exits_nonzero` (src/delivery/mod.rs:370) - covered by `each_failure_class_maps_to_a_distinct_sysexits_code` (src/delivery/mod.rs:382). The keeper pins the exact codes 64/65/69/72 (all nonzero) plus distinctness and sysexits range, so the nonzero loop asserts nothing extra.
- duplicate: `redaction_preserves_dispatch_evidence_while_removing_secret` (src/bazel_evidence.rs:585) - covered by `redaction_preserves_dispatch_evidence_on_secret_lines` (src/bazel_evidence.rs:574). Both feed `remote execution started authorization: Bearer <token> UNAUTHENTICATED` through `redact_text` and assert the secret is gone plus `dispatchEvidence=true`/`retryLocally=false`; only the token literal differs.

## Keep
- async_gate.rs - remaining 47 pin scanner behavior: denied-call flagging in async contexts, method-call lock detection (awaited/chain/lookalike/string/raw-string boundaries), marker-hatch semantics, inventory validation, write-inventory regeneration, run() fail-closed modes.
- bazel_evidence.rs - remaining 8 pin failure classification (auth, post-dispatch uncertainty, warning vs build) and redaction boundaries (retry-class preservation, quoted/multiline credentials, warning markers).
- blocking_census.rs - remaining 19 pin deny-list parse/classification, context splitting, occurrence counting, suppression scanning, the clippy command contract, and the first-diagnostic/first-json-error selection rules.
- changelog.rs - remaining 38 pin fragment parse rejections (10), fold semantics (6), fragment loading (4), fold_repo transactions incl. symlinked trees (7), and crash recovery at every journal stage with rollback/forward idempotence (8).
- delivery/command.rs - remaining 22 pin the help surface, option parsing, usage errors, and the golden_contract module (wire schema, domains, versioned fingerprint, fail-closed unpinned version).
- delivery/eligibility.rs - remaining 21 pin every eligibility failure mode plus capture/fallback CLI behavior.
- delivery/evidence.rs - remaining 18 pin import addressing, raw-output redaction, stale/tampered rejection, lane layout, end-to-end snapshot→import→seal, rebase invalidation, stdout leak-freedom.
- delivery/history_proof.rs - remaining 11 pin the proof verdict for unchanged/rebase plus each material-change failure.
- delivery/mod.rs - remaining 2 pin the exact sysexits mapping and the unimplemented-error naming.
- delivery/model.rs - remaining 19 pin digest determinism/order-insensitivity, per-input digest sensitivity, newtype validation incl. Deserialize, and the wave namespace rules.
- delivery/recovery.rs - remaining 13 pin attestation validation (fields, timestamps, TTL, clock), ledger state transitions, closure pinning, write-once import.
- delivery/seal.rs - remaining 9 pin seal acceptance, missing/failed/stale evidence refusal, rebase invalidation, idempotent re-import, stray files, forged-digest read-back.
- delivery/snapshot.rs - remaining 27 pin git-failure redaction/classification, digest stability/sensitivity, rebase semantics, write-refusal, tamper verification, CLI usage errors, state-root refusal, rederive, bounded reads.
- delivery/storage.rs - remaining 27 pin write-once, wave addressing, state-root refusal branches, fd-anchored read/write/list, symlink refusal, atomic replace, temp cleanup, ancestor-swap pinning, concurrent create, per-class diagnostic redaction.
- diagnostic_redaction.rs - remaining 10 pin redaction boundaries (prefix siblings, metacharacters, symlinked checkouts, backticks, ANSI), fail-closed filtering, and truncation/multibyte/malformed-tail handling.
- gen_broker_operations.rs - remaining 16 pin catalog resolution rules, triage byte-exactness, fd-kind rendering, merge semantics (replace/append/refuse/drop/survive), round-trip.
- gen_layer_catalogs.rs - remaining 6 pin vocabulary coverage and projection rules.
- inventory.rs - remaining 7 pin TOML parsing, marker/compat token scanning, surface classification, path validation.
- main.rs - remaining 2 pin schema determinism and committed drift.
- nix_inventories.rs - remaining 6 pin projection owner/key rules, vocabulary closedness, byte-stable renders.
- operation_row_authority.rs - remaining 7 pin parity directions, the drift gate, the service-facet bound.
- production_closure.rs - remaining 4 pin dev-edge exclusion, cfg target matching, audit projection expansion, context pruning.
- provider_crate_policy.rs - remaining 59 pin the policy-lint surface: matrix closedness, driver-signal probes, integration ratchet, shared-knowledge probes (family/role/ServerState/self-binding), ratchet mechanics, generated provenance, visibility grants, banned-API allows, committed-scope classification.
- provider_packaging.rs - remaining 11 pin catalog field-group closedness, digest/exclusion lists, matrix/artifact layout, byte-identical renders, catalog↔contract parity, non-empty generation.
- provider_registration_authority.rs - remaining 7 pin service parity directions, the drift gate, idempotent regeneration.
- resource_type_authority.rs - remaining 15 pin type/role parity, per-artifact drift gates, nix registry derivation, the authority bound, role rendering.
- semantic_service_schemas.rs - remaining 7 pin artifact-name uniqueness, rendered-layer strictness/identity, projection constraints, factory-field publication, committed drift, the completeness control.
- zone_schema.rs - remaining 9 pin spec field closedness/sorting, registry projection, dash hygiene, determinism, namespace prefixes, ref typing, committed drift.

## gap
- service_catalog.rs declaration sanity and drift/regeneration gates (packages/xtask/src/service_catalog.rs) - `declaration_errors` rules (identity mismatch, provider-ref mismatch, duplicate service package, fixed-UID rule) and `check`/`regenerate` have no test anywhere in the crate; only wired from main.rs.
- deadcode.rs gate wrapper error paths (packages/xtask/src/deadcode.rs) - tool-missing (`which`), command-failure, and repo-root-failure paths of the dead-code gate are untested; only exercisable with a fake PATH.

cross-check: bazel_evidence unit tests and tests/bazel_evidence.rs pin the same classification/redaction families (unit: exact classification JSON; integration: real CLI runs with exit codes) - judged complementary, each pins extras; C2 may confirm.
