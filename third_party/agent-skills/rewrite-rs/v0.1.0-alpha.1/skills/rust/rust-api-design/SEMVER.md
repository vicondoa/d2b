# Semver

The full breaking-change reference for a released crate. Apply it by hand when
`cargo semver-checks` is unavailable, and use it as the table to propose from
when a change is borderline.

| Change | Breaking? | Why | Mitigation |
|---|---|---|---|
| Adding an enum variant | Yes, unless `#[non_exhaustive]` | Downstream exhaustive `match` no longer compiles | Mark the enum `#[non_exhaustive]` |
| Adding a public struct field | Yes, unless `#[non_exhaustive]` | Breaks literal construction and exhaustive deconstruction separately | Mark the struct `#[non_exhaustive]` |
| Adding a required trait method | Yes | Existing implementers no longer compile | No, if it has a default body |
| Adding a trait supertrait | Yes | Implementers must now satisfy the new bound too | None after release — plan it in |
| Sealing a trait after release | Yes | Downstream implementers are no longer legal | Seal at release time, not later |
| Removing a trait impl | Yes | Code that relied on the impl no longer compiles | Deprecate first, remove in a major version |
| Adding a trait impl | Usually no | New capabilities break no one | Yes when it creates inference ambiguity or a coherence conflict for a downstream impl |
| Changing a concrete return type to `impl Trait` | Yes | The concrete type was part of the API — callers stored it, named it | Keep the concrete type; hide it only for types you never exposed |
| Narrowing a generic bound | Yes | Callers who satisfied the old bound may not satisfy the new one | Widen instead, or pay the major version |
| Widening a generic bound | No | Everything that worked before still works | — |
| Renaming a public item | Yes | The old path no longer exists | Re-export the old name as a deprecated alias |
| Adding `#[must_use]` | No | Warning at worst, for callers who discarded the value | — |
| Raising the MSRV | Not technically semver | Treat it as breaking anyway: a caller on the old toolchain stops compiling, which is what a semver break *is* — document the MSRV change in the release notes | Keep the MSRV pinned in `Cargo.toml` and bump it deliberately, with notice |
| Adding a default cargo feature | Usually no | Opt-in by default changes what compiles for new dependents | Yes if it changes behaviour — a feature that flips semantics on by default is a behaviour change, not a build change |
| Removing a feature | Yes | Dependents that enabled it no longer build | Deprecate first, remove in a major version |

## `#[non_exhaustive]`, applied at release time

The attribute is the only way to keep "add a variant" and "add a field"
non-breaking — but adding it *later* is itself a breaking change, because
downstream code that constructed the type literally or matched it exhaustively
stops compiling the moment it appears. Apply it at release time to every enum
and struct the crate expects to grow. The test: if the type will ever gain a
variant or a field without a major version, it needs the attribute on day one.
