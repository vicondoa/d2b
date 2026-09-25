---
name: idiomatic-rust
description: Write Rust that reads like Rust — iterator pipelines over index loops, From/Into over ad-hoc converters, derives over hand-written impls, newtypes over bare primitives. Use when writing new Rust, when reviewing Rust that reads like a translation from another language, or when the user asks how to make Rust code more idiomatic or less repetitive.
---

# Idiomatic Rust

This skill is about *expression*: what form does a Rust reader expect to see?

## The shape of idiomatic Rust

Prefer expressions over statements: a `match` that returns a value beats one that
assigns to a `let mut` in every arm, and a chain that produces the answer beats a
flag variable set in a loop and checked after it — let the type system carry
invariants instead of runtime checks. `let ... else` keeps an early return at the
top instead of drifting the happy path rightward under a nested `if let`;
`matches!` is the boolean test on a pattern. If-let chains compose conditions where
the toolchain supports them — a recent edition, so check the repo MSRV first.

## Iterators

Reach for the iterator pipeline before the index loop — `for i in 0..v.len() { ...
v[i] ... }` almost always has an iterator equivalent, and the iterator version fails
to compile on bad bounds instead of panicking at runtime. Collect into the type you
want, not a `Vec` you then convert:

```rust,ignore
// Reads like a translation.
let mut names = Vec::new();
for user in &users {
    names.push(user.name.clone());
}

// Reads like Rust.
let names: Vec<String> = users.iter().map(|u| u.name.clone()).collect();
```

`collect` also targets `HashMap`, `HashSet`, `String`, and `Result<Vec<_>, _>` —
that last is how `?` composes with iteration.

```rust
use std::num::ParseIntError;

fn parse_all(lines: &[&str]) -> Result<Vec<i64>, ParseIntError> {
    lines.iter().map(|line| line.parse::<i64>()).collect::<Result<Vec<_>, _>>()
}
```

Prefer the plain `for` loop when the body has real side effects or an early exit a
combinator would obscure — clarity beats iterator purity.

## Conversions

When one type can be built from another, implement `From` — `Into` comes free
through the blanket impl, so never write both directions by hand. When the
conversion can fail, implement `TryFrom` instead of `From` plus a panic or a
sentinel value.

```rust
struct Celsius(f64);
struct Fahrenheit(f64);

impl From<Celsius> for Fahrenheit {
    fn from(c: Celsius) -> Self {
        Fahrenheit(c.0 * 9.0 / 5.0 + 32.0)
    }
}
```

## Derives before hand-written impls

Reach for `#[derive(Debug, Clone, PartialEq, Default, Hash)]` before writing it by
hand — a derived impl is generated from the actual fields, so it stays in sync as
the struct changes. Write the manual version only when the derive would be wrong:

- A `Default` that must preserve an invariant the field-wise default breaks (a
  `Percentage` summing to 100, not zero per field).
- A `Debug` that must redact a secret field (an API key, a password) rather than
  print it.
- `PartialEq` where logically-equal values can differ in a field that shouldn't
  count (a cached hash, a timestamp).

For an enum, `#[derive(Default)]` with `#[default]` on the intended variant is the
same rule — `BOILERPLATE.md` in this directory has the pair.

## Newtypes

A newtype costs nothing at runtime and makes a swapped-argument call a type error.
Reach for one wherever a bare `u64`, `String`, or `f64` stands in for a domain
concept (an ID, a currency amount) rather than a number.

## Naming

Names earn every word: `Bookings`, not `BookingService` — `Service`, `Manager`,
`Helper`, `Util`, `Data`, and `Info` mark a name nobody finished. The
`as_`/`to_`/`into_` prefixes promise a cost — free view, allocation, consumption.
No `get_` on a field accessor; acronyms are words (`HttpClient`, `Uuid`); a helper
that does not need `Self` is a free function, not an `impl` block used as a
namespace. The tiers table and worked pairs live in `NAMING.md`.

## Boilerplate that is actually a design signal

Not all repetition is a `derive` away from disappearing — five near-identical
`match` arms, or a wrapper with ten forwarding methods, can point at a missing
abstraction. See `BOILERPLATE.md` for the pairs, including the `Deref` trap.

## Deferrals

This skill governs expression, not decision-making: the clone-versus-borrow call is
`/ownership-not-clone`, error type shape is `/rust-errors`, and making an illegal
state unrepresentable is `/type-driven-design`.

## Verification

Same behavior, different shape — run the standard trio:

```bash
cargo clippy --all-targets   # add -- -D warnings only if the repo has no lint config
cargo fmt --check
cargo test
```

Run the trio at the lint level the repo configures; treat `clippy::pedantic`
findings as proposals, not violations, unless the repo opted into that group.
