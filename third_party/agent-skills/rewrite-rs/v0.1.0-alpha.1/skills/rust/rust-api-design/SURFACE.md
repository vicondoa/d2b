# The surface

The depth for the surface section of `SKILL.md`: the shapes a public signature
refuses, the paths items may occupy, the parameter types that cost nothing, and
the derives a public type owes its callers.

## Abstractions do not visibly nest

A caller should not have to understand two layers to use one. A return type of
`Wrapper<Handle<Inner>>` makes the caller learn the whole stack to write one
line.

## No `Arc`, `Rc`, `Box` or `RefCell` in a public signature

Those are how the crate stores the thing, not what the caller has. Take `&T` or
`impl Into<T>`, return the value or a handle type that hides the pointer.

## Essential functionality is inherent

The methods a type cannot be used without are inherent methods, not trait
methods a caller must import a trait to reach. A trait is for the shared
vocabulary, not for the core of the type.

## Parameter consistency

The same concept has the same name, position and type across every function in
the crate. `path` first everywhere, or last everywhere; never both.

## Balanced modules, and splitting the crate

A module of thirty items next to one of two is usually one module that grew and
one that never did. When genuinely in doubt about whether something belongs,
split the crate: two crates with a clear dependency direction beat one crate
with an internal boundary nobody enforces.

## One path per item

An item is public at exactly one path. Re-exporting it under a second path
across iterations, so that both keep working, is the agent smell: it looks like
compatibility and is actually a surface nobody can document. If a path was
wrong, move it and take the breaking change.

## Do not leak external types

A type from a dependency in your public signature makes that dependency version
part of your semver contract. Wrap it, or convert at the boundary. The error
shape is the case `/rust-errors` owns — a dependency error type in a signature
is the same leak, with the same fix. The exception is a type so ubiquitous it
is effectively vocabulary (`std` types, and in practice `http::Uri`-class
crates the whole ecosystem shares) — and that exception is a judgment you write
down, not a default.

## Re-exporting a foreign type, when you must

If a caller genuinely needs the dependency type, `pub use` it so the caller can
name it without adding the dependency themselves, and treat its version as part
of your contract from then on.

## Escape hatches

A crate that abstracts something should let a caller reach the thing underneath
(`fn inner(&self) -> &T`, `into_inner`). Without one, the first unanticipated
requirement forks your crate.

## Ergonomic parameter types

Take `impl AsRef<str>` or `impl Into<String>` where callers plausibly hold
either form, `impl AsRef<Path>` over `&Path` for a path parameter, and
`impl RangeBounds<usize>` over three overloads. Return `impl Iterator<Item = T>`
rather than `Vec<T>` when the caller may not want the allocation. Use sans-IO
design where the protocol logic is a state machine the caller drives, so it
works under any runtime and tests without a network. And derive the common
traits — `Debug` always, `Clone` and `PartialEq` where they make sense — while
adding them is still cheap: a missing derive can become a breaking change to
add later when it creates inference ambiguity or conflicts with a downstream
impl, and which side of that line a given derive falls on is in `SEMVER.md`.

## Public types are `Send`, `Debug`, and sometimes `Display`

A public type that is not `Send` cannot cross a task boundary, which callers
discover late and cannot fix. Every public type derives `Debug`. A type meant to
be read by a human implements `Display`. And a `Debug` on a type holding a
secret needs a test proving the secret does not appear in the output — the
redacting-newtype pattern is in `/rust-observability`.
