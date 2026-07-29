//! Wire DTOs for the session-host ↔ window-frontend protocol (plan §10).
//!
//! Every message that concerns a buffer carries a full [`DownRef`] rather than a
//! bare id, so a signal always identifies exactly *which* reference it retires.
//! Every message carries a [`Generation`]; stale generations are dropped.

use serde::{Deserialize, Serialize};

use crate::model::{
    ids::{Generation, SurfaceId},
    ledger::DownRef,
};

/// Bumped on any incompatible change. Negotiated in `Hello`/`Welcome`.
pub const PROTO_VERSION: u32 = 1;

/// Hard ceiling on a single datagram. `SOCK_SEQPACKET` preserves boundaries, so
/// this bounds memory per message rather than per stream.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

/// Hard ceiling on attached descriptors, matching the dmabuf plane maximum.
pub const MAX_FRAME_FDS: usize = 4;

/// Why a buffer import failed. Kept closed so it cannot carry paths or argv.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportFailure {
    /// The `(format, modifier)` pair was not in the new generation's feedback.
    Unsupported,
    /// Planes disagreed on modifier and the host binding requires uniformity.
    MixedPlaneModifiers,
    /// `zwp_linux_buffer_params_v1.failed`.
    CompositorRejected,
    /// Local resource failure in the frontend.
    Resource,
}

/// Why the session left the attached state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DetachReason {
    /// Operator ran `d2b-wlattach detach`. The supported, exact path.
    UserRequested,
    /// The session is shutting down.
    SessionEnding,
}

/// Surface role, as reconstructed on the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Toplevel,
    Subsurface,
    Popup,
}

/// A buffer handed to the frontend. Pixel data never appears here — descriptors
/// travel out of band via `SCM_RIGHTS`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BufferRef {
    Shm {
        /// Byte offset of the image within the pool.
        offset: u32,
        width: i32,
        height: i32,
        stride: u32,
        format: u32,
        /// Total pool size, required to re-map on a fresh connection.
        pool_size: u64,
    },
    Dmabuf {
        width: i32,
        height: i32,
        format: u32,
        /// Per-plane, because clients bound below dmabuf v5 may differ.
        planes: smallvec::SmallVec<[PlaneRef; 4]>,
        /// `Y_INVERT` / `INTERLACED` / `BOTTOM_FIRST`. Dropping these
        /// reconstructs visibly wrong content.
        flags: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneRef {
    pub index: u32,
    pub offset: u32,
    pub stride: u32,
    pub modifier: u64,
}

impl BufferRef {
    /// How many descriptors this message must carry. Validated on receipt so a
    /// malformed peer cannot leak or exhaust descriptors.
    pub fn expected_fds(&self) -> usize {
        match self {
            BufferRef::Shm { .. } => 1,
            BufferRef::Dmabuf { planes, .. } => planes.len(),
        }
    }
}

/// Session host → window frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToFrontend {
    Welcome {
        proto: u32,
        generation: Generation,
    },
    CreateSurface {
        generation: Generation,
        surface: SurfaceId,
        role: Role,
        parent: Option<SurfaceId>,
    },
    SetToplevel {
        generation: Generation,
        surface: SurfaceId,
        title: String,
        app_id: String,
        min_size: (i32, i32),
        max_size: (i32, i32),
        maximized: bool,
        fullscreen: bool,
    },
    Commit {
        generation: Generation,
        r#ref: DownRef,
        buffer: Option<BufferRef>,
        buffer_scale: i32,
        damage_full: bool,
    },
    DestroySurface {
        generation: Generation,
        surface: SurfaceId,
    },
    Detach {
        generation: Generation,
        reason: DetachReason,
    },
}

