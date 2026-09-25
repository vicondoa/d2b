---
name: ownership-not-clone
description: Use ownership and borrowing instead of reaching for clone, Rc, RefCell, or Arc<Mutex<_>> to silence the borrow checker. Every clone must be explainable. Use when code clones to make an error go away, when a borrow-checker fight is being resolved by copying data, when reviewing Rust dense with .clone() or Rc<RefCell<_>>, or when the user asks whether a clone is necessary.
---

# Ownership, Not Clone

The borrow checker is a map of where ownership is being fudged, not an obstacle to
route around. This skill decides who holds a value and for how long.

## The rule

Every `clone` must be explainable in one sentence that is not "the borrow checker
complained." A clone that buys a real thing — a value that must outlive the borrow
— is fine; one that only silences an error is a deferred design decision.

## Read the error, not the workaround

`E0502` (a mutable borrow while an immutable one is still live) and `E0499` (two
mutable borrows of the same place) name a lifetime conflict; the fix is in the
structure the error describes, not in the `clone` that silences it:

```rust,ignore
// No clone: `first` is Copy, so the borrow ends at the let.
let first = items[0];
for _ in 0..n {
    items.push(first);
}
```

## Borrow splitting

The borrow checker tracks a whole value through method calls, but fields
individually through direct field access. A `&mut self` method that also needs
`self.other_field` is the classic false conflict — destructure once:

```rust,ignore
// The two fields are separate lets and borrow independently.
impl Server {
    fn handle(&mut self) {
        let Self { connections, log, .. } = self;
        for conn in connections.iter_mut() {
            log.record(conn.id());
        }
    }
}
```

`split_at_mut` hands back two disjoint `&mut` halves of one slice; when
destructuring is not enough, extract a free function that takes the two fields.

## Take the cheapest thing that works, in argument position

`&str` over `&String`, `&[T]` over `&Vec<T>`, `impl AsRef<Path>` over a `PathBuf`
for a path a function only reads. A signature that takes `String` when it only
reads forces every caller into a clone — the bug is in the signature.

```rust,ignore
// Forces a clone at every call site that holds a &str.
fn log_message(message: String) { ... }

// Reads only — callers pass a reference and copy nothing.
fn log_message(message: &str) { ... }
```

## `Cow` for genuine borrow-or-own

`Cow<'_, str>` earns its keep when the common path returns the input untouched
and the rare path allocates.

```rust
use std::borrow::Cow;
// The common path borrows; only input containing a bracket allocates.
fn sanitize(input: &str) -> Cow<'_, str> {
    if input.contains(['<', '>']) {
        Cow::Owned(input.replace('<', "&lt;").replace('>', "&gt;"))
    } else {
        Cow::Borrowed(input)
    }
}
```

## When `Rc`/`Arc`/`RefCell`/`Mutex` express a real requirement

Shared ownership is justified when the *runtime* structure genuinely has multiple
owners with no single one outliving the rest, and not when a single owner is
merely inconvenient to name. Run the four-question test in `SHARED-STATE.md`
before adding a `RefCell`; the decision table is in `CLONE-DECISIONS.md`; the
handle clone that passes the one-sentence test is in `SHARED-STATE.md`.

## `mem::take` and `mem::replace`

`mem::take` and `mem::replace` move a value out of a `&mut` by leaving a default
in its place — the answer to "I need to own this but I only have a mutable borrow":

```rust,ignore
let old = mem::take(&mut self.buffer);
```

## Avoid statics

A `static` holding mutable or lazily-initialised state is a lifetime nobody wrote
down and an ownership story nobody can follow. Pass the value, or hold it in the
type that owns the work.

## Deferrals

Reshaping how correct code is expressed is `/idiomatic-rust`; modelling invariants
into types is `/type-driven-design`; error types are `/rust-errors`; shared state
across `.await` points is `/async-rust`.

## Verification

Keeping or removing a clone changes no behaviour, so the claim "this still works"
is settled by the suite, at the lint level the repo configures:

```bash
cargo clippy --all-targets   # add -- -D warnings only if the repo has no lint config
cargo test
```

`clippy::redundant_clone` is `nursery` and off by default — run it one-off with
`cargo clippy -- -W clippy::redundant_clone` as a proposal generator, never write
it into the repo lint config; its findings are candidates, not edits.
