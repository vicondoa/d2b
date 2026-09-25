# Allocation

The depth behind `Allocation is usually the answer` in `SKILL.md`: buffer
reuse, the buffer-passing shapes, and the owned-string forms. The complete
examples compile as written; every fragment that refers to a symbol not
defined in the block is fenced `rust,ignore`.

## Buffer reuse across iterations

Declare the buffer outside the loop, clear it inside. The allocation happens
once and grows to the longest item, instead of a heap allocation on every pass:

```rust
fn report(lines: &[&str]) {
    let mut buf = String::new();
    for line in lines {
        buf.clear();
        buf.push_str(line);
        eprintln!("{buf}");
    }
}
```

The allocate-per-iteration shape — a fresh `String::new()` inside the loop —
pays at least one allocation per non-empty item, and the allocator traffic is
exactly what the profile names in this category.

## Passing the buffer instead of returning a fresh one

The function-level version of the same move: the caller owns the buffer and
its capacity, and the function writes into it. The caller clears between uses,
so the function appends:

```rust
use std::fmt::Write;

fn describe(stats: &[u64], into: &mut String) {
    for (i, s) in stats.iter().enumerate() {
        if i > 0 {
            into.push(',');
        }
        // write! formats into the existing buffer — no fresh String per element
        let _ = write!(into, "{s}");
    }
}
```

`write!` into a `&mut String` is the sanctioned alternative to `format!` in a
tight loop: the formatting happens into the reused buffer, and the `Result` is
unfailable for a `String`, so discarding it is correct, not a swallowed error.

## Stack-versus-heap: the SmallVec shape

When the element count is small, bounded, and known at compile time, a stack
slot first beats a heap allocation per call — the pattern the `smallvec` and
`arrayvec` crates implement, though the shape matters more than the crate:

```rust,ignore
// at most 4 IDs in practice: a stack slot, spilling to the heap only past 4
let mut ids: smallvec::SmallVec<[u32; 4]> = smallvec::SmallVec::new();
```

Reach for it only where the allocation was measured: the fixed capacity is a
commitment, and a bound that is wrong by one element pays the heap path on
every call.

## Arena allocation

Many values created together and dropped together: one slab, a bump offset per
value, and one `drop` instead of N. The cost is that the values share a
lifetime — none can outlive the slab — and the cast from raw slab bytes to
`&mut T` is unsafe, so a real arena is built and justified under
`/unsafe-rust`, not here:

```rust,ignore
struct Arena {
    slab: Box<[u8]>,
    offset: usize,
}

impl Arena {
    fn alloc<T: Copy>(&mut self, value: T) -> &mut T {
        // bump the offset by the size and alignment of T, copy the value in,
        // and cast the slab at the offset — the cast needs an unsafe block
    }
}
```

## Shared strings: `Rc<str>` and `Arc<str>`

A shared immutable string is `Rc<str>` or `Arc<str>`, not `Rc<String>` or
`Arc<String>`. The wrapped form is a double allocation: one for the smart
pointer and its refcount, one for the bytes it points at. The direct form
stores the bytes in the pointer allocation, so cloning the handle is a
refcount bump and nothing else:

```rust
use std::sync::Arc;

fn shared(borrowed: &str, owned: String) -> (Arc<str>, Arc<str>) {
    (Arc::from(borrowed), Arc::from(&*owned))
}
```

## `shrink_to_fit` after build-then-hold

A buffer built large and held small keeps the large capacity for the whole
remaining life of the value. Once the build phase ends and the value is held,
`shrink_to_fit` asks the allocator for the surplus back — a hint, so the
allocator may or may not honour it, and it is not a per-iteration move:

```rust
fn trimmed() -> Vec<u8> {
    let mut v = vec![0u8; 1_000_000];
    v.truncate(128);
    v.shrink_to_fit();
    v
}
```
