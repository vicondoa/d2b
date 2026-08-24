//! Resource-backed exec connector and named-stream helpers.
//!
//! Attached execution is admitted through `EphemeralProcess` resources and
//! ComponentSession named streams. Production composition has no
//! feature-specific component-session connector.

use std::sync::Arc;
#[cfg(test)]
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_resource::v3::ResourceRef;

use crate::exec_session::{
    ComponentSessionExecClient, Established, ExecEstablishError, ExecGuestClient,
    ExecGuestConnector, ExecOpError, ExecSessionInfo, ExecStartSpec,
};
#[cfg(test)]
use crate::terminal_session::{
    OutputStreamSel, ReadOutputOutcome, TerminalBackend, WaitOutcome, WriteStdinOutcome,
};
/// The resource handle returned after an EphemeralProcess Create admission.
///
/// The handle contains only the resource identity and transport-neutral
/// stream metadata. Command data, credentials, paths, and process identities
/// remain owned by the Resource API and Process Provider.
#[derive(Clone, PartialEq, Eq)]
pub struct EphemeralProcessHandle {
    resource_ref: ResourceRef,
    stdout_offset: u64,
    stderr_offset: u64,
    control_sequence: u64,
}

impl std::fmt::Debug for EphemeralProcessHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EphemeralProcessHandle")
            .field("resource_ref", &"<redacted>")
            .field("stdout_offset", &self.stdout_offset)
            .field("stderr_offset", &self.stderr_offset)
            .field("control_sequence", &self.control_sequence)
            .finish()
    }
}

impl EphemeralProcessHandle {
    /// Construct a handle from an authenticated Resource API response.
    pub fn new(
        resource_ref: ResourceRef,
        stdout_offset: u64,
        stderr_offset: u64,
        control_sequence: u64,
    ) -> Result<Self, ExecEstablishError> {
        if resource_ref.resource_type().as_str() != "EphemeralProcess" {
            return Err(ExecEstablishError::Protocol);
        }
        Ok(Self {
            resource_ref,
            stdout_offset,
            stderr_offset,
            control_sequence,
        })
    }

    /// Borrow the created EphemeralProcess reference.
    pub const fn resource_ref(&self) -> &ResourceRef {
        &self.resource_ref
    }

    /// Return the initial stdout cursor.
    pub const fn stdout_offset(&self) -> u64 {
        self.stdout_offset
    }

    /// Return the initial stderr cursor.
    pub const fn stderr_offset(&self) -> u64 {
        self.stderr_offset
    }

    /// Return the initial in-stream control sequence.
    pub const fn control_sequence(&self) -> u64 {
        self.control_sequence
    }
}

/// Resource API seam used by the ComponentSession exec connector.
///
/// Implementations create an EphemeralProcess through the authenticated
/// Resource API, then open its admitted named stream. The connector has no
/// broker, socket, path, or direct child-process authority.
#[async_trait]
pub trait ProcessResourcePort: Send + Sync {
    /// Create one target-local EphemeralProcess resource.
    async fn create_ephemeral_process(
        &self,
        execution_ref: &ResourceRef,
        spec: &ExecStartSpec,
    ) -> Result<EphemeralProcessHandle, ExecEstablishError>;

    /// Attach the authenticated named stream for the created resource.
    async fn attach_process(
        &self,
        process: &EphemeralProcessHandle,
        tty: bool,
        initial_size: Option<(u32, u32)>,
    ) -> Result<Arc<dyn ExecGuestClient>, ExecEstablishError>;
}

/// Exec connector backed by Process/EphemeralProcess resources and a
/// ComponentSession named stream.
pub struct ResourceExecConnector<P> {
    port: P,
    execution_ref: ResourceRef,
}

impl<P> std::fmt::Debug for ResourceExecConnector<P> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ResourceExecConnector(<authenticated-resource-port>)")
    }
}

impl<P> ResourceExecConnector<P> {
    /// Bind the connector to one exact Host or Guest execution target.
    pub fn new(port: P, execution_ref: ResourceRef) -> Result<Self, ExecEstablishError> {
        if !matches!(execution_ref.resource_type().as_str(), "Host" | "Guest") {
            return Err(ExecEstablishError::Protocol);
        }
        Ok(Self {
            port,
            execution_ref,
        })
    }

    /// Borrow the target bound before Resource API admission.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }
}

#[async_trait]
impl<P> ExecGuestConnector for ResourceExecConnector<P>
where
    P: ProcessResourcePort,
{
    async fn establish(&self, spec: &ExecStartSpec) -> Result<Established, ExecEstablishError> {
        if spec.detached {
            return Err(ExecEstablishError::Capability);
        }
        let process = self
            .port
            .create_ephemeral_process(&self.execution_ref, spec)
            .await?;
        let client = self
            .port
            .attach_process(&process, spec.tty, spec.term_size)
            .await?;
        Ok(Established {
            client,
            info: ExecSessionInfo {
                tty: spec.tty,
                stdout_offset: process.stdout_offset(),
                stderr_offset: process.stderr_offset(),
            },
            control_seq: process.control_sequence(),
            caps: crate::exec_session::NegotiatedCaps {
                tty: spec.tty,
                signals: true,
                tty_resize: spec.tty,
                output: true,
            },
        })
    }
}

