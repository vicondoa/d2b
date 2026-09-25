# Naming

Names a Rust reader can guess: short type names, the cost prefixes, accessor
shape, acronym case, and where a function belongs.

## Short names, no weasel words

A name earns every word in it. `BookingService` says "booking" plus two words of
nothing: `Bookings` is the type, and `Service`, `Manager`, `Helper`, `Util`, `Data`,
`Info` are the words that mark a name nobody finished. The test: read the name at
the call site.

```rust,ignore
bookings.cancel(id)
booking_service.cancel(id)
```

The second line does the same job with dead weight in the receiver.

## The conversion cost tiers

The three prefixes are a promise about cost, and Rust readers rely on it: `as_` is
a free borrow-to-borrow view, `to_` is expensive and allocates or clones, `into_`
consumes and is usually cheap. A `to_` that is free is merely surprising; an `as_`
that allocates is a lie the reader will not check.

| prefix | promise | `std` example |
|--------|---------|---------------|
| `as_` | free, borrow-to-borrow view | `String::as_str` returns `&str` |
| `to_` | expensive; allocates or clones | `ToOwned::to_string` returns `String` |
| `into_` | consumes; usually cheap | `String::into_bytes` returns `Vec<u8>` |

The promise covers your own types too — a `to_foo` free function or inherent method
where a `From` impl belongs breaks it twice: callers bound against `Into<Foo>` can't
see it, and the cost it promises is never kept. The counter-example:

```rust
struct Log(Vec<String>);

impl Log {
    fn as_summary(&self) -> String {
        self.0.join(", ")
    }
}

fn main() {
    let log = Log(vec!["a".to_string(), "b".to_string()]);
    println!("{}", log.as_summary());
}
```

The `as_` prefix promises a free borrow; the body allocates a `String`.

## No `get_` prefix

The field accessor is named for the field: `fn name(&self)`, the mutable one
`fn name_mut(&mut self)`.

```rust,ignore
// before
impl User {
    fn get_name(&self) -> &str {
        &self.name
    }
}
```

```rust
struct User {
    name: String,
}

impl User {
    fn name(&self) -> &str {
        &self.name
    }

    fn name_mut(&mut self) -> &mut String {
        &mut self.name
    }
}

fn main() {
    let mut user = User { name: "Ada".to_string() };
    user.name_mut().push_str(" Lopez");
    println!("{}", user.name());
}
```

`get_` survives only where the operation genuinely looks something up and can fail,
as in `HashMap::get` — there is no field to name the accessor after, and the
`Option` return is the whole point of the call.

## Acronyms are words

`HttpClient` not `HTTPClient`, `Uuid` not `UUID`, `parse_url` not `parse_URL`.
Consistency here is what makes a name guessable, which is the only thing case
conventions buy:

```rust,ignore
// before
struct HTTPClient;

fn parse_URL(s: &str) -> Option<Url> {
    todo!()
}

// after
struct HttpClient;

fn parse_url(s: &str) -> Option<Url> {
    todo!()
}
```

## Regular functions over associated functions

A function that does not need `Self` and is not a constructor is a free function in
the module. Reaching for an `impl` block to namespace a helper produces
`Config::merge_maps(a, b)` where `merge_maps(a, b)` was available, and the
associated form cannot be imported by name:

```rust,ignore
// before — the `impl` block is a namespace, not a home
impl Config {
    fn merge_maps(base: &Map, extra: &Map) -> Map {
        // ...
    }
}

Config::merge_maps(&a, &b)
```

```rust
use std::collections::HashMap;

fn merge_maps(
    base: &HashMap<String, i64>,
    extra: &HashMap<String, i64>,
) -> HashMap<String, i64> {
    let mut out = base.clone();
    for (key, value) in extra {
        out.insert(key.clone(), *value);
    }
    out
}

fn main() {
    let mut base = HashMap::new();
    base.insert("a".to_string(), 1);
    let mut extra = HashMap::new();
    extra.insert("b".to_string(), 2);
    let merged = merge_maps(&base, &extra);
    println!("{}", merged.len());
}
```

The free form is what `use crate::config::merge_maps;` pulls into scope at a call
site; the associated form is only reachable through the type.

## Names that mirror the source language

`getUserById` transliterated to `get_user_by_id` is a Rust name only in its case
convention; a name that carries assumptions over from the source language is the
general shape of the problem the `/port-to-rust` skill covers.
