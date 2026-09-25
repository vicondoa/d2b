# Test design

The depth behind the five-way table in `SKILL.md`: where each form lives, what
it can reach, what it costs to run, and a worked example for each.

## The five forms in full

| Form | File | Reaches | Cost |
|---|---|---|---|
| Unit | `#[cfg(test)] mod tests` in the module under test | Private items — the test is inside the module | Cheapest: compiles into the crate, runs with the rest |
| Integration | `tests/<name>.rs` — one binary per file | The public API only: the crate is an external dependency | A separate compile and link per file |
| Doc | `///` examples, run by `cargo test --doc` | The public API only: each example is its own crate | A compile per example — the slowest form |
| Property | A `proptest!` block, in unit or integration | Whatever the enclosing form reaches | 250 iterations by default, plus a regression file on failure |
| Snapshot | An `assert_snapshot!` in unit or integration | Whatever the enclosing form reaches | Trivial once the snapshot exists — the cost is the review |

**Unit** — the default for a pure function with interesting edges, and the
only form that reaches private items:

```rust,ignore
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_at_rejects_negative_and_oversize() {
        let entries = [10, 20, 30];
        assert_eq!(entry_at(&entries, -1), None);
        assert_eq!(entry_at(&entries, 3), None);
    }
}
```

`entry_at` is private, and this test can call it because it lives in the
module. The same assertion from `tests/` would not compile.

**Integration** — the assertion is about what a real caller sees, so the test
imports the crate the way a consumer would:

```rust,ignore
// tests/load_config.rs
use mycrate::Config;
use std::path::Path;

#[test]
fn base_config_overrides_are_applied() {
    let config = Config::load(Path::new("fixtures/base.toml"))
        .expect("fixture must parse");
    assert_eq!(config.workers, 4);
}
```

What this form catches that unit tests cannot: the public surface itself — a
constructor the internal tests never call, a visibility change that compiles
inside the crate but breaks outside it.

**Doc** — the example is documentation first, and `cargo test --doc` keeps it
honest:

```rust
/// Counts the words in a line.
///
/// ```
/// use mycrate::count_words;
/// assert_eq!(count_words("two words"), 2);
/// ```
pub fn count_words(line: &str) -> usize {
    line.split_whitespace().count()
}
```

A doc test that is really a unit test in disguise — private access, setup
plumbing, three edge cases — belongs in `mod tests`, where it is cheap to run
and easy to find.

**Property** — the rule holds for a whole input space, so the test states the
rule once and lets the generator supply the inputs:

```rust,ignore
proptest! {
    #[test]
    fn config_roundtrips(config in any::<Config>()) {
        let text = toml::to_string(&config)?;
        let parsed: Config = toml::from_str(&text)?;
        prop_assert_eq!(config, parsed);
    }
}
```

A failing case is shrunk to a minimal repro and saved to a regression file, so
the next run fails on the same input in one case instead of 250. The three
property shapes worth reaching for first:

- **Round-trip** — encode then decode gets back to the start, as above.
- **Invariant preservation** — the operation keeps a stated property true for
  every input: `normalize(normalize(x)) == normalize(x)`, a shrink that never
  grows.
- **Equivalence** — the fast implementation and a
  slow-but-obviously-correct reference agree on every generated input.

**Snapshot** — the output is large enough that an inline expected value would
be unreadable, so it is stored and diffed:

```rust,ignore
#[test]
fn render_error_report() {
    let report = render_error_report(&err);
    assert_snapshot!(report);
}
```

The first run writes `error_report.snap.new`; the review decides what happens
to it. The discipline is the next section — a snapshot test without a real
review is just a file that changes.

## Snapshot discipline

`insta` review is a review, not a rubber stamp: an accepted snapshot diff
nobody read is a regression committed with a green suite. Read the diff line
by line, and state the reason the output changed — a fix, a deliberate format
change, or a bug. Snapshots must be deterministic or they teach the team to
accept diffs blindly: sort map keys before rendering, redact timestamps and
absolute paths, and pin any locale- or machine-dependent formatting. A
snapshot that differs across machines is noise, and noise teaches the eye to
skip the signal.

## Golden files

When the artifact is a file rather than a value — generated bindings,
formatted output, an emitted template — commit the expected artifact and diff
against it:

```rust,ignore
#[test]
fn generated_bindings_are_current() {
    let expected = std::fs::read_to_string("tests/fixtures/bindings.expected.rs")
        .expect("golden file committed");
    let actual = generate_bindings();
    assert_eq!(expected, actual);
}
```

The regeneration command is documented next to the test — a comment naming the
exact invocation — because the day the generator changes, the artifact is
updated by running that one command and reviewing the diff, not by regenerating
and committing whatever came out.

## Fixtures and helpers

Shared setup for integration tests lives in `tests/common/mod.rs`, pulled into
each test binary with `mod common;`, and file-shaped inputs live under
`tests/fixtures/`. A builder beats a 40-line struct literal repeated in nine
tests:

```rust,ignore
// tests/common/mod.rs
pub fn config_with(workers: usize) -> mycrate::Config {
    let mut config = mycrate::Config::default();
    config.workers = workers;
    config
}
```

A helper that itself has branching logic needs its own test, or it is
untested code deciding whether other code is tested.

## Making a dependency substitutable

Two patterns, and the rule that picks between them.

**A trait plus a mock** — for an abstraction the crate owns and would
define anyway: a repository, a gateway, a policy. The trait is worth its
keep independently of testing, so the test does not distort the design.

**A private enum core behind a `test-util` feature** — for syscalls,
clocks, and entropy inside a library shipped to others. The type stays
concrete, the enum inside it has a real variant and a fake variant, and
the fake constructor is `#[cfg(feature = "test-util")]`:

```rust,ignore
pub struct Clock {
    core: Core,
}

enum Core {
    System,                  // the real thing
    #[cfg(feature = "test-util")]
    Fake(FakeClock),         // the test double, absent from a normal build
}

#[cfg(feature = "test-util")]
impl Clock {
    pub fn with_fake(clock: FakeClock) -> Self {
        Self { core: Core::Fake(clock) }
    }
}
```

**The rule**, stated as the reason rather than a preference: a trait for a
substitutable clock costs a public generic parameter or a vtable on every
type that touches it — a permanent change to the public API paid for a
test-only need. When the abstraction only exists to be swapped in tests,
keep it private.

Feature-gate test utilities under a named feature so downstream crates
can use the fakes deliberately, and so they are absent from a normal
build. Integration tests live under `tests/`, where they see only the
public API — which is what makes them a check on the surface rather
than on the internals.

## What not to test

Derived `Debug`, `Clone`, and `Default` — the derive either works or it does
not, and the compiler has already decided. Third-party crate behaviour, which
that crate tests against its own contract. The compiler, which is tested by
the people who write it. Each line spent there is a line not spent on the
parser edge case that actually ships.
