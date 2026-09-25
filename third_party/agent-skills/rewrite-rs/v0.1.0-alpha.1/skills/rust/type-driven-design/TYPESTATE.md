# Typestate

A worked example, the builder variant, and the costs — the depth for the
typestate section of `SKILL.md`.

## Connection as a state machine

The stage is a type parameter, the transition consumes `self`, and a method
available in one state does not exist in another:

```rust
use std::marker::PhantomData;

struct Disconnected;
struct Connected;

struct Connection<State> {
    addr: String,
    _state: PhantomData<State>,
}

impl Connection<Disconnected> {
    fn connect(self) -> Connection<Connected> {
        // establish the socket, then move on to the next state
        Connection { addr: self.addr, _state: PhantomData }
    }
}

impl Connection<Connected> {
    fn send(self, payload: &[u8]) -> Self {
        // write the payload
        self
    }

    fn close(self) -> Connection<Disconnected> {
        Connection { addr: self.addr, _state: PhantomData }
    }
}

fn main() {
    let conn = Connection { addr: "localhost:8080".into(), _state: PhantomData };
    // conn.send(b"hello");
    // ^ error[E0599]: no method named `send` found for struct
    //   `Connection<Disconnected>` in the current scope
    // `send` exists only on Connection<Connected> — the state that has it
    // was never constructed, so the mistake cannot compile.
    let conn = conn.connect();
    conn.send(b"hello");
}
```

The compile error a too-early caller gets is quoted above on purpose: it names
the state, not the intent, which is the recurring cost of the technique (see
below).

## The builder variant

The same trick tracks required fields, so `build()` only exists once all of them
are set:

```rust
use std::marker::PhantomData;

struct Missing;
struct Complete;

struct User {
    name: String,
    age: u32,
}

struct UserBuilder<Stage> {
    name: String,
    age: u32,
    _stage: PhantomData<Stage>,
}

impl UserBuilder<Missing> {
    fn new(name: &str) -> Self {
        UserBuilder { name: name.into(), age: 0, _stage: PhantomData }
    }

    fn age(mut self, age: u32) -> UserBuilder<Complete> {
        self.age = age;
        UserBuilder { name: self.name, age: self.age, _stage: PhantomData }
    }
}

impl UserBuilder<Complete> {
    fn build(self) -> User {
        User { name: self.name, age: self.age }
    }
}
```

`UserBuilder::new("x").build()` is a compile error until `age` is set. The
alternative — a runtime-checked builder that returns
`Result<User, MissingField>` — keeps the failure at runtime, and so does every
test that forgets a field.

State the trade plainly: when the field set is large, mostly optional, and
assembled dynamically (config files, request builders), the runtime version is
the better trade. A typestate builder with ten type parameters is the
over-engineering case `SKILL.md` warns about — the runtime `Result` builder
costs less and reads better at that size.

## The costs

- **Error messages.** `E0599: no method named send found for
  Connection<Disconnected>` is precise to a Rust reader and opaque to one who
  has not met typestate. The message names the state, not the intent.
- **Storage.** Heterogeneous states cannot share a `Vec`: there is no single
  `Connection<_>` that holds both, unless the states are erased behind an enum
  or a trait object — which gives back part of what the type parameter took
  away.
- **Public API.** The state parameter leaks into every caller signature. A
  library that hands out `Connection<Disconnected>` has committed its users to
  the type, and adding a state later is a change to review with care — the
  semver treatment is `/rust-api-design`.

## The decision list

- Use typestate when out-of-order use is a real bug class *and* the state count
  is small and fixed — a connection lifecycle, a two-step protocol, a builder
  with three required fields.
- Use a runtime state enum with a `Result` when the states are many, dynamic, or
  stored together in a collection.
- Use plain methods with documented order when the cost of a mistake is a
  recoverable `Err`, not corruption.
