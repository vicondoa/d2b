# U25 d2b-core-controller

net: -45 lines, -0 deps

- shrink 15 hand-written single-string redaction Debug impls to the exported `redacted_debug!` macro. The macro home is packages/d2b-contracts-resource/src/v3/execution_policy.rs:22 (`#[macro_export]`, emits exactly `write_str("Type(<redacted>)")`); this crate already depends on d2b-contracts-resource (Cargo.toml:21, BUILD.bazel deps) and imports other items from that crate, so the macro is importable but unused (0 call sites in crate). Measured single-string `write_str("Type(<redacted>)")` Debug bodies: authority.rs x10 (ResolvedExternalNicIdentity, ExternalNicOwnerProof, ExternalNicClaimRequest, ExternalNicAuthorityKey, ExternalNicLease, AuthorityDigest, AuthorityOwnerProof, DurableAuthorityOwnerProof, AuthorityKey, AuthorityLease), controller_assignment.rs x1 (AssignmentEpoch), migration.rs x1 (LegacyTpmStateId; LegacyTpmMigrationDecision uses `<sealed>` label, separate). Each body is `impl Debug { fn fmt { formatter.write_str("X(<redacted>)") } }` = ~4-5 lines → one `redacted_debug!(X);` line. net -45. [packages/d2b-core-controller/src/authority.rs] (leaf)
- delete test-only constructors on the production authority index. `HostGlobalAuthorityIndex::new_for_tests_ready` (authority.rs:1806), `new_unrehydrated` (1823), `drain_guest` (2494, cfg(test)) and the cfg(test) recovery_receipt family (1864-2100) exist only for in-crate tests; `new_unrehydrated` is also used by main.rs:469 test, so keep it or gate the pair together; the cfg(test) helpers are single-caller test scaffolding. [packages/d2b-core-controller/src/authority.rs] (leaf)
- shrink hand-rolled assignment-transport JSON codec to derive+serde where shape allows. controller_assignment.rs hand-writes encode_assignment/decode_assignment/require_exact_keys/encode_bounded_json (lines 481-970, 1419-2015) on serde_json::Value with manual key checks; the durable-claim types in authority.rs already use `#[derive(Serialize, Deserialize)]` + `deny_unknown_fields` - the assignment codec re-implements that pattern by hand for the same wire discipline. Wire shape stays; the hand-rolled key-exactness and bounded-size logic collapses onto derive + a size guard. [packages/d2b-core-controller/src/controller_assignment.rs] (leaf)

## Consistency notes

Types-layer crate (U25 is a controller/authority crate, not a metadata
driver crate): the redaction Debug shape is the same `redacted_debug!`
macro that d2b-contracts-provider/d2b-contracts-resource already export and
use; canonical home is packages/d2b-contracts-resource/src/v3/execution_policy.rs:22.
No duplicate type definitions found in-crate; the crate's Debug impls are
hand-written where the shared macro exists (reported above).

## Reopened refusals

none - U25 ledger has no [refused] rows; all three rows (#A6, #C2, #C3) are
[not applied] and were re-verified still present at HEAD.

## Checked

Read U1 packet U25 section (lines 184-187): #A6/#C2/#C3 all [not applied].
Read src/lib.rs, src/main.rs, src/migration.rs, src/authority.rs,
src/authority_persistence.rs, src/controller_assignment.rs (structural +
Debug impl bodies), BUILD.bazel, Cargo.toml. Grep evidence: `redacted_debug!`
call sites in this crate = 0; macro home located at
packages/d2b-contracts-resource/src/v3/execution_policy.rs:22 with
`#[macro_export]`; crate deps include d2b-contracts-resource (Cargo.toml:21,
BUILD.bazel:23,38,61) and d2b-contracts-provider (Cargo.toml:19,
BUILD.bazel:22,37,60). Single-string `write_str("Type(<redacted>)")` Debug
bodies counted per file: authority.rs 10, controller_assignment.rs 1,
migration.rs 1 (15 total; `<sealed>`/`<store-bound>` variants are distinct
labels and not macro-equivalent). Test-only constructors verified at
authority.rs:1806/1823/2494 and cfg(test) recovery helpers 1864-2100. No
caller search needed for the macro finding (it is a replacement, not a
deletion); the test-constructor finding is crate-local (cfg(test) +
test-support feature gate). LOC measured by body count, not estimated.