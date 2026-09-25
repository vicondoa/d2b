# Smell baseline

The judgment calls no lint can make — what the review adds beyond clippy.
Each row names the smell, how it shows up in a diff, why it is a defect rather
than a preference, and the skill that owns the standard. A row is a trigger,
not a verdict: the named skill owns the rule, and the cases where context
flips the call are at the bottom of this file.

| Smell | How it shows up in a diff | Why it is a defect, not a preference | Owning skill |
|---|---|---|---|
| `clone` with no explaining reason at the call site | A `.clone()` added to silence a borrow error, no comment, no local need for the copy | A clone that only silences an error is a deferred design decision; the ownership structure is wrong, and every reader inherits the allocation plus the unasked why | `/ownership-not-clone` |
| `Rc<RefCell<_>>` or `Arc<Mutex<_>>` introduced to resolve a borrow error | A new indirection layer where a plain reference, or a shorter borrow, would do | Shared machinery bought to appease the checker adds refcount or lock cost, a deadlock surface, and `Send`/`Sync` friction to every use | `/ownership-not-clone` |
| `unwrap`/`expect` on caller-controlled input, or anywhere in a library path | A new `.unwrap()` on a value a caller can influence, in library code | A library panic turns a caller-recoverable failure into a crash; the failure belongs in a `Result` the caller can match | `/rust-errors` |
| An error enum with one variant carrying a `String` for everything | A new error type shaped like `Failed(String)`, or a match arm that parses the message | Callers cannot tell apart the failure classes the code already knows — retryable from fatal — so the type is a log line, not a contract | `/rust-errors` |
| `bool` or `&str` parameters that encode a mode | `fn render(pretty: bool)`, or callers passing `"strict"` and `"lenient"` | The type admits every combination, most of them nonsense; the impossible combination is a latent bug the compiler cannot name | `/type-driven-design` |
| A constructor that can build a state the domain forbids | A public literal or `new` accepting a field combination the invariants rule out | Every constructible invalid state forces a runtime check at every call site; one forgotten check ships the bug | `/type-driven-design` |
| `pub` on an item with no caller outside the crate | A new `pub` fn or type whose only uses are in-crate | `pub` is a semver promise to every downstream; an item with no external caller is a commitment with no counterparty | `/rust-api-design` |
| A public function taking `String`/`Vec<T>` where `&str`/`&[T]` would do | A new or changed signature that takes an owned collection it only reads | It forces a clone at every caller that holds a reference; the allocation lands at the call site, but the defect is in the signature | `/rust-api-design` |
| A new public enum or struct field without `#[non_exhaustive]` consideration | A new variant or field on a released type, with no attribute and no note | Without it, the next additive change is a breaking one; the attribute is the cheap way to keep the type evolvable | `/rust-api-design` |
| Blocking I/O, `std::thread::sleep`, or a `std::sync` guard held across `.await` | A sync call or a sleep inside an `async fn`; a live guard over an await point | It stalls the executor thread and starves every other task on it; the failure shows up as latency on unrelated code | `/async-rust` |
| `unsafe` without `// SAFETY:`, a `// SAFETY:` that restates the code, or a `pub unsafe fn` without `# Safety` | A new unsafe block or fn whose invariant is not written down, or whose comment only repeats what the code does | The written invariant is what makes the block sound; unwritten, or restated instead of stated, it cannot be checked, and the next caller change can break it silently | `/unsafe-rust` |
| An index loop that is a `map`/`filter`/`collect` in disguise | `for i in 0..v.len()` where the body only reads `v[i]` and accumulates | The iterator form fails to compile on a bounds mistake instead of panicking at runtime, and it reads like Rust to a Rust reader | `/idiomatic-rust` |
| A behaviour change with no test that would fail without it | A diff that changes observable behaviour and adds no test, or only tests that pass against the old code | The regression is unfalsifiable: the next refactor breaks the behaviour and nothing says so | `/rust-testing` |
| A test asserting on `Display` output where a variant match is the contract | `assert_eq!(err.to_string(), "...")` where the callers would match on the variant | The string is formatting, not a contract; the test breaks on a wording change and stays green on a variant change | `/rust-testing` |
| A public error enum variant carrying a type from a dependency | A new variant shaped like `Parse(toml::de::Error)` on a published error type | The variant pins the dependency into the contract, and the next failure mode added to the enum breaks every caller match | `/rust-errors` |
| An `Arc`, `Rc`, `Box` or `RefCell` in a public signature | A new or changed `pub fn` that takes or returns one of the four | The pointer is how the crate stores the thing, not what the caller has; exposed, the storage shape becomes semver and every caller imports the machinery | `/rust-api-design` |
| A `pub` item with no doc comment, a `Result`-returning public function with no `# Errors`, a `pub unsafe fn` with no `# Safety` | A new public item, error function, or `unsafe` fn shipped without the section rustdoc would render for callers | The doc comment is the API contract: a caller matching the error has nowhere to read the variants, and an `unsafe` fn without `# Safety` asks for trust in what is not written | `/rust-docs` |
| `println!` in library code, or a log call interpolating values into the message instead of passing fields | A `println!` in a library crate, or `warn!("failed for {id}")` where the value belongs in a named field | The library writes to a channel it does not own, and a value interpolated into a message is text, not data: it cannot be queried, filtered, or redacted, which is what the field is for | `/rust-observability` |
| An `as` between integer types where `try_from` was available | A new `n as u32`, or a narrowing `as i8`, added to make a type mismatch compile | The cast discards the overflow without a trace; the value wraps silently where `try_from` would have made the failure a `Result` someone handles | `/type-driven-design` |
| A guard held across a call that can lock again | A live `MutexGuard` across a call that acquires the same lock, or across a callback that reaches it | Re-acquiring a lock the thread already holds is a self-deadlock: the program stops on a line that reads like ordinary refactoring | `/rust-concurrency` |
| A committed `target-cpu` or `panic = "abort"` in a library profile | A `[profile.release]` section in a library crate setting either | The library ships code tuned to the machine it was built on, or aborts a panic the downstream binary meant to catch: a profile setting becomes a promise to every consumer | `/rust-performance` |

## Smells that come from generated code

These are review lenses, not topics, which is why they live here rather than
in a skill of their own:

| Smell | What it looks like | Where it is settled |
|---|---|---|
| A test asserting ground truth it computed itself | the expected value derived from the logic under test | `/rust-testing` |
| The same item public at two paths | a `pub use` added beside the original rather than replacing it, so both keep working | `/rust-api-design` |
| Design journal or compliance table in user-facing docs | "\| Rule \| Applied \|" tables, iteration narration, or rationale in item docs | `/rust-docs` |
| A port shaped like its source | `throw_if_null`-style helpers, a class hierarchy transliterated to traits, a striking structural similarity to the original | `/port-to-rust` |

They pass review because they look like diligence, and they are what an agent
produces when it optimises for resembling correct work.

## Smells that are not defects in every repo

The same line can be right in context: a `clone` in a startup path that runs
once, where the allocation is noise; `unwrap` in a test, or in a `main` that
documents its own preconditions; `pub` on a crate that is deliberately a
facade for its internals. The rule: when the context is genuinely ambiguous,
the reviewer asks rather than asserts, and the finding is filed as a note.

One named exception the baseline must not flag: a service type that holds an
`Arc<Inner>` and derives `Clone` clones one pointer, so every task gets its
own handle to the same thing. `/ownership-not-clone` names that handle clone
the rule passing, not a defect.