/// Window frontend → session host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToHost {
    Hello {
        proto: u32,
    },
    OutputInfo {
        generation: Generation,
        width: i32,
        height: i32,
        scale: i32,
        refresh_mhz: u32,
    },
    /// The compositor asked this toplevel to close. Advisory: it is forwarded to
    /// the application, which decides. It does **not** end the session.
    CloseRequested {
        generation: Generation,
        surface: SurfaceId,
    },
    Configured {
        generation: Generation,
        surface: SurfaceId,
        width: i32,
        height: i32,
    },
    /// Host `wl_buffer` created. Not yet submitted.
    ImportCreated {
        r#ref: DownRef,
    },
    ImportFailed {
        r#ref: DownRef,
        reason: ImportFailure,
    },
    /// Sent *immediately before* `wl_surface.commit`, so the conservative state
    /// is always reached first.
    HostCommitted {
        r#ref: DownRef,
    },
    /// Created but destroyed without ever being committed.
    ImportAbandoned {
        r#ref: DownRef,
    },
    BufferReleased {
        r#ref: DownRef,
    },
    Presented {
        r#ref: DownRef,
        outputs: smallvec::SmallVec<[u32; 2]>,
    },
    PresentationDiscarded {
        r#ref: DownRef,
    },
    PresentationTimeout {
        r#ref: DownRef,
    },
    Bye,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::model::ids::BufferUseId;

    fn a_ref() -> DownRef {
        DownRef {
            generation: Generation(7),
            surface: SurfaceId(3),
            use_id: BufferUseId(11),
            seq: 42,
        }
    }

    #[test]
    fn messages_round_trip_through_postcard() {
        let msg = ToHost::HostCommitted { r#ref: a_ref() };
        let bytes = postcard::to_allocvec(&msg).unwrap();
        let back: ToHost = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn dmabuf_expected_fd_count_matches_plane_count() {
        let b = BufferRef::Dmabuf {
            width: 8,
            height: 8,
            format: 0x3432_4258,
            planes: smallvec::smallvec![
                PlaneRef {
                    index: 0,
                    offset: 0,
                    stride: 32,
                    modifier: 0
                },
                PlaneRef {
                    index: 1,
                    offset: 0,
                    stride: 32,
                    modifier: 0
                },
            ],
            flags: 0,
        };
        assert_eq!(b.expected_fds(), 2);
    }

    #[test]
    fn shm_expects_exactly_one_fd() {
        let b = BufferRef::Shm {
            offset: 0,
            width: 4,
            height: 4,
            stride: 16,
            format: 0,
            pool_size: 256,
        };
        assert_eq!(b.expected_fds(), 1);
    }

    /// Buffer references must survive the wire intact — losing `flags` or a
    /// per-plane modifier reconstructs visibly wrong content.
    #[test]
    fn buffer_ref_preserves_flags_and_per_plane_modifiers() {
        let b = BufferRef::Dmabuf {
            width: 1258,
            height: 1352,
            format: 0x3432_4258,
            planes: smallvec::smallvec![PlaneRef {
                index: 0,
                offset: 0,
                stride: 5056,
                modifier: 0x0300_0000_00e0_8014,
            }],
            flags: 0b1,
        };
        let bytes = postcard::to_allocvec(&b).unwrap();
        let back: BufferRef = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(b, back);
    }
}

/// Input travelling from the window frontend up to the session host.
///
/// Coordinates are in the frontend surface's local space. Serials are
/// deliberately absent: they are scoped to a compositor connection and must
/// never be replayed across generations, so the session host mints its own.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum InputEvent {
    PointerEnter {
        x: f64,
        y: f64,
    },
    PointerMotion {
        x: f64,
        y: f64,
    },
    PointerButton {
        button: u32,
        pressed: bool,
    },
    PointerAxis {
        horizontal: f64,
        vertical: f64,
    },
    PointerLeave,
    KeyboardEnter,
    /// `key` is the Wayland keycode (the evdev code), exactly as received.
    KeyboardKey {
        key: u32,
        pressed: bool,
    },
    KeyboardLeave,
}
