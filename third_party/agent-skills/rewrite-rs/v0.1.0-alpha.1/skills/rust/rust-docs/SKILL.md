---
name: rust-docs
description: Doc comments as API contract — a one-line first sentence, module-level docs, Examples, Errors, Panics and Safety sections, doctests that actually run, and intra-doc links. Use when writing or reviewing rustdoc comments, when a public item is undocumented, when doctests are marked ignore or fail, when magic values appear undocumented, or when the user asks how to document a Rust crate.
---

# Rust Docs

A doc comment is an API contract, not narration: it tells the caller what
the item does, what it requires, what it returns, and how it fails — and
keeps everything the contract does not need out of the rendered docs.

## The contract test

A doc comment answers what a caller must know to call the item correctly,
and nothing more; it does not narrate the implementation. The test: could
the body be rewritten without changing the doc comment? If not, the comment
is describing the implementation, and it will rot at the first refactor.

## The first sentence carries the load

Roughly fifteen words, one line, no trailing detail: it is what appears in
the module index next to the item name, and the only documentation most
readers will ever see. Weak then strong:

```rust,ignore
/// Constructs a new instance.
///
/// Takes the input, checks it, transforms it, and returns a result, or an
/// error when the input was not acceptable in the way described.

/// Parses a port number from a string, failing when it is out of range.
```

The weak first sentence echoes the item name and pushes the payload into a
trailing paragraph; the strong one carries the whole index entry on its own.

## Module documentation

`//!` at the top of the module says what the module is for and how its
items fit together. A module of well-documented items with no module doc
leaves the reader to infer the shape from a list of names.

## The canonical sections

`# Examples`, `# Errors`, `# Panics`, `# Safety`, in that order, each with
its own trigger:

- `# Examples` — on any public item where the signature alone does not
  show the use; the doctest lives here.
- `# Errors` — on anything returning `Result`, saying which conditions
  produce which failure.
- `# Panics` — on anything that can panic; its absence is itself a claim.
- `# Safety` — on every `pub unsafe fn`, stating the invariant the caller
  must uphold; the depth of the invariant is `/unsafe-rust`.

## Doctests are tests

Doctests compile and run under `cargo test`. Use `?` rather than
`.unwrap()`, so the example shows the code a caller would actually write,
and hide setup lines with a leading `#`, so the example stays short without
becoming a fragment:

```rust
/// Parses a port number.
///
/// # Examples
///
/// ```
/// # use std::num::ParseIntError;
/// let port: u16 = "8080".parse()?;
/// assert_eq!(port, 8080);
/// # Ok::<(), ParseIntError>(())
/// ```
pub fn parse_port(text: &str) -> Result<u16, std::num::ParseIntError> {
    text.parse()
}
```

The rule that matters: `ignore` on a doctest means nothing checks it, so it
is a fragment on purpose or rot waiting to happen. `no_run` when the example
must compile but must not execute — it opens a socket, it needs a file.
`compile_fail` when not compiling is the point.

## Intra-doc links

`[`Config`]` resolves to the item; renaming it leaves a broken-link
warning that a lint level that treats warnings as errors turns into a
build failure — what makes a link better than a backticked name that
nothing checks. `#[doc(inline)]` on a `pub use` so a re-exported item
documents in place, not behind a path the reader cannot import from.

## Document magic values with the why

A constant of `30` documents as the timeout the upstream service enforces,
not as "thirty seconds", which the reader could already read off the value.
A number whose reason nobody recorded is a number nobody can change.

## What does not belong in user-facing docs

No design journals, no changelog narration inside item docs, no "rule
applied" compliance tables: artifacts of the writing process, and a reader
looking up a function signature has no use for them. Keep the decision
record in the repo, out of the rendered documentation.

## Deferrals

Whether the function should panic at all is `/rust-errors`; the invariant a
`# Safety` section states is `/unsafe-rust`; whether the item should be
public is `/rust-api-design`.

## Verification

Run at the lint level the target repo configures; the fallback applies only
when the repo configures no lints at all:

```bash
cargo doc --no-deps        # warnings here are broken links and bad attributes
cargo test --doc
cargo clippy --all-targets # add -- -D warnings only if the repo has no lint config
```

Where the crate has opted in to `#![warn(missing_docs)]`, the undocumented
public item is a warning instead of a discovery. Propose it for a library,
do not impose it, and never add it to a binary crate.
