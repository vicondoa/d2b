# Boilerplate patterns

Before/after pairs for repetition that has a direct idiomatic replacement, plus one
case where the "idiomatic-looking" replacement is actually the wrong call.

## Manual `impl Default` on an enum

Before:

```rust
enum Mode {
    Interactive,
    Batch,
}

impl Default for Mode {
    fn default() -> Self {
        Mode::Interactive
    }
}
```

After:

```rust
#[derive(Default)]
enum Mode {
    #[default]
    Interactive,
    Batch,
}
```

The derive plus `#[default]` says the same thing in two lines instead of six, and it
can't drift out of sync with the variant list the way a hand-written `match`-free
`fn default()` silently can if a variant is renamed.

## Hand-rolled builder for a small update

Before:

```rust
struct Config {
    timeout_ms: u64,
    retries: u32,
    verbose: bool,
}

fn with_timeout(config: &Config, timeout_ms: u64) -> Config {
    Config {
        timeout_ms,
        retries: config.retries,
        verbose: config.verbose,
    }
}
```

After:

```rust,ignore
let updated = Config { timeout_ms: 5_000, ..config };
```

Struct update syntax (`..config`) says "everything else stays the same" directly,
instead of naming every unchanged field by hand. Reach for a real builder type only
when construction has multiple required steps, validation between them, or more
fields than fit legibly in one literal — struct update syntax is for the common case
of changing one or two fields on an otherwise-complete value.

## Repeated `match` on `Option`

Before:

```rust,ignore
let name = match user.nickname {
    Some(n) => n,
    None => user.legal_name.clone(),
};

let upper = match user.nickname {
    Some(ref n) => Some(n.to_uppercase()),
    None => None,
};
```

After:

```rust,ignore
let name = user.nickname.clone().unwrap_or_else(|| user.legal_name.clone());
let upper = user.nickname.as_ref().map(|n| n.to_uppercase());
```

`unwrap_or`, `unwrap_or_else`, `map`, and `and_then` cover the vast majority of
`Option`/`Result` handling. Reach for the explicit `match` when an arm needs to do
something the combinators can't express cleanly — early `return`, a `?`, or logging
distinct to one branch.

## Repeated error conversion

Before:

```rust
#[derive(serde::Deserialize)]
struct Config {
    timeout_ms: u64,
    retries: u32,
    verbose: bool,
}

enum AppError {
    Io(std::io::Error),
    Parse(toml::de::Error),
}

fn load(path: &str) -> Result<Config, AppError> {
    let text = std::fs::read_to_string(path).map_err(|e| AppError::Io(e))?;
    let config: Config = toml::from_str(&text).map_err(|e| AppError::Parse(e))?;
    Ok(config)
}
```

After:

```rust
#[derive(serde::Deserialize)]
struct Config {
    timeout_ms: u64,
    retries: u32,
    verbose: bool,
}

#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(#[from] toml::de::Error),
}

fn load(path: &str) -> Result<Config, AppError> {
    let text = std::fs::read_to_string(path)?;
    let config: Config = toml::from_str(&text)?;
    Ok(config)
}
```

`#[from]` generates the `From` impl that lets `?` convert the error automatically, so
every call site drops its `.map_err(...)` closure. This is expression-level
boilerplate removal; deciding how the error *type itself* should be shaped is the
a call for `/rust-errors` to make, not this skill.

## Forwarding methods on a wrapper type — the `Deref` trap

Before:

```rust,ignore
struct LoggingConnection {
    inner: Connection,
}

impl LoggingConnection {
    fn query(&self, sql: &str) -> Result<Rows, DbError> {
        self.inner.query(sql)
    }
    fn close(&self) {
        self.inner.close()
    }
    // ...ten more one-line forwards
}
```

It's tempting to delete all of these with:

```rust,ignore
impl std::ops::Deref for LoggingConnection {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.inner
    }
}
```

Don't. `Deref` is for smart pointers — types whose whole job is to transparently
stand in for the thing they contain, like `Box<T>` or `Rc<T>`. `LoggingConnection`
is not a pointer to a `Connection`; it's a distinct type that happens to hold one,
and its entire reason to exist is to add behavior (logging) on top. Implementing
`Deref` for inheritance-style forwarding lets `connection.query(...)` compile by
auto-deref, but it also silently exposes every other `Connection` method the wrapper
never intended to expose, and it makes `&LoggingConnection` coerce to `&Connection`
in places that quietly bypass the logging the wrapper exists to add. That defeats the
purpose of the wrapper without a compiler warning anywhere.

The idiomatic fix for genuine forwarding boilerplate is usually one of:

- Keep the explicit forwarding methods — ten one-line functions are not a real
  maintenance burden, and they're a place to add behavior (a log line, a metric)
  later without touching call sites.
- If the wrapper only needs a subset of the surface on the inner type, forward exactly
  that subset explicitly rather than the whole type by `Deref`.
- If the wrapper truly adds nothing and is pure indirection, delete the wrapper.

Reserve `Deref` for types that are actually smart pointers.
