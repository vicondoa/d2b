# Safety review

A checklist to apply to a diff line by line. Each entry names what to look for
and the question that settles it.

## UB is not a bad outcome at runtime

`unsafe` means undefined behaviour, not "dangerous", and the distinction is
load-bearing: it is the compiler being permitted to assume the situation
cannot arise — which is why UB shows up as an unrelated function miscompiling
six months later.

## Per `unsafe` block

- Is there a `// SAFETY:` comment?
- Does it state an invariant the caller upholds — not restate what the code
  does?
- Is the invariant actually true for *every* caller, not just the current one?
- Is the block as small as it can be? The check that makes the invariant true
  belongs in the safe code just outside.

## Assume the code you call misbehaves

Inside an `unsafe` block, assume the code you call misbehaves. A closure can
panic mid-operation and leave your structure half-updated — poison the state or
restore the invariant before the unwind escapes. A `Deref` impl can return a
different reference each call, a `Clone` can panic, a `Drop` can run at a
moment you did not choose. Any invariant you derive from a user-supplied trait
impl is an invariant you do not have.

## Per `unsafe fn`

- Is there a `# Safety` doc section stating what the caller must guarantee?
- Could this be a safe function with the check inside instead? If the only
  reason it is `unsafe` is that the caller "should know better," the invariant
  is not enforceable at the boundary, and the design is wrong.

## Per raw pointer

- Where did it come from — and is that origin still true at the point of use?
- Is the pointed-to memory still alive, correctly aligned, and non-null?
- Is any `&mut` derived from it aliased by any other live reference in scope?
- If it came from a `Vec`, is the `Vec` guaranteed not to reallocate between
  origin and use?

## Per `transmute`

- Are both types `#[repr(C)]` or `#[repr(transparent)]`, or otherwise guaranteed
  the same layout and size?
- Would `bytemuck` or `zerocopy` do the conversion safely?
- Is a plain pointer cast enough?

## Per FFI call

- Can the C side unwind into Rust? If a Rust callback can panic, wrap it so a
  panic cannot cross the boundary — `catch_unwind` at the callback, or a
  documented no-panic contract on the C side.
- Who owns the allocation, and which allocator frees it? Memory allocated on one
  side is freed on that side.
- Are string encodings and NUL termination handled — `CStr` to read, `CString`
  to write?
- Are the `#[repr(C)]` layouts confirmed on both sides of the boundary,
  including field order and padding assumptions?
- Are the edition 2024 marks in place — `unsafe extern { }` around extern
  declarations, `#[unsafe(no_mangle)]`, `#[unsafe(export_name = "...")]`?
  Those attributes always were unsafe assertions, and the edition now says so.

All three marks, on a minimal boundary:

```rust
unsafe extern "C" {
    fn strlen(s: *const u8) -> usize;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_add(a: u32, b: u32) -> u32 {
    a.wrapping_add(b)
}

#[unsafe(export_name = "add")]
pub unsafe extern "C" fn rust_add_named(a: u32, b: u32) -> u32 {
    a.wrapping_add(b)
}
```

## Per hand-written `Send`/`Sync` impl

- What makes it true, stated in a comment for a reviewer who did not write it?
  "All the fields are `Send`" is not an answer when one of them is a raw
  pointer.

## Sound and unsound, side by side

The same function in both forms — the difference is where the check lives.

Unsound: the safe signature lets a caller pass an out-of-range index, and the
pointer arithmetic trusts it:

```rust
/// Splits a slice at `mid`, returning two mutable references.
pub fn split_at_mut<T>(slice: &mut [T], mid: usize) -> (&mut [T], &mut [T]) {
    let ptr = slice.as_mut_ptr();
    // No check: a caller passing `mid > slice.len()` computes a pointer past
    // the end — and `slice.len() - mid` underflows. No current caller needs to
    // do that for the bug to exist: the signature allows it.
    unsafe {
        (std::slice::from_raw_parts_mut(ptr, mid),
         std::slice::from_raw_parts_mut(ptr.add(mid), slice.len() - mid))
    }
}
```

Sound: the check runs in the safe code before the pointer arithmetic, and the
`// SAFETY:` comment states only what is true *after* it:

```rust
pub fn split_at_mut<T>(slice: &mut [T], mid: usize) -> (&mut [T], &mut [T]) {
    assert!(mid <= slice.len(), "mid out of range: {mid} > {}", slice.len());
    let len = slice.len();
    let ptr = slice.as_mut_ptr();
    // SAFETY: `mid <= len` was checked above, so both ranges
    // `ptr..ptr+mid` and `ptr+mid..ptr+len` are in bounds, disjoint, and
    // cover the allocation; `slice` is the only live reference here.
    unsafe {
        (std::slice::from_raw_parts_mut(ptr, mid),
         std::slice::from_raw_parts_mut(ptr.add(mid), len - mid))
    }
}
```

The `// SAFETY:` comment in the second version is false in the first — that is
the whole test. If the comment only becomes true after a check, the check
belongs outside the block.
