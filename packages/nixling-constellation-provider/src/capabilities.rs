//! Typed capability descriptors (ADR 0032). Capabilities are structured
//! data, not comments. Each provider family advertises a descriptor;
//! callers route by required capability and fail closed when absent.

use nixling_constellation_core::{Capability, CapabilitySet};
use serde::{Deserialize, Serialize};

/// Capabilities advertised by a [`crate::RuntimeProvider`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCapabilitySet {
    /// The underlying capability set.
    pub caps: CapabilitySet,
}

/// Capabilities advertised by a [`crate::WorkloadProvider`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadCapabilitySet {
    /// The underlying capability set.
    pub caps: CapabilitySet,
}

/// Capabilities advertised by a [`crate::DisplayProvider`]. Display
/// providers advertise window-forwarding, display-streaming, clipboard,
/// audio, and input independently, plus a latency class and whether the
/// transport supports reconnect.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayCapabilitySet {
    /// The underlying capability set (window-forwarding/clipboard/audio/…).
    pub caps: CapabilitySet,
    /// Whether SHM buffer forwarding is supported.
    pub shm_buffers: bool,
    /// Whether dmabuf forwarding is available (false for ACA sandboxes).
    pub dmabuf: bool,
    /// Whether the display transport supports reconnect.
    pub reconnect: bool,
}

/// Capabilities advertised by a [`crate::NodeProvider`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeCapabilitySet {
    /// The underlying capability set.
    pub caps: CapabilitySet,
}

macro_rules! caps_accessor {
    ($t:ty) => {
        impl $t {
            /// True iff the capability is advertised.
            pub fn has(&self, cap: Capability) -> bool {
                self.caps.has(cap)
            }
        }
    };
}

caps_accessor!(RuntimeCapabilitySet);
caps_accessor!(WorkloadCapabilitySet);
caps_accessor!(DisplayCapabilitySet);
caps_accessor!(NodeCapabilitySet);
