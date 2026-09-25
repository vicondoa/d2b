# Clone decisions

The reflex, the question to ask before copying, and the verdict. A clone is not
banned; an unexplained clone is.

| Situation | Reflex | Ask first | Verdict |
|---|---|---|---|
| Value must outlive the borrow it came from | `.clone()` | Can the borrow end earlier? | Clone if not — explainable |
| Function signature takes `String`, caller has `&str` | caller clones | Should the signature take `&str`? | Fix the signature |
| Passing shared read-only data to a spawned task | `.clone()` the data | Is `Arc<T>` enough? | `Arc::clone` — cheap and explainable |
| Two `&mut` borrows of different fields | clone one field | Can the struct be destructured? | Destructure — no clone |
| Loop body needs the collection it mutates | clone the collection | Can the reads be hoisted out? | Hoist |
| `Copy`-sized struct (a few words) | worry about it | Is it `Copy`? | Derive `Copy`, stop thinking about it |

## Signature-driven clone, removed

Before:

```rust,ignore
fn first_word(text: &str) -> &str { ... }

fn report(name: String) {
    println!("{:?}", first_word(&name));
}

// Every caller that holds a &str pays a copy to get here.
report("hello world".to_string());
```

After:

```rust,ignore
fn report(name: &str) {
    println!("{:?}", first_word(name));
}

// The caller copies nothing — the signature reads, so it borrows.
report("hello world");
```

`report` only reads its argument, so it takes `&str`. The clone disappears from
every call site at once, which is the tell that the copy was always the
signature fault, not the caller fault.

## Loop-carried clone, removed by hoisting

Before:

```rust
fn count_prefix(items: &Vec<String>, prefix: &str) -> usize {
    let mut count = 0;
    let n = items.len();
    for i in 0..n {
        // The whole vec is cloned each pass to keep `items` borrowable.
        let snapshot = items.clone();
        if snapshot[i].starts_with(prefix) {
            count += 1;
        }
    }
    count
}
```

After:

```rust
fn count_prefix(items: &[String], prefix: &str) -> usize {
    items.iter().filter(|item| item.starts_with(prefix)).count()
}
```

The read and the count are one pass over references; there is nothing to snapshot,
so nothing to clone. The signature taking `&[String]` rather than `&Vec<String>`
is the same cheapest-thing rule as the first pair.

## A borrow that spans the loop, the clone hoisted

Before:

```rust,ignore
// The borrow spans the whole loop, so the push conflicts.
let first = &items[0];
for _ in 0..n {
    items.push(first.clone()); // E0502: items is borrowed immutably by `first`
}
```

After:

```rust,ignore
// One clone, taken before the mutable borrow begins. Explainable: the value
// must outlive the loop that mutates the vector.
let first = items[0].clone();
for _ in 0..n {
    items.push(first.clone());
}
```

If the element is `Copy`, the read lifts out of the loop with no clone at all —
the borrow ends at the `let`.

## The `&mut self` field conflict, resolved by destructuring

```rust,ignore
// Rejected: self is mutably borrowed by the loop, so self.log is unreachable.
impl Server {
    fn handle(&mut self) {
        for conn in &mut self.connections {
            self.log.record(conn.id()); // E0502
        }
    }
}
```

The reflex is to clone `self.connections`; the answer is to destructure, so the
two fields borrow independently.

## A clone that stays — with the explanation

```rust,ignore
fn spawn_workers(jobs: Vec<Job>, workers: usize) -> Vec<JoinHandle<Worker>> {
    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        // Explainable: each worker owns its copy of the job list, and the
        // list must outlive the handle this scope returns.
        handles.push(tokio::spawn(worker(jobs.clone())));
    }
    handles
}
```

The clone survives the review because the comment states what it buys — an owned
value that outlives the borrow — in one sentence a reviewer can check against the
code. The skill is not "zero clones." It is "no unexplained clones."
