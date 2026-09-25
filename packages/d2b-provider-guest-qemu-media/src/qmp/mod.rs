//! QMP capability negotiation and typed command dispatch.

use std::collections::VecDeque;

use d2b_contracts_resource::v3::BoundedToken;

/// A typed QMP command accepted by the Provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QmpCommand {
    /// Negotiate QMP capabilities.
    Capabilities,
    /// Add a block backend from an inherited fd slot.
    BlockdevAdd {
        /// QMP node name.
        node_name: String,
        /// Inherited LaunchTicket fd slot.
        fd_slot: i32,
        /// Read-only policy.
        read_only: bool,
    },
    /// Add a guest device backed by a block node.
    DeviceAdd {
        /// QEMU device id.
        device_id: String,
        /// Block node name.
        drive: String,
    },
    /// Remove a guest device.
    DeviceDel {
        /// QEMU device id.
        device_id: String,
    },
    /// Remove a block backend.
    BlockdevDel {
        /// QMP node name.
        node_name: String,
    },
}

/// QMP greeting received from the runner Endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QmpGreeting {
    /// QEMU version string.
    pub version: String,
}

/// QMP guest status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QmpVmStatus {
    /// Guest is running.
    Running,
    /// Guest is paused.
    Paused,
    /// Guest has stopped.
    Stopped,
}

/// Typed QMP command response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QmpReply {
    /// Command succeeded without a status payload.
    Ok,
}

impl QmpReply {
    /// Construct an empty successful response.
    pub const fn ok() -> Self {
        Self::Ok
    }
}

/// Stable QMP failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QmpError {
    /// Greeting was not received.
    GreetingTimeout,
    /// Greeting was malformed.
    GreetingInvalid,
    /// Capability negotiation failed.
    CapabilitiesFailed,
    /// Command was attempted before negotiation.
    NotReady,
    /// QMP returned an error.
    CommandFailed,
    /// A command or health probe timed out.
    Timeout,
    /// A response did not match the expected typed payload.
    Protocol,
    /// An inherited fd slot was invalid.
    InvalidFdSlot,
    /// A QMP object identifier was invalid.
    InvalidObjectId,
}

impl QmpError {
    /// Return the stable Provider error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::GreetingTimeout => "qmp-greeting-timeout",
            Self::GreetingInvalid => "qmp-greeting-invalid",
            Self::CapabilitiesFailed => "qmp-capabilities-failed",
            Self::NotReady => "qmp-not-ready",
            Self::CommandFailed => "qmp-command-failed",
            Self::Timeout => "qmp-command-timeout",
            Self::Protocol => "qmp-protocol-invalid",
            Self::InvalidFdSlot => "qmp-fd-slot-invalid",
            Self::InvalidObjectId => "qmp-object-id-invalid",
        }
    }
}

impl core::fmt::Display for QmpError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for QmpError {}

/// Minimal transport seam. Endpoint resolution and fd ownership stay outside
/// the Provider and are represented by this injected typed boundary.
pub trait QmpTransport {
    /// Receive the initial greeting.
    fn receive_greeting(&mut self) -> Result<QmpGreeting, QmpError>;
    /// Execute one typed command.
    fn execute(&mut self, command: &QmpCommand) -> Result<QmpReply, QmpError>;
}

/// Scripted transport used by hermetic tests and fake integration fixtures.
#[derive(Debug, Clone)]
pub struct ScriptedQmpTransport {
    greeting: Option<QmpGreeting>,
    replies: VecDeque<Result<QmpReply, QmpError>>,
}

impl ScriptedQmpTransport {
    /// Construct an empty scripted transport.
    pub fn new() -> Self {
        Self {
            greeting: None,
            replies: VecDeque::new(),
        }
    }

    /// Set the greeting version.
    pub fn with_greeting(mut self, version: impl Into<String>) -> Self {
        self.greeting = Some(QmpGreeting {
            version: version.into(),
        });
        self
    }

    /// Append a successful response.
    pub fn with_reply(mut self, reply: QmpReply) -> Self {
        self.replies.push_back(Ok(reply));
        self
    }

