//! Opaque Network attachment routing and fd lifetime evidence.

use d2b_contracts_resource::v3::ResourceRef;

/// One connected tap attachment owned by Core until child handoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TapAttachment {
    /// Authorizing Network reference.
    pub network_ref: ResourceRef,
    /// Whether the parent copy still has close-on-exec set.
    pub cloexec: bool,
    /// Whether the parent copy is closed.
    pub closed: bool,
}

/// Network routing event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkLaunchEvent {
    /// Create the persistent tap realization.
    CreatePersistentTap,
    /// Apply bridge isolation flags.
    SetBridgePortFlags,
    /// Transfer the child fd slot.
    AttachToLaunchTicket,
    /// Close all parent fd copies.
    CloseFdCopies,
    /// Remove the generation-fenced tap realization.
    DeletePersistentTap,
}

/// Network routing failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkLaunchError {
    /// Network authorization or resolution failed.
    Unavailable,
    /// A launch was attempted after the attachment was closed.
    Closed,
}

/// Core-owned Network attachment router.
#[derive(Debug, Clone)]
pub struct TapLaunchRouter {
    attachment: TapAttachment,
    events: Vec<NetworkLaunchEvent>,
}

impl TapLaunchRouter {
    /// Prepare a tap attachment in the canonical effect order.
    pub fn prepare(network_ref: ResourceRef) -> Result<Self, NetworkLaunchError> {
        if network_ref.resource_type().as_str() != "Network" {
            return Err(NetworkLaunchError::Unavailable);
        }
        Ok(Self {
            attachment: TapAttachment {
                network_ref,
                cloexec: true,
                closed: false,
            },
            events: vec![
                NetworkLaunchEvent::CreatePersistentTap,
                NetworkLaunchEvent::SetBridgePortFlags,
            ],
        })
    }

    /// Attach the child slot immediately before spawn.
    pub fn attach_to_launch_ticket(&mut self) -> Result<(), NetworkLaunchError> {
        if self.attachment.closed {
            return Err(NetworkLaunchError::Closed);
        }
        self.attachment.cloexec = false;
        self.events.push(NetworkLaunchEvent::AttachToLaunchTicket);
        Ok(())
    }

    /// Record successful child handoff and restore the parent CLOEXEC posture.
    pub fn child_handoff_complete(&mut self) {
        self.attachment.cloexec = true;
    }

    /// Close copies before deleting the tap on a failed launch.
    pub fn fail_launch(&mut self) {
        self.attachment.closed = true;
        self.attachment.cloexec = true;
        self.events.push(NetworkLaunchEvent::CloseFdCopies);
        self.events.push(NetworkLaunchEvent::DeletePersistentTap);
    }

    /// Close copies before normal generation-fenced cleanup.
    pub fn finish(&mut self) {
        self.attachment.closed = true;
        self.attachment.cloexec = true;
        self.events.push(NetworkLaunchEvent::CloseFdCopies);
        self.events.push(NetworkLaunchEvent::DeletePersistentTap);
    }

    /// Borrow the attachment evidence.
    pub const fn attachment(&self) -> &TapAttachment {
        &self.attachment
    }

    /// Borrow the ordered effect evidence.
    pub fn events(&self) -> &[NetworkLaunchEvent] {
        &self.events
    }
}
