//! QMP media hotplug orchestration.

use crate::qmp::{QmpError, QmpSession, QmpTransport};

/// Hotplug operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotplugOperation {
    /// Attach a Volume.
    Attach,
    /// Detach a Volume.
    Detach,
}

/// Hotplug result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotplugResult {
    /// QMP accepted the transaction.
    Ready,
    /// The Guest must retry after a transient QMP failure.
    Degraded,
}

/// Typed hotplug controller over one negotiated QMP session.
pub struct HotplugController<T> {
    session: QmpSession<T>,
}

impl<T: QmpTransport> HotplugController<T> {
    /// Construct a hotplug controller.
    pub fn new(session: QmpSession<T>) -> Self {
        Self { session }
    }

    /// Attach one media fd slot.
    pub fn attach(
        &mut self,
        node_name: &str,
        fd_slot: i32,
        read_only: bool,
    ) -> Result<HotplugResult, QmpError> {
        self.session
            .attach_media(node_name, fd_slot, read_only)
            .map(|_| HotplugResult::Ready)
    }

    /// Detach one media node.
    pub fn detach(&mut self, node_name: &str) -> Result<HotplugResult, QmpError> {
        self.session
            .detach_media(node_name)
            .map(|_| HotplugResult::Ready)
    }

    /// Borrow the QMP command evidence.
    pub fn commands(&self) -> &[crate::qmp::QmpCommand] {
        self.session.commands()
    }

    /// Consume the controller and return its session.
    pub fn into_session(self) -> QmpSession<T> {
        self.session
    }
}
