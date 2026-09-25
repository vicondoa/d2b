---
name: rust-code-review
description: Review Rust changes on two axes at once — standards (idioms, ownership, errors, API surface, unsafe) and spec (does the change do what was asked) — with a Rust smell baseline layered on the repo lint configuration. Use when reviewing a Rust diff, pull request, or branch, when asked whether a change is ready to merge, or when a review needs to cover more than what clippy already reports.
---

# Rust Code Review

A review is a statement about what a change owes — to the request that asked
for it, and to the standards the repo already lives by. This skill runs both
statements as separate passes and merges them into one report led by a
verdict. It never restates a rule a smell maps to: the craft skills and
`/rust-testing` own the standards, and the review routes to them.

## Two axes, run separately

A single reviewer reading for both correctness-against-the-request and craft
does neither well: spec drift hides behind clean-looking code, and style nits
crowd out the missing edge case. Run a **standards pass** and a **spec pass**
as separate passes — as parallel sub-agents when the harness supports it,
sequentially when it does not — then merge. The prompts for both, the merge
rules, and the report shape are in `REVIEW-PASSES.md`.

## The machine goes first

Before either pass reads a line, run the verification step below and read the
output. Anything clippy already reports is not a review finding; it is a build
failure someone forgot to run. Reviewer attention is for what the tools cannot
see: the judgment call, the missing edge case, the scope that crept in.

## Read the repo lint configuration before judging style

`[lints.clippy]` in `Cargo.toml`, a `clippy.toml`, a `rustfmt.toml`,
`#![deny(...)]` in the crate root. A finding that contradicts a level the repo
deliberately set is not a finding — it is a proposal, and it says so
explicitly. Never re-run at a stricter level than the repo configures and
report the extra hits as defects. The smell baseline layers on top of this
configuration, never in place of it — a finding that the repo lint config
already permits is a finding about the config, raised separately.

## The smell baseline

What this skill adds beyond clippy: the judgment calls no lint can make. The
full table — the smell, how it shows up in a diff, why it is a defect, and the
owning skill — is `SMELLS.md`. The top ones:

- an unexplained `clone` at the call site — `/ownership-not-clone`
- `unwrap` or `expect` on a path a caller controls — `/rust-errors`
- `pub` on an item with no external caller — `/rust-api-design`
- a boolean or stringly-typed parameter that should be an enum —
  `/type-driven-design`
- blocking work inside an async function — `/async-rust`
- an `unsafe` block with no `// SAFETY:` comment — `/unsafe-rust`
- a behaviour change with no test that would have caught it — `/rust-testing`

An `#[allow(lint)]` that outlived its reason is invisible and permanent.
`#[expect(lint, reason = "...")]` fails once the situation it was written for
goes away, which turns a silenced lint into one that reports itself when it
becomes stale. In review, an `#[allow]` with no reason is a finding; with a
reason it is a conversation.

## Severity, and saying nothing

Three levels only: **blocking** (wrong behaviour, unsound, breaks the public
API without a version bump), **should-fix** (a real defect the author would
want to know about), **note** (a genuine improvement the author may decline).
Anything below that threshold is dropped rather than written down — a review
of forty nits and one soundness bug buries the soundness bug. No
praise-padding, and no restating the diff back to the author.

## Finding format

One line each, most severe first: `path:line — severity: what is wrong. What
to do instead.`

```
src/cache.rs:41 — blocking: guard held across .await; drop it before the fetch.
src/parse.rs:8 — should-fix: unwrap on caller input; return the ParseError instead.
src/lib.rs:102 — note: this fn takes String where &str would do.
```

A finding with no concrete fix is a question, and is phrased as one.

## What this skill does not do

It does not rewrite the code — findings, not patches, unless the author asks.
It does not review a diff it cannot build: a check that cannot run is named in
the report, not papered over. It does not decide the parity contract of a
port; that is `/port-to-rust`.

## Verification

Run before reporting, and quote the results in the report:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features   # at the repo configured level; add -- -D warnings only if there is no lint config
cargo test --all-features
```

If any of the three cannot run — no network for dependencies, no nightly for a
required component — the report says which check did not run rather than
implying a clean bill.
