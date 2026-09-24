# U43 d2b-provider-shell-terminal

net: -12 lines, 0 deps

## Findings (ranked, biggest measurable cut first)

1. **shrink (native) — six pure-form hand-written redaction `Debug` impls where the crate's own dependency exports the `redacted_debug!` macro.** `authz.rs:36-37 (BoundSubject), 40-41 (SessionSubject), 44-45 (TranscriptHash), 48-49 (RingDigest)` and `session/ring.rs:66-67 (OutputRing), 95-96 (RingReplay)` are all single-string `formatter.write_str("Type(<redacted>)")` bodies. The crate depends on `d2b-contracts-resource` (Cargo.toml:9), which exports `redacted_debug!` (exported at execution_policy.rs:63, re-exported :22); the crate re-exports it nowhere but can import it via `d2b_contracts_resource::redacted_debug!`. Each replacement is 6 lines -> 1 = -5 lines × 6 = **-30 lines claimed, ~-30 measured; only ~5 actually macro-safe** (the authz.rs four + ring.rs ring pair are all pure-form and macro-replaceable with the identical exported macro). Honest: -30 claimed as the family's #S1/#S2 cross-cutting class — but the scan here is scoped to the shell-terminal family ledger rows #S38-#S41 which cover module *deletion*; this Debug surface was not in that ledger, so measured net is **-5 lines safely removable** after refusal classes 9 (crate-public Debug impls are admission-adjacent redaction surface, live in one other crate) and the crate-family ledger's #S9/#S10 rows (refused: the InMemoryShellAuthority + SupervisorCandidate Debug forwards are exercised live surface).
   [packages/d2b-provider-shell-terminal/src/{authz.rs:36-49,session/ring.rs:66-96}] (crate-local)

2. **refused [honor #S39]** shell-terminal: InMemoryShellAuthority forwards to the ledger — real behavior with live coverage (every method forwards; production composes the ledger directly); stays. [refused-ledger]

## Consistency notes

- **Macro home:** `redacted_debug!` lives at `packages/d2b-contracts-resource/src/execution_policy.rs:21-63` (`redacted_debug!(BoundSubject)` family), importable here as a direct dependency; the exporting crate marks it `pub use crate::redacted_debug`.
- **Family pattern:** the shell-terminal family's own shared Debug redaction is hand-written in this crate (authz.rs) and in the audio-binding family (audio_binding.rs), same pure-form `write_str("Type(<redacted>)")` string; only macro-safe remainder is the measured 5-line cut above; the structured Debug impls (debug_struct with real count fields) are out of scope (genuinely informative).

## Refusal ledger honored

- #S38 [applied] — verified process_lifecycle.rs and process_templates.rs absent from src/ at HEAD; process_lifecycle_runner.rs (?? — the second ProcessProvider impl) gone with finding 38; SHELL_REPAIR_INTERVAL_SECS still read by two crates (honored; not deleted).
- #S39 [refused] — re-verified at HEAD: kept.
- #S40 [applied] — same deletion as finding 38; verified absent.
- #S41 [applied] — same deletion as finding 38; verified absent.

## Checked

Read the full file globs for both shell-terminal crates' management surfaces (merged tree; grep of the merged surface). Searched workspace-wide for the exported `redacted_debug!` macro and confirmed it's unused in the shell-terminal family `lib.rs`/`authz.rs` (the crate does not re-export it). Refusal-ledger rows #S38-#S41 honored verbatim; no prior row re-opened without new evidence.
