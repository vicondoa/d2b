//! Stable identities.
//!
//! Three levels of buffer identity are required; conflating any two of them was
//! a defect in earlier designs:
//!
//! * [`BackingId`] - the underlying storage (dmabuf planes / shm pool).
//!   Shareable across several `wl_buffer` objects.
//! * [`AppBufferId`] - the application's `wl_buffer` object. Clients routinely
//!   **reuse** these, so it cannot own "has been released".
//! * [`BufferUseId`] - one attach-to-release *epoch*. Release is owed once per
//!   epoch, not once per object.

use serde::{Deserialize, Serialize};

macro_rules! opaque_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        pub struct $name(pub u64);

        impl $name {
            pub const fn get(self) -> u64 {
                self.0
            }
        }
    };
}

opaque_id!(
    /// Underlying storage shared by one or more `wl_buffer` objects.
    BackingId
);
opaque_id!(
    /// An application `wl_buffer` object. Reusable.
    AppBufferId
);
opaque_id!(
    /// One attach-to-release epoch.
    BufferUseId
);
opaque_id!(
    /// A surface in the shadow tree. Monotonic; never reused.
    SurfaceId
);
opaque_id!(
    /// Incremented on every attach. Everything generation-scoped dies on detach.
    Generation
);

/// Monotonic allocator for the opaque ids above.
///
/// Ids are never reused, so a stale reference can always be recognised rather
/// than silently aliasing a live object.
#[derive(Debug, Default)]
pub struct IdAllocator {
    next: u64,
}

impl IdAllocator {
    pub fn new() -> Self {
        Self { next: 1 }
    }

    fn bump(&mut self) -> u64 {
        // Start at 1 so 0 is never a valid id.
        if self.next == 0 {
            self.next = 1;
        }
        let v = self.next;
        self.next = self.next.saturating_add(1);
        v
    }

    pub fn backing(&mut self) -> BackingId {
        BackingId(self.bump())
    }
    pub fn app_buffer(&mut self) -> AppBufferId {
        AppBufferId(self.bump())
    }
    pub fn buffer_use(&mut self) -> BufferUseId {
        BufferUseId(self.bump())
    }
    pub fn surface(&mut self) -> SurfaceId {
        SurfaceId(self.bump())
    }
    pub fn generation(&mut self) -> Generation {
        Generation(self.bump())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_monotonic_and_never_zero() {
        let mut a = IdAllocator::new();
        let first = a.surface();
        let second = a.surface();
        assert_ne!(first.get(), 0);
        assert!(second.get() > first.get());
    }

    #[test]
    fn ids_are_never_reused_across_kinds() {
        let mut a = IdAllocator::new();
        let s = a.surface().get();
        let b = a.backing().get();
        let u = a.buffer_use().get();
        assert_ne!(s, b);
        assert_ne!(b, u);
    }
}
