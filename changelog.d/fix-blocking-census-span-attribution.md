# `fix-blocking-census-span-attribution.md`

### Fixed

- `cargo xtask blocking-census`: a `disallowed_methods` diagnostic is now
  attributed to the span that actually renders the named call instead of to
  `spans.first()`. A macro-generated call carries the expansion's span there,
  and `#[tokio::test]` expands to `Runtime::block_on`, so every async test
  reported one phantom `tokio::runtime::Runtime::block_on` against its body's
  last statement - an `assert_eq!` line, at a column that names no call. On
  this branch that inflated `d2bd` from its real 10 to 39 and added five more
  phantom `block_on` rows in `d2b-broker`, `d2b-provider-credential-entra`,
  `d2b-provider-credential-managed-identity` and `d2b-provider-endpoint`,
  against source whose call sites never changed.
- Macro-generated and otherwise unattributable diagnostics are still reported,
  but are no longer charged to a crate's committed count, so an
  unattributable hit cannot move a baseline line. The attribution requirement
  is scoped to the instance-method entries, which have no second meter: the
  textual entries keep counting every diagnostic, because a bare-import call
  (`use std::fs;` then `read_to_string(...)`) carries no full-path text at its
  span and filtering those would have silently dropped real calls.
- A diagnostic emitted while compiling a dependency is no longer charged to the
  crate under test: a `-p d2bd` run emits hits for nine other crates, and the
  crate-prefix filter now applies to every entry class.

### Changed

- The `packages/d2bd` `clippy::disallowed_methods` suppression baseline moves
  from 437 to 449, for the 12 suppressions this branch added. Each of the 12
  covers a `tokio::runtime::Runtime::block_on` that `#[tokio::test]`'s own
  expansion emits, not a call any author wrote. They are load-bearing for
  compilation rather than an author bending the ratchet: deleting one allow on
  `production_composition_publishes_display_wiring_to_the_committed_session`
  was attempted and produced 17 `Runtime::block_on` build errors, and an
  isolated probe crate reproduces the same failure from a bare
  `#[tokio::test] async fn` with no allow at all. clippy cannot distinguish a
  call the author wrote from one the expansion wrote, and its only
  configuration escape, `allow-macro`, was rejected because it also silences
  genuine hand-written `block_on` calls inside `macro_rules!` bodies -
  verified in both directions on the probe crate (0 diagnostics with
  `allow-macro`, 1 without). Only this entry moves; no other crate's
  suppression baseline and no deny-entry count is touched.

### Known issue

- `make check-clippy` on `packages/d2b-broker` now reports 38 deny-level
  `Runtime::block_on` diagnostics that did not appear before this branch.
  They are the same `#[tokio::test]` expansion class described above, not new
  blocking work: 36 in `tests/authority_publication.rs` (which contains zero
  occurrences of the text `block_on` and exactly 36 `#[tokio::test]`
  attributes), plus 2 in the crate's own `cfg(test)` module
  (`runtime.rs:17511`, `runtime.rs:17606`, each the last statement of a
  `#[tokio::test]` body - one lands on a `.await`, one on a bare `);`).
  `cargo xtask blocking-census` reports all 38 without charging them, thanks to
  the attribution fix above; the raw clippy lane sees them because it is a
  different gate. Suppressing them would mean ~38 per-site
  `#[cfg(test)] helper` allows, moving the `packages/d2b-broker` suppression
  baseline from 706 to roughly 744 on a crate that was not audited for it, so
  they are recorded here instead of papered over. This is not known-good: the
  lane reaches further than it used to, because the fix below unblocked it.

  The lane became reachable because `packages/d2b-broker/src/runtime.rs` no
  longer calls `Runtime::block_on` from inside a tokio task. That call was a
  latent panic, not lint debt: on tokio 1.53.1 `Runtime::block_on` panics with
  "Cannot start a runtime from within a runtime" when the thread is already
  driving a runtime, so the admitted-effect connection task was being killed
  rather than merely parked.

  Tracked for a macro-aware `disallowed_methods` (or an upstream clippy
  change) in issue #634, covering `packages/d2bd`'s 12 suppressions and
  `packages/d2b-broker`'s 38 together.
