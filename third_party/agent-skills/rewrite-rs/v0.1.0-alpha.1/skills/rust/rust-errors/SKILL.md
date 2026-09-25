---
name: rust-errors
description: Design Rust error types and panic policy — Result over unwrap, thiserror for libraries, anyhow for binaries, context that survives the call stack. Use when code calls unwrap or expect outside tests, when designing or refactoring an error enum, when choosing between thiserror and anyhow, or when the user asks how errors should be handled or propagated.
---

# Rust Errors

An error type is a contract about what can go wrong and what the caller does
about it: which failures panic, which become a `Result`, which shape the error
takes, how much context survives the call stack. Making the failure impossible
is `/type-driven-design`; whether shape changes break callers, `/rust-api-design`.

## The panic policy

`unwrap` and `expect` are assertions about invariants, not error handling.
Acceptable: in tests, in `main` for a startup precondition, and after a check
the compiler cannot see — where `expect` names the invariant and carries the
values. Unacceptable: in library code on any input-derived value, or anywhere
the justification is "this can't fail" without saying why.

Not every failure is a `Result`. A broken internal invariant with no
caller-recoverable path should panic — the program is already in a state the type
system promised it would not reach. Input validation is the opposite: whatever
the input, the caller can respond, so it is always a `Result`.

## `Result` all the way to a boundary

Errors propagate with `?` to the layer that can actually decide — retry, report,
exit. `?` converts through `From`, which is why a `#[from]` on an enum variant
removes a `map_err` closure at every call site.

## Which shape: the boundary decides

One question settles it: **does this error cross a public API boundary you will
maintain across releases?**

No — an internal crate, a closed set of failures, an error the caller only ever
prints — it is an enum, and the enum is the cheaper, better answer:

```rust,ignore
#[derive(Debug, thiserror::Error)]
enum LoadError {
    #[error("cannot read {path}")]
    Read { path: PathBuf, #[source] source: std::io::Error },
    #[error("malformed config in {path}")]
    Parse { path: PathBuf, #[source] source: toml::de::Error },
}
```

Yes — a published library whose callers upgrade without editing their code — it is
a struct wrapping a private kind, exposing the questions callers actually ask:

```rust,ignore
pub struct ConfigError { kind: ErrorKind }

impl ConfigError {
    pub fn is_not_found(&self) -> bool { /* match on the private kind */ }
    pub fn path(&self) -> &Path { /* ... */ }
}
```

The full struct pattern, compiling, is in `ERROR-TYPES.md`.

The public enum fails at that boundary in four independent ways: a variant like
`Parse(toml::de::Error)` promises for ever which TOML crate you use; adding a
failure mode breaks caller matches; `#[non_exhaustive]` does not save it, because
it forces every caller into a `_ =>` arm from day one and so removes the exhaustive
matching that was the reason to expose an enum; and it pushes work onto callers,
who re-derive "the config file is missing" from
`ConfigError::Io(e) if e.kind() == NotFound`. The precedent is `std::io::Error`,
which is exactly this pattern.

`thiserror` still does the mechanical work either way — `#[source]` chaining and
`Display` forwarding. Binaries use `anyhow`: a binary error is a message a human
reads once, so a boxed dynamic error with context is enough. A binary large enough
to have internal library crates uses `thiserror` inside them and `anyhow` only at
the top, and `anyhow` in a published library forces every downstream caller to give
up matching on concrete types — that is the actual reason for the split, not taste.

## Error taxonomy

Split the enum by what the caller does about it, not by where it was raised. If
two variants always get identical handling, they are one variant. If one variant
is retryable and another fatal, they must be distinguishable without
string-matching a message — an `is_retryable()` or `kind()` accessor on a coarse
enum. On a public struct error the growth question does not arise, because the
kind is private and adding to it is not a breaking change; `#[non_exhaustive]` is
for the enum that is public despite the guidance above, buying non-breaking
growth at the cost of exhaustive matching. The semver rules live in
`/rust-api-design`.

## Panics that are the right answer

Panic means stop the program, not "return an error loudly": a detected bug — a
violated internal invariant — panics rather than returning an `Error`, because
there is no caller who can do anything correct with it. `catch_unwind` is a
last resort, followed by a controlled restart of the unit that panicked, never
by carrying on as though nothing happened. For invariants too costly to check
in release builds, `debug_assert!` keeps the check in dev and test builds.

## Deferrals

Making the illegal state unrepresentable is `/type-driven-design`; whether a
shape change is a semver break is `/rust-api-design`; cancellation and timeouts
are `/async-rust`; expression-level cleanup is `/idiomatic-rust`.

## Verification

```bash
cargo clippy --all-targets   # add -- -D warnings only if the repo has no lint config
cargo test
```

Run clippy at the lint level configured in the repo — a `clippy.toml` or its own
lint attributes stay, and a stricter level never goes on top. Then the targeted
audit the agent runs itself and reads:

```bash
rg '\.unwrap\(\)|\.expect\(' --glob '!**/tests/**' --glob '!**/benches/**' src/
```

Every surviving hit needs a one-line justification. `clippy::unwrap_used` and
`clippy::expect_used` are `restriction` lints: propose them for the repo lint
config, do not switch them on unilaterally, or override a level already set.