/// Open the standard Process named stream on an already authenticated
/// ComponentSession driver.
pub async fn open_component_session_process<D>(
    driver: D,
    stream_number: u16,
) -> Result<Arc<dyn ExecGuestClient>, ExecEstablishError>
where
    D: d2b_session::ComponentSessionDriver + 'static,
{
    let client = ComponentSessionExecClient::open(
        driver,
        stream_number,
        d2b_contracts_zone_session::v3::component_session::MAX_NAMED_STREAM_QUEUE_BYTES,
        d2b_contracts_zone_session::v3::component_session::MAX_NAMED_STREAM_QUEUE_BYTES,
    )
    .await
    .map_err(map_component_session_exec_error)?;
    Ok(Arc::new(client))
}

fn map_component_session_exec_error(error: ExecOpError) -> ExecEstablishError {
    match error {
        ExecOpError::Timeout => ExecEstablishError::Timeout,
        ExecOpError::Auth => ExecEstablishError::Auth,
        ExecOpError::StaleSession => ExecEstablishError::OldGeneration,
        ExecOpError::Transport => ExecEstablishError::Transport,
        ExecOpError::Protocol
        | ExecOpError::OldGeneration
        | ExecOpError::Capability
        | ExecOpError::DetachedUnavailable
        | ExecOpError::Guest(_) => ExecEstablishError::Protocol,
    }
}

// ===========================================================================
// Resource connector tests.
// ===========================================================================
#[cfg(test)]
mod tests {
    use super::*;

    struct StubResourceBackend;

    #[async_trait]
    impl TerminalBackend for StubResourceBackend {
        type Error = ExecOpError;

        async fn write_stdin(
            &self,
            offset: u64,
            data: Vec<u8>,
            eof: bool,
            _timeout: Duration,
        ) -> Result<WriteStdinOutcome, Self::Error> {
            Ok(WriteStdinOutcome {
                accepted_len: data.len() as u64,
                next_offset: offset + data.len() as u64,
                backpressured: false,
                stdin_closed: eof,
            })
        }

        async fn read_output(
            &self,
            _stream: OutputStreamSel,
            offset: u64,
            _max_len: u64,
            _wait: bool,
            _timeout_ms: u64,
            _timeout: Duration,
        ) -> Result<ReadOutputOutcome, Self::Error> {
            Ok(ReadOutputOutcome {
                data: Vec::new(),
                next_offset: offset,
                eof: true,
                dropped_bytes: 0,
                truncated: false,
                timed_out: false,
            })
        }

        async fn signal(
            &self,
            _control_seq: u64,
            _signo: u32,
            _timeout: Duration,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn resize(
            &self,
            _control_seq: u64,
            _rows: u32,
            _cols: u32,
            _timeout: Duration,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn wait(
            &self,
            _timeout_ms: u64,
            _timeout: Duration,
        ) -> Result<WaitOutcome, Self::Error> {
            Ok(WaitOutcome {
                running: true,
                terminal: None,
            })
        }

        async fn close_stdin(&self, _offset: u64, _timeout: Duration) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingResourcePort {
        created: std::sync::Mutex<Vec<ResourceRef>>,
    }

    #[async_trait]
    impl ProcessResourcePort for RecordingResourcePort {
        async fn create_ephemeral_process(
            &self,
            execution_ref: &ResourceRef,
            _spec: &ExecStartSpec,
        ) -> Result<EphemeralProcessHandle, ExecEstablishError> {
            self.created.lock().unwrap().push(execution_ref.clone());
            EphemeralProcessHandle::new(
                ResourceRef::parse("EphemeralProcess/run").unwrap(),
                3,
                4,
                5,
            )
        }

        async fn attach_process(
            &self,
            _process: &EphemeralProcessHandle,
            _tty: bool,
            _initial_size: Option<(u32, u32)>,
        ) -> Result<Arc<dyn ExecGuestClient>, ExecEstablishError> {
            Ok(Arc::new(StubResourceBackend))
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resource_exec_connector_creates_and_attaches_one_ephemeral_process() {
        let port = RecordingResourcePort::default();
        let connector =
            ResourceExecConnector::new(port, ResourceRef::parse("Guest/work").unwrap()).unwrap();
        let spec = ExecStartSpec {
            vm: "work".to_owned(),
            request_id: None,
            argv: vec!["true".to_owned()],
            tty: false,
            detached: false,
            env: Vec::new(),
            cwd: None,
            term_size: None,
        };
        let established = connector.establish(&spec).await.unwrap();
        assert_eq!(established.info.stdout_offset, 3);
        assert_eq!(established.info.stderr_offset, 4);
        assert_eq!(established.control_seq, 5);
        assert!(established.caps.output);
        assert!(!established.caps.tty);
    }

    #[test]
    fn resource_exec_connector_rejects_non_execution_targets() {
        assert_eq!(
            ResourceExecConnector::<RecordingResourcePort>::new(
                RecordingResourcePort::default(),
                ResourceRef::parse("Process/not-a-target").unwrap(),
            )
            .unwrap_err()
            .slug(),
            ExecEstablishError::Protocol.slug()
        );
    }

    #[test]
    fn ephemeral_process_handle_debug_redacts_resource_identity() {
        let handle = EphemeralProcessHandle::new(
            ResourceRef::parse("EphemeralProcess/secret-command").unwrap(),
            0,
            0,
            0,
        )
        .unwrap();
        let rendered = format!("{handle:?}");
        assert!(!rendered.contains("secret-command"));
        assert!(rendered.contains("resource_ref"));
    }
}
