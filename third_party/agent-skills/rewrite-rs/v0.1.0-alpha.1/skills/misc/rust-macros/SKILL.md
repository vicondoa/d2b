---
name: rust-macros
description: Write a macro only when a function, a trait, or a generic cannot do the job — then by-example before proc-macro, with hygiene, a `_private` helper module, and spanned compile errors instead of panics. Use when writing or reviewing macro_rules! or a proc macro, when a derive or attribute macro is being added, when macro hygiene or `$crate` comes up, or when the user asks whether something should be a macro.
---

# Rust Macros

The first question a macro must answer is why it is not a function, a trait,
or a generic — and usually the answer is that it is not.

## A macro is a last resort

Most macros exist to avoid typing, and the cost they charge is paid by every
reader afterwards: no jump-to-definition worth the name, error messages
pointing at expansions, no type checking until the expansion happens. The
genuine answers are three — a variadic interface, generating an impl per
type from a list, and a DSL whose syntax is not Rust. Name the case; if it
is not one of the three, reach for the non-macro and say which.

## By-example before procedural

`macro_rules!` is in the same crate, needs no dependency, and can be read. A
proc macro needs its own crate, a `syn`/`quote` dependency pair, and compiles
before the crate that uses it. Reach for it when the input has to be parsed
as Rust syntax — what derives and attribute macros are — and not before.

## Hygiene, and `$crate`

A macro expands at the call site, where the names it mentions may mean
something else. `$crate` resolves to the defining crate no matter where the
expansion lands, and a macro that names any item from its own crate without
it works only until someone invokes it from a module that shadows the path.
Local variables a `macro_rules!` introduces are hygienic and cannot collide;
paths and types are not.

## Fragment specifiers say what you accept

`expr`, `ty`, `ident`, `pat`, `literal`, `tt` — pick the narrowest that fits,
because the specifier is the error message. A macro taking `tt` accepts
anything and reports the failure somewhere inside the expansion, where no
one will look.

```rust
macro_rules! min_of {
    ($a:expr, $b:expr) => { if $a < $b { $a } else { $b } };
}
pub fn pair_min(a: i32, b: i32) -> i32 {
    min_of!(a, b)
}
```

## A macro does not lie about the signature

If the macro generates a function, the generated signature is the one the
caller sees in an error, so it carries real types and real names. Never
generate an item whose name the user did not write and cannot search for —
an implied item is a symbol in errors from a place the reader cannot find.

## The `_private` helper module

Anything the expansion must reference goes in a `#[doc(hidden)] pub mod
_private`, referenced through `$crate::_private::…`, so it is reachable by
the expansion and marked as no part of the public API. Without it, the
helpers become public surface you cannot change.

```rust,ignore
#[doc(hidden)]
pub mod _private {
    use std::sync::atomic::{AtomicUsize, Ordering};
    pub fn next_id() -> usize {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        NEXT.fetch_add(1, Ordering::Relaxed) + 1
    }
}
macro_rules! next_id {
    () => {
        $crate::_private::next_id()
    };
}
pub fn assign() -> usize {
    next_id!()
}
```

## Proc-macro crates: keep the implementation separate

The proc-macro crate can export nothing but macros, so the logic lives in a
plain sibling crate that the proc-macro crate calls; the sibling is
unit-testable without a compiler harness, which is the actual reason.

## Errors are spanned, never panics

A proc macro that panics reports "proc macro panicked" with no location. A
`syn::Error` built with `new_spanned` and turned into a compile error by
`to_compile_error` points the compiler at the user's own code — the
difference between a macro people use and one they work around.

```rust,ignore
fn expand(input: &syn::DeriveInput) -> proc_macro2::TokenStream {
    if input.generics.type_params().count() > 1 {
        return syn::Error::new_spanned(&input.generics, "too many generics")
            .to_compile_error();
    }
    // build the derived impl with quote
}
```

## Deferrals

Whether the generated API should exist at all is `/rust-api-design`. What a
generated error type looks like is `/rust-errors`. Derives that already exist
and should be used instead of a hand-written impl are `/idiomatic-rust`.
Serde attribute behaviour is `/rust-serde`.

## Verification

```bash
cargo expand --lib          # read the expansion; if it is not installed, propose it, do not require it
cargo test                  # including a trybuild suite if the crate has one
cargo clippy --all-targets  # at the level the repo configures
```

A macro crate without a `trybuild` (or equivalent) test of its failure cases
has never checked the thing users complain about, which is the error message.
