//! The crate's **only** `unsafe` code.
//!
//! Quarantined here deliberately, following the `d2b-priv-broker` precedent of
//! `unsafe_code = "deny"` plus one audited module, so every unsafe expression in
//! the prototype is reviewable in a single short file.
//!
//! The data path itself needs no `unsafe`: DMA-BUF descriptors move by
//! `SCM_RIGHTS` through `rustix`'s safe ancillary API. The one exception is
//! reading `wl_shm` pixel content, because Smithay 0.7 exposes mapped contents
//! only as `FnOnce(*const u8, usize, BufferData)` with no safe accessor
//! (`smithay-0.7.0/src/wayland/shm/mod.rs:241`). Reimplementing pool mapping
//! ourselves would require strictly *more* unsafe.

#![allow(unsafe_code)]

use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::wayland::shm::with_buffer_contents;

use super::host::Snapshot;

/// Refuse to retain more than this per snapshot (~64 MiB), so a client cannot
/// make us allocate unboundedly by declaring a huge pool.
const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;

/// Copy a committed `wl_shm` buffer's pixels into owned storage.
///
/// Returns `None` for a buffer that is not SHM-managed (DMA-BUF takes the
/// descriptor pass-through path), whose declared geometry does not fit the real
/// mapping, or which exceeds [`MAX_SNAPSHOT_BYTES`].
///
/// # Why this does not build a slice
///
/// The pool is **shared, client-writable memory**. Smithay documents that
/// constructing a Rust reference or slice over it is undefined behaviour: a
/// hostile or merely racy client may mutate the bytes concurrently, and Rust
/// references carry a no-concurrent-mutation guarantee the client is under no
/// obligation to honour. Bounds validation proves the address range is mapped;
/// it says nothing about immutability. So we never materialise a reference - we
/// copy with volatile reads straight into an owned `Vec`.
///
/// The residual, deliberately accepted: a client mutating the pool mid-copy can
/// produce a **torn** image, a mix of two frames. That is a visual artefact, not
/// a memory-safety violation, and it is what any compositor doing a software
/// copy is exposed to.
pub fn copy_shm(buffer: &WlBuffer) -> Option<Snapshot> {
    with_buffer_contents(buffer, |ptr, len, data| {
        // Checked arithmetic throughout: a client controls every input here.
        let stride: usize = usize::try_from(data.stride).ok()?;
        let height: usize = usize::try_from(data.height).ok()?;
        let offset: usize = usize::try_from(data.offset).ok()?;
        let needed = stride.checked_mul(height)?;
        let end = offset.checked_add(needed)?;

        if needed == 0 || needed > MAX_SNAPSHOT_BYTES || end > len || ptr.is_null() {
            return None;
        }

        let t0 = std::time::Instant::now();
        let mut pixels = vec![0u8; needed];
        // SAFETY: `offset <= end <= len`, so this stays inside the mapping.
        let base = unsafe { ptr.add(offset) };

        // Copy a machine word at a time where alignment allows. A byte-at-a-time
        // volatile loop over a multi-megabyte frame costs tens of milliseconds -
        // the difference between a usable window and a slideshow.
        const W: usize = std::mem::size_of::<usize>();
        let mut i = 0usize;

        // Byte-wise until the source is word-aligned.
        while i < needed && !(base as usize).wrapping_add(i).is_multiple_of(W) {
            // SAFETY: `ptr` is a live mapping of at least `len` bytes for the
            // duration of this callback (Smithay's contract, with SIGBUS from
            // client truncation trapped by its handler), and
            // `offset + i < end <= len` by the checks above. `read_volatile`
            // forms no reference, so a racing client tears pixels rather than
            // causing Rust UB.
            pixels[i] = unsafe { base.add(i).read_volatile() };
            i += 1;
        }
        // Word-wise through the aligned bulk.
        while i + W <= needed {
            // SAFETY: as above, and `base + i` is word-aligned here, so the
            // `usize` read is correctly aligned.
            let word = unsafe { base.add(i).cast::<usize>().read_volatile() };
            pixels[i..i + W].copy_from_slice(&word.to_ne_bytes());
            i += W;
        }
        // Byte-wise remainder.
        while i < needed {
            // SAFETY: as above.
            pixels[i] = unsafe { base.add(i).read_volatile() };
            i += 1;
        }

        log::debug!("copy_shm {needed} bytes in {:?}", t0.elapsed());
        Some(Snapshot {
            width: data.width,
            height: data.height,
            stride: data.stride,
            format: data.format as u32,
            pixels,
        })
    })
    .ok()
    .flatten()
}
