//! USBIP Process and EphemeralProcess declarations.

use core::fmt;

/// Source of a USBIP attachment request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachSource {
    /// A Device/Binding desired-state reconciliation.
    Declared,
    /// An authorized explicit attach request.
    Explicit,
}

/// One-shot USBIP operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EphemeralProcessKind {
    /// Bind the resolved bus ID to usbip-host.
    Bind,
    /// Unbind the resolved bus ID from usbip-host.
    Unbind,
}

/// Opaque one-shot Process intent. Core resolves its ticket to the actual
/// executable and bus ID only after the Device claim is admitted.
#[derive(Clone, PartialEq, Eq)]
pub struct EphemeralProcessIntent {
    kind: EphemeralProcessKind,
    source: AttachSource,
    ticket: [u8; 16],
}

impl EphemeralProcessIntent {
    /// Construct an opaque bind/unbind intent.
    pub const fn from_core(
        kind: EphemeralProcessKind,
        source: AttachSource,
        ticket: [u8; 16],
    ) -> Self {
        Self {
            kind,
            source,
            ticket,
        }
    }

    /// Return the one-shot operation.
    pub const fn kind(&self) -> EphemeralProcessKind {
        self.kind
    }

    /// Return whether the intent came from declared or explicit attach.
    pub const fn source(&self) -> AttachSource {
        self.source
    }
}

impl fmt::Debug for EphemeralProcessIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EphemeralProcessIntent")
            .field("kind", &self.kind)
            .field("source", &self.source)
            .finish()
    }
}

/// Long-lived per-Device USBIP daemon Process declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbipDaemonProcess {
    /// Signed component template selected by the Provider descriptor.
    pub template: &'static str,
    /// Host placement.
    pub placement: &'static str,
}

impl UsbipDaemonProcess {
    /// Return the fixed per-Device daemon declaration.
    pub const fn declaration() -> Self {
        Self {
            template: "usbip-daemon",
            placement: "host",
        }
    }
}