    /// Append a failed response.
    pub fn with_error(mut self, error: QmpError) -> Self {
        self.replies.push_back(Err(error));
        self
    }
}

impl Default for ScriptedQmpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl QmpTransport for ScriptedQmpTransport {
    fn receive_greeting(&mut self) -> Result<QmpGreeting, QmpError> {
        self.greeting.take().ok_or(QmpError::GreetingTimeout)
    }

    fn execute(&mut self, _command: &QmpCommand) -> Result<QmpReply, QmpError> {
        self.replies.pop_front().unwrap_or(Err(QmpError::Timeout))
    }
}

/// A negotiated QMP session.
#[derive(Debug)]
pub struct QmpSession<T> {
    transport: T,
    negotiated: bool,
    commands: VecDeque<QmpCommand>,
}

impl<T: QmpTransport> QmpSession<T> {
    /// Construct a session over an authenticated Endpoint transport.
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            negotiated: false,
            commands: VecDeque::new(),
        }
    }

    /// Negotiate QMP capabilities.
    ///
    /// # Errors
    ///
    /// Returns [`QmpError::GreetingInvalid`] when the greeting version is
    /// empty or overlong, and the transport and command errors when the
    /// greeting or capabilities exchange fails.
    pub fn negotiate(&mut self) -> Result<(), QmpError> {
        self.negotiated = false;
        let greeting = self.transport.receive_greeting()?;
        if greeting.version.is_empty() || greeting.version.len() > 64 {
            return Err(QmpError::GreetingInvalid);
        }
        self.execute(QmpCommand::Capabilities)?;
        self.negotiated = true;
        Ok(())
    }

    /// Attach one media fd slot through the QMP block/device sequence.
    pub fn attach_media(
        &mut self,
        node_name: &str,
        fd_slot: i32,
        read_only: bool,
    ) -> Result<(), QmpError> {
        validate_object_id(node_name)?;
        if fd_slot < 3 {
            return Err(QmpError::InvalidFdSlot);
        }
        self.execute(QmpCommand::BlockdevAdd {
            node_name: node_name.to_owned(),
            fd_slot,
            read_only,
        })?;
        if let Err(error) = self.execute(QmpCommand::DeviceAdd {
            device_id: node_name.to_owned(),
            drive: node_name.to_owned(),
        }) {
            if let Err(rollback_error) = self.execute(QmpCommand::BlockdevDel {
                node_name: node_name.to_owned(),
            }) {
                tracing::warn!(
                    node = %node_name,
                    provider = "runtime-qemu-media",
                    "blockdev rollback failed after DeviceAdd failure; orphaned block node possible"
                );
                let _ = rollback_error;
            }
            return Err(error);
        }
        Ok(())
    }

    /// Detach one media node through the reverse QMP sequence.
    pub fn detach_media(&mut self, node_name: &str) -> Result<(), QmpError> {
        validate_object_id(node_name)?;
        self.execute(QmpCommand::DeviceDel {
            device_id: node_name.to_owned(),
        })?;
        self.execute(QmpCommand::BlockdevDel {
            node_name: node_name.to_owned(),
        })?;
        Ok(())
    }

    /// Borrow dispatched commands.
    pub fn commands(&self) -> impl Iterator<Item = &QmpCommand> {
        self.commands.iter()
    }

    fn execute(&mut self, command: QmpCommand) -> Result<QmpReply, QmpError> {
        if !matches!(command, QmpCommand::Capabilities) && !self.negotiated {
            return Err(QmpError::NotReady);
        }
        let Self {
            transport,
            commands,
            ..
        } = self;
        commands.push_back(command);
        if commands.len() > 128 {
            commands.pop_front();
        }
        transport.execute(commands.back().expect("command just pushed"))
    }
}

fn validate_object_id(value: &str) -> Result<(), QmpError> {
    if BoundedToken::parse(value).is_err() {
        Err(QmpError::InvalidObjectId)
    } else {
        Ok(())
    }
}
