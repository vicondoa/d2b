# Error types

The depth for `SKILL.md`: the public struct error in full, the closed enum and
its variant shapes, context and message style, the `anyhow` side, the boundary
between the two, and what `thiserror` eliminates from hand-rolled code.

## The public struct error, in full

The published-library answer to the boundary question, compiling with
`thiserror` as the only dependency:

```rust
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
#[error("{kind}")]
pub struct ConfigError {
    #[source]
    kind: ErrorKind,
}

#[derive(Debug, thiserror::Error)]
enum ErrorKind {
    #[error("cannot read {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("malformed config in {path}")]
    Parse {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl ConfigError {
    pub fn is_not_found(&self) -> bool {
        matches!(&self.kind, ErrorKind::Read { source, .. } if source.kind() == io::ErrorKind::NotFound)
    }

    pub fn path(&self) -> &Path {
        match &self.kind {
            ErrorKind::Read { path, .. } | ErrorKind::Parse { path, .. } => path,
        }
    }
}
```

The kind is private because the privacy is the whole mechanism: with a private
kind, new variants and dependency swaps are internal changes, not breaking
ones.

Choose the predicates from the questions callers would otherwise answer by
matching — one per question. If no caller can name a question, the error needs
no predicates and probably no struct.

A caller then writes `if err.is_not_found() { /* create the default */ }`
instead of matching a variant and inspecting an `io::ErrorKind` the crate
already knew about.

## The enum, for the closed case

For the closed set of failures — an internal crate, an error the caller only
ever prints — the enum is the cheaper, better answer. Every variant shape, with
when it is the right one:

```rust,ignore
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// The upstream error is the whole story: #[from] generates the From impl,
    /// so ? converts it at the call site.
    #[error("upstream request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// This layer knows facts the source does not carry. #[source] names the
    /// underlying error so error chains reach it.
    #[error("rate limited until {retry_after}")]
    RateLimited {
        retry_after: DateTime<Utc>,
        #[source]
        source: reqwest::Error,
    },

    /// Pure pass-through: there is no context of this layer to add, so the
    /// inner error prints as itself.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
```

A tuple variant is right when the inner type is the whole story. A struct
variant is right when the error needs its own facts alongside the source —
`retry_after` is known only to this layer, so it lives here. `transparent` is
right only while no context of your own exists; the day one appears, the variant
was never a pass-through and the `#[error(...)]` message gets the layer fact.

`?` converts through `From`, which is why a `#[from]` on an enum variant removes
a `map_err` closure at every call site. `Config` below is a `serde` type so the
example is self-contained:

```rust
#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(#[from] toml::de::Error),
}

#[derive(serde::Deserialize)]
struct Config {
    name: String,
}

// No map_err closures — ? converts through the From impls.
fn load(path: &str) -> Result<Config, AppError> {
    let text = std::fs::read_to_string(path)?;
    let config = toml::from_str(&text)?;
    Ok(config)
}
```

## Context and message style

Each layer adds what it uniquely knows — which path, which config key, which
request ID — and adds it once. The two failure modes to reject in review: no
context at all, where the error arrives as a bare OS message —
`No such file or directory (os error 2)` names the syscall, not the mistake —
and the same fact restated at four layers, where the message ends up as
`opening config: opening config: failed to read /etc/app/config.toml`.

Error message style: lowercase, no trailing period, no `error:` prefix — the
message is composed into someone else's sentence, and `Display` output that
starts with a capital and ends in a period reads as broken when it is.

## Panic and expect messages

An `expect` message describes the invariant that was violated, not the
operation that failed — `expect("config path is set by the caller")`, not
`expect("failed to get config")`. The second reads like a log line; the first
tells the next debugger exactly which assumption broke. A panic message carries
the actual values: `assert_eq!(len, cap)` over `assert!(len == cap)`, and an
`expect` that names the invariant and the value that broke it.

## The `anyhow` side

`.context("...")` is the common form. Use the closure form,
`.with_context(|| ...)`, when building the message allocates — a format string
with a path in it — because the closure defers the allocation to the failure
path:

```rust,ignore
let data = std::fs::read_to_string(path)
    .with_context(|| format!("reading config file {}", path.display()))?;
```

`anyhow!` builds an ad-hoc error with no source, for the failure that is its own
explanation: `anyhow!("no worker claimed job {id} within the deadline")`. And for
the rare case where a binary must inspect a concrete error type, `downcast_ref`
reaches back through the box:

```rust,ignore
fn report(err: &anyhow::Error) -> ExitCode {
    if err.downcast_ref::<ConfigError>()
        .is_some_and(|e| e.is_not_found())
    {
        eprintln!("hint: run `app init` to create the config file");
    }
    ExitCode::FAILURE
}
```

`downcast_ref` is the escape hatch that keeps `anyhow` honest at the top of a
binary: match what you can name, print the rest as a chain.

## Converting at the boundary

A `thiserror` type flows into `anyhow` for free — it implements
`std::error::Error + Send + Sync + 'static`, which is all `anyhow` asks for:

```rust,ignore
fn main() -> anyhow::Result<()> {
    let config = config::load()?; // ConfigError becomes anyhow::Error silently
    run(config)
}
```

The reverse throws the API away: an `anyhow::Error` inside a library enum is an
opaque box. Callers can print it but cannot match it, so the library has handed
back exactly the "no such file or directory" problem it exists to fix. A
published library never puts `anyhow::Error` in a public type.

## Hand-rolled, before and after

Before:

```rust
use std::fmt;

#[derive(Debug)]
pub struct ParseError {
    inner: toml::de::Error,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "parse error: {}", self.inner)
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.inner)
    }
}

impl From<toml::de::Error> for ParseError {
    fn from(inner: toml::de::Error) -> Self {
        ParseError { inner }
    }
}
```

After:

```rust
#[derive(Debug, thiserror::Error)]
#[error("parse error: {0}")]
pub struct ParseError(#[from] toml::de::Error);
```

What the derive eliminated: the `Display` impl, the `Error` impl (with the
`source()` plumbing), and the `From` impl — the three blocks that exist only so
that `?` and `{}` keep working, and that silently drift the day the inner type
changes.

## `Display` and the source chain

Each `Display` impl prints only its own layer; the full picture comes from
walking `source()`. The `{:#}` formatter does that walk for any
`std::error::Error`, and an `anyhow::Error` printed plainly already shows its
context chain. That is why `RateLimited` above carries its own message and
delegates the upstream detail to `#[source]`: the combined print gives
"rate limited until 2026-08-15T12:00:00Z" followed by the HTTP error — not the
HTTP error repeated twice.

## `Box<dyn Error>` — the third option

For an internal binary that never matches on error types, wants one boxed error
type at the top, and has no `anyhow` in the dependency budget,
`Box<dyn std::error::Error + Send + Sync>` is genuinely enough. It is the same
dynamic-dispatch idea as `anyhow` minus the context API: there is no
`.context()`, no formatted source chain, and no way to attach a fact at the
failure point other than building a small wrapper type by hand. The moment a
caller needs to say *which* thing failed, the box is not enough — that is when
`thiserror` or `anyhow` earns its place.
