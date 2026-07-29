//! The crate's **only** `unsafe` code.
//!
//! Quarantined here deliberately, following the `d2b-priv-broker` precedent of
//! `unsafe_code = "deny"` plus one audited module, so that every unsafe
//! expression in the prototype is reviewable in a single short file.
//!
//! The prototype's data path genuinely needs no `unsafe`: DMA-BUF descriptors
//! move by `SCM_RIGHTS` through `rustix`'s safe ancillary API, which was proved
//! out before implementation began. The one exception is reading `wl_shm` pixel
//! content: Smithay 0.7 exposes mapped buffer contents only as
//! `FnOnce(*const u8, usize, BufferData)`, with no safe slice accessor
//! (`smithay-0.7.0/src/wayland/shm/mod.rs:241`). Rather than reimplement
//! `wl_shm` pool mapping ourselves — which would need considerably *more*
//! unsafe — we borrow Smithay's mapping for the duration of its own callback.

#![allow(unsafe_code)]

/// Borrow a Smithay-mapped SHM buffer as a slice.
///
/// # Safety
///
/// Callers must only invoke this from inside a
/// [`smithay::wayland::shm::with_buffer_contents`] callback, passing that
/// callback's own `ptr` and `len` unmodified.
///
/// Within that callback Smithay guarantees:
///
/// * `ptr` is non-null and points to a live `mmap` of the client's pool,
/// * the mapping is readable for at least `len` bytes,
/// * the mapping outlives the callback (Smithay holds the pool alive), and
/// * SIGBUS from a client truncating the pool is trapped by Smithay's handler,
///   which turns the access into an error rather than a crash.
///
/// The returned slice must not outlive the callback. Callers immediately copy
/// out of it and never store it.
pub unsafe fn borrow_pool<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() || len == 0 {
        return &[];
    }
    // SAFETY: guaranteed by the caller contract above, which is discharged by
    // only ever calling this from within `with_buffer_contents`.
    unsafe { std::slice::from_raw_parts(ptr, len) }
}

use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::wayland::shm::with_buffer_contents;

use super::host::Snapshot;

/// Copy a committed `wl_shm` buffer's pixels into an owned snapshot.
///
/// This is the *entire* reason the crate needs `unsafe` at all, so the whole
/// operation lives here rather than only the pointer dereference — otherwise the
/// call site would need its own `unsafe` block and the quarantine would leak.
///
/// Returns `None` for a buffer that is not SHM-managed (a DMA-BUF takes the
/// descriptor pass-through path instead) or whose pool the client has
/// truncated.
///
/// Copying, rather than retaining the client's pool descriptor, is deliberate:
/// it lets the application's buffer be released immediately so its pool keeps
/// turning over while detached, and it removes any question of pool lifetime
/// across a frontend restart. The zero-copy guarantee is scoped to DMA-BUF.
pub fn copy_shm(buffer: &WlBuffer) -> Option<Snapshot> {
    with_buffer_contents(buffer, |ptr, len, data| {
        // SAFETY: we are inside `with_buffer_contents`, passing its own ptr and
        // len unmodified, and we copy out before the callback returns.
        let src = unsafe { borrow_pool(ptr, len) };

        // Never trust the client's geometry against the real mapping.
        let stride = data.stride.max(0) as usize;
        let height = data.height.max(0) as usize;
        let needed = stride.saturating_mul(height);
        let offset = data.offset.max(0) as usize;
        let end = offset.saturating_add(needed);
        if needed == 0 || end > src.len() {
            return None;
        }

        Some(Snapshot {
            width: data.width,
            height: data.height,
            stride: data.stride,
            format: data.format as u32,
            pixels: src[offset..end].to_vec(),
        })
    })
    .ok()
    .flatten()
}
